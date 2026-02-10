use async_trait::async_trait;

use crate::error::Result;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub output: String,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> &serde_json::Value;

    /// Batch execution mode
    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult>;

    /// Streaming execution mode - emits events via the provided channel
    async fn stream_execute(
        &self,
        args: serde_json::Value,
        tx: mpsc::Sender<crate::events::Event>,
    ) -> Result<ToolResult>;
}
