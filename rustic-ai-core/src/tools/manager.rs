use crate::config::schema::{
    AgentPermissionMode, McpConfig, PermissionConfig, PluginConfig, ToolConfig, WorkflowsConfig,
};
use crate::error::{Error, Result};
use crate::events::Event;
use crate::permissions::{
    AskResolution, CommandPatternBucket, PermissionContext, PermissionDecision, PermissionPolicy,
};
use crate::skills::SkillRegistry;
use crate::tools::plugin::PluginLoader;
use crate::tools::{
    filesystem::FilesystemTool, http::HttpTool, mcp::McpToolAdapter, shell::ShellTool,
    skill::SkillTool, ssh::SshTool, sub_agent::SubAgentTool, Tool, ToolExecutionContext,
};
use crate::workflows::{
    WorkflowExecutor, WorkflowExecutorConfig, WorkflowRegistry, WorkflowRunRequest,
};
use crate::{agents::coordinator::AgentCoordinator, conversation::session_manager::SessionManager};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock as StdRwLock};
use tokio::sync::mpsc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
struct LazyToolSpec {
    priority: usize,
}

#[derive(Clone)]
pub struct ToolManager {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
    tool_configs: Arc<RwLock<HashMap<String, ToolConfig>>>,
    permission_policy: Arc<RwLock<Box<dyn PermissionPolicy + Send + Sync>>>,
    execution_context: ToolExecutionContext,
    workflows_enabled: bool,
    workflows: Arc<WorkflowRegistry>,
    workflows_config: Arc<WorkflowsConfig>,
    skills: Arc<SkillRegistry>,
    agents: Arc<StdRwLock<Option<Arc<AgentCoordinator>>>>,
    session_manager: Arc<SessionManager>,
    permission_config: Arc<PermissionConfig>,
    mcp_enabled: bool,
    mcp_config: Arc<McpConfig>,
    skills_enabled: bool,
    lazy_loaders: Arc<RwLock<HashMap<String, LazyToolSpec>>>,
    active_tools: Arc<RwLock<HashSet<String>>>,
}

pub struct ToolManagerInit {
    pub permission_policy: Box<dyn PermissionPolicy + Send + Sync>,
    pub permission_config: Arc<PermissionConfig>,
    pub mcp_enabled: bool,
    pub mcp_config: Arc<McpConfig>,
    pub skills_enabled: bool,
    pub skills: Arc<SkillRegistry>,
    pub workflows_enabled: bool,
    pub workflows: Arc<WorkflowRegistry>,
    pub workflows_config: Arc<WorkflowsConfig>,
    pub session_manager: Arc<SessionManager>,
    pub plugins_enabled: bool,
    pub plugin_config: Arc<PluginConfig>,
    pub tool_configs: Vec<ToolConfig>,
    pub execution_context: ToolExecutionContext,
}

impl ToolManager {
    fn is_lazy_tool(tool_name: &str) -> bool {
        matches!(tool_name, "mcp" | "ssh" | "skill" | "sub_agent")
    }

    fn tool_priority(tool_name: &str) -> usize {
        match tool_name {
            "filesystem" => 100,
            "shell" => 95,
            "http" => 90,
            "workflow" => 85,
            "sub_agent" => 80,
            "skill" => 70,
            "ssh" => 65,
            "mcp" => 60,
            _ => 40,
        }
    }

