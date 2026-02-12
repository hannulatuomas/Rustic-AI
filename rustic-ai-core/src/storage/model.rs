use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::providers::types::ChatMessage;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,
    pub agent_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SessionConfig {
    pub summarization_enabled: Option<bool>,
    pub summarization_provider_name: Option<String>,
    pub summary_max_tokens: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: Uuid,
    pub session_id: Uuid,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

/// Pending tool execution state that persists across process restarts
///
/// When an agent encounters a tool that requires permission (Ask decision),
/// this state is saved before the tool loop exits. After the user approves
/// the permission, the agent can resume exactly where it left off without
/// re-generating the entire assistant response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingToolState {
    pub session_id: Uuid,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub round_index: usize,
    pub tool_messages: Vec<ChatMessage>,
    pub context_snapshot: Vec<ChatMessage>,
    pub created_at: DateTime<Utc>,
}

/// Status of a TODO item
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Todo,
    InProgress,
    Blocked,
    Completed,
    Cancelled,
}

/// Priority level of a TODO item
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoPriority {
    Low,
    Medium,
    High,
    Critical,
}

/// Additional metadata associated with a TODO item
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TodoMetadata {
    pub files: Vec<String>,
    pub tools: Vec<String>,
    pub routing_trace_id: Option<Uuid>,
    pub sub_agent_output_id: Option<Uuid>,
    pub summary_id: Option<String>,
    pub reason: Option<String>,
}

/// A TODO item that can be session-scoped or project-scoped
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: Uuid,
    pub project_id: Option<String>,
    pub session_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub description: Option<String>,
    pub status: TodoStatus,
    pub priority: TodoPriority,
    pub tags: Vec<String>,
    pub metadata: TodoMetadata,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Filter options for listing TODOs
#[derive(Debug, Clone, Default)]
pub struct TodoFilter {
    pub session_id: Option<Uuid>,
    pub project_id: Option<String>,
    pub parent_id: Option<Uuid>,
    pub status: Option<TodoStatus>,
    pub priority: Option<TodoPriority>,
    pub tags: Option<Vec<String>>,
    pub limit: Option<usize>,
}

/// Update options for a TODO item
#[derive(Debug, Clone, Default)]
pub struct TodoUpdate {
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub status: Option<TodoStatus>,
    pub priority: Option<TodoPriority>,
    pub tags: Option<Option<Vec<String>>>,
    pub metadata: Option<TodoMetadata>,
}

/// Cached output from sub-agent execution for reuse
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentOutput {
    pub id: Uuid,
    pub caller_agent: String,
    pub target_agent: String,
    pub task_key: String,
    pub task_type: Option<String>,
    pub output: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
}

/// Filter options for listing sub-agent outputs
#[derive(Debug, Clone, Default)]
pub struct SubAgentOutputFilter {
    pub caller_agent: Option<String>,
    pub target_agent: Option<String>,
    pub task_key: Option<String>,
    pub task_type: Option<String>,
    pub exclude_expired: bool,
    pub limit: Option<usize>,
}

/// Routing trace for dynamic routing decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingTrace {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task: String,
    pub selected_agent: String,
    pub reason: String,
    pub confidence: f32,
    pub policy: String,
    pub alternatives: Vec<String>,
    pub fallback_used: bool,
    pub context_pressure: Option<f32>,
    pub created_at: DateTime<Utc>,
}

/// Filter options for listing routing traces
#[derive(Debug, Clone, Default)]
pub struct RoutingTraceFilter {
    pub session_id: Option<Uuid>,
    pub selected_agent: Option<String>,
    pub min_confidence: Option<f32>,
    pub fallback_only: bool,
    pub limit: Option<usize>,
}
