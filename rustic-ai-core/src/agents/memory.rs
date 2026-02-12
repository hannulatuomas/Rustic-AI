use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

use crate::agents::context::{
    ContextDeduplicator, ConversationSummary, MessageImportance, MessageScorer,
};
use crate::config::schema::SummaryTriggerMode;
use crate::error::Result;
use crate::events::Event;
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};

const DEFAULT_SUMMARY_MAX_TOKENS: usize = 500;
const DEFAULT_SUMMARY_CACHE_ENTRIES: usize = 64;
const DEFAULT_MESSAGE_WINDOW_THRESHOLD: usize = 12;
const DEFAULT_TOKEN_THRESHOLD_PERCENT: f64 = 0.6;
const PROVIDER_SUMMARY_TIMEOUT_SECS: u64 = 15;

#[derive(Debug, Clone, Copy)]
pub struct AgentMemoryConfig {
    pub aggressive_summary_enabled: bool,
    pub trigger_mode: SummaryTriggerMode,
    pub message_window_threshold: Option<usize>,
    pub token_threshold_percent: Option<f64>,
    pub include_user_task: bool,
    pub include_completion_summary: bool,
    pub quality_tracking_enabled: bool,
}

impl Default for AgentMemoryConfig {
    fn default() -> Self {
        Self {
            aggressive_summary_enabled: false,
            trigger_mode: SummaryTriggerMode::Hybrid,
            message_window_threshold: Some(DEFAULT_MESSAGE_WINDOW_THRESHOLD),
            token_threshold_percent: Some(DEFAULT_TOKEN_THRESHOLD_PERCENT),
            include_user_task: true,
            include_completion_summary: true,
            quality_tracking_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SummaryQualityTracking {
    acceptance_count: u32,
    rejection_count: u32,
}

#[derive(Debug, Clone)]
pub struct SummarySignal {
    pub trigger: SummaryTriggerMode,
    pub message_count: usize,
    pub token_pressure: f64,
    pub summary_length: usize,
    pub summary_key: String,
    pub rating: i8,
    pub implicit: bool,
    pub acceptance_count: u32,
    pub has_user_task: bool,
    pub has_completion_summary: bool,
}

#[derive(Debug, Clone)]
pub struct AgentMemory {
    context_window_size: usize,
    summary_enabled: bool,
    aggressive_summary_enabled: bool,
    summary_max_tokens: usize,
    summary_cache_max_entries: usize,
    summary_cache: Arc<RwLock<SummaryCacheState>>,
    trigger_mode: SummaryTriggerMode,
    message_window_threshold: usize,
    token_threshold_percent: f64,
    include_user_task: bool,
    include_completion_summary: bool,
    quality_tracking_enabled: bool,
    quality_tracking: Arc<RwLock<HashMap<String, SummaryQualityTracking>>>,
    last_summary_signal: Arc<RwLock<Option<SummarySignal>>>,
}

impl AgentMemory {
    pub fn new(
        context_window_size: usize,
        summary_enabled: bool,
        summary_max_tokens: Option<usize>,
        summary_cache_max_entries: Option<usize>,
        config: AgentMemoryConfig,
    ) -> Self {
        Self {
            context_window_size,
            summary_enabled,
            aggressive_summary_enabled: config.aggressive_summary_enabled,
            summary_max_tokens: summary_max_tokens.unwrap_or(DEFAULT_SUMMARY_MAX_TOKENS),
            summary_cache_max_entries: summary_cache_max_entries
                .unwrap_or(DEFAULT_SUMMARY_CACHE_ENTRIES),
            summary_cache: Arc::new(RwLock::new(SummaryCacheState::default())),
            trigger_mode: config.trigger_mode,
            message_window_threshold: config
                .message_window_threshold
                .unwrap_or(DEFAULT_MESSAGE_WINDOW_THRESHOLD),
            token_threshold_percent: config
                .token_threshold_percent
                .unwrap_or(DEFAULT_TOKEN_THRESHOLD_PERCENT),
            include_user_task: config.include_user_task,
            include_completion_summary: config.include_completion_summary,
            quality_tracking_enabled: config.quality_tracking_enabled,
            quality_tracking: Arc::new(RwLock::new(HashMap::new())),
            last_summary_signal: Arc::new(RwLock::new(None)),
        }
    }

    /// Build context window for a session
    ///
    /// Loads recent messages and applies token budget limits.
    /// For now, this is a simple implementation that keeps the most recent messages
    /// within the context_window_size. In Phase 10, we'll add summarization.
    pub async fn build_context_window(
        &self,
        messages: Vec<ChatMessage>,
        system_prompt: &str,
        summarizer: Option<&dyn ModelProvider>,
    ) -> Result<Vec<ChatMessage>> {
        // Start with system prompt
        let mut context = vec![ChatMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
            name: None,
            tool_calls: None,
        }];

        // Calculate approximate token count (4 chars per token is a rough estimate)
        let system_tokens = system_prompt.chars().count() / 4;
        let remaining_tokens = self.context_window_size.saturating_sub(system_tokens);

        let task_keywords = Self::extract_task_keywords(&messages);
        let profile = MessageScorer::profile_from_messages(&messages);
        let deduped_messages = ContextDeduplicator::deduplicate(messages);

        let candidates = deduped_messages
            .into_iter()
            .enumerate()
            .map(|(idx, message)| CandidateMessage {
                index: idx,
                token_estimate: std::cmp::max(1, message.content.chars().count() / 4),
                importance: MessageScorer::score(&message, &task_keywords, profile),
                message,
            })
            .collect::<Vec<_>>();

        let mut selected_indexes = HashSet::new();
        let mut selected = Vec::new();
        let mut used_tokens = 0usize;

        for importance in [
            MessageImportance::Critical,
            MessageImportance::High,
            MessageImportance::Medium,
            MessageImportance::Low,
        ] {
            for candidate in candidates.iter().rev() {
                if candidate.importance != importance {
                    continue;
                }
                if used_tokens + candidate.token_estimate > remaining_tokens {
                    continue;
                }
                if selected_indexes.contains(&candidate.index) {
                    continue;
                }

                used_tokens += candidate.token_estimate;
                selected.push(candidate.clone());
                selected_indexes.insert(candidate.index);
            }
        }

        let omitted = candidates
            .iter()
            .filter(|candidate| !selected_indexes.contains(&candidate.index))
            .map(|candidate| candidate.message.clone())
            .collect::<Vec<_>>();

        let token_pressure = used_tokens as f64 / remaining_tokens.max(1) as f64;

        let should_summarize = self.should_trigger_summary(&omitted, token_pressure);

        if should_summarize && self.summary_enabled && !omitted.is_empty() {
            let (summary, from_cache) = self
                .get_or_create_summary(&omitted, summarizer, token_pressure)
                .await?;
            if !summary.content.trim().is_empty() {
                context.push(ChatMessage {
                    role: "system".to_owned(),
                    content: format!(
                        "Conversation summary ({} messages): {}",
                        summary.source_message_count, summary.content
                    ),
                    name: None,
                    tool_calls: None,
                });

                let mut acceptance_count = 0;
                if self.quality_tracking_enabled {
                    self.record_summary_quality(&summary.key, from_cache).await;
                    if let Some((accepted, _rejected)) =
                        self.get_summary_quality(&summary.key).await
                    {
                        acceptance_count = accepted;
                    }
                }

                *self.last_summary_signal.write().await = Some(SummarySignal {
                    trigger: self.trigger_mode,
                    message_count: summary.source_message_count,
                    token_pressure,
                    summary_length: summary.content.len(),
                    summary_key: summary.key.clone(),
                    rating: if from_cache { 1 } else { -1 },
                    implicit: true,
                    acceptance_count,
                    has_user_task: self.include_user_task,
                    has_completion_summary: self.include_completion_summary,
                });
            }
        }

        selected.sort_by_key(|candidate| candidate.index);
        for candidate in selected {
            context.push(candidate.message);
        }
        Ok(context)
    }

    fn should_trigger_summary(
        &self,
        omitted_messages: &[ChatMessage],
        token_pressure: f64,
    ) -> bool {
        if !self.aggressive_summary_enabled {
            return false;
        }

        let omitted_count = omitted_messages.len();

        match self.trigger_mode {
            SummaryTriggerMode::FixedMessageCount => omitted_count >= self.message_window_threshold,
            SummaryTriggerMode::TokenThreshold => token_pressure >= self.token_threshold_percent,
            SummaryTriggerMode::TurnBased => {
                // For turn-based, we trigger when we have enough omitted messages
                omitted_count >= self.message_window_threshold
            }
            SummaryTriggerMode::Hybrid => {
                // Hybrid: trigger if EITHER condition is met
                omitted_count >= self.message_window_threshold
                    || token_pressure >= self.token_threshold_percent
            }
        }
    }

    async fn get_or_create_summary(
        &self,
        omitted_messages: &[ChatMessage],
        summarizer: Option<&dyn ModelProvider>,
        token_pressure: f64,
    ) -> Result<(ConversationSummary, bool)> {
        let key = Self::summary_cache_key(omitted_messages);
        if let Some(existing) = self.summary_cache.read().await.entries.get(&key).cloned() {
            return Ok((existing, true));
        }

        let generated = if let Some(provider) = summarizer {
            match self
                .generate_provider_summary(omitted_messages, provider, token_pressure)
                .await
            {
                Ok(content) => ConversationSummary {
                    key: key.clone(),
                    content,
                    source_message_count: omitted_messages.len(),
                    generated_with_provider: true,
                },
                Err(_) => self.heuristic_summary(omitted_messages, key.clone(), token_pressure),
            }
        } else {
            self.heuristic_summary(omitted_messages, key.clone(), token_pressure)
        };

        {
            let mut cache = self.summary_cache.write().await;
            cache.order.push(generated.key.clone());
            cache
                .entries
                .insert(generated.key.clone(), generated.clone());
            while cache.order.len() > self.summary_cache_max_entries {
                let oldest = cache.order.remove(0);
                cache.entries.remove(&oldest);
            }
        }

        Ok((generated, false))
    }

    pub async fn take_last_summary_signal(&self) -> Option<SummarySignal> {
        self.last_summary_signal.write().await.take()
    }

    async fn generate_provider_summary(
        &self,
        messages: &[ChatMessage],
        provider: &dyn ModelProvider,
        _token_pressure: f64,
    ) -> Result<String> {
        let (user_task, completion_summary) = self.extract_task_and_completion(messages);

        let mut prompt_messages = Vec::with_capacity(messages.len() + 1);
        let mut system_instruction =
            "Summarize prior conversation context for continued coding work. Keep key decisions, constraints, in-progress work, and unresolved items. Use concise plain text."
                .to_owned();

        if self.include_user_task {
            system_instruction.push_str(&format!("\n\nUSER REQUESTED: {}", user_task));
        }

        if self.include_completion_summary {
            system_instruction.push_str(&format!("\n\nWE COMPLETED: {}", completion_summary));
        }

        prompt_messages.push(ChatMessage {
            role: "system".to_owned(),
            content: system_instruction,
            name: None,
            tool_calls: None,
        });
        prompt_messages.extend_from_slice(messages);

        timeout(
            Duration::from_secs(PROVIDER_SUMMARY_TIMEOUT_SECS),
            provider.generate(
                &prompt_messages,
                &GenerateOptions {
                    temperature: 0.2,
                    max_tokens: self.summary_max_tokens,
                    top_p: None,
                    top_k: None,
                    stop_sequences: None,
                    presence_penalty: None,
                    frequency_penalty: None,
                },
            ),
        )
        .await
        .map_err(|_| crate::error::Error::Provider("summary generation timed out".to_owned()))?
    }

    fn extract_task_and_completion(&self, messages: &[ChatMessage]) -> (String, String) {
        let user_task = messages
            .iter()
            .find(|msg| msg.role == "user")
            .map(|msg| msg.content.trim().to_string())
            .unwrap_or_else(|| "No clear task identified.".to_string());

        let completion_summary = messages
            .iter()
            .filter(|msg| msg.role == "assistant")
            .map(|msg| msg.content.trim())
            .collect::<Vec<_>>()
            .join(" | ");

        (
            user_task,
            if completion_summary.is_empty() {
                "No completions yet.".to_string()
            } else {
                completion_summary
            },
        )
    }

    fn heuristic_summary(
        &self,
        messages: &[ChatMessage],
        key: String,
        token_pressure: f64,
    ) -> ConversationSummary {
        let (user_task, completion_summary) = self.extract_heuristic_task_and_completion(messages);

        let mut summary_parts = Vec::new();

        if self.include_user_task {
            summary_parts.push(format!("USER REQUESTED: {}", user_task));
        }

        if self.include_completion_summary {
            summary_parts.push(format!("WE COMPLETED: {}", completion_summary));
        }

        let snippets = messages
            .iter()
            .rev()
            .take(6)
            .map(|message| {
                let compact = message
                    .content
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let trimmed = compact.chars().take(120).collect::<String>();
                format!("[{}] {}", message.role, trimmed)
            })
            .collect::<Vec<_>>();

        if !snippets.is_empty() {
            summary_parts.push(format!("Context: {}", snippets.join(" | ")));
        }

        summary_parts.push(format!("Token pressure: {:.1}%", token_pressure * 100.0));

        ConversationSummary {
            key,
            content: summary_parts.join("\n\n"),
            source_message_count: messages.len(),
            generated_with_provider: false,
        }
    }

    fn extract_heuristic_task_and_completion(&self, messages: &[ChatMessage]) -> (String, String) {
        let user_task = messages
            .iter()
            .find(|msg| msg.role == "user")
            .map(|msg| msg.content.chars().take(150).collect::<String>())
            .unwrap_or_else(|| "No clear task".to_string());

        let completion_summary = messages
            .iter()
            .filter(|msg| msg.role == "assistant")
            .map(|msg| msg.content.chars().take(100).collect::<String>())
            .collect::<Vec<_>>()
            .join("; ");

        (
            user_task,
            if completion_summary.is_empty() {
                "No completions".to_string()
            } else {
                completion_summary
            },
        )
    }

    fn summary_cache_key(messages: &[ChatMessage]) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for message in messages {
            message.role.hash(&mut hasher);
            message.content.hash(&mut hasher);
        }
        format!("summary:{:x}", hasher.finish())
    }

    /// Record implicit quality feedback for a summary (acceptance/rejection)
    pub async fn record_summary_quality(&self, summary_key: &str, accepted: bool) {
        if !self.quality_tracking_enabled {
            return;
        }

        let mut tracking = self.quality_tracking.write().await;
        let entry = tracking
            .entry(summary_key.to_string())
            .or_insert(SummaryQualityTracking {
                acceptance_count: 0,
                rejection_count: 0,
            });

        if accepted {
            entry.acceptance_count += 1;
        } else {
            entry.rejection_count += 1;
        }
    }

    /// Get quality tracking data for a summary
    pub async fn get_summary_quality(&self, summary_key: &str) -> Option<(u32, u32)> {
        let tracking = self.quality_tracking.read().await;
        tracking
            .get(summary_key)
            .map(|t| (t.acceptance_count, t.rejection_count))
    }

    /// Create SummaryGenerated event for emission by caller
    pub fn create_summary_generated_event(
        &self,
        session_id: String,
        agent: String,
        summary_key: String,
        message_count: usize,
        token_pressure: f64,
        summary_length: usize,
    ) -> Event {
        Event::SummaryGenerated {
            session_id,
            agent,
            trigger: self.trigger_mode,
            message_count,
            token_pressure,
            summary_length,
            summary_key,
            has_user_task: self.include_user_task,
            has_completion_summary: self.include_completion_summary,
        }
    }

    /// Create SummaryQualityUpdated event for emission by caller
    pub fn create_summary_quality_updated_event(
        &self,
        session_id: String,
        summary_key: String,
        rating: i8,
        implicit: bool,
        acceptance_count: u32,
    ) -> Event {
        Event::SummaryQualityUpdated {
            session_id,
            summary_key,
            rating,
            implicit,
            acceptance_count,
        }
    }

    fn extract_task_keywords(messages: &[ChatMessage]) -> Vec<String> {
        let Some(last_user_message) = messages.iter().rev().find(|msg| msg.role == "user") else {
            return Vec::new();
        };

        let mut keywords = Vec::new();
        for token in last_user_message
            .content
            .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
        {
            let normalized = token.trim().to_ascii_lowercase();
            if normalized.len() < 4 {
                continue;
            }
            if keywords.iter().any(|existing| existing == &normalized) {
                continue;
            }
            keywords.push(normalized);
            if keywords.len() >= 10 {
                break;
            }
        }
        keywords
    }
}

#[derive(Debug, Clone)]
struct CandidateMessage {
    index: usize,
    token_estimate: usize,
    importance: MessageImportance,
    message: ChatMessage,
}

#[derive(Debug, Default, Clone)]
struct SummaryCacheState {
    entries: HashMap<String, ConversationSummary>,
    order: Vec<String>,
}
