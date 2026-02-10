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
