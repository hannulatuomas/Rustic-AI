use crate::config::schema::{McpConfig, PermissionConfig, PluginConfig, ToolConfig};
use crate::error::{Error, Result};
use crate::events::Event;
use crate::permissions::{
    AskResolution, CommandPatternBucket, PermissionContext, PermissionDecision, PermissionPolicy,
};
use crate::tools::plugin::PluginLoader;
use crate::tools::{
    filesystem::FilesystemTool, http::HttpTool, mcp::McpToolAdapter, shell::ShellTool,
    ssh::SshTool, Tool, ToolExecutionContext,
};
use serde_json::Value;
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

pub struct ToolManagerInit {
    pub permission_policy: Box<dyn PermissionPolicy + Send + Sync>,
    pub permission_config: Arc<PermissionConfig>,
    pub mcp_enabled: bool,
    pub mcp_config: Arc<McpConfig>,
    pub plugins_enabled: bool,
    pub plugin_config: Arc<PluginConfig>,
    pub tool_configs: Vec<ToolConfig>,
    pub execution_context: ToolExecutionContext,
}

impl ToolManager {
    fn shell_command_program(command: &str) -> Option<String> {
        command
            .split_whitespace()
            .next()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn shell_matches_pattern(program: &str, pattern: &str) -> bool {
        if program == pattern {
            return true;
        }

        let program_name = std::path::Path::new(program)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(program);
        let pattern_name = std::path::Path::new(pattern)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(pattern);
        program_name == pattern_name
    }

    fn shell_requires_sudo(config: &ToolConfig, args: &Value) -> bool {
        if !config.enabled {
            return false;
        }

        let command = match args.get("command").and_then(|value| value.as_str()) {
            Some(command) => command,
            None => return false,
        };

        if config.require_sudo {
            return true;
        }

        if let Some(program) = Self::shell_command_program(command) {
            for pattern in &config.privileged_command_patterns {
                if let Some(pattern_program) = Self::shell_command_program(pattern) {
                    if Self::shell_matches_pattern(&program, &pattern_program) {
                        return true;
                    }
                }
            }
        }

        let lowered = command.to_ascii_lowercase();
        lowered.starts_with("sudo ") || lowered.contains(" sudo ")
    }

    fn shell_args_with_session(args: &Value, session_id: &str) -> Value {
        let mut enriched = args.clone();
        if let Some(obj) = enriched.as_object_mut() {
            obj.insert(
                "_session_id".to_owned(),
                serde_json::Value::String(session_id.to_owned()),
            );
        }
        enriched
    }

    fn shell_args_with_secret(args: &Value, session_id: &str, password: &str) -> Value {
        let mut enriched = Self::shell_args_with_session(args, session_id);
        if let Some(obj) = enriched.as_object_mut() {
            obj.insert(
                "_sudo_password".to_owned(),
                serde_json::Value::String(password.to_owned()),
            );
        }
        enriched
    }

    pub fn new(init: ToolManagerInit) -> Self {
        let ToolManagerInit {
            permission_policy,
            permission_config,
            mcp_enabled,
            mcp_config,
            plugins_enabled,
            plugin_config,
            tool_configs,
            execution_context,
        } = init;

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
                "ssh" => Arc::new(SshTool::new(config.clone())),
                "mcp" => {
                    if !mcp_enabled {
                        continue;
                    }
                    Arc::new(McpToolAdapter::new(config.clone(), mcp_config.clone()))
                }
                _ => {
                    // For now, skip unknown tools
                    continue;
                }
            };

            tools.insert(config.name.clone(), tool);
            configs.insert(config.name.clone(), config);
        }

        if plugins_enabled {
            match PluginLoader::load_plugins(
                &plugin_config,
                &execution_context,
                permission_config.default_tool_permission,
            ) {
                Ok(loaded_plugins) => {
                    for plugin in loaded_plugins {
                        tools.insert(plugin.name.clone(), plugin.tool);
                        configs.insert(plugin.name, plugin.config);
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, "failed to load plugins; continuing without plugin tools");
                }
            }
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
                if tool_name == "shell" {
                    let shell_config = {
                        let configs = self.tool_configs.read().await;
                        configs.get(tool_name).cloned()
                    };

                    if let Some(config) = shell_config {
                        let has_secret = args
                            .get("_sudo_password")
                            .and_then(|value| value.as_str())
                            .is_some();
                        if !has_secret && Self::shell_requires_sudo(&config, &args) {
                            let command = args
                                .get("command")
                                .and_then(|value| value.as_str())
                                .unwrap_or_default()
                                .to_owned();
                            let _ = event_tx.try_send(Event::SudoSecretPrompt {
                                session_id,
                                tool: tool_name.to_owned(),
                                args: args.clone(),
                                command,
                                reason: "sudo privileges required".to_owned(),
                            });
                            return Ok(None);
                        }
                    }
                }

                // Execute directly
                let tool_args = if tool_name == "shell" {
                    Self::shell_args_with_session(&args, &session_id)
                } else {
                    args.clone()
                };
                let result = tool
                    .stream_execute(tool_args, event_tx, &self.execution_context)
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
            session_id: session_id.clone(),
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

        if tool_name == "shell" {
            let shell_config = {
                let configs = self.tool_configs.read().await;
                configs.get(tool_name).cloned()
            };

            if let Some(config) = shell_config {
                let has_secret = args
                    .get("_sudo_password")
                    .and_then(|value| value.as_str())
                    .is_some();
                if !has_secret && Self::shell_requires_sudo(&config, &args) {
                    let command = args
                        .get("command")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    let _ = event_tx.try_send(Event::SudoSecretPrompt {
                        session_id,
                        tool: tool_name.to_owned(),
                        args: args.clone(),
                        command,
                        reason: "sudo privileges required".to_owned(),
                    });
                    return Ok(None);
                }
            }
        }

        let tool_args = if tool_name == "shell" {
            Self::shell_args_with_session(&args, &session_id)
        } else {
            args
        };

        let result = tool
            .stream_execute(tool_args, event_tx, &self.execution_context)
            .await?;
        Ok(Some(result))
    }

    pub async fn resolve_sudo_prompt(
        &self,
        session_id: String,
        tool_name: &str,
        args: serde_json::Value,
        password: String,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<Option<crate::tools::ToolResult>> {
        if tool_name != "shell" {
            return Err(Error::Tool(
                "sudo prompt resolution is only supported for shell tool".to_owned(),
            ));
        }

        let tool = {
            let tools = self.tools.read().await;
            tools.get(tool_name).cloned()
        };
        let tool = tool.ok_or_else(|| Error::Tool(format!("tool '{tool_name}' not found")))?;

        let tool_args = Self::shell_args_with_secret(&args, &session_id, &password);
        let result = tool
            .stream_execute(tool_args, event_tx, &self.execution_context)
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
