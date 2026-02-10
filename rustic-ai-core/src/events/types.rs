use crate::permissions::AskResolution;

#[derive(Debug, Clone)]
pub enum Event {
    Progress(String),
    ModelChunk {
        session_id: String,
        agent: String,
        text: String,
    },
    AgentThinking {
        session_id: String,
        agent: String,
    },
    ToolStarted {
        tool: String,
        args: serde_json::Value,
    },
    ToolOutput {
        tool: String,
        stdout_chunk: String,
        stderr_chunk: String,
    },
    ToolCompleted {
        tool: String,
        exit_code: i32,
    },
    PermissionRequest {
        session_id: String,
        tool: String,
        args: serde_json::Value,
    },
    PermissionDecision {
        session_id: String,
        tool: String,
        decision: AskResolution,
    },
    SessionUpdated(String),
    Error(String),
}
