use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandPatternBucket {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone)]
pub struct PermissionContext {
    pub session_id: String,
    pub agent_name: Option<String>,
    pub working_directory: PathBuf,
}

pub trait PermissionPolicy: Send + Sync {
    fn check_tool_permission(
        &self,
        tool: &str,
        args: &serde_json::Value,
        context: &PermissionContext,
    ) -> PermissionDecision;
    fn record_permission(
        &mut self,
        tool: &str,
        args: &serde_json::Value,
        context: &PermissionContext,
        decision: AskResolution,
    );

    fn add_session_allowed_path(&mut self, _session_id: &str, _path: &str) {}

    fn add_session_command_pattern(
        &mut self,
        _session_id: &str,
        _bucket: CommandPatternBucket,
        _pattern: &str,
    ) {
    }

    fn add_global_allowed_path(&mut self, _path: &str) {}

    fn add_project_allowed_path(&mut self, _path: &str) {}

    fn add_global_command_pattern(&mut self, _bucket: CommandPatternBucket, _pattern: &str) {}

    fn add_project_command_pattern(&mut self, _bucket: CommandPatternBucket, _pattern: &str) {}
}
