use crate::agents::memory::{AgentMemory, AgentMemoryConfig};
use crate::agents::todo_extractor;
use crate::config::schema::AgentConfig;
use crate::conversation::session_manager::SessionManager;
use crate::error::Result;
use crate::events::Event;
use crate::learning::{LearningManager, MistakeType};
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};
use crate::rag::HybridRetriever;
use crate::storage::PendingToolState;
use crate::ToolManager;
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration, Instant};
use tokio_util::sync::CancellationToken;

const DEFAULT_MAX_TOOL_ROUNDS: usize = 4;
const DEFAULT_MAX_TOOLS_PER_ROUND: usize = 8;
const DEFAULT_MAX_TOTAL_TOOL_CALLS_PER_TURN: usize = 24;
const DEFAULT_MAX_TURN_DURATION_SECONDS: u64 = 300;
const DEFAULT_TOOL_SHORTLIST_ITEMS: usize = 12;
const DEFAULT_TOOL_SHORTLIST_CHAR_BUDGET: usize = 1200;
const HARD_MAX_TOOL_ROUNDS: usize = 32;
const HARD_MAX_TOOLS_PER_ROUND: usize = 64;
const HARD_MAX_TOTAL_TOOL_CALLS_PER_TURN: usize = 256;

#[derive(Clone)]
pub struct Agent {
    config: AgentConfig,
    memory: AgentMemory,
    provider: Arc<dyn ModelProvider>,
    tool_manager: Arc<ToolManager>,
    session_manager: Arc<SessionManager>,
    learning: Arc<LearningManager>,
    retriever: Arc<HybridRetriever>,
}

#[derive(Debug, Clone, Deserialize)]
struct ParsedToolCall {
    tool: String,
    args: serde_json::Value,
}

#[derive(Debug, Clone)]
struct ToolExecutionResult {
    message: ChatMessage,
    pending: bool,
}

#[derive(Clone, Copy)]
struct ResponseMeta<'a> {
    session_id_str: &'a str,
    agent_name: &'a str,
}

#[derive(Clone, Copy)]
enum TurnDurationBudget {
    Unlimited,
    Remaining(Duration),
    Exhausted,
}

enum ToolCallOutcome {
    Completed(Option<crate::tools::ToolResult>),
    Exhausted,
}

#[derive(Clone)]
struct ToolCallExecutionContext<'a> {
    session_id_str: &'a str,
    agent_name: &'a str,
    event_tx: mpsc::Sender<Event>,
    cancellation_token: Option<CancellationToken>,
    turn_started: Instant,
    max_turn_duration: Option<Duration>,
}

impl Agent {
    fn is_cancelled(cancellation_token: Option<&CancellationToken>) -> bool {
        cancellation_token
            .map(CancellationToken::is_cancelled)
            .unwrap_or(false)
    }

    fn interrupted_error() -> crate::Error {
        crate::Error::Timeout("agent turn interrupted by user".to_owned())
    }

    fn enforce_tool_call_budget(
        total_tool_calls_executed: usize,
        max_total_tool_calls: usize,
        event_tx: &mpsc::Sender<Event>,
    ) -> bool {
        if total_tool_calls_executed >= max_total_tool_calls {
            let _ = event_tx.try_send(Event::Progress(format!(
                "agent reached max total tool calls per turn ({max_total_tool_calls}); stopping autonomous loop"
            )));
            true
        } else {
            false
        }
    }

    fn turn_duration_budget(
        turn_started: Instant,
        max_turn_duration: Option<Duration>,
        event_tx: &mpsc::Sender<Event>,
        while_waiting_on_tool: bool,
    ) -> TurnDurationBudget {
        let Some(max_duration) = max_turn_duration else {
            return TurnDurationBudget::Unlimited;
        };

        if turn_started.elapsed() >= max_duration {
            let suffix = if while_waiting_on_tool {
                " while waiting on tool"
            } else {
                ""
            };
            let _ = event_tx.try_send(Event::Progress(format!(
                "agent reached max turn duration ({}s){suffix}; stopping autonomous loop",
                max_duration.as_secs()
            )));
            return TurnDurationBudget::Exhausted;
        }

        match max_duration.checked_sub(turn_started.elapsed()) {
            Some(remaining) if !remaining.is_zero() => TurnDurationBudget::Remaining(remaining),
            _ => {
                let suffix = if while_waiting_on_tool {
                    " while waiting on tool"
                } else {
                    ""
                };
                let _ = event_tx.try_send(Event::Progress(format!(
                    "agent reached max turn duration ({}s){suffix}; stopping autonomous loop",
                    max_duration.as_secs()
                )));
                TurnDurationBudget::Exhausted
            }
        }
    }

    async fn execute_tool_with_turn_budget(
        &self,
        call: &ParsedToolCall,
        exec_ctx: &ToolCallExecutionContext<'_>,
    ) -> Result<ToolCallOutcome> {
        match Self::turn_duration_budget(
            exec_ctx.turn_started,
            exec_ctx.max_turn_duration,
            &exec_ctx.event_tx,
            false,
        ) {
            TurnDurationBudget::Exhausted => Ok(ToolCallOutcome::Exhausted),
            TurnDurationBudget::Remaining(remaining) => {
                match timeout(
                    remaining,
                    self.tool_manager.execute_tool_with_cancel(
                        exec_ctx.session_id_str.to_owned(),
                        Some(exec_ctx.agent_name.to_owned()),
                        &call.tool,
                        call.args.clone(),
                        exec_ctx.event_tx.clone(),
                        exec_ctx.cancellation_token.clone(),
                    ),
                )
                .await
                {
                    Ok(result) => Ok(ToolCallOutcome::Completed(result?)),
                    Err(_) => {
                        let _ = Self::turn_duration_budget(
                            exec_ctx.turn_started,
                            exec_ctx.max_turn_duration,
                            &exec_ctx.event_tx,
                            true,
                        );
                        Ok(ToolCallOutcome::Exhausted)
                    }
                }
            }
            TurnDurationBudget::Unlimited => Ok(ToolCallOutcome::Completed(
                self.tool_manager
                    .execute_tool_with_cancel(
                        exec_ctx.session_id_str.to_owned(),
                        Some(exec_ctx.agent_name.to_owned()),
                        &call.tool,
                        call.args.clone(),
                        exec_ctx.event_tx.clone(),
                        exec_ctx.cancellation_token.clone(),
                    )
                    .await?,
            )),
        }
    }