    fn tool_static_description(tool_name: &str) -> &'static str {
        match tool_name {
            "shell" => "Execute shell commands with streaming output",
            "filesystem" => "Read and modify files and directories",
            "http" => "Perform HTTP requests",
            "ssh" => "Execute and transfer files over SSH",
            "skill" => "Invoke loaded instruction/script skills",
            "workflow" => "Run workflow entrypoints",
            "sub_agent" => "Delegate task to another configured agent",
            "mcp" => "Invoke MCP server tools",
            _ => "Configured tool",
        }
    }

    fn create_tool_instance(&self, config: &ToolConfig) -> Option<Arc<dyn Tool>> {
        match config.name.as_str() {
            "shell" => Some(Arc::new(ShellTool::new(
                config.clone(),
                self.permission_config.sudo_cache_ttl_secs,
            ))),
            "filesystem" => Some(Arc::new(FilesystemTool::new(config.clone()))),
            "http" => Some(Arc::new(HttpTool::new(config.clone()))),
            "ssh" => Some(Arc::new(SshTool::new(config.clone()))),
            "skill" => {
                if !self.skills_enabled {
                    return None;
                }
                Some(Arc::new(SkillTool::new(
                    config.clone(),
                    self.skills.clone(),
                )))
            }
            "mcp" => {
                if !self.mcp_enabled {
                    return None;
                }
                Some(Arc::new(McpToolAdapter::new(
                    config.clone(),
                    self.mcp_config.clone(),
                )))
            }
            "sub_agent" => Some(Arc::new(SubAgentTool::new(
                config.clone(),
                self.agents.clone(),
            ))),
            _ => None,
        }
    }

    async fn get_or_load_tool(&self, tool_name: &str) -> Result<Arc<dyn Tool>> {
        if let Some(tool) = self.tools.read().await.get(tool_name).cloned() {
            return Ok(tool);
        }

        let config = self
            .tool_configs
            .read()
            .await
            .get(tool_name)
            .cloned()
            .ok_or_else(|| Error::Tool(format!("tool '{tool_name}' not found")))?;

        let tool = self.create_tool_instance(&config).ok_or_else(|| {
            Error::Tool(format!(
                "tool '{}' is configured but cannot be instantiated in current runtime",
                tool_name
            ))
        })?;

        let mut tools = self.tools.write().await;
        if let Some(existing) = tools.get(tool_name).cloned() {
            return Ok(existing);
        }
        tools.insert(tool_name.to_owned(), tool.clone());
        Ok(tool)
    }

    async fn run_tool_stream(
        &self,
        tool_name: &str,
        tool: Arc<dyn Tool>,
        tool_args: Value,
        event_tx: mpsc::Sender<Event>,
        execution_context: &ToolExecutionContext,
    ) -> Result<crate::tools::ToolResult> {
        self.active_tools.write().await.insert(tool_name.to_owned());
        let result = tool
            .stream_execute(tool_args, event_tx, execution_context)
            .await;
        self.active_tools.write().await.remove(tool_name);
        result
    }

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

    async fn check_and_emit_sudo_prompt_if_needed(
        &self,
        session_id: &str,
        tool_name: &str,
        args: &Value,
        event_tx: &mpsc::Sender<Event>,
    ) -> Result<bool> {
        if tool_name != "shell" {
            return Ok(false);
        }

        let shell_config = {
            let configs = self.tool_configs.read().await;
            configs.get(tool_name).cloned()
        };

        let Some(config) = shell_config else {
            return Ok(false);
        };

        let has_secret = args
            .get("_sudo_password")
            .and_then(|value| value.as_str())
            .is_some();
        if has_secret || !Self::shell_requires_sudo(&config, args) {
            return Ok(false);
        }

        let command = args
            .get("command")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
        let _ = event_tx.try_send(Event::SudoSecretPrompt {
            session_id: session_id.to_owned(),
            tool: tool_name.to_owned(),
            args: args.clone(),
            command,
            reason: "sudo privileges required".to_owned(),
        });
        Ok(true)
    }

    pub fn new(init: ToolManagerInit) -> Self {
        let ToolManagerInit {
            permission_policy,
            permission_config,
            mcp_enabled,
            mcp_config,
            skills_enabled,
            skills,
            workflows_enabled,
            workflows,
            workflows_config,
            session_manager,
            plugins_enabled,
            plugin_config,
            tool_configs,
            execution_context,
        } = init;

        let mut tools = HashMap::new();
        let mut configs = HashMap::new();
        let mut lazy_loaders = HashMap::new();
        let agents = Arc::new(StdRwLock::new(None));

        for config in tool_configs {
            if !config.enabled {
                continue;
            }

            configs.insert(config.name.clone(), config.clone());

            if Self::is_lazy_tool(&config.name) {
                lazy_loaders.insert(
                    config.name.clone(),
                    LazyToolSpec {
                        priority: Self::tool_priority(&config.name),
                    },
                );
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
                "skill" => {
                    if !skills_enabled {
                        continue;
                    }
                    Arc::new(SkillTool::new(config.clone(), skills.clone()))
                }
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
            workflows_enabled,
            workflows,
            workflows_config,
            skills,
            agents,
            session_manager,
            permission_config,
            mcp_enabled,
            mcp_config,
            skills_enabled,
            lazy_loaders: Arc::new(RwLock::new(lazy_loaders)),
            active_tools: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    fn build_execution_context(
        &self,
        session_id: &str,
        agent_name: Option<&str>,
    ) -> ToolExecutionContext {
        let mut context = self.execution_context.clone();
        context.session_id = uuid::Uuid::parse_str(session_id).ok();
        context.agent_name = agent_name.map(ToOwned::to_owned);
        context.sub_agent_depth = 0;

        if let Some(agent_name) = agent_name {
            if let Ok(guard) = self.agents.read() {
                if let Some(coordinator) = guard.as_ref() {
                    if let Some(config) = coordinator.get_agent_config(agent_name) {
                        context.agent_permission_mode = config.permission_mode;
                    }
                }
            }
        }

        context
    }

    fn build_permission_context(
        &self,
        session_id: String,
        agent_name: Option<String>,
    ) -> PermissionContext {
        let mut context = PermissionContext {
            session_id,
            agent_name,
            working_directory: self.execution_context.working_directory.clone(),
            agent_permission_mode: AgentPermissionMode::ReadWrite,
        };

        if let Some(ref agent_name) = context.agent_name {
            if let Ok(guard) = self.agents.read() {
                if let Some(coordinator) = guard.as_ref() {
                    if let Some(config) = coordinator.get_agent_config(agent_name) {
                        context.agent_permission_mode = config.permission_mode;
                    }
                }
            }
        }

        context
    }

    pub fn attach_agents(&self, agents: Arc<AgentCoordinator>) {
        let mut guard = self
            .agents
            .write()
            .expect("tool manager agents lock poisoned");
        *guard = Some(agents);

        let sub_agent_config = {
            let configs = self.tool_configs.blocking_read();
            configs.get("sub_agent").cloned()
        };
        if let Some(config) = sub_agent_config {
            let mut tools = self.tools.blocking_write();
            tools.insert(
                "sub_agent".to_owned(),
                Arc::new(SubAgentTool::new(config, self.agents.clone())),
            );
        }
    }

    async fn execute_workflow_call(
        &self,
        session_id: String,
        agent_name: Option<String>,
        args: Value,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<Option<crate::tools::ToolResult>> {
        if !self.workflows_enabled {
            return Err(Error::Tool(
                "workflow tool is disabled by configuration".to_owned(),
            ));
        }

        let workflow_name = args
            .get("workflow")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool("missing 'workflow' argument".to_owned()))?;
        let entrypoint = args
            .get("entrypoint")
            .and_then(Value::as_str)
            .unwrap_or("start");
        let input = args
            .get("input")
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

        let agents = {
            let guard = self
                .agents
                .read()
                .expect("tool manager agents lock poisoned");
            guard.clone()
        }
        .ok_or_else(|| {
            Error::Tool("workflow executor is not ready: agents not attached".to_owned())
        })?;

        let executor = WorkflowExecutor::new(
            self.workflows.clone(),
            self.skills.clone(),
            agents,
            self.session_manager.clone(),
            WorkflowExecutorConfig {
                max_recursion_depth: self.workflows_config.max_recursion_depth,
                max_steps_per_run: self.workflows_config.max_steps_per_run,
                working_directory: self.execution_context.working_directory.clone(),
                compatibility_preset: self.workflows_config.compatibility_preset,
                switch_case_sensitive_default: self.workflows_config.switch_case_sensitive_default,
                switch_pattern_priority: self.workflows_config.switch_pattern_priority.clone(),
                loop_continue_on_iteration_error_default: self
                    .workflows_config
                    .loop_continue_on_iteration_error_default,
                wait_timeout_succeeds: self.workflows_config.wait_timeout_succeeds,
                condition_missing_path_as_false: self
                    .workflows_config
                    .condition_missing_path_as_false,
                default_continue_on_error: self.workflows_config.default_continue_on_error,
                continue_on_error_routing: self.workflows_config.continue_on_error_routing.clone(),
                execution_error_policy: self.workflows_config.execution_error_policy.clone(),
                default_retry_count: self.workflows_config.default_retry_count,
                default_retry_backoff_ms: self.workflows_config.default_retry_backoff_ms,
                default_retry_backoff_multiplier: self
                    .workflows_config
                    .default_retry_backoff_multiplier,
                default_retry_backoff_max_ms: self.workflows_config.default_retry_backoff_max_ms,
                condition_group_max_depth: self.workflows_config.condition_group_max_depth,
                expression_max_length: self.workflows_config.expression_max_length,
                expression_max_depth: self.workflows_config.expression_max_depth,
                loop_default_max_iterations: self.workflows_config.loop_default_max_iterations,
                loop_default_max_parallelism: self.workflows_config.loop_default_max_parallelism,
                loop_hard_max_parallelism: self.workflows_config.loop_hard_max_parallelism,
                wait_default_poll_interval_ms: self.workflows_config.wait_default_poll_interval_ms,
                wait_default_timeout_seconds: self.workflows_config.wait_default_timeout_seconds,
            },
        );

        let result = executor
            .run(
                WorkflowRunRequest {
                    workflow_name: workflow_name.to_owned(),
                    entrypoint: entrypoint.to_owned(),
                    session_id,
                    agent_name,
                    input,
                    recursion_depth: 0,
                    workflow_stack: Vec::new(),
                },
                self,
                event_tx,
            )
            .await?;

        Ok(Some(crate::tools::ToolResult {
            success: result.success,
            exit_code: Some(if result.success { 0 } else { 1 }),
            output: serde_json::to_string(&json!({
                "success": result.success,
                "steps_executed": result.steps_executed,
                "outputs": result.outputs,
            }))
            .unwrap_or_else(|_| "{}".to_owned()),
        }))
    }

    pub async fn register_tool(&self, name: String, tool: Arc<dyn Tool>, config: ToolConfig) {
        let mut tools = self.tools.write().await;
        let mut configs = self.tool_configs.write().await;
        tools.insert(name.clone(), tool);
        configs.insert(name, config);
    }

    pub async fn has_tool(&self, name: &str) -> bool {
        self.tool_configs.read().await.contains_key(name)
    }

    pub async fn get_tool_config(&self, name: &str) -> Option<ToolConfig> {
        self.tool_configs.read().await.get(name).cloned()
    }

    pub async fn unload_unused(&self, keep_tools: &[String]) -> usize {
        let keep = keep_tools.iter().cloned().collect::<HashSet<_>>();
        let lazy = self.lazy_loaders.read().await;
        let mut tools = self.tools.write().await;
        let active = self.active_tools.read().await;

        let unloadable = tools
            .keys()
            .filter(|name| {
                lazy.contains_key(*name) && !keep.contains(*name) && !active.contains(*name)
            })
            .cloned()
            .collect::<Vec<_>>();

        for name in &unloadable {
            tools.remove(name);
        }

        unloadable.len()
    }

    pub async fn get_tool_descriptions(
        &self,
        focus: Option<&str>,
        max_items: Option<usize>,
    ) -> Vec<(String, String)> {
        let focus = focus.map(|value| value.to_ascii_lowercase());
        let tools = self.tools.read().await;
        let lazy = self.lazy_loaders.read().await;
        let configs = self.tool_configs.read().await;

        let mut rows = configs
            .keys()
            .map(|name| {
                let description = tools
                    .get(name)
                    .map(|tool| tool.description().to_owned())
                    .unwrap_or_else(|| Self::tool_static_description(name).to_owned());
                let mut priority = Self::tool_priority(name);
                if let Some(spec) = lazy.get(name) {
                    priority = std::cmp::max(priority, spec.priority);
                }
                if let Some(focus) = &focus {
                    if name.to_ascii_lowercase().contains(focus)
                        || description.to_ascii_lowercase().contains(focus)
                    {
                        priority += 1000;
                    }
                }
                (name.clone(), description, priority)
            })
            .collect::<Vec<_>>();

        rows.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(&right.0)));

        let mut listed = rows
            .into_iter()
            .map(|(name, desc, _)| (name, desc))
            .collect::<Vec<_>>();
        if let Some(max_items) = max_items {
            listed.truncate(max_items);
        }
        listed
    }

    pub async fn find_tools_by_basket(&self, basket: &str) -> Vec<String> {
        self.tool_configs
            .read()
            .await
            .iter()
            .filter_map(|(name, config)| {
                config
                    .taxonomy_membership
                    .iter()
                    .any(|entry| entry.basket == basket)
                    .then_some(name.clone())
            })
            .collect()
    }

    pub async fn find_tools_by_sub_basket(&self, basket: &str, sub_basket: &str) -> Vec<String> {
        self.tool_configs
            .read()
            .await
            .iter()
            .filter_map(|(name, config)| {
                config
                    .taxonomy_membership
                    .iter()
                    .any(|entry| {
                        entry.basket == basket
                            && entry
                                .sub_basket
                                .as_deref()
                                .map(|value| value == sub_basket)
                                .unwrap_or(false)
                    })
                    .then_some(name.clone())
            })
            .collect()
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
        let has_tool_config = {
            let configs = self.tool_configs.read().await;
            configs.contains_key(tool_name)
        };
        if !has_tool_config {
            return Err(Error::Tool(format!("tool '{tool_name}' not found")));
        }

        let permission_context = self.build_permission_context(session_id.clone(), agent_name);

        // Check permission
        let permission = {
            let policy = self.permission_policy.read().await;
            policy.check_tool_permission(tool_name, &args, &permission_context)
        };

        match permission {
            PermissionDecision::Allow
            | PermissionDecision::AllowRead
            | PermissionDecision::AllowWrite => {
                if tool_name == "workflow" {
                    return self
                        .execute_workflow_call(
                            session_id,
                            permission_context.agent_name,
                            args,
                            event_tx,
                        )
                        .await;
                }

                let tool = self.get_or_load_tool(tool_name).await?;

                if self
                    .check_and_emit_sudo_prompt_if_needed(&session_id, tool_name, &args, &event_tx)
                    .await?
                {
                    return Ok(None);
                }

                // Execute directly
                let tool_args = if tool_name == "shell" {
                    Self::shell_args_with_session(&args, &session_id)
                } else {
                    args.clone()
                };
                let execution_context = self
                    .build_execution_context(&session_id, permission_context.agent_name.as_deref());
                let result = self
                    .run_tool_stream(tool_name, tool, tool_args, event_tx, &execution_context)
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
        let permission_context = self.build_permission_context(session_id.clone(), agent_name);

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

        if tool_name == "workflow" {
            return self
                .execute_workflow_call(session_id, permission_context.agent_name, args, event_tx)
                .await;
        }

        // Execute tool
        let tool = self.get_or_load_tool(tool_name).await?;

        if self
            .check_and_emit_sudo_prompt_if_needed(&session_id, tool_name, &args, &event_tx)
            .await?
        {
            return Ok(None);
        }

        let tool_args = if tool_name == "shell" {
            Self::shell_args_with_session(&args, &session_id)
        } else {
            args
        };

        let execution_context =
            self.build_execution_context(&session_id, permission_context.agent_name.as_deref());
        let result = self
            .run_tool_stream(tool_name, tool, tool_args, event_tx, &execution_context)
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

        let tool = self.get_or_load_tool(tool_name).await?;

        let tool_args = Self::shell_args_with_secret(&args, &session_id, &password);
        let execution_context = self.build_execution_context(&session_id, None);
        let result = self
            .run_tool_stream(tool_name, tool, tool_args, event_tx, &execution_context)
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
