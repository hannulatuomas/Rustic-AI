use crate::agents::memory::AgentMemory;
use crate::config::schema::AgentConfig;
use crate::conversation::session_manager::SessionManager;
use crate::error::Result;
use crate::events::Event;
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};
use crate::storage::PendingToolState;
use crate::ToolManager;
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration, Instant};

const DEFAULT_MAX_TOOL_ROUNDS: usize = 4;
const DEFAULT_MAX_TOOLS_PER_ROUND: usize = 8;
const DEFAULT_MAX_TOTAL_TOOL_CALLS_PER_TURN: usize = 24;
const DEFAULT_MAX_TURN_DURATION_SECONDS: u64 = 300;
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

impl Agent {
    pub fn new(
        config: AgentConfig,
        provider: Arc<dyn ModelProvider>,
        tool_manager: Arc<ToolManager>,
        session_manager: Arc<SessionManager>,
    ) -> Self {
        let memory = AgentMemory::new(
            config.context_window_size,
            config.context_summary_enabled.unwrap_or(true),
            config.context_summary_max_tokens,
            config.context_summary_cache_entries,
        );

        Self {
            config,
            memory,
            provider,
            tool_manager,
            session_manager,
        }
    }

    fn system_prompt(&self) -> String {
        let base_prompt = if let Some(ref template) = self.config.system_prompt_template {
            template.clone()
        } else {
            "You are a helpful AI assistant.".to_string()
        };

        if self.config.tools.is_empty() {
            return base_prompt;
        }

        let tools = self.config.tools.join(", ");
        format!(
            "{base_prompt}\n\nWhen you need a tool, emit a single-line JSON object only with shape: {{\"tool\":\"<tool_name>\",\"args\":{{...}}}}. Available tools: {tools}."
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
            .execute_tool(
                session_id_str.to_owned(),
                Some(agent_name.to_owned()),
                &call.tool,
                call.args.clone(),
                event_tx,
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

        let system_prompt = self.system_prompt();
        let mut context = self
            .memory
            .build_context_window(chat_messages, &system_prompt, Some(self.provider.as_ref()))
            .await?;

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
    ) -> Result<()> {
        let options = self.generation_options();
        let max_rounds = self.effective_max_tool_rounds();
        let max_tools_per_round = self.effective_max_tools_per_round();
        let max_total_tool_calls = self.effective_max_total_tool_calls_per_turn();
        let max_turn_duration = self.effective_max_turn_duration();
        let turn_started = Instant::now();
        let mut total_tool_calls_executed = 0usize;

        for round_index in 0..max_rounds {
            // Save context snapshot before generating assistant response
            // This will be used to resume after permission approval
            let context_snapshot = context.clone();

            let remaining_duration = if let Some(max_duration) = max_turn_duration {
                if turn_started.elapsed() >= max_duration {
                    let _ = event_tx.try_send(Event::Progress(format!(
                        "agent reached max turn duration ({}s); stopping autonomous loop",
                        max_duration.as_secs()
                    )));
                    return Ok(());
                }
                max_duration.checked_sub(turn_started.elapsed())
            } else {
                None
            };

            let response = self
                .generate_response_with_events(
                    &context,
                    &options,
                    session_id_str,
                    agent_name,
                    event_tx.clone(),
                    remaining_duration,
                )
                .await?;

            self.session_manager
                .append_message(session_id, "assistant", &response)
                .await?;

            let mut parsed_tool_calls = self.extract_tool_calls(&response);
            if parsed_tool_calls.is_empty() {
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
                if total_tool_calls_executed >= max_total_tool_calls {
                    let _ = event_tx.try_send(Event::Progress(format!(
                        "agent reached max total tool calls per turn ({max_total_tool_calls}); stopping autonomous loop"
                    )));
                    return Ok(());
                }
                total_tool_calls_executed += 1;

                if !self.config.tools.iter().any(|name| name == &call.tool) {
                    let message = format!(
                        "tool '{}' is not allowed for agent '{}'",
                        call.tool, self.config.name
                    );
                    self.session_manager
                        .append_message(session_id, "tool", &message)
                        .await?;
                    tool_messages.push(ChatMessage {
                        role: "tool".to_string(),
                        content: message,
                        name: Some(call.tool.clone()),
                        tool_calls: None,
                    });
                    continue;
                }

                let tool_result = if let Some(max_duration) = max_turn_duration {
                    if turn_started.elapsed() >= max_duration {
                        let _ = event_tx.try_send(Event::Progress(format!(
                            "agent reached max turn duration ({}s); stopping autonomous loop",
                            max_duration.as_secs()
                        )));
                        return Ok(());
                    }

                    let remaining = max_duration
                        .checked_sub(turn_started.elapsed())
                        .unwrap_or(Duration::from_secs(0));
                    if remaining.is_zero() {
                        let _ = event_tx.try_send(Event::Progress(format!(
                            "agent reached max turn duration ({}s); stopping autonomous loop",
                            max_duration.as_secs()
                        )));
                        return Ok(());
                    }

                    match timeout(
                        remaining,
                        self.tool_manager.execute_tool(
                            session_id_str.to_owned(),
                            Some(agent_name.to_owned()),
                            &call.tool,
                            call.args.clone(),
                            event_tx.clone(),
                        ),
                    )
                    .await
                    {
                        Ok(result) => result?,
                        Err(_) => {
                            let _ = event_tx.try_send(Event::Progress(format!(
                                "agent reached max turn duration ({}s) while waiting on tool; stopping autonomous loop",
                                max_duration.as_secs()
                            )));
                            return Ok(());
                        }
                    }
                } else {
                    self.tool_manager
                        .execute_tool(
                            session_id_str.to_owned(),
                            Some(agent_name.to_owned()),
                            &call.tool,
                            call.args.clone(),
                            event_tx.clone(),
                        )
                        .await?
                };

                let tool_message = match &tool_result {
                    Some(result) => {
                        format!(
                            "{{\"tool\":\"{}\",\"success\":{},\"exit_code\":{},\"output\":{}}}",
                            call.tool,
                            result.success,
                            result.exit_code.unwrap_or_default(),
                            serde_json::to_string(&result.output)
                                .unwrap_or_else(|_| "\"\"".to_string())
                        )
                    }
                    None => format!(
                        "tool '{}' requires user input before execution and is pending",
                        call.tool
                    ),
                };

                self.session_manager
                    .append_message(session_id, "tool", &tool_message)
                    .await?;

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
        session_id_str: &str,
        agent_name: &str,
        event_tx: mpsc::Sender<Event>,
        remaining_duration: Option<Duration>,
    ) -> Result<String> {
        if self.provider.supports_streaming() {
            let receiver_result = if let Some(remaining) = remaining_duration {
                timeout(remaining, self.provider.stream_generate(context, options))
                    .await
                    .map_err(|_| {
                        crate::Error::Provider(
                            "streaming response timed out before stream started".to_owned(),
                        )
                    })?
            } else {
                self.provider.stream_generate(context, options).await
            };

            match receiver_result {
                Ok(mut rx) => {
                    let consume_stream = async {
                        let mut buffer = String::new();
                        while let Some(chunk) = rx.recv().await {
                            let _ = event_tx.try_send(Event::ModelChunk {
                                session_id: session_id_str.to_owned(),
                                agent: agent_name.to_owned(),
                                text: chunk.clone(),
                            });
                            buffer.push_str(&chunk);
                        }
                        buffer
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

                    return Ok(streamed);
                }
                Err(err) => {
                    let _ = event_tx.try_send(Event::Progress(format!(
                        "provider streaming unavailable, falling back to non-streaming: {err}"
                    )));
                }
            }
        }

        let response = if let Some(remaining) = remaining_duration {
            timeout(remaining, self.provider.generate(context, options))
                .await
                .map_err(|_| {
                    crate::Error::Provider(
                        "model response timed out while waiting for non-streaming output"
                            .to_owned(),
                    )
                })??
        } else {
            self.provider.generate(context, options).await?
        };

        for chunk in response.split_inclusive('\n') {
            let _ = event_tx.try_send(Event::ModelChunk {
                session_id: session_id_str.to_owned(),
                agent: agent_name.to_owned(),
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
        let context_window = self.load_context_window_from_session(session_id).await?;
        let assistant_response = context_window
            .iter()
            .rev()
            .find(|msg| msg.role == "assistant")
            .map(|msg| msg.content.clone())
            .unwrap_or_default();

        // Check if there are more tool calls in the assistant response
        let parsed_tool_calls = self.extract_tool_calls(&assistant_response);

        // Find where our pending tool was in the list and continue from there
        let pending_tool_index = parsed_tool_calls
            .iter()
            .position(|call| call.tool == resumed_call.tool)
            .unwrap_or(0);

        let remaining_tools = parsed_tool_calls
            .into_iter()
            .skip(pending_tool_index + 1)
            .take(max_tools_per_round.saturating_sub(tool_messages.len()));

        for call in remaining_tools {
            if total_tool_calls_executed >= max_total_tool_calls {
                let _ = event_tx.try_send(Event::Progress(format!(
                    "agent reached max total tool calls per turn ({max_total_tool_calls}); stopping autonomous loop"
                )));
                break;
            }

            let exec = self
                .execute_tool_call_and_record(
                    session_id,
                    session_id_str,
                    agent_name,
                    &call,
                    event_tx.clone(),
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
            let remaining_duration = if let Some(max_duration) = max_turn_duration {
                if turn_started.elapsed() >= max_duration {
                    let _ = event_tx.try_send(Event::Progress(format!(
                        "agent reached max turn duration ({}s); stopping autonomous loop",
                        max_duration.as_secs()
                    )));
                    break;
                }
                max_duration.checked_sub(turn_started.elapsed())
            } else {
                None
            };

            let response = self
                .generate_response_with_events(
                    &context,
                    &options,
                    session_id_str,
                    agent_name,
                    event_tx.clone(),
                    remaining_duration,
                )
                .await?;

            self.session_manager
                .append_message(session_id, "assistant", &response)
                .await?;

            let mut parsed_tool_calls = self.extract_tool_calls(&response);
            if parsed_tool_calls.is_empty() {
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
                if total_tool_calls_executed >= max_total_tool_calls {
                    let _ = event_tx.try_send(Event::Progress(format!(
                        "agent reached max total tool calls per turn ({max_total_tool_calls}); stopping autonomous loop"
                    )));
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
            self.resume_from_pending_tool(session_id, &session_id_str, &agent_name, event_tx)
                .await?;
        } else {
            // No pending state - reload context and continue
            let context_window = self.load_context_window_from_session(session_id).await?;
            self.run_assistant_tool_loop(
                session_id,
                &session_id_str,
                &agent_name,
                context_window,
                event_tx,
            )
            .await?;
        }

        Ok(())
    }

    /// Execute a turn of the agent loop with streaming
    pub async fn start_turn(
        &self,
        session_id: uuid::Uuid,
        input: String,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<()> {
        let agent_name = self.config.name.clone();
        let session_id_str = session_id.to_string();

        // 1. Emit AgentThinking event
        let _ = event_tx.try_send(Event::AgentThinking {
            session_id: session_id_str.clone(),
            agent: agent_name.clone(),
        });

        // 2-4. Load session history and build context window
        let context_window = self.load_context_window_from_session(session_id).await?;

        // 5. Add user input to context
        let mut full_context = context_window;
        full_context.push(ChatMessage {
            role: "user".to_string(),
            content: input.clone(),
            name: None,
            tool_calls: None,
        });

        // 6. Append user message to session
        self.session_manager
            .append_message(session_id, "user", &input)
            .await?;

        // 7+. Run autonomous assistant/tool loop with configured limits
        self.run_assistant_tool_loop(
            session_id,
            &session_id_str,
            &agent_name,
            full_context,
            event_tx,
        )
        .await
    }

    pub async fn generate_from_context(
        &self,
        mut context: Vec<ChatMessage>,
        task: &str,
        session_id: String,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<String> {
        let system_prompt = self.system_prompt();
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

        self.generate_response_with_events(
            &context,
            &self.generation_options(),
            &session_id,
            &self.config.name,
            event_tx,
            None,
        )
        .await
    }

    pub fn config(&self) -> &AgentConfig {
        &self.config
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