    pub fn new(
        config: AgentConfig,
        provider: Arc<dyn ModelProvider>,
        tool_manager: Arc<ToolManager>,
        session_manager: Arc<SessionManager>,
        learning: Arc<LearningManager>,
        retriever: Arc<HybridRetriever>,
        aggressive_summary_enabled: bool,
    ) -> Self {
        let memory_config = AgentMemoryConfig {
            aggressive_summary_enabled,
            ..Default::default()
        };

        let memory = AgentMemory::new(
            config.context_window_size,
            config.context_summary_enabled.unwrap_or(true),
            config.context_summary_max_tokens,
            config.context_summary_cache_entries,
            memory_config,
        );

        Self {
            config,
            memory,
            provider,
            tool_manager,
            session_manager,
            learning,
            retriever,
        }
    }

    async fn system_prompt(&self, focus_hint: Option<&str>) -> String {
        let base_prompt = if let Some(ref template) = self.config.system_prompt_template {
            template.clone()
        } else {
            "You are a helpful AI assistant.".to_string()
        };

        if self.config.tools.is_empty() {
            return base_prompt;
        }

        let tools = self.config.tools.join(", ");
        let configured = self
            .config
            .tools
            .iter()
            .map(|tool| tool.as_str())
            .collect::<HashSet<_>>();
        let shortlist_items = self
            .config
            .tool_shortlist_max_items
            .unwrap_or(DEFAULT_TOOL_SHORTLIST_ITEMS);
        let shortlist_cap = usize::min(self.config.tools.len(), shortlist_items);
        let shortlist_char_budget = self
            .config
            .tool_shortlist_char_budget
            .unwrap_or(DEFAULT_TOOL_SHORTLIST_CHAR_BUDGET);
        let mut prioritized_tools = self
            .tool_manager
            .get_tool_descriptions(focus_hint, Some(shortlist_cap))
            .await
            .into_iter()
            .filter(|(name, _)| configured.contains(name.as_str()))
            .collect::<Vec<_>>();

        if prioritized_tools.is_empty() {
            prioritized_tools = self
                .tool_manager
                .get_tool_descriptions(None, None)
                .await
                .into_iter()
                .filter(|(name, _)| configured.contains(name.as_str()))
                .collect();
        }

        let mut prioritized_chunks = Vec::new();
        let mut used_chars = 0usize;
        for (name, description) in prioritized_tools {
            let compact_description = description.split_whitespace().collect::<Vec<_>>().join(" ");
            let chunk = format!("{name}: {compact_description}");
            if !prioritized_chunks.is_empty() && used_chars + chunk.len() > shortlist_char_budget {
                break;
            }
            used_chars += chunk.len();
            prioritized_chunks.push(chunk);
        }

        let prioritized = if prioritized_chunks.is_empty() {
            tools.clone()
        } else {
            prioritized_chunks.join("; ")
        };

        format!(
            "{base_prompt}\n\nWhen you need a tool, emit a single-line JSON object only with shape: {{\"tool\":\"<tool_name>\",\"args\":{{...}}}}. Configured tools: {tools}. Prioritize these tools for this task: {prioritized}."
        )
    }

    fn extract_tool_calls(&self, response: &str) -> Vec<ParsedToolCall> {
        let trimmed = response.trim();
        if let Ok(call) = serde_json::from_str::<ParsedToolCall>(trimmed) {
            return vec![call];
        }

        if let Ok(calls) = serde_json::from_str::<Vec<ParsedToolCall>>(trimmed) {
            return calls;
        }

        let mut calls = Vec::new();
        for line in response.lines() {
            let line = line.trim();
            if !(line.starts_with('{') && line.ends_with('}')) {
                continue;
            }

            if let Ok(call) = serde_json::from_str::<ParsedToolCall>(line) {
                calls.push(call);
            }
        }

        calls
    }

    fn generation_options(&self) -> GenerateOptions {
        GenerateOptions {
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            top_p: None,
            top_k: None,
            stop_sequences: None,
            presence_penalty: None,
            frequency_penalty: None,
        }
    }

