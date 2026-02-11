use async_trait::async_trait;
use std::path::PathBuf;
use uuid::Uuid;

use crate::config::schema::AgentPermissionMode;
use crate::error::Result;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct ToolExecutionContext {
    pub working_directory: PathBuf,
    pub session_id: Option<Uuid>,
    pub agent_name: Option<String>,
    pub agent_permission_mode: AgentPermissionMode,
    pub sub_agent_depth: usize,
    pub cancellation_token: Option<CancellationToken>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> &serde_json::Value;

    /// Batch execution mode
    async fn execute(
        &self,
        args: serde_json::Value,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult>;

    /// Streaming execution mode - emits events via the provided channel
    async fn stream_execute(
        &self,
        args: serde_json::Value,
        tx: mpsc::Sender<crate::events::Event>,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult>;
}
