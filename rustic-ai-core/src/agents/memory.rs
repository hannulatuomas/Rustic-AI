use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

use crate::agents::context::{
    ContextDeduplicator, ConversationSummary, MessageImportance, MessageScorer,
};
use crate::error::Result;
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};

const DEFAULT_SUMMARY_MAX_TOKENS: usize = 500;
const DEFAULT_SUMMARY_CACHE_ENTRIES: usize = 64;
const PROVIDER_SUMMARY_TIMEOUT_SECS: u64 = 15;

#[derive(Debug, Clone)]
pub struct AgentMemory {
    context_window_size: usize,
    summary_enabled: bool,
    summary_max_tokens: usize,
    summary_cache_max_entries: usize,
    summary_cache: Arc<RwLock<SummaryCacheState>>,
}

impl AgentMemory {
    pub fn new(
        context_window_size: usize,
        summary_enabled: bool,
        summary_max_tokens: Option<usize>,
        summary_cache_max_entries: Option<usize>,
    ) -> Self {
        Self {
            context_window_size,
            summary_enabled,
            summary_max_tokens: summary_max_tokens.unwrap_or(DEFAULT_SUMMARY_MAX_TOKENS),
            summary_cache_max_entries: summary_cache_max_entries
                .unwrap_or(DEFAULT_SUMMARY_CACHE_ENTRIES),
            summary_cache: Arc::new(RwLock::new(SummaryCacheState::default())),
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

        if self.summary_enabled && !omitted.is_empty() {
            let summary = self.get_or_create_summary(&omitted, summarizer).await?;
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
            }
        }

        selected.sort_by_key(|candidate| candidate.index);
        for candidate in selected {
            context.push(candidate.message);
        }
        Ok(context)
    }

    async fn get_or_create_summary(
        &self,
        omitted_messages: &[ChatMessage],
        summarizer: Option<&dyn ModelProvider>,
    ) -> Result<ConversationSummary> {
        let key = Self::summary_cache_key(omitted_messages);
        if let Some(existing) = self.summary_cache.read().await.entries.get(&key).cloned() {
            return Ok(existing);
        }

        let generated = if let Some(provider) = summarizer {
            match self
                .generate_provider_summary(omitted_messages, provider)
                .await
            {
                Ok(content) => ConversationSummary {
                    key: key.clone(),
                    content,
                    source_message_count: omitted_messages.len(),
                    generated_with_provider: true,
                },
                Err(_) => Self::heuristic_summary(omitted_messages, key.clone()),
            }
        } else {
            Self::heuristic_summary(omitted_messages, key.clone())
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

        Ok(generated)
    }

    async fn generate_provider_summary(
        &self,
        messages: &[ChatMessage],
        provider: &dyn ModelProvider,
    ) -> Result<String> {
        let mut prompt_messages = Vec::with_capacity(messages.len() + 1);
        prompt_messages.push(ChatMessage {
            role: "system".to_owned(),
            content: "Summarize prior conversation context for continued coding work. Keep key decisions, constraints, in-progress work, and unresolved items. Use concise plain text.".to_owned(),
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

    fn heuristic_summary(messages: &[ChatMessage], key: String) -> ConversationSummary {
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

        ConversationSummary {
            key,
            content: snippets.join(" | "),
            source_message_count: messages.len(),
            generated_with_provider: false,
        }
    }

    fn summary_cache_key(messages: &[ChatMessage]) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for message in messages {
            message.role.hash(&mut hasher);
            message.content.hash(&mut hasher);
        }
        format!("summary:{:x}", hasher.finish())
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
