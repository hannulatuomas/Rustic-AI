use crate::error::Result;
use crate::providers::types::ChatMessage;

#[derive(Debug, Clone)]
pub struct AgentMemory {
    context_window_size: usize,
}

impl AgentMemory {
    pub fn new(context_window_size: usize) -> Self {
        Self {
            context_window_size,
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

        // Add messages from newest to oldest until we hit the token limit
        // This is a simple LIFO (last-in-first-out) approach
        for message in messages.into_iter().rev() {
            let message_tokens = message.content.chars().count() / 4;
            if message_tokens > remaining_tokens {
                break;
            }

            // Insert at the beginning (after system prompt) so we maintain order
            // But since we're iterating in reverse, we need to push then reverse at end
            context.push(message);
        }

        // Reverse to get correct order (oldest after system, newest last)
        context[1..].reverse();
        Ok(context)
    }
}