    fn latest_user_task(context: &[ChatMessage]) -> Option<String> {
        context
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.clone())
    }

    fn tools_used_from_context(context: &[ChatMessage]) -> Vec<String> {
        let mut tools = Vec::new();
        for message in context {
            if message.role != "tool" {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&message.content) {
                let maybe_tool_name = value.get("tool").and_then(serde_json::Value::as_str);
                if let Some(tool_name) = maybe_tool_name {
                    if !tools.iter().any(|existing| existing == tool_name) {
                        tools.push(tool_name.to_owned());
                    }
                }
            }
        }
        tools
    }

    async fn maybe_apply_preferred_approach(
        &self,
        session_id: uuid::Uuid,
        session_id_str: &str,
        context: &mut Vec<ChatMessage>,
        event_tx: &mpsc::Sender<Event>,
    ) -> Result<()> {
        if !self.learning.enabled() {
            return Ok(());
        }

        if let Some(preferred_approach) = self
            .learning
            .get_preferred_approach(session_id, "task_execution")
            .await?
        {
            context.insert(
                1,
                ChatMessage {
                    role: "system".to_owned(),
                    content: format!("User preference for task execution: {}", preferred_approach),
                    name: None,
                    tool_calls: None,
                },
            );

            let _ = event_tx.try_send(Event::LearningPreferenceApplied {
                session_id: session_id_str.to_owned(),
                agent: self.config.name.clone(),
                key: "preferred_approach.task_execution".to_owned(),
            });
        }

        Ok(())
    }

    async fn emit_pattern_warnings(
        &self,
        session_id_str: &str,
        event_tx: &mpsc::Sender<Event>,
    ) -> Result<()> {
        if !self.learning.enabled() {
            return Ok(());
        }

        let patterns = self
            .learning
            .get_active_patterns(&self.config.name, 5)
            .await?;
        for pattern in patterns {
            let _ = event_tx.try_send(Event::LearningPatternWarning {
                session_id: session_id_str.to_owned(),
                agent: self.config.name.clone(),
                mistake_type: pattern.mistake_type.as_str().to_owned(),
                frequency: pattern.frequency,
                suggested_fix: pattern.suggested_fix.clone(),
            });
        }

        Ok(())
    }

    async fn maybe_inject_retrieval_context(
        &self,
        session_id_str: &str,
        input: &str,
        context: &mut Vec<ChatMessage>,
        event_tx: &mpsc::Sender<Event>,
    ) -> Result<()> {
        let retrieval = self.retriever.retrieve(input).await?;
        if retrieval.snippets.is_empty() {
            return Ok(());
        }

        let prompt_block = self.retriever.format_for_prompt(&retrieval.snippets);
        if prompt_block.is_empty() {
            return Ok(());
        }

        let message = ChatMessage {
            role: if self.retriever.inject_as_system_message() {
                "system".to_owned()
            } else {
                "user".to_owned()
            },
            content: prompt_block,
            name: None,
            tool_calls: None,
        };

        let insertion_index = context.len().saturating_sub(1);
        context.insert(insertion_index, message);
        self.compact_context_for_rag(context);

        let _ = event_tx.try_send(Event::RetrievalContextInjected {
            session_id: session_id_str.to_owned(),
            agent: self.config.name.clone(),
            snippets: retrieval.snippets.len(),
            keyword_hits: retrieval.keyword_hits,
            vector_hits: retrieval.vector_hits,
        });

        Ok(())
    }

    fn compact_context_for_rag(&self, context: &mut Vec<ChatMessage>) {
        let token_budget = self.config.context_window_size;
        if token_budget == 0 || context.len() <= 2 {
            return;
        }

        while estimate_context_tokens(context) > token_budget {
            let mut removed = false;
            for index in 1..context.len().saturating_sub(1) {
                let candidate = &context[index];
                if candidate.role == "system"
                    && candidate
                        .content
                        .starts_with("Retrieved context from code index:")
                {
                    continue;
                }

                context.remove(index);
                removed = true;
                break;
            }

            if !removed {
                break;
            }
        }
    }

    fn render_tool_output_message(
        tool_name: &str,
        result: Option<&crate::tools::ToolResult>,
    ) -> String {
        match result {
            Some(result) => format!(
                "{{\"tool\":\"{}\",\"success\":{},\"exit_code\":{},\"output\":{}}}",
                tool_name,
                result.success,
                result.exit_code.unwrap_or_default(),
                serde_json::to_string(&result.output).unwrap_or_else(|_| "\"\"".to_string())
            ),
            None => format!(
                "tool '{}' requires user input before execution and is pending",
                tool_name
            ),
        }
    }

    async fn execute_tool_call_and_record(
        &self,
        session_id: uuid::Uuid,
        session_id_str: &str,
        agent_name: &str,
        call: &ParsedToolCall,
        event_tx: mpsc::Sender<Event>,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<ToolExecutionResult> {
        if !self.config.tools.iter().any(|name| name == &call.tool) {
            let message = format!(
                "tool '{}' is not allowed for agent '{}'",
                call.tool, self.config.name
            );
            self.session_manager
                .append_message(session_id, "tool", &message)
                .await?;
            return Ok(ToolExecutionResult {
                message: ChatMessage {
                    role: "tool".to_string(),
                    content: message,
                    name: Some(call.tool.clone()),
                    tool_calls: None,
                },
                pending: false,
            });
        }

        let tool_result = self
            .tool_manager
            .execute_tool_with_cancel(
                session_id_str.to_owned(),
                Some(agent_name.to_owned()),
                &call.tool,
                call.args.clone(),
                event_tx,
                cancellation_token,
            )
            .await?;

        let tool_message = Self::render_tool_output_message(&call.tool, tool_result.as_ref());
        self.session_manager
            .append_message(session_id, "tool", &tool_message)
            .await?;

        Ok(ToolExecutionResult {
            message: ChatMessage {
                role: "tool".to_string(),
                content: tool_message,
                name: Some(call.tool.clone()),
                tool_calls: None,
            },
            pending: tool_result.is_none(),
        })
    }

    async fn handle_disallowed_tool_call(
        &self,
        session_id: uuid::Uuid,
        session_id_str: &str,
        agent_name: &str,
        call: &ParsedToolCall,
        event_tx: &mpsc::Sender<Event>,
    ) -> Result<ChatMessage> {
        let message = format!(
            "tool '{}' is not allowed for agent '{}'",
            call.tool, self.config.name
        );
        if self.learning.enabled() {
            let pattern = self
                .learning
                .record_mistake(
                    agent_name,
                    MistakeType::WrongApproach,
                    format!("tool={} not allowed", call.tool),
                )
                .await?;
            let _ = event_tx.try_send(Event::LearningPatternWarning {
                session_id: session_id_str.to_owned(),
                agent: agent_name.to_owned(),
                mistake_type: pattern.mistake_type.as_str().to_owned(),
                frequency: pattern.frequency,
                suggested_fix: pattern.suggested_fix,
            });
        }
        self.session_manager
            .append_message(session_id, "tool", &message)
            .await?;
        Ok(ChatMessage {
            role: "tool".to_string(),
            content: message,
            name: Some(call.tool.clone()),
            tool_calls: None,
        })
    }

    async fn persist_pending_state(
        &self,
        session_id: uuid::Uuid,
        call: &ParsedToolCall,
        round_index: usize,
        tool_messages: Vec<ChatMessage>,
        context_snapshot: Vec<ChatMessage>,
    ) -> Result<()> {
        let pending_state = PendingToolState {
            session_id,
            tool_name: call.tool.clone(),
            args: call.args.clone(),
            round_index,
            tool_messages,
            context_snapshot,
            created_at: Utc::now(),
        };
        self.session_manager.set_pending_tool(pending_state).await
    }

    fn effective_max_tool_rounds(&self) -> usize {
        match self.config.max_tool_rounds {
            None => DEFAULT_MAX_TOOL_ROUNDS,
            Some(0) => usize::MAX,
            Some(value) => value.min(HARD_MAX_TOOL_ROUNDS),
        }
    }

    fn effective_max_tools_per_round(&self) -> usize {
        match self.config.max_tools_per_round {
            None => DEFAULT_MAX_TOOLS_PER_ROUND,
            Some(0) => usize::MAX,
            Some(value) => value.min(HARD_MAX_TOOLS_PER_ROUND),
        }
    }

    fn effective_max_total_tool_calls_per_turn(&self) -> usize {
        match self.config.max_total_tool_calls_per_turn {
            None => DEFAULT_MAX_TOTAL_TOOL_CALLS_PER_TURN,
            Some(0) => usize::MAX,
            Some(value) => value.min(HARD_MAX_TOTAL_TOOL_CALLS_PER_TURN),
        }
    }

    fn effective_max_turn_duration(&self) -> Option<Duration> {
        match self.config.max_turn_duration_seconds {
            None => Some(Duration::from_secs(DEFAULT_MAX_TURN_DURATION_SECONDS)),
            Some(0) => None,
            Some(seconds) => Some(Duration::from_secs(seconds)),
        }
    }

    async fn load_context_window_from_session(
        &self,
        session_id: uuid::Uuid,
        session_id_str: &str,
        focus_hint: Option<&str>,
        event_tx: &mpsc::Sender<Event>,
    ) -> Result<Vec<ChatMessage>> {
        let messages = self
            .session_manager
            .get_session_messages(session_id)
            .await?;
        let chat_messages: Vec<ChatMessage> = messages
            .into_iter()
            .map(|msg| ChatMessage {
                role: msg.role,
                content: msg.content,
                name: None,
                tool_calls: None,
            })
            .collect();

        let system_prompt = self.system_prompt(focus_hint).await;
        let mut context = self
            .memory
            .build_context_window(chat_messages, &system_prompt, Some(self.provider.as_ref()))
            .await?;

        if let Some(signal) = self.memory.take_last_summary_signal().await {
            let _ = event_tx.try_send(Event::SummaryGenerated {
                session_id: session_id_str.to_owned(),
                agent: self.config.name.clone(),
                trigger: signal.trigger,
                message_count: signal.message_count,
                token_pressure: signal.token_pressure,
                summary_length: signal.summary_length,
                summary_key: signal.summary_key.clone(),
                has_user_task: signal.has_user_task,
                has_completion_summary: signal.has_completion_summary,
            });
            let _ = event_tx.try_send(Event::SummaryQualityUpdated {
                session_id: session_id_str.to_owned(),
                summary_key: signal.summary_key,
                rating: signal.rating,
                implicit: signal.implicit,
                acceptance_count: signal.acceptance_count,
            });
        }

        let mut system_context_blocks = Vec::new();

        if let Some(topics) = self.session_manager.get_session_topics(session_id).await? {
            if !topics.is_empty() {
                system_context_blocks.push(format!(
                    "Active topics for this session: {}",
                    topics.join(", ")
                ));
            }
        }

        let rules = self
            .session_manager
            .get_applicable_rules(session_id, None, None)
            .await?
            .into_iter()
            .take(8)
            .map(|rule| format!("[{}] {}", rule.metadata.path, rule.content.trim()))
            .collect::<Vec<_>>();
        if !rules.is_empty() {
            system_context_blocks.push(format!("Applicable rules:\n{}", rules.join("\n\n")));
        }

        if let Some(profile) = self.session_manager.project_profile() {
            let mut profile_lines = Vec::new();
            if !profile.name.trim().is_empty() {
                profile_lines.push(format!("name={}", profile.name));
            }
            if !profile.root_path.trim().is_empty() {
                profile_lines.push(format!("root_path={}", profile.root_path));
            }
            if !profile.tech_stack.is_empty() {
                profile_lines.push(format!("tech_stack={}", profile.tech_stack.join(", ")));
            }
            if !profile.goals.is_empty() {
                profile_lines.push(format!("goals={}", profile.goals.join(" | ")));
            }
            if !profile.preferences.is_empty() {
                profile_lines.push(format!("preferences={}", profile.preferences.join(" | ")));
            }
            if !profile.style_guidelines.is_empty() {
                profile_lines.push(format!(
                    "style_guidelines={}",
                    profile.style_guidelines.join(" | ")
                ));
            }
            if !profile_lines.is_empty() {
                system_context_blocks
                    .push(format!("Project profile:\n{}", profile_lines.join("\n")));
            }
        }

        if !system_context_blocks.is_empty() {
            context.insert(
                1,
                ChatMessage {
                    role: "system".to_owned(),
                    content: system_context_blocks.join("\n\n"),
                    name: None,
                    tool_calls: None,
                },
            );
        }

        Ok(context)
    }

    async fn run_assistant_tool_loop(
        &self,
        session_id: uuid::Uuid,
        session_id_str: &str,
        agent_name: &str,
        mut context: Vec<ChatMessage>,
        event_tx: mpsc::Sender<Event>,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<()> {
        let options = self.generation_options();
        let max_rounds = self.effective_max_tool_rounds();
        let max_tools_per_round = self.effective_max_tools_per_round();
        let max_total_tool_calls = self.effective_max_total_tool_calls_per_turn();
        let max_turn_duration = self.effective_max_turn_duration();
        let turn_started = Instant::now();
        let mut total_tool_calls_executed = 0usize;
        let task_description = Self::latest_user_task(&context).unwrap_or_default();
        let mut tools_used = Self::tools_used_from_context(&context);

        self.emit_pattern_warnings(session_id_str, &event_tx)
            .await?;

        for round_index in 0..max_rounds {
            if Self::is_cancelled(cancellation_token.as_ref()) {
                let _ = event_tx.try_send(Event::Progress(
                    "agent turn interrupted by user request".to_owned(),
                ));
                return Err(Self::interrupted_error());
            }

            // Save context snapshot before generating assistant response
            // This will be used to resume after permission approval
            let context_snapshot = context.clone();

            let remaining_duration =
                match Self::turn_duration_budget(turn_started, max_turn_duration, &event_tx, false)
                {
                    TurnDurationBudget::Unlimited => None,
                    TurnDurationBudget::Remaining(duration) => Some(duration),
                    TurnDurationBudget::Exhausted => return Ok(()),
                };

            let response = self
                .generate_response_with_events(
                    &context,
                    &options,
                    ResponseMeta {
                        session_id_str,
                        agent_name,
                    },
                    event_tx.clone(),
                    remaining_duration,
                    cancellation_token.clone(),
                )
                .await?;

            self.session_manager
                .append_message(session_id, "assistant", &response)
                .await?;

            let mut parsed_tool_calls = self.extract_tool_calls(&response);
            if parsed_tool_calls.is_empty() {
                if self.learning.enabled() {
                    let pattern = self
                        .learning
                        .record_success(
                            session_id,
                            agent_name,
                            &task_description,
                            &tools_used,
                            &response,
                        )
                        .await?;
                    let _ = event_tx.try_send(Event::LearningSuccessPatternRecorded {
                        session_id: session_id_str.to_owned(),
                        agent: agent_name.to_owned(),
                        pattern_name: pattern.name,
                        category: pattern.category.as_str().to_owned(),
                    });
                }

                // Auto-create TODOs from response when enabled
                let _ = self.maybe_auto_create_todos(session_id, &response).await;

                return Ok(());
            }

            if parsed_tool_calls.len() > max_tools_per_round {
                let _ = event_tx.try_send(Event::Progress(format!(
                    "tool calls truncated to {} (requested {})",
                    max_tools_per_round,
                    parsed_tool_calls.len()
                )));
                parsed_tool_calls.truncate(max_tools_per_round);
            }

            let mut tool_messages = Vec::new();
            for call in parsed_tool_calls {
                if Self::is_cancelled(cancellation_token.as_ref()) {
                    let _ = event_tx.try_send(Event::Progress(
                        "agent turn interrupted by user request".to_owned(),
                    ));
                    return Err(Self::interrupted_error());
                }

                if Self::enforce_tool_call_budget(
                    total_tool_calls_executed,
                    max_total_tool_calls,
                    &event_tx,
                ) {
                    return Ok(());
                }
                total_tool_calls_executed += 1;

                if !self.config.tools.iter().any(|name| name == &call.tool) {
                    let disallowed = self
                        .handle_disallowed_tool_call(
                            session_id,
                            session_id_str,
                            agent_name,
                            &call,
                            &event_tx,
                        )
                        .await?;
                    tool_messages.push(disallowed);
                    continue;
                }

                let tool_result = match self
                    .execute_tool_with_turn_budget(
                        &call,
                        &ToolCallExecutionContext {
                            session_id_str,
                            agent_name,
                            event_tx: event_tx.clone(),
                            cancellation_token: cancellation_token.clone(),
                            turn_started,
                            max_turn_duration,
                        },
                    )
                    .await?
                {
                    ToolCallOutcome::Exhausted => return Ok(()),
                    ToolCallOutcome::Completed(result) => result,
                };

                let tool_message =
                    Self::render_tool_output_message(&call.tool, tool_result.as_ref());

                self.session_manager
                    .append_message(session_id, "tool", &tool_message)
                    .await?;

                if !tools_used.iter().any(|tool| tool == &call.tool) {
                    tools_used.push(call.tool.clone());
                }

                if self.learning.enabled() {
                    if let Some(result) = tool_result.as_ref() {
                        if !result.success || result.exit_code.unwrap_or_default() != 0 {
                            let pattern = self
                                .learning
                                .record_tool_failure(
                                    agent_name,
                                    &call.tool,
                                    result.exit_code,
                                    &result.output,
                                )
                                .await?;
                            let _ = event_tx.try_send(Event::LearningPatternWarning {
                                session_id: session_id_str.to_owned(),
                                agent: agent_name.to_owned(),
                                mistake_type: pattern.mistake_type.as_str().to_owned(),
                                frequency: pattern.frequency,
                                suggested_fix: pattern.suggested_fix,
                            });
                        }
                    }
                }

                tool_messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: tool_message,
                    name: Some(call.tool.clone()),
                    tool_calls: None,
                });

                // Store pending tool state if permission was denied/asked
                if tool_result.is_none() {
                    let pending_state = PendingToolState {
                        session_id,
                        tool_name: call.tool.clone(),
                        args: call.args,
                        round_index,
                        tool_messages,
                        context_snapshot,
                        created_at: Utc::now(),
                    };
                    self.session_manager.set_pending_tool(pending_state).await?;
                    return Ok(());
                }
            }

            context.push(ChatMessage {
                role: "assistant".to_string(),
                content: response,
                name: None,
                tool_calls: None,
            });
            context.extend(tool_messages);

            if round_index + 1 == max_rounds {
                let _ = event_tx.try_send(Event::Progress(format!(
                    "agent reached max tool rounds ({max_rounds}); stopping autonomous loop"
                )));
            }
        }

        Ok(())
    }

    async fn generate_response_with_events(
        &self,
        context: &[ChatMessage],
        options: &GenerateOptions,
        response_meta: ResponseMeta<'_>,
        event_tx: mpsc::Sender<Event>,
        remaining_duration: Option<Duration>,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<String> {
        if Self::is_cancelled(cancellation_token.as_ref()) {
            return Err(Self::interrupted_error());
        }

        if self.provider.supports_streaming() {
            let stream_start = async {
                if let Some(remaining) = remaining_duration {
                    timeout(remaining, self.provider.stream_generate(context, options))
                        .await
                        .map_err(|_| {
                            crate::Error::Provider(
                                "streaming response timed out before stream started".to_owned(),
                            )
                        })?
                } else {
                    self.provider.stream_generate(context, options).await
                }
            };

            let receiver_result = if let Some(token) = cancellation_token.clone() {
                tokio::select! {
                    result = stream_start => result,
                    _ = token.cancelled() => return Err(Self::interrupted_error()),
                }
            } else {
                stream_start.await
            };

            match receiver_result {
                Ok(mut rx) => {
                    let consume_stream = async {
                        let mut buffer = String::new();
                        loop {
                            let maybe_chunk = if let Some(token) = cancellation_token.clone() {
                                tokio::select! {
                                    chunk = rx.recv() => chunk,
                                    _ = token.cancelled() => return Err(Self::interrupted_error()),
                                }
                            } else {
                                rx.recv().await
                            };

                            let Some(chunk) = maybe_chunk else {
                                break;
                            };

                            let _ = event_tx.try_send(Event::ModelChunk {
                                session_id: response_meta.session_id_str.to_owned(),
                                agent: response_meta.agent_name.to_owned(),
                                text: chunk.clone(),
                            });
                            buffer.push_str(&chunk);
                        }
                        Ok::<String, crate::Error>(buffer)
                    };

                    let streamed = if let Some(remaining) = remaining_duration {
                        timeout(remaining, consume_stream).await.map_err(|_| {
                            crate::Error::Provider(
                                "streaming response timed out while consuming model output"
                                    .to_owned(),
                            )
                        })?
                    } else {
                        consume_stream.await
                    };

                    return streamed;
                }
                Err(err) => {
                    let _ = event_tx.try_send(Event::Progress(format!(
                        "provider streaming unavailable, falling back to non-streaming: {err}"
                    )));
                }
            }
        }

        let generate = async {
            if let Some(remaining) = remaining_duration {
                timeout(remaining, self.provider.generate(context, options))
                    .await
                    .map_err(|_| {
                        crate::Error::Provider(
                            "model response timed out while waiting for non-streaming output"
                                .to_owned(),
                        )
                    })?
            } else {
                self.provider.generate(context, options).await
            }
        };

        let response = if let Some(token) = cancellation_token {
            tokio::select! {
                result = generate => result?,
                _ = token.cancelled() => return Err(Self::interrupted_error()),
            }
        } else {
            generate.await?
        };

        for chunk in response.split_inclusive('\n') {
            let _ = event_tx.try_send(Event::ModelChunk {
                session_id: response_meta.session_id_str.to_owned(),
                agent: response_meta.agent_name.to_owned(),
                text: chunk.to_string(),
            });
        }

        Ok(response)
    }

    pub async fn resume_from_pending_tool(
        &self,
        session_id: uuid::Uuid,
        session_id_str: &str,
        agent_name: &str,
        event_tx: mpsc::Sender<Event>,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<()> {
        let pending = self
            .session_manager
            .get_and_clear_pending_tool(session_id)
            .await?;

        let pending = pending.ok_or_else(|| {
            crate::Error::Config("no pending tool state found to resume from".to_owned())
        })?;

        let options = self.generation_options();
        let max_tools_per_round = self.effective_max_tools_per_round();
        let max_total_tool_calls = self.effective_max_total_tool_calls_per_turn();
        let max_turn_duration = self.effective_max_turn_duration();

        let mut context = pending.context_snapshot;
        let mut tool_messages = pending.tool_messages;
        let round_index = pending.round_index;

        // Execute the previously pending tool directly
        let resumed_call = ParsedToolCall {
            tool: pending.tool_name.clone(),
            args: pending.args.clone(),
        };
        let resumed_result = self
            .execute_tool_call_and_record(
                session_id,
                session_id_str,
                agent_name,
                &resumed_call,
                event_tx.clone(),
                cancellation_token.clone(),
            )
            .await?;
        tool_messages.push(resumed_result.message);

        // If permission was denied again, store new pending state and exit
        if resumed_result.pending {
            self.persist_pending_state(
                session_id,
                &resumed_call,
                round_index,
                tool_messages,
                context,
            )
            .await?;
            return Ok(());
        }

        // Continue processing remaining tools in this round
        let max_rounds = self.effective_max_tool_rounds();
        let turn_started = Instant::now();
        let mut total_tool_calls_executed = 1usize; // Already executed 1 tool

        // We need to load the assistant response that was generated before the pending tool
        // This is the last assistant message before the tool messages
        let context_window = self
            .load_context_window_from_session(session_id, session_id_str, None, &event_tx)
            .await?;
        let assistant_response = context_window
            .iter()
            .rev()
            .find(|msg| msg.role == "assistant")
            .map(|msg| msg.content.clone())
            .unwrap_or_default();

        // Check if there are more tool calls in the assistant response
        let parsed_tool_calls = self.extract_tool_calls(&assistant_response);

        if parsed_tool_calls.is_empty() {
            // Auto-create TODOs from response when enabled
            let _ = self
                .maybe_auto_create_todos(session_id, &assistant_response)
                .await;
            return Ok(());
        }

        // Find where our pending tool was in list and continue from there
        let pending_tool_index = parsed_tool_calls
            .iter()
            .position(|call| call.tool == resumed_call.tool)
            .unwrap_or(0);

        let remaining_tools = parsed_tool_calls
            .into_iter()
            .skip(pending_tool_index + 1)
            .take(max_tools_per_round.saturating_sub(tool_messages.len()));

        for call in remaining_tools {
            if Self::enforce_tool_call_budget(
                total_tool_calls_executed,
                max_total_tool_calls,
                &event_tx,
            ) {
                break;
            }

            let exec = self
                .execute_tool_call_and_record(
                    session_id,
                    session_id_str,
                    agent_name,
                    &call,
                    event_tx.clone(),
                    cancellation_token.clone(),
                )
                .await?;
            tool_messages.push(exec.message);

            total_tool_calls_executed += 1;

            if exec.pending {
                self.persist_pending_state(
                    session_id,
                    &call,
                    round_index,
                    tool_messages,
                    context.clone(),
                )
                .await?;
                return Ok(());
            }
        }

        // Add assistant response and tool messages to context, then continue autonomous loop
        context.push(ChatMessage {
            role: "assistant".to_string(),
            content: assistant_response,
            name: None,
            tool_calls: None,
        });
        context.extend(tool_messages);

        // Continue autonomous tool loop from next round
        for r in (round_index + 1)..max_rounds {
            if Self::is_cancelled(cancellation_token.as_ref()) {
                let _ = event_tx.try_send(Event::Progress(
                    "agent turn interrupted by user request".to_owned(),
                ));
                return Err(Self::interrupted_error());
            }

            let remaining_duration =
                match Self::turn_duration_budget(turn_started, max_turn_duration, &event_tx, false)
                {
                    TurnDurationBudget::Unlimited => None,
                    TurnDurationBudget::Remaining(duration) => Some(duration),
                    TurnDurationBudget::Exhausted => break,
                };

            let response = self
                .generate_response_with_events(
                    &context,
                    &options,
                    ResponseMeta {
                        session_id_str,
                        agent_name,
                    },
                    event_tx.clone(),
                    remaining_duration,
                    cancellation_token.clone(),
                )
                .await?;

            self.session_manager
                .append_message(session_id, "assistant", &response)
                .await?;
            let mut parsed_tool_calls = self.extract_tool_calls(&response);

            if parsed_tool_calls.is_empty() {
                // Auto-create TODOs from response when enabled
                let _ = self.maybe_auto_create_todos(session_id, &response).await;
                return Ok(());
            }

            if parsed_tool_calls.len() > max_tools_per_round {
                let _ = event_tx.try_send(Event::Progress(format!(
                    "tool calls truncated to {} (requested {})",
                    max_tools_per_round,
                    parsed_tool_calls.len()
                )));
                parsed_tool_calls.truncate(max_tools_per_round);
            }

            let mut tool_messages_round = Vec::new();
            for call in parsed_tool_calls {
                if Self::enforce_tool_call_budget(
                    total_tool_calls_executed,
                    max_total_tool_calls,
                    &event_tx,
                ) {
                    return Ok(());
                }
                total_tool_calls_executed += 1;

                let exec = self
                    .execute_tool_call_and_record(
                        session_id,
                        session_id_str,
                        agent_name,
                        &call,
                        event_tx.clone(),
                        cancellation_token.clone(),
                    )
                    .await?;
                tool_messages_round.push(exec.message);

                if exec.pending {
                    self.persist_pending_state(
                        session_id,
                        &call,
                        r,
                        tool_messages_round,
                        context.clone(),
                    )
                    .await?;
                    return Ok(());
                }
            }

            context.push(ChatMessage {
                role: "assistant".to_string(),
                content: response,
                name: None,
                tool_calls: None,
            });
            context.extend(tool_messages_round);

            if r + 1 == max_rounds {
                let _ = event_tx.try_send(Event::Progress(format!(
                    "agent reached max tool rounds ({max_rounds}); stopping autonomous loop"
                )));
            }
        }

        Ok(())
    }

    pub async fn continue_after_tool(
        &self,
        session_id: uuid::Uuid,
        event_tx: mpsc::Sender<Event>,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<()> {
        let agent_name = self.config.name.clone();
        let session_id_str = session_id.to_string();

        let _ = event_tx.try_send(Event::AgentThinking {
            session_id: session_id_str.clone(),
            agent: agent_name.clone(),
        });

        // Check for pending tool state and resume if found
        let has_pending = self.session_manager.has_pending_tool(session_id).await?;

        if has_pending {
            // Resume from pending tool state
            let resumed = self
                .resume_from_pending_tool(
                    session_id,
                    &session_id_str,
                    &agent_name,
                    event_tx,
                    cancellation_token,
                )
                .await;
            if let Err(err) = resumed {
                if self.learning.enabled() {
                    let _ = self
                        .learning
                        .record_error_message(&agent_name, &err.to_string())
                        .await;
                }
                return Err(err);
            }
        } else {
            // No pending state - reload context and continue
            let mut context_window = self
                .load_context_window_from_session(session_id, &session_id_str, None, &event_tx)
                .await?;
            if let Some(query) = Self::latest_user_task(&context_window) {
                self.maybe_inject_retrieval_context(
                    &session_id_str,
                    &query,
                    &mut context_window,
                    &event_tx,
                )
                .await?;
            }
            let continued = self
                .run_assistant_tool_loop(
                    session_id,
                    &session_id_str,
                    &agent_name,
                    context_window,
                    event_tx,
                    cancellation_token,
                )
                .await;
            if let Err(err) = continued {
                if self.learning.enabled() {
                    let _ = self
                        .learning
                        .record_error_message(&agent_name, &err.to_string())
                        .await;
                }
                return Err(err);
            }
        }

        Ok(())
    }

    /// Execute a turn of the agent loop with streaming
    pub async fn start_turn(
        &self,
        session_id: uuid::Uuid,
        input: String,
        event_tx: mpsc::Sender<Event>,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<()> {
        let agent_name = self.config.name.clone();
        let session_id_str = session_id.to_string();

        // 1. Emit AgentThinking event
        let _ = event_tx.try_send(Event::AgentThinking {
            session_id: session_id_str.clone(),
            agent: agent_name.clone(),
        });

        // 2-4. Load session history and build context window
        let context_window = self
            .load_context_window_from_session(session_id, &session_id_str, Some(&input), &event_tx)
            .await?;

        // 5. Add user input to context
        let mut full_context = context_window;
        full_context.push(ChatMessage {
            role: "user".to_string(),
            content: input.clone(),
            name: None,
            tool_calls: None,
        });

        self.maybe_apply_preferred_approach(
            session_id,
            &session_id_str,
            &mut full_context,
            &event_tx,
        )
        .await?;

        self.maybe_inject_retrieval_context(&session_id_str, &input, &mut full_context, &event_tx)
            .await?;

        // 6. Append user message to session
        self.session_manager
            .append_message(session_id, "user", &input)
            .await?;

        // Auto-create TODOs for complex multi-step user tasks.
        let _ = self
            .maybe_auto_create_todos_from_input(session_id, &input)
            .await;

        // 7+. Run autonomous assistant/tool loop with configured limits
        let turn_result = self
            .run_assistant_tool_loop(
                session_id,
                &session_id_str,
                &agent_name,
                full_context,
                event_tx,
                cancellation_token,
            )
            .await;

        if self.learning.enabled() {
            if let Err(err) = &turn_result {
                let _ = self
                    .learning
                    .record_error_message(&agent_name, &err.to_string())
                    .await;
            }
        }

        turn_result
    }

    pub async fn generate_from_context(
        &self,
        mut context: Vec<ChatMessage>,
        task: &str,
        session_id: String,
        event_tx: mpsc::Sender<Event>,
        cancellation_token: Option<CancellationToken>,
    ) -> Result<String> {
        let system_prompt = self.system_prompt(Some(task)).await;
        if !context.iter().any(|message| message.role == "system") {
            context.insert(
                0,
                ChatMessage {
                    role: "system".to_owned(),
                    content: system_prompt,
                    name: None,
                    tool_calls: None,
                },
            );
        }

        context.push(ChatMessage {
            role: "user".to_owned(),
            content: task.to_owned(),
            name: None,
            tool_calls: None,
        });

        self.maybe_inject_retrieval_context(&session_id, task, &mut context, &event_tx)
            .await?;

        let response = self
            .generate_response_with_events(
                &context,
                &self.generation_options(),
                ResponseMeta {
                    session_id_str: &session_id,
                    agent_name: &self.config.name,
                },
                event_tx,
                None,
                cancellation_token,
            )
            .await;

        if self.learning.enabled() {
            if let Err(err) = &response {
                if let Ok(parsed_session_id) = uuid::Uuid::parse_str(&session_id) {
                    let _ = self
                        .learning
                        .record_implicit_event(
                            parsed_session_id,
                            &self.config.name,
                            &Event::Error(err.to_string()),
                            Some(task.to_owned()),
                        )
                        .await;
                }
            }
        }

        response
    }

    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    pub async fn record_summary_feedback(
        &self,
        session_id: &str,
        summary_key: &str,
        accepted: bool,
    ) -> Option<Event> {
        self.memory
            .record_summary_quality(summary_key, accepted)
            .await;
        self.memory
            .get_summary_quality(summary_key)
            .await
            .map(|(acceptance_count, _)| {
                self.memory.create_summary_quality_updated_event(
                    session_id.to_string(),
                    summary_key.to_string(),
                    if accepted { 1 } else { -1 },
                    false,
                    acceptance_count,
                )
            })
    }

    /// Auto-create TODOs from agent response when enabled
    ///
    /// Parses markdown checklist items or lines starting with TODO: or - [ ]
    /// and creates session TODOs. When todo_project_scope is true, also creates
    /// a project-scoped parent TODO and links child TODOs via parent_id.
    async fn maybe_auto_create_todos(&self, session_id: uuid::Uuid, response: &str) -> Result<()> {
        todo_extractor::auto_create_todos_from_response(
            self.session_manager.as_ref(),
            &self.config,
            session_id,
            response,
        )
        .await
    }

    async fn maybe_auto_create_todos_from_input(
        &self,
        session_id: uuid::Uuid,
        input: &str,
    ) -> Result<()> {
        todo_extractor::auto_create_todos_from_input(
            self.session_manager.as_ref(),
            &self.config,
            session_id,
            input,
        )
        .await
    }
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("config", &self.config)
            .field("memory", &self.memory)
            .field("provider", &"<dyn ModelProvider>")
            .field("tool_manager", &self.tool_manager)
            .field("session_manager", &"<SessionManager>")
            .finish()
    }
}

fn estimate_context_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|message| {
            std::cmp::max(1, message.role.len() / 4) + std::cmp::max(1, message.content.len() / 4)
        })
        .sum()
}
