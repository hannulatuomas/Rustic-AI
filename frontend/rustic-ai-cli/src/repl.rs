use crate::cli::OutputFormat;
use crate::renderer::Renderer;
use chrono::Utc;
use rustic_ai_core::error::Result;
use rustic_ai_core::events::Event;
use rustic_ai_core::permissions::{AskResolution, CommandPatternBucket};
use rustic_ai_core::rules::TopicTracker;
use rustic_ai_core::workflows::{WorkflowExecutor, WorkflowExecutorConfig, WorkflowRunRequest};
use rustic_ai_core::RusticAI;
use serde_json::Value;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone)]
struct PendingPermissionRequest {
    session_id: String,
    tool: String,
    args: Value,
}

#[derive(Debug, Clone)]
struct PendingSudoRequest {
    session_id: String,
    tool: String,
    args: Value,
    command: String,
    reason: String,
}

pub struct Repl {
    app: Arc<RusticAI>,
    agent_name: Option<String>,
    output_format: OutputFormat,
    config_path: PathBuf,
}

impl Repl {
    pub fn new(
        app: Arc<RusticAI>,
        agent_name: Option<String>,
        output_format: OutputFormat,
        config_path: PathBuf,
    ) -> Self {
        Self {
            app,
            agent_name,
            output_format,
            config_path,
        }
    }

    fn read_json_or_empty(path: &Path) -> Result<serde_json::Value> {
        if !path.exists() {
            return Ok(serde_json::json!({}));
        }

        let content = std::fs::read_to_string(path).map_err(|err| {
            rustic_ai_core::Error::Config(format!(
                "failed to read config fragment '{}': {err}",
                path.display()
            ))
        })?;

        let parsed = serde_json::from_str::<serde_json::Value>(&content).map_err(|err| {
            rustic_ai_core::Error::Config(format!(
                "failed to parse config fragment '{}': {err}",
                path.display()
            ))
        })?;

        if !parsed.is_object() {
            return Err(rustic_ai_core::Error::Config(format!(
                "config fragment '{}' must be a JSON object",
                path.display()
            )));
        }

        Ok(parsed)
    }

    fn ensure_permissions_fragment(root: &mut serde_json::Value) -> Result<&mut serde_json::Value> {
        let root_obj = root.as_object_mut().ok_or_else(|| {
            rustic_ai_core::Error::Config("config fragment root must be an object".to_owned())
        })?;

        if !root_obj.contains_key("permissions") {
            root_obj.insert("permissions".to_owned(), serde_json::json!({}));
        }

        root_obj
            .get_mut("permissions")
            .ok_or_else(|| rustic_ai_core::Error::Config("missing permissions section".to_owned()))
    }

