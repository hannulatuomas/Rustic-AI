use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct GenerateOptions {
    pub temperature: f32,
    pub max_tokens: usize,
    pub top_p: Option<f32>,
    pub top_k: Option<usize>,
    pub stop_sequences: Option<Vec<String>>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub call_type: String,
    pub function_name: String,
    pub arguments_json: String,
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn generate(&self, messages: &[ChatMessage], options: &GenerateOptions)
        -> Result<String>;

    async fn stream_generate(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
    ) -> Result<mpsc::Receiver<String>> {
        let _ = (messages, options);
        Err(Error::Provider(format!(
            "provider '{}' does not support streaming",
            self.name()
        )))
    }

    async fn count_tokens(&self, messages: &[ChatMessage]) -> Result<usize> {
        let token_estimate = messages
            .iter()
            .map(|message| {
                let role_tokens = std::cmp::max(1, message.role.len() / 4);
                let content_tokens = std::cmp::max(1, message.content.chars().count() / 4);
                let name_tokens = message
                    .name
                    .as_ref()
                    .map(|name| std::cmp::max(1, name.len() / 4))
                    .unwrap_or(0);
                let tool_call_tokens = message
                    .tool_calls
                    .as_ref()
                    .map(|calls| {
                        calls
                            .iter()
                            .map(|call| {
                                std::cmp::max(1, call.id.len() / 4)
                                    + std::cmp::max(1, call.call_type.len() / 4)
                                    + std::cmp::max(1, call.function_name.len() / 4)
                                    + std::cmp::max(1, call.arguments_json.chars().count() / 4)
                            })
                            .sum::<usize>()
                    })
                    .unwrap_or(0);

                role_tokens + content_tokens + name_tokens + tool_call_tokens
            })
            .sum();

        Ok(token_estimate)
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    fn supports_functions(&self) -> bool {
        false
    }
}

impl std::fmt::Debug for dyn ModelProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ModelProvider").field(&"<dyn>").finish()
    }
}
