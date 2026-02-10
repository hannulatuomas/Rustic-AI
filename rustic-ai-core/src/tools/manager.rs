use crate::config::schema::{PermissionConfig, ToolConfig};
use crate::error::{Error, Result};
use crate::events::Event;
use crate::permissions::{
    AskResolution, CommandPatternBucket, PermissionContext, PermissionDecision, PermissionPolicy,
};
use crate::tools::{
    filesystem::FilesystemTool, http::HttpTool, shell::ShellTool, Tool, ToolExecutionContext,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct ToolManager {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
    tool_configs: Arc<RwLock<HashMap<String, ToolConfig>>>,
    permission_policy: Arc<RwLock<Box<dyn PermissionPolicy + Send + Sync>>>,
    execution_context: ToolExecutionContext,
}

impl ToolManager {
    pub fn new(
        permission_policy: Box<dyn PermissionPolicy + Send + Sync>,
        permission_config: Arc<PermissionConfig>,
        tool_configs: Vec<ToolConfig>,
        execution_context: ToolExecutionContext,
    ) -> Self {
        let mut tools = HashMap::new();
        let mut configs = HashMap::new();

        for config in tool_configs {
            if !config.enabled {
                continue;
            }

            let tool: Arc<dyn Tool> = match config.name.as_str() {
                "shell" => Arc::new(ShellTool::new(
                    config.clone(),
                    permission_config.sudo_cache_ttl_secs,
                )),
                "filesystem" => Arc::new(FilesystemTool::new(config.clone())),
                "http" => Arc::new(HttpTool::new(config.clone())),
                _ => {
                    // For now, skip unknown tools
                    continue;
                }
            };

            tools.insert(config.name.clone(), tool);
            configs.insert(config.name.clone(), config);
        }

        Self {
            tools: Arc::new(RwLock::new(tools)),
            tool_configs: Arc::new(RwLock::new(configs)),
            permission_policy: Arc::new(RwLock::new(permission_policy)),
            execution_context,
        }
    }

    pub async fn register_tool(&self, name: String, tool: Arc<dyn Tool>, config: ToolConfig) {
        let mut tools = self.tools.write().await;
        let mut configs = self.tool_configs.write().await;
        tools.insert(name.clone(), tool);
        configs.insert(name, config);
    }

    pub async fn has_tool(&self, name: &str) -> bool {
        self.tools.read().await.contains_key(name)
    }

    pub async fn get_tool_config(&self, name: &str) -> Option<ToolConfig> {
        self.tool_configs.read().await.get(name).cloned()
    }

    /// Execute a tool with permission checking and streaming
    ///
    /// Returns None if permission was denied
    /// Returns Some(ToolResult) if execution completed
    pub async fn execute_tool(
        &self,
        session_id: String,
        agent_name: Option<String>,
        tool_name: &str,
        args: serde_json::Value,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<Option<crate::tools::ToolResult>> {
        // Check if tool exists
        let tool = {
            let tools = self.tools.read().await;
            tools.get(tool_name).cloned()
        };

        let tool = tool.ok_or_else(|| Error::Tool(format!("tool '{tool_name}' not found")))?;

        let permission_context = PermissionContext {
            session_id: session_id.clone(),
            agent_name,
            working_directory: self.execution_context.working_directory.clone(),
        };

        // Check permission
        let permission = {
            let policy = self.permission_policy.read().await;
            policy.check_tool_permission(tool_name, &args, &permission_context)
        };

        match permission {
            PermissionDecision::Allow => {
                // Execute directly
                let result = tool
                    .stream_execute(args.clone(), event_tx, &self.execution_context)
                    .await?;
                Ok(Some(result))
            }
            PermissionDecision::Deny => {
                // Emit decision and return None
                let _ = event_tx.try_send(Event::PermissionDecision {
                    session_id,
                    tool: tool_name.to_string(),
                    decision: AskResolution::Deny,
                });
                Ok(None)
            }
            PermissionDecision::Ask => {
                // Emit request - caller must handle this and call resolve_permission
                let _ = event_tx.try_send(Event::PermissionRequest {
                    session_id,
                    tool: tool_name.to_string(),
                    args,
                });
                Ok(None)
            }
        }
    }

    /// Resolve a permission decision and optionally execute the tool
    pub async fn resolve_permission(
        &self,
        session_id: String,
        agent_name: Option<String>,
        tool_name: &str,
        args: serde_json::Value,
        decision: AskResolution,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<Option<crate::tools::ToolResult>> {
        let permission_context = PermissionContext {
            session_id: session_id.clone(),
            agent_name,
            working_directory: self.execution_context.working_directory.clone(),
        };

        // Record decision in policy
        {
            let mut policy = self.permission_policy.write().await;
            policy.record_permission(tool_name, &args, &permission_context, decision);
        }

        // Emit decision event
        let _ = event_tx.try_send(Event::PermissionDecision {
            session_id,
            tool: tool_name.to_string(),
            decision,
        });

        // If denied, return None
        if matches!(decision, AskResolution::Deny) {
            return Ok(None);
        }

        // Execute tool
        let tool = {
            let tools = self.tools.read().await;
            tools.get(tool_name).cloned()
        };

        let tool = tool.ok_or_else(|| Error::Tool(format!("tool '{tool_name}' not found")))?;

        let result = tool
            .stream_execute(args, event_tx, &self.execution_context)
            .await?;
        Ok(Some(result))
    }

    pub async fn add_session_allowed_path(&self, session_id: &str, path: &str) {
        let mut policy = self.permission_policy.write().await;
        policy.add_session_allowed_path(session_id, path);
    }

    pub async fn add_session_command_pattern(
        &self,
        session_id: &str,
        bucket: CommandPatternBucket,
        pattern: &str,
    ) {
        let mut policy = self.permission_policy.write().await;
        policy.add_session_command_pattern(session_id, bucket, pattern);
    }

    pub async fn add_global_allowed_path(&self, path: &str) {
        let mut policy = self.permission_policy.write().await;
        policy.add_global_allowed_path(path);
    }

    pub async fn add_project_allowed_path(&self, path: &str) {
        let mut policy = self.permission_policy.write().await;
        policy.add_project_allowed_path(path);
    }

    pub async fn add_global_command_pattern(&self, bucket: CommandPatternBucket, pattern: &str) {
        let mut policy = self.permission_policy.write().await;
        policy.add_global_command_pattern(bucket, pattern);
    }

    pub async fn add_project_command_pattern(&self, bucket: CommandPatternBucket, pattern: &str) {
        let mut policy = self.permission_policy.write().await;
        policy.add_project_command_pattern(bucket, pattern);
    }
}

impl std::fmt::Debug for ToolManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolManager")
            .field("tools", &"<tools>")
            .field("tool_configs", &"<tool_configs>")
            .field("permission_policy", &"<dyn PermissionPolicy>")
            .field("execution_context", &self.execution_context)
            .finish()
    }
}