    fn ensure_object<'a>(
        value: &'a mut serde_json::Value,
        key: &str,
    ) -> Result<&'a mut serde_json::Value> {
        let map = value.as_object_mut().ok_or_else(|| {
            rustic_ai_core::Error::Config(format!("expected object while preparing '{key}'"))
        })?;

        if !map.contains_key(key) {
            map.insert(key.to_owned(), serde_json::json!({}));
        }

        map.get_mut(key)
            .ok_or_else(|| rustic_ai_core::Error::Config(format!("missing object key '{key}'")))
    }

    fn ensure_array<'a>(
        value: &'a mut serde_json::Value,
        key: &str,
    ) -> Result<&'a mut Vec<serde_json::Value>> {
        let map = value.as_object_mut().ok_or_else(|| {
            rustic_ai_core::Error::Config(format!("expected object while preparing array '{key}'"))
        })?;

        if !map.contains_key(key) {
            map.insert(key.to_owned(), serde_json::json!([]));
        }

        map.get_mut(key)
            .and_then(|entry| entry.as_array_mut())
            .ok_or_else(|| rustic_ai_core::Error::Config(format!("key '{key}' must be an array")))
    }

    fn push_unique_string(array: &mut Vec<serde_json::Value>, value: &str) {
        let exists = array.iter().any(|entry| entry.as_str() == Some(value));
        if !exists {
            array.push(serde_json::Value::String(value.to_owned()));
        }
    }

    fn global_permissions_fragment_path(&self) -> PathBuf {
        let global_root = if let Some(path) = &self.app.config().storage.global_root_path {
            PathBuf::from(path)
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".rustic-ai")
        } else {
            PathBuf::from(".rustic-ai")
        };

        global_root.join("config").join("permissions.json")
    }

    fn project_permissions_fragment_path(&self) -> PathBuf {
        self.app
            .work_dir()
            .join(&self.app.config().storage.default_root_dir_name)
            .join("config")
            .join("permissions.json")
    }

    fn persist_allowed_path(&self, global_scope: bool, path: &str) -> Result<PathBuf> {
        let fragment_path = if global_scope {
            self.global_permissions_fragment_path()
        } else {
            self.project_permissions_fragment_path()
        };

        if let Some(parent) = fragment_path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                rustic_ai_core::Error::Config(format!(
                    "failed to create config fragment directory '{}': {err}",
                    parent.display()
                ))
            })?;
        }

        let mut root = Self::read_json_or_empty(&fragment_path)?;
        let permissions = Self::ensure_permissions_fragment(&mut root)?;
        let key = if global_scope {
            "globally_allowed_paths"
        } else {
            "project_allowed_paths"
        };
        let array = Self::ensure_array(permissions, key)?;
        Self::push_unique_string(array, path);

        let serialized = serde_json::to_string_pretty(&root)?;
        std::fs::write(&fragment_path, serialized).map_err(|err| {
            rustic_ai_core::Error::Config(format!(
                "failed to write config fragment '{}': {err}",
                fragment_path.display()
            ))
        })?;

        Ok(fragment_path)
    }

    fn persist_command_pattern(
        &self,
        global_scope: bool,
        bucket: CommandPatternBucket,
        pattern: &str,
    ) -> Result<PathBuf> {
        let fragment_path = if global_scope {
            self.global_permissions_fragment_path()
        } else {
            self.project_permissions_fragment_path()
        };

        if let Some(parent) = fragment_path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                rustic_ai_core::Error::Config(format!(
                    "failed to create config fragment directory '{}': {err}",
                    parent.display()
                ))
            })?;
        }

        let mut root = Self::read_json_or_empty(&fragment_path)?;
        let permissions = Self::ensure_permissions_fragment(&mut root)?;
        let section_key = if global_scope {
            "global_command_patterns"
        } else {
            "project_command_patterns"
        };
        let patterns = Self::ensure_object(permissions, section_key)?;
        let list_key = match bucket {
            CommandPatternBucket::Allow => "allow",
            CommandPatternBucket::Ask => "ask",
            CommandPatternBucket::Deny => "deny",
        };
        let array = Self::ensure_array(patterns, list_key)?;
        Self::push_unique_string(array, pattern);

        let serialized = serde_json::to_string_pretty(&root)?;
        std::fs::write(&fragment_path, serialized).map_err(|err| {
            rustic_ai_core::Error::Config(format!(
                "failed to write config fragment '{}': {err}",
                fragment_path.display()
            ))
        })?;

        Ok(fragment_path)
    }

    async fn run_workflow(
        &self,
        session_id: uuid::Uuid,
        agent_name: &str,
        workflow_name: &str,
        entrypoint: &str,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<rustic_ai_core::workflows::WorkflowExecutionResult> {
        let wf_cfg = &self.app.config().workflows;
        let executor = WorkflowExecutor::new(
            self.app.runtime().workflows.clone(),
            self.app.runtime().skills.clone(),
            std::sync::Arc::new(self.app.runtime().agents.clone()),
            self.app.session_manager().clone(),
            WorkflowExecutorConfig {
                max_recursion_depth: wf_cfg.max_recursion_depth,
                max_steps_per_run: wf_cfg.max_steps_per_run,
                working_directory: self.app.work_dir().to_path_buf(),
                compatibility_preset: wf_cfg.compatibility_preset,
                switch_case_sensitive_default: wf_cfg.switch_case_sensitive_default,
                switch_pattern_priority: wf_cfg.switch_pattern_priority.clone(),
                loop_continue_on_iteration_error_default: wf_cfg
                    .loop_continue_on_iteration_error_default,
                wait_timeout_succeeds: wf_cfg.wait_timeout_succeeds,
                condition_missing_path_as_false: wf_cfg.condition_missing_path_as_false,
                default_continue_on_error: wf_cfg.default_continue_on_error,
                continue_on_error_routing: wf_cfg.continue_on_error_routing.clone(),
                execution_error_policy: wf_cfg.execution_error_policy.clone(),
                default_retry_count: wf_cfg.default_retry_count,
                default_retry_backoff_ms: wf_cfg.default_retry_backoff_ms,
                default_retry_backoff_multiplier: wf_cfg.default_retry_backoff_multiplier,
                default_retry_backoff_max_ms: wf_cfg.default_retry_backoff_max_ms,
                condition_group_max_depth: wf_cfg.condition_group_max_depth,
                expression_max_length: wf_cfg.expression_max_length,
                expression_max_depth: wf_cfg.expression_max_depth,
                loop_default_max_iterations: wf_cfg.loop_default_max_iterations,
                loop_default_max_parallelism: wf_cfg.loop_default_max_parallelism,
                loop_hard_max_parallelism: wf_cfg.loop_hard_max_parallelism,
                wait_default_poll_interval_ms: wf_cfg.wait_default_poll_interval_ms,
                wait_default_timeout_seconds: wf_cfg.wait_default_timeout_seconds,
            },
        );

        executor
            .run(
                WorkflowRunRequest {
                    workflow_name: workflow_name.to_owned(),
                    entrypoint: entrypoint.to_owned(),
                    session_id: session_id.to_string(),
                    agent_name: Some(agent_name.to_owned()),
                    input: Value::Object(serde_json::Map::new()),
                    recursion_depth: 0,
                    workflow_stack: Vec::new(),
                },
                self.app.runtime().tools.as_ref(),
                event_tx,
            )
            .await
    }

    async fn run_trigger_matches(
        &self,
        session_id: uuid::Uuid,
        agent_name: &str,
        matches: Vec<rustic_ai_core::workflows::WorkflowTriggerMatch>,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<()> {
        for matched in matches {
            let result = self
                .run_workflow(
                    session_id,
                    agent_name,
                    &matched.workflow_name,
                    &matched.entrypoint,
                    event_tx.clone(),
                )
                .await;

            match (&matched.reason, result) {
                (
                    rustic_ai_core::workflows::WorkflowTriggerReason::Event { event_name },
                    Ok(run),
                ) => {
                    println!(
                        "Triggered by event '{}' -> workflow '{}' (entrypoint '{}'): success={}, steps={}",
                        event_name, matched.workflow_name, matched.entrypoint, run.success, run.steps_executed
                    );
                }
                (
                    rustic_ai_core::workflows::WorkflowTriggerReason::Cron { expression },
                    Ok(run),
                ) => {
                    println!(
                        "Triggered by cron '{}' -> workflow '{}' (entrypoint '{}'): success={}, steps={}",
                        expression, matched.workflow_name, matched.entrypoint, run.success, run.steps_executed
                    );
                }
                (_, Err(err)) => {
                    println!(
                        "Triggered workflow '{}' (entrypoint '{}') failed: {}",
                        matched.workflow_name, matched.entrypoint, err
                    );
                }
            }
        }

        Ok(())
    }

    pub async fn run(&self) -> Result<()> {
        let runtime = tokio::runtime::Runtime::new().map_err(|err| {
            rustic_ai_core::Error::Config(format!("failed to create tokio runtime: {err}"))
        })?;

        let session_id = runtime.block_on(async {
            let sessions = self.app.session_manager().list_sessions(None).await?;
            if sessions.is_empty() {
                self.app.session_manager().create_session("default").await
            } else {
                Ok(sessions[0].id)
            }
        })?;

        let mut topic_tracker = TopicTracker::new(
            self.app.config().rules.topic_debounce_interval_secs,
            self.app.config().rules.topic_similarity_threshold,
        );
        let mut trigger_engine = rustic_ai_core::workflows::WorkflowTriggerEngine::new(Utc::now());

        println!("Session: {session_id}");

        let (event_tx, mut event_rx) = mpsc::channel(100);

        let renderer = Renderer::new(self.output_format);
        let pending_permission: Arc<Mutex<Option<PendingPermissionRequest>>> =
            Arc::new(Mutex::new(None));
        let pending_sudo: Arc<Mutex<Option<PendingSudoRequest>>> = Arc::new(Mutex::new(None));
        let pending_for_listener = pending_permission.clone();
        let pending_sudo_for_listener = pending_sudo.clone();
        let renderer_handle = tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                if let Event::PermissionRequest {
                    session_id,
                    tool,
                    args,
                } = &event
                {
                    let mut guard = pending_for_listener.lock().await;
                    *guard = Some(PendingPermissionRequest {
                        session_id: session_id.clone(),
                        tool: tool.clone(),
                        args: args.clone(),
                    });
                }

                if let Event::SudoSecretPrompt {
                    session_id,
                    tool,
                    args,
                    command,
                    reason,
                } = &event
                {
                    let mut sudo_guard = pending_sudo_for_listener.lock().await;
                    *sudo_guard = Some(PendingSudoRequest {
                        session_id: session_id.clone(),
                        tool: tool.clone(),
                        args: args.clone(),
                        command: command.clone(),
                        reason: reason.clone(),
                    });
                }

                renderer.render_event(&event);
            }
        });

        let agent_name = self
            .agent_name
            .clone()
            .unwrap_or_else(|| "default".to_string());
        if !self.app.runtime().agents.has_agent(&agent_name) {
            return Err(rustic_ai_core::Error::NotFound(format!(
                "agent '{}' not found",
                agent_name
            )));
        }

        println!();
        println!("Rustic-AI Interactive Chat");
        println!("Type 'exit' or press Ctrl-C to quit");
        println!("Config: {}", self.config_path.display());
        println!(
            "Permission shortcuts: /perm path add [global|project|session] <path>, /perm cmd <allow|ask|deny> [global|project|session] <pattern>"
        );
        println!("Workflow triggers: /workflow trigger event <name> | /workflow trigger cron");
        println!();

        loop {
            let pending_sudo_request = {
                let guard = pending_sudo.lock().await;
                guard.clone()
            };

            if let Some(request) = pending_sudo_request {
                println!("[sudo] {}", request.reason);
                println!("[sudo] command: {}", request.command);
                let mut password =
                    rpassword::prompt_password("[sudo] Password: ").map_err(|err| {
                        rustic_ai_core::Error::Io(io::Error::other(format!(
                            "failed reading sudo password: {err}"
                        )))
                    })?;

                if password.trim().is_empty() {
                    println!("[sudo] Empty password entered; request cancelled.");
                    let mut guard = pending_sudo.lock().await;
                    *guard = None;
                    continue;
                }

                let resolved = self
                    .app
                    .runtime()
                    .tools
                    .resolve_sudo_prompt(
                        request.session_id.clone(),
                        &request.tool,
                        request.args.clone(),
                        password.clone(),
                        event_tx.clone(),
                    )
                    .await;
                password.clear();

                match resolved {
                    Ok(Some(result)) => {
                        let message = format!(
                            "{{\"tool\":\"{}\",\"success\":{},\"exit_code\":{},\"output\":{}}}",
                            request.tool,
                            result.success,
                            result.exit_code.unwrap_or_default(),
                            serde_json::to_string(&result.output)
                                .unwrap_or_else(|_| "\"\"".to_string())
                        );
                        self.app
                            .session_manager()
                            .append_message(session_id, "tool", &message)
                            .await?;

                        let agent = self.app.runtime().agents.get_agent(Some(&agent_name))?;
                        if let Err(err) = agent
                            .continue_after_tool(session_id, event_tx.clone())
                            .await
                        {
                            let _ = event_tx.try_send(Event::Error(err.to_string()));
                        }

                        let mut permission_guard = pending_permission.lock().await;
                        *permission_guard = None;
                        let mut sudo_guard = pending_sudo.lock().await;
                        *sudo_guard = None;
                        continue;
                    }
                    Ok(None) => {
                        let mut sudo_guard = pending_sudo.lock().await;
                        *sudo_guard = None;
                        continue;
                    }
                    Err(err) => {
                        let _ = event_tx.try_send(Event::Error(err.to_string()));
                        let mut sudo_guard = pending_sudo.lock().await;
                        *sudo_guard = None;
                        continue;
                    }
                }
            }

            if self.app.config().features.triggers_enabled {
                let due =
                    trigger_engine.due_cron(self.app.runtime().workflows.as_ref(), Utc::now());
                if !due.is_empty() {
                    self.run_trigger_matches(session_id, &agent_name, due, event_tx.clone())
                        .await?;
                }
            }

            print!("> ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin()
                .read_line(&mut input)
                .map_err(rustic_ai_core::Error::Io)?;

            let input = input.trim();

            if input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit") {
                println!("Goodbye!");
                break;
            }

            if input.is_empty() {
                continue;
            }

            if input.starts_with('y') || input.starts_with('n') || input.starts_with('a') {
                let decision = if input.starts_with('y') {
                    AskResolution::AllowOnce
                } else if input.starts_with('n') {
                    AskResolution::Deny
                } else {
                    AskResolution::AllowInSession
                };

                let pending = {
                    let guard = pending_permission.lock().await;
                    guard.clone()
                };

                if let Some(request) = pending {
                    let resolved = self
                        .app
                        .runtime()
                        .tools
                        .resolve_permission(
                            request.session_id.clone(),
                            Some(agent_name.clone()),
                            &request.tool,
                            request.args.clone(),
                            decision,
                            event_tx.clone(),
                        )
                        .await?;

                    if let Some(result) = resolved {
                        let message = format!(
                            "{{\"tool\":\"{}\",\"success\":{},\"exit_code\":{},\"output\":{}}}",
                            request.tool,
                            result.success,
                            result.exit_code.unwrap_or_default(),
                            serde_json::to_string(&result.output)
                                .unwrap_or_else(|_| "\"\"".to_string())
                        );
                        self.app
                            .session_manager()
                            .append_message(session_id, "tool", &message)
                            .await?;

                        let agent = self.app.runtime().agents.get_agent(Some(&agent_name))?;
                        if let Err(err) = agent
                            .continue_after_tool(session_id, event_tx.clone())
                            .await
                        {
                            let _ = event_tx.try_send(Event::Error(err.to_string()));
                        }
                    }

                    let mut guard = pending_permission.lock().await;
                    *guard = None;
                } else {
                    println!("No pending permission request.");
                }
                continue;
            }

            if let Some(rest) = input.strip_prefix("/perm ") {
                let parts = rest.split_whitespace().collect::<Vec<_>>();
                if parts.len() >= 3 && parts[0] == "path" && parts[1] == "add" {
                    let (scope, start_index) = match parts.get(2).copied() {
                        Some("global") | Some("project") | Some("session") => (parts[2], 3usize),
                        _ => ("session", 2usize),
                    };
                    if parts.len() <= start_index {
                        println!("Missing path value.");
                        continue;
                    }

                    let path = parts[start_index..].join(" ");
                    match scope {
                        "global" => {
                            self.app
                                .runtime()
                                .tools
                                .add_global_allowed_path(&path)
                                .await;
                            match self.persist_allowed_path(true, &path) {
                                Ok(file) => {
                                    println!(
                                        "Added global allowed path: {path} (persisted to {})",
                                        file.display()
                                    );
                                }
                                Err(err) => {
                                    println!(
                                        "Added runtime global allowed path: {path} (persist failed: {err})"
                                    );
                                }
                            }
                        }
                        "project" => {
                            self.app
                                .runtime()
                                .tools
                                .add_project_allowed_path(&path)
                                .await;
                            match self.persist_allowed_path(false, &path) {
                                Ok(file) => {
                                    println!(
                                        "Added project allowed path: {path} (persisted to {})",
                                        file.display()
                                    );
                                }
                                Err(err) => {
                                    println!(
                                        "Added runtime project allowed path: {path} (persist failed: {err})"
                                    );
                                }
                            }
                        }
                        _ => {
                            self.app
                                .runtime()
                                .tools
                                .add_session_allowed_path(&session_id.to_string(), &path)
                                .await;
                            println!("Added session allowed path: {path}");
                        }
                    }
                    continue;
                }

                if parts.len() >= 3 && parts[0] == "cmd" {
                    let bucket = match parts[1] {
                        "allow" => Some(CommandPatternBucket::Allow),
                        "ask" => Some(CommandPatternBucket::Ask),
                        "deny" => Some(CommandPatternBucket::Deny),
                        _ => None,
                    };

                    if let Some(bucket) = bucket {
                        let (scope, start_index) = match parts.get(2).copied() {
                            Some("global") | Some("project") | Some("session") => {
                                (parts[2], 3usize)
                            }
                            _ => ("session", 2usize),
                        };
                        if parts.len() <= start_index {
                            println!("Missing command pattern value.");
                            continue;
                        }

                        let pattern = parts[start_index..].join(" ");
                        match scope {
                            "global" => {
                                self.app
                                    .runtime()
                                    .tools
                                    .add_global_command_pattern(bucket, &pattern)
                                    .await;
                                match self.persist_command_pattern(true, bucket, &pattern) {
                                    Ok(file) => {
                                        println!(
                                            "Added global command pattern ({:?}): {} (persisted to {})",
                                            bucket,
                                            pattern,
                                            file.display()
                                        );
                                    }
                                    Err(err) => {
                                        println!(
                                            "Added runtime global command pattern ({:?}): {} (persist failed: {})",
                                            bucket, pattern, err
                                        );
                                    }
                                }
                            }
                            "project" => {
                                self.app
                                    .runtime()
                                    .tools
                                    .add_project_command_pattern(bucket, &pattern)
                                    .await;
                                match self.persist_command_pattern(false, bucket, &pattern) {
                                    Ok(file) => {
                                        println!(
                                            "Added project command pattern ({:?}): {} (persisted to {})",
                                            bucket,
                                            pattern,
                                            file.display()
                                        );
                                    }
                                    Err(err) => {
                                        println!(
                                            "Added runtime project command pattern ({:?}): {} (persist failed: {})",
                                            bucket, pattern, err
                                        );
                                    }
                                }
                            }
                            _ => {
                                self.app
                                    .runtime()
                                    .tools
                                    .add_session_command_pattern(
                                        &session_id.to_string(),
                                        bucket,
                                        &pattern,
                                    )
                                    .await;
                                println!(
                                    "Added session command pattern ({:?}): {}",
                                    bucket, pattern
                                );
                            }
                        }
                        continue;
                    }
                }

                println!("Permission commands:");
                println!("  /perm path add [global|project|session] <path>");
                println!("  /perm cmd <allow|ask|deny> [global|project|session] <pattern>");
                continue;
            }

            if input == "/skills list" {
                let skills = self.app.runtime().skills.list();
                if skills.is_empty() {
                    println!("No skills loaded.");
                } else {
                    println!("Loaded skills ({}):", skills.len());
                    for name in skills {
                        println!("  - {name}");
                    }
                }
                continue;
            }

            if let Some(name) = input.strip_prefix("/skills show ") {
                let name = name.trim();
                if name.is_empty() {
                    println!("Usage: /skills show <skill_name>");
                    continue;
                }
                if let Some(skill) = self.app.runtime().skills.get(name) {
                    let spec = skill.spec();
                    println!("Skill: {}", spec.name);
                    println!("Description: {}", spec.description);
                    println!("Timeout: {}s", spec.timeout_seconds);
                    println!("Schema: {}", spec.schema);
                } else {
                    println!("Skill '{name}' not found.");
                }
                continue;
            }

            if let Some(rest) = input
                .strip_prefix("/workflow run ")
                .or_else(|| input.strip_prefix("/workflows run "))
            {
                let parts = rest.split_whitespace().collect::<Vec<_>>();
                if parts.is_empty() {
                    println!("Usage: /workflow run <workflow_name> [entrypoint]");
                    continue;
                }
                let workflow_name = parts[0];
                let entrypoint = parts.get(1).copied().unwrap_or("start");

                match self
                    .run_workflow(
                        session_id,
                        &agent_name,
                        workflow_name,
                        entrypoint,
                        event_tx.clone(),
                    )
                    .await
                {
                    Ok(result) => {
                        println!(
                            "Workflow '{}' completed: success={}, steps={}, outputs={}",
                            workflow_name,
                            result.success,
                            result.steps_executed,
                            result.outputs.len()
                        );
                    }
                    Err(err) => {
                        println!("Workflow '{}' failed: {}", workflow_name, err);
                    }
                }
                continue;
            }

            if input == "/workflows list" || input == "/workflow list" {
                let workflows = self.app.runtime().workflows.list();
                if workflows.is_empty() {
                    println!("No workflows loaded.");
                } else {
                    println!("Loaded workflows ({}):", workflows.len());
                    for name in workflows {
                        println!("  - {name}");
                    }
                }
                continue;
            }

            if let Some(name) = input
                .strip_prefix("/workflows show ")
                .or_else(|| input.strip_prefix("/workflow show "))
            {
                let name = name.trim();
                if name.is_empty() {
                    println!("Usage: /workflows show <workflow_name>");
                    continue;
                }
                if let Some(workflow) = self.app.runtime().workflows.get(name) {
                    println!("Workflow: {}", workflow.name);
                    println!("Description: {}", workflow.description);
                    println!("Entrypoints: {}", workflow.entrypoints.len());
                    for (entry, cfg) in &workflow.entrypoints {
                        println!(
                            "  - {} -> {} (cron: {}, events: {}, webhooks: {})",
                            entry,
                            cfg.step,
                            cfg.triggers.cron.len(),
                            cfg.triggers.events.len(),
                            cfg.triggers.webhooks.len()
                        );
                    }
                    println!("Steps: {}", workflow.steps.len());
                    for step in &workflow.steps {
                        let kind = match step.kind {
                            rustic_ai_core::workflows::WorkflowStepKind::Tool => "tool",
                            rustic_ai_core::workflows::WorkflowStepKind::Skill => "skill",
                            rustic_ai_core::workflows::WorkflowStepKind::Agent => "agent",
                            rustic_ai_core::workflows::WorkflowStepKind::Workflow => "workflow",
                            rustic_ai_core::workflows::WorkflowStepKind::Condition => "condition",
                            rustic_ai_core::workflows::WorkflowStepKind::Wait => "wait",
                            rustic_ai_core::workflows::WorkflowStepKind::Loop => "loop",
                            rustic_ai_core::workflows::WorkflowStepKind::Merge => "merge",
                            rustic_ai_core::workflows::WorkflowStepKind::Switch => "switch",
                        };

                        let next = step.next.as_deref().unwrap_or("<end>");
                        println!("  - {} [{}] next={}", step.id, kind, next);

                        match step.kind {
                            rustic_ai_core::workflows::WorkflowStepKind::Condition => {
                                if let Some(group) =
                                    step.config.get("group").and_then(Value::as_object)
                                {
                                    let op = group
                                        .get("operator")
                                        .and_then(Value::as_str)
                                        .unwrap_or("and");
                                    let count = group
                                        .get("conditions")
                                        .and_then(Value::as_array)
                                        .map(|v| v.len())
                                        .unwrap_or(0);
                                    println!("      group: operator={}, conditions={}", op, count);
                                } else if let Some(expression) =
                                    step.config.get("expression").and_then(Value::as_str)
                                {
                                    println!("      expression: {}", expression);
                                } else if let Some(path) =
                                    step.config.get("path").and_then(Value::as_str)
                                {
                                    let op = step
                                        .config
                                        .get("operator")
                                        .and_then(Value::as_str)
                                        .unwrap_or("exists");
                                    println!("      path: {} ({})", path, op);
                                }
                            }
                            rustic_ai_core::workflows::WorkflowStepKind::Wait => {
                                let duration = step
                                    .config
                                    .get("duration_seconds")
                                    .and_then(Value::as_u64)
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "none".to_owned());
                                let until = step
                                    .config
                                    .get("until_expression")
                                    .and_then(Value::as_str)
                                    .unwrap_or("none");
                                println!("      duration_seconds={}, until={}", duration, until);
                            }
                            rustic_ai_core::workflows::WorkflowStepKind::Loop => {
                                let parallel = step
                                    .config
                                    .get("parallel")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false);
                                let max_iterations = step
                                    .config
                                    .get("max_iterations")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(1_000);
                                let max_parallelism = step
                                    .config
                                    .get("max_parallelism")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(8);
                                let item_var = step
                                    .config
                                    .get("item_variable")
                                    .and_then(Value::as_str)
                                    .unwrap_or("item");
                                println!(
                                    "      parallel={}, max_iterations={}, max_parallelism={}, item_variable={}",
                                    parallel, max_iterations, max_parallelism, item_var
                                );
                            }
                            rustic_ai_core::workflows::WorkflowStepKind::Merge => {
                                let mode = step
                                    .config
                                    .get("mode")
                                    .and_then(Value::as_str)
                                    .unwrap_or("merge");
                                let input_count = step
                                    .config
                                    .get("inputs")
                                    .and_then(Value::as_object)
                                    .map(|v| v.len())
                                    .unwrap_or(0);
                                println!("      mode={}, inputs={}", mode, input_count);
                            }
                            rustic_ai_core::workflows::WorkflowStepKind::Switch => {
                                let case_count = step
                                    .config
                                    .get("cases")
                                    .and_then(Value::as_object)
                                    .map(|v| v.len())
                                    .unwrap_or(0);
                                let pattern_case_count = step
                                    .config
                                    .get("pattern_cases")
                                    .and_then(Value::as_array)
                                    .map(|v| v.len())
                                    .unwrap_or(0);
                                let default = step
                                    .config
                                    .get("default")
                                    .and_then(Value::as_str)
                                    .unwrap_or("none");
                                println!(
                                    "      cases={}, pattern_cases={}, default={}",
                                    case_count, pattern_case_count, default
                                );
                            }
                            _ => {}
                        }
                    }
                } else {
                    println!("Workflow '{name}' not found.");
                }
                continue;
            }

            if let Some(rest) = input
                .strip_prefix("/workflow trigger ")
                .or_else(|| input.strip_prefix("/workflows trigger "))
            {
                let parts = rest.split_whitespace().collect::<Vec<_>>();
                if parts.is_empty() {
                    println!("Usage: /workflow trigger <event <name>|cron>");
                    continue;
                }

                match parts[0] {
                    "event" => {
                        if parts.len() < 2 {
                            println!("Usage: /workflow trigger event <event_name>");
                            continue;
                        }
                        let event_name = parts[1..].join(" ");
                        let matches = rustic_ai_core::workflows::WorkflowTriggerEngine::for_event(
                            self.app.runtime().workflows.as_ref(),
                            &event_name,
                        );
                        if matches.is_empty() {
                            println!("No workflows matched event '{}'.", event_name);
                            continue;
                        }
                        self.run_trigger_matches(
                            session_id,
                            &agent_name,
                            matches,
                            event_tx.clone(),
                        )
                        .await?;
                    }
                    "cron" => {
                        let matches = trigger_engine
                            .due_cron(self.app.runtime().workflows.as_ref(), Utc::now());
                        if matches.is_empty() {
                            println!("No cron workflows due.");
                            continue;
                        }
                        self.run_trigger_matches(
                            session_id,
                            &agent_name,
                            matches,
                            event_tx.clone(),
                        )
                        .await?;
                    }
                    other => {
                        println!(
                            "Unknown trigger source '{}'. Use '/workflow trigger event <name>' or '/workflow trigger cron'.",
                            other
                        );
                    }
                }
                continue;
            }

            let topic_updated = self
                .app
                .session_manager()
                .maybe_refresh_topics(
                    session_id,
                    &self.app.runtime().providers,
                    self.app.topic_inference(),
                    &mut topic_tracker,
                )
                .await
                .unwrap_or(false);
            if topic_updated {
                let current = topic_tracker.current_topics();
                if !current.is_empty() {
                    println!("[topics] {}", current.join(", "));
                }
            }

            let agent = self.app.runtime().agents.get_agent(Some(&agent_name))?;

            let agent_clone = agent.clone();
            let session_id_clone = session_id;
            let input_clone = input.to_string();
            let event_tx_clone = event_tx.clone();
            let event_tx_error = event_tx.clone();

            tokio::spawn(async move {
                if let Err(err) = agent_clone
                    .start_turn(session_id_clone, input_clone, event_tx_clone)
                    .await
                {
                    let _ = event_tx_error.try_send(Event::Error(err.to_string()));
                }
            });

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        renderer_handle.await.ok();
        Ok(())
    }
}
