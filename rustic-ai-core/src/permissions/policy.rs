use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AskResolution {
    AllowOnce,
    AllowInSession,
    Deny,
}

pub trait PermissionPolicy: Send + Sync {
    fn check_tool_permission(&self, tool: &str, args: &serde_json::Value) -> PermissionDecision;
    fn record_permission(&mut self, tool: &str, args: &serde_json::Value, decision: AskResolution);
}
