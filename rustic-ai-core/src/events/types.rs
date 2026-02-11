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
    WorkflowStarted {
        workflow: String,
        entrypoint: String,
        recursion_depth: usize,
    },
    WorkflowStepStarted {
        workflow: String,
        step_id: String,
        step_name: String,
        kind: String,
    },
    WorkflowStepCompleted {
        workflow: String,
        step_id: String,
        success: bool,
        output_count: usize,
    },
    WorkflowCompleted {
        workflow: String,
        success: bool,
        steps_executed: usize,
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
    /// Secret prompt for privileged commands like sudo
    /// Password is sent securely via stdin, never echoed or logged
    SudoSecretPrompt {
        session_id: String,
        tool: String,
        args: serde_json::Value,
        command: String,
        reason: String,
    },
    SubAgentCallStarted {
        session_id: String,
        caller_agent: String,
        target_agent: String,
        max_context_messages: usize,
    },
    SubAgentCallCompleted {
        session_id: String,
        caller_agent: String,
        target_agent: String,
        success: bool,
    },
    SessionUpdated(String),
    Error(String),
}
