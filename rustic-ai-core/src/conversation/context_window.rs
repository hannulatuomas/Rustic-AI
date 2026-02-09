use crate::error::Result;
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};

#[derive(Debug, Clone)]
pub struct ContextWindowManager {
    pub max_context_tokens: usize,
    pub summary_max_tokens: usize,
    pub summarization_enabled: bool,
}

impl ContextWindowManager {
    pub fn new(
        max_context_tokens: usize,
        summary_max_tokens: usize,
        summarization_enabled: bool,
    ) -> Self {
        Self {
            max_context_tokens,
            summary_max_tokens,
            summarization_enabled,
        }
    }

    pub async fn build_context(
        &self,
        messages: &[ChatMessage],
        provider: Option<&dyn ModelProvider>,
    ) -> Result<Vec<ChatMessage>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }

        let mut running_tokens = 0usize;
        let mut selected = Vec::new();

        for message in messages.iter().rev() {
            let message_tokens = estimate_tokens(&message.content);
            if running_tokens + message_tokens > self.max_context_tokens {
                break;
            }
            running_tokens += message_tokens;
            selected.push(message.clone());
        }

        selected.reverse();

        if selected.len() == messages.len() {
            return Ok(selected);
        }

        if !self.summarization_enabled {
            return Ok(selected);
        }

        let Some(provider) = provider else {
            return Ok(selected);
        };

        let truncated_count = messages.len().saturating_sub(selected.len());
        if truncated_count == 0 {
            return Ok(selected);
        }

        let summary = self
            .summarize(&messages[..truncated_count], provider)
            .await?;
        let mut final_context = Vec::with_capacity(selected.len() + 1);
        final_context.push(ChatMessage {
            role: "system".to_owned(),
            content: format!("Conversation summary (older context): {summary}"),
        });
        final_context.extend(selected);
        Ok(final_context)
    }

    async fn summarize(
        &self,
        messages: &[ChatMessage],
        provider: &dyn ModelProvider,
    ) -> Result<String> {
        let mut summary_messages = vec![ChatMessage {
            role: "system".to_owned(),
            content: "Summarize this conversation history focusing on goals, decisions, and unresolved items. Return plain text only.".to_owned(),
        }];

        summary_messages.extend(messages.iter().cloned());
        provider
            .generate(
                &summary_messages,
                &GenerateOptions {
                    temperature: 0.2,
                    max_tokens: self.summary_max_tokens,
                },
            )
            .await
    }
}

fn estimate_tokens(content: &str) -> usize {
    let chars = content.chars().count();
    std::cmp::max(1, chars / 4)
}
