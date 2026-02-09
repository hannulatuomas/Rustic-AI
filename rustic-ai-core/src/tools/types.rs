use async_trait::async_trait;

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult>;
}
