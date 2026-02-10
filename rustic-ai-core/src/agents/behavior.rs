use crate::agents::memory::AgentMemory;
use crate::config::schema::AgentConfig;
use crate::conversation::session_manager::SessionManager;
use crate::error::Result;
use crate::events::Event;
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};
use crate::ToolManager;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct Agent {
    config: AgentConfig,
    memory: AgentMemory,
    provider: Arc<dyn ModelProvider>,
    tool_manager: Arc<ToolManager>,
    session_manager: Arc<SessionManager>,
}

impl Agent {
    pub fn new(
        config: AgentConfig,
        provider: Arc<dyn ModelProvider>,
        tool_manager: Arc<ToolManager>,
        session_manager: Arc<SessionManager>,
    ) -> Self {
        let memory = AgentMemory::new(config.context_window_size);

        Self {
            config,
            memory,
            provider,
            tool_manager,
            session_manager,
        }
    }

    fn system_prompt(&self) -> String {
        if let Some(ref template) = self.config.system_prompt_template {
            template.clone()
        } else {
            "You are a helpful AI assistant.".to_string()
        }
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

        // 2. Load session history
        let messages = self
            .session_manager
            .get_session_messages(session_id)
            .await?;

        // 3. Convert storage Message to provider ChatMessage
        let chat_messages: Vec<ChatMessage> = messages
            .into_iter()
            .map(|msg| ChatMessage {
                role: msg.role,
                content: msg.content,
            })
            .collect();

        // 4. Build context window
        let system_prompt = self.system_prompt();
        let context_window = self
            .memory
            .build_context_window(chat_messages, &system_prompt)
            .await?;

        // 5. Add user input to context
        let mut full_context = context_window;
        full_context.push(ChatMessage {
            role: "user".to_string(),
            content: input.clone(),
        });

        // 6. Append user message to session
        self.session_manager
            .append_message(session_id, "user", &input)
            .await?;

        // 7. Call provider
        let options = GenerateOptions {
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
        };

        let response = self.provider.generate(&full_context, &options).await?;

        // 8. Stream model response
        for chunk in response.split_inclusive('\n') {
            let _ = event_tx.try_send(Event::ModelChunk {
                session_id: session_id_str.clone(),
                agent: agent_name.clone(),
                text: chunk.to_string(),
            });
        }

        // 9. Parse response for tool calls
        // For now, this is a simple implementation. In Phase 3,
        // we'll add proper tool call parsing from the provider response.
        // The current approach assumes the provider response may contain
        // tool invocation instructions that the agent needs to interpret.

        // For the current implementation, we'll assume the model response
        // is just text. Tool call parsing will be added when we have
        // a standardized tool calling format.

        // 10. Append assistant message to session
        self.session_manager
            .append_message(session_id, "assistant", &response)
            .await?;

        // 11. Parse and execute tool calls (placeholder)
        // TODO: Implement proper tool call parsing in Phase 3
        // For now, we'll skip tool execution to get the basic flow working

        Ok(())
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
