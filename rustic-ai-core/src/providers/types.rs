use async_trait::async_trait;

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct GenerateOptions {
    pub temperature: f32,
    pub max_tokens: usize,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn generate(&self, messages: &[ChatMessage], options: &GenerateOptions)
        -> Result<String>;
}
