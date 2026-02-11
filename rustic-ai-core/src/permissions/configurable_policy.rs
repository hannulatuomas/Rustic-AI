use crate::config::schema::{
    AgentPermissionMode, CommandPatternConfig, DecisionScope, PermissionConfig, PermissionMode,
};
use crate::permissions::policy::{
    AskResolution, CommandPatternBucket, PermissionContext, PermissionDecision, PermissionPolicy,
};
use crate::rules::discovery::simple_glob_match;

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolAccessKind {
    Read,
    Write,
    Unknown,
}

const DEFAULT_READ_ONLY_SHELL_WRITE_PATTERNS: &[&str] = &[
    " rm ",
    " mv ",
    " cp ",
    " mkdir ",
    " rmdir ",
    " touch ",
    " chmod ",
    " chown ",
    " tee ",
    " sed -i",
    " >",
    " >>",
    " git commit",
    " git push",
];

#[derive(Debug, Clone)]
pub struct ConfigurablePermissionPolicy {
    config: PermissionConfig,
    tool_specific_modes: HashMap<String, PermissionMode>,
    agent_tool_allowlist: HashMap<String, HashSet<String>>,
    allowed_root: PathBuf,
    runtime_global_allowed_paths: HashSet<String>,
    runtime_project_allowed_paths: HashSet<String>,
    session_allowed_paths: HashMap<String, HashSet<String>>,
    runtime_global_command_patterns: CommandPatternConfig,
    runtime_project_command_patterns: CommandPatternConfig,
    session_command_patterns: HashMap<String, CommandPatternConfig>,
    denied_cache: HashMap<(String, String), u64>, // (session_tool, args_signature) -> expiry_timestamp
    allowed_cache: HashMap<(String, String), DecisionScope>, // (session_tool, args_signature) -> scope
}

impl ConfigurablePermissionPolicy {
    pub fn new(
        config: PermissionConfig,
        tool_configs: Vec<(String, PermissionMode)>,
        allowed_root: PathBuf,
        agent_tool_allowlist: HashMap<String, HashSet<String>>,
    ) -> Self {
        let mut tool_specific_modes = HashMap::new();
        for (tool_name, mode) in tool_configs {
            tool_specific_modes.insert(tool_name, mode);
        }

        Self {
            config,
            tool_specific_modes,
            agent_tool_allowlist,
            allowed_root,
            runtime_global_allowed_paths: HashSet::new(),
            runtime_project_allowed_paths: HashSet::new(),
            session_allowed_paths: HashMap::new(),
            runtime_global_command_patterns: CommandPatternConfig::default(),
            runtime_project_command_patterns: CommandPatternConfig::default(),
            session_command_patterns: HashMap::new(),
            denied_cache: HashMap::new(),
            allowed_cache: HashMap::new(),
        }
    }

    fn get_tool_mode(&self, tool: &str) -> PermissionMode {
        self.tool_specific_modes
            .get(tool)
            .copied()
            .unwrap_or(self.config.default_tool_permission)
    }

    fn args_signature(&self, args: &serde_json::Value) -> String {
        serde_json::to_string(args).unwrap_or_else(|_| format!("{:?}", args))
    }

    fn normalize_path(path: &Path) -> PathBuf {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    let _ = normalized.pop();
                }
                _ => normalized.push(component.as_os_str()),
            }
        }
        normalized
    }

    fn resolve_candidate_path(raw_path: &str, working_dir: &Path) -> PathBuf {
        let expanded = Self::expand_home(raw_path);
        let raw = expanded.as_path();
        if raw.is_absolute() {
            Self::normalize_path(raw)
        } else {
            Self::normalize_path(&working_dir.join(raw))
        }
    }

    fn expand_home(raw: &str) -> PathBuf {
        if let Some(suffix) = raw.strip_prefix("~/") {
            if let Ok(home) = std::env::var("HOME") {
                return PathBuf::from(home).join(suffix);
            }
        }
        PathBuf::from(raw)
    }

    fn resolve_allowed_roots(&self, context: &PermissionContext) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        roots.push(self.allowed_root.clone());

        let base = context.working_directory.clone();
        for configured in &self.config.globally_allowed_paths {
            if configured.trim().is_empty() {
                continue;
            }
            let configured_path = Self::expand_home(configured);
            roots.push(if configured_path.is_absolute() {
                configured_path
            } else {
                base.join(configured_path)
            });
        }
        for configured in &self.runtime_global_allowed_paths {
            if configured.trim().is_empty() {
                continue;
            }
            let configured_path = Self::expand_home(configured);
            roots.push(if configured_path.is_absolute() {
                configured_path
            } else {
                base.join(configured_path)
            });
        }
        for configured in &self.config.project_allowed_paths {
            if configured.trim().is_empty() {
                continue;
            }
            let configured_path = Self::expand_home(configured);
            roots.push(if configured_path.is_absolute() {
                configured_path
            } else {
                base.join(configured_path)
            });
        }
        for configured in &self.runtime_project_allowed_paths {
            if configured.trim().is_empty() {
                continue;
            }
            let configured_path = Self::expand_home(configured);
            roots.push(if configured_path.is_absolute() {
                configured_path
            } else {
                base.join(configured_path)
            });
        }
        if let Some(session_paths) = self.session_allowed_paths.get(&context.session_id) {
            for configured in session_paths {
                let configured_path = Self::expand_home(configured);
                roots.push(if configured_path.is_absolute() {
                    configured_path
                } else {
                    base.join(configured_path)
                });
            }
        }

        roots
    }

    fn operation_paths_within_allowed_roots(
        &self,
        tool: &str,
        args: &serde_json::Value,
        context: &PermissionContext,
    ) -> bool {
        let mut paths = Vec::new();
        match tool {
            "filesystem" => {
                let operation = args
                    .get("operation")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                match operation {
                    "copy" | "move" => {
                        if let Some(source) = args.get("source").and_then(|value| value.as_str()) {
                            paths.push(source.to_owned());
                        }
                        if let Some(destination) =
                            args.get("destination").and_then(|value| value.as_str())
                        {
                            paths.push(destination.to_owned());
                        }
                    }
                    "list" | "glob" => {
                        paths.push(
                            args.get("path")
                                .and_then(|value| value.as_str())
                                .unwrap_or(".")
                                .to_owned(),
                        );
                    }
                    _ => {
                        if let Some(path) = args.get("path").and_then(|value| value.as_str()) {
                            paths.push(path.to_owned());
                        }
                    }
                }
            }
            "shell" => {
                if let Some(path) = args
                    .get("working_directory")
                    .and_then(|value| value.as_str())
                {
                    paths.push(path.to_owned());
                }
            }
            _ => return true,
        }

        let canonical_roots = self
            .resolve_allowed_roots(context)
            .into_iter()
            .map(|path| path.canonicalize().unwrap_or(path))
            .collect::<Vec<_>>();
        let canonical_working_dir = context
            .working_directory
            .canonicalize()
            .unwrap_or_else(|_| context.working_directory.clone());

        for path in paths {
            let resolved = Self::resolve_candidate_path(&path, &canonical_working_dir);
            let candidate = resolved.canonicalize().unwrap_or(resolved);

            if !canonical_roots
                .iter()
                .any(|root| candidate.starts_with(root))
            {
                return false;
            }
        }

        true
    }

    fn extract_shell_command(args: &serde_json::Value) -> Option<String> {
        args.get("command")
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn matches_any_pattern(value: &str, patterns: &[String]) -> bool {
        let lowered = value.to_ascii_lowercase();
        patterns
            .iter()
            .filter(|pattern| !pattern.trim().is_empty())
            .any(|pattern| {
                let normalized = pattern.trim().to_ascii_lowercase();
                simple_glob_match(&normalized, &lowered)
                    || lowered.contains(&normalized)
                    || normalized.contains(&lowered)
            })
    }

    fn check_command_patterns(
        &self,
        tool: &str,
        args: &serde_json::Value,
        context: &PermissionContext,
    ) -> Option<PermissionDecision> {
        if tool != "shell" {
            return None;
        }

        let command = Self::extract_shell_command(args)?;

        let session_patterns = self
            .session_command_patterns
            .get(&context.session_id)
            .cloned()
            .unwrap_or_default();

        let deny_patterns = [
            self.config.global_command_patterns.deny.as_slice(),
            self.runtime_global_command_patterns.deny.as_slice(),
            self.config.project_command_patterns.deny.as_slice(),
            self.runtime_project_command_patterns.deny.as_slice(),
            session_patterns.deny.as_slice(),
        ]
        .concat();
        if Self::matches_any_pattern(&command, &deny_patterns) {
            return Some(PermissionDecision::Deny);
        }

        let ask_patterns = [
            self.config.global_command_patterns.ask.as_slice(),
            self.runtime_global_command_patterns.ask.as_slice(),
            self.config.project_command_patterns.ask.as_slice(),
            self.runtime_project_command_patterns.ask.as_slice(),
            session_patterns.ask.as_slice(),
        ]
        .concat();
        if Self::matches_any_pattern(&command, &ask_patterns) {
            return Some(PermissionDecision::Ask);
        }

        let allow_patterns = [
            self.config.global_command_patterns.allow.as_slice(),
            self.runtime_global_command_patterns.allow.as_slice(),
            self.config.project_command_patterns.allow.as_slice(),
            self.runtime_project_command_patterns.allow.as_slice(),
            session_patterns.allow.as_slice(),
        ]
        .concat();
        if Self::matches_any_pattern(&command, &allow_patterns) {
            return Some(PermissionDecision::Allow);
        }

        None
    }

    fn check_denied_cache(&self, context: &PermissionContext, tool: &str, args_sig: &str) -> bool {
        if let Some(&expiry) = self.denied_cache.get(&(
            format!("{}::{}", context.session_id, tool),
            args_sig.to_string(),
        )) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now < expiry {
                return true; // Still denied
            }
        }
        false
    }

    fn check_allowed_cache(
        &self,
        context: &PermissionContext,
        tool: &str,
        args_sig: &str,
    ) -> Option<AskResolution> {
        if let Some(&scope) = self.allowed_cache.get(&(
            format!("{}::{}", context.session_id, tool),
            args_sig.to_string(),
        )) {
            return match scope {
                DecisionScope::Session => Some(AskResolution::AllowInSession),
                DecisionScope::Project | DecisionScope::Global => Some(AskResolution::AllowOnce),
            };
        }
        None
    }

    fn infer_tool_access_kind(tool: &str, args: &serde_json::Value) -> ToolAccessKind {
        match tool {
            "filesystem" => {
                let operation = args
                    .get("operation")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                match operation {
                    "read" | "list" | "info" | "glob" | "hash" => ToolAccessKind::Read,
                    "write" | "edit" | "mkdir" | "delete" | "copy" | "move" => {
                        ToolAccessKind::Write
                    }
                    _ => ToolAccessKind::Unknown,
                }
            }
            "http" => {
                let method = args
                    .get("method")
                    .and_then(|value| value.as_str())
                    .unwrap_or("GET")
                    .to_ascii_uppercase();
                match method.as_str() {
                    "GET" | "HEAD" | "OPTIONS" => ToolAccessKind::Read,
                    "POST" | "PUT" | "PATCH" | "DELETE" => ToolAccessKind::Write,
                    _ => ToolAccessKind::Unknown,
                }
            }
            "ssh" => {
                let operation = args
                    .get("operation")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                match operation {
                    "list_sessions" | "scp_download" => ToolAccessKind::Read,
                    "connect" | "disconnect" | "close_all" | "scp_upload" => ToolAccessKind::Write,
                    "exec" => ToolAccessKind::Unknown,
                    _ => ToolAccessKind::Unknown,
                }
            }
            "shell" => {
                let command = args
                    .get("command")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if command.trim().is_empty() {
                    return ToolAccessKind::Unknown;
                }

                if command.contains(" >")
                    || command.contains(">>")
                    || command.contains(" tee ")
                    || command.contains(" install ")
                {
                    return ToolAccessKind::Write;
                }

                let write_verbs = [
                    "rm", "mv", "cp", "mkdir", "rmdir", "touch", "chmod", "chown", "sed", "apt",
                    "apt-get", "yum", "dnf", "apk", "pip", "npm", "pnpm", "yarn", "cargo",
                ];
                let normalized = format!(" {command} ");
                if write_verbs
                    .iter()
                    .any(|verb| normalized.contains(&format!(" {verb} ")))
                {
                    ToolAccessKind::Write
                } else {
                    ToolAccessKind::Read
                }
            }
            "workflow" | "sub_agent" => ToolAccessKind::Write,
            "skill" | "mcp" => ToolAccessKind::Unknown,
            _ => ToolAccessKind::Unknown,
        }
    }

    fn allow_decision_for_access(access: ToolAccessKind) -> PermissionDecision {
        match access {
            ToolAccessKind::Read => PermissionDecision::AllowRead,
            ToolAccessKind::Write => PermissionDecision::AllowWrite,
            ToolAccessKind::Unknown => PermissionDecision::AllowRead,
        }
    }

    fn shell_access_from_patterns(&self, args: &serde_json::Value) -> ToolAccessKind {
        let command = args
            .get("command")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if command.trim().is_empty() {
            return ToolAccessKind::Unknown;
        }

        let normalized = format!(" {command} ");
        let patterns = if self.config.read_only_shell_write_patterns.is_empty() {
            DEFAULT_READ_ONLY_SHELL_WRITE_PATTERNS
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        } else {
            self.config
                .read_only_shell_write_patterns
                .iter()
                .map(|pattern| pattern.to_ascii_lowercase())
                .collect::<Vec<_>>()
        };

        if patterns.iter().any(|pattern| normalized.contains(pattern)) {
            ToolAccessKind::Write
        } else {
            ToolAccessKind::Read
        }
    }

    fn enforce_cache_bounds(&mut self) {
        let max_entries = self.config.permission_cache_max_entries;
        if max_entries == 0 {
            self.denied_cache.clear();
            self.allowed_cache.clear();
            return;
        }

        while self.denied_cache.len() > max_entries {
            if let Some(key) = self.denied_cache.keys().next().cloned() {
                self.denied_cache.remove(&key);
            } else {
                break;
            }
        }

        while self.allowed_cache.len() > max_entries {
            if let Some(key) = self.allowed_cache.keys().next().cloned() {
                self.allowed_cache.remove(&key);
            } else {
                break;
            }
        }
    }
}

impl PermissionPolicy for ConfigurablePermissionPolicy {
    fn check_tool_permission(
        &self,
        tool: &str,
        args: &serde_json::Value,
        context: &PermissionContext,
    ) -> PermissionDecision {
        let access_kind = if tool == "shell" {
            self.shell_access_from_patterns(args)
        } else {
            Self::infer_tool_access_kind(tool, args)
        };

        if context.agent_permission_mode == AgentPermissionMode::ReadOnly
            && matches!(access_kind, ToolAccessKind::Write | ToolAccessKind::Unknown)
        {
            return PermissionDecision::Deny;
        }

        if let Some(agent_name) = &context.agent_name {
            if let Some(allowed_tools) = self.agent_tool_allowlist.get(agent_name) {
                if !allowed_tools.contains(tool) {
                    return PermissionDecision::Deny;
                }
            }
        }

        if let Some(pattern_decision) = self.check_command_patterns(tool, args, context) {
            match pattern_decision {
                PermissionDecision::Deny => return PermissionDecision::Deny,
                PermissionDecision::Ask => return PermissionDecision::Ask,
                PermissionDecision::Allow
                | PermissionDecision::AllowRead
                | PermissionDecision::AllowWrite => {}
            }
        }

        if !self.operation_paths_within_allowed_roots(tool, args, context) {
            return PermissionDecision::Ask;
        }

        let mode = self.get_tool_mode(tool);

        match mode {
            PermissionMode::Allow => Self::allow_decision_for_access(access_kind),
            PermissionMode::Deny => PermissionDecision::Deny,
            PermissionMode::Ask => {
                let args_sig = self.args_signature(args);

                // Check denied cache first
                if self.check_denied_cache(context, tool, &args_sig) {
                    return PermissionDecision::Deny;
                }

                // Check allowed cache
                if let Some(resolution) = self.check_allowed_cache(context, tool, &args_sig) {
                    return match resolution {
                        AskResolution::AllowOnce => Self::allow_decision_for_access(access_kind),
                        AskResolution::AllowInSession => {
                            Self::allow_decision_for_access(access_kind)
                        }
                        AskResolution::Deny => PermissionDecision::Deny,
                    };
                }

                // Need to ask user
                PermissionDecision::Ask
            }
        }
    }

    fn record_permission(
        &mut self,
        tool: &str,
        args: &serde_json::Value,
        context: &PermissionContext,
        decision: AskResolution,
    ) {
        let args_sig = self.args_signature(args);
        let cache_key = (format!("{}::{}", context.session_id, tool), args_sig);

        match decision {
            AskResolution::AllowOnce | AskResolution::AllowInSession => {
                let scope = match decision {
                    AskResolution::AllowInSession => DecisionScope::Session,
                    _ => self.config.ask_decisions_persist_scope,
                };
                self.allowed_cache.insert(cache_key, scope);
            }
            AskResolution::Deny => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let expiry = now + self.config.remember_denied_duration_secs;
                self.denied_cache.insert(cache_key, expiry);
            }
        }

        self.enforce_cache_bounds();
    }

    fn add_session_allowed_path(&mut self, session_id: &str, path: &str) {
        if path.trim().is_empty() {
            return;
        }
        self.session_allowed_paths
            .entry(session_id.to_owned())
            .or_default()
            .insert(path.trim().to_owned());
    }

    fn add_session_command_pattern(
        &mut self,
        session_id: &str,
        bucket: CommandPatternBucket,
        pattern: &str,
    ) {
        if pattern.trim().is_empty() {
            return;
        }

        let patterns = self
            .session_command_patterns
            .entry(session_id.to_owned())
            .or_default();
        let value = pattern.trim().to_owned();
        match bucket {
            CommandPatternBucket::Allow => patterns.allow.push(value),
            CommandPatternBucket::Ask => patterns.ask.push(value),
            CommandPatternBucket::Deny => patterns.deny.push(value),
        }
    }

    fn add_global_allowed_path(&mut self, path: &str) {
        if path.trim().is_empty() {
            return;
        }
        self.runtime_global_allowed_paths
            .insert(path.trim().to_owned());
    }

    fn add_project_allowed_path(&mut self, path: &str) {
        if path.trim().is_empty() {
            return;
        }
        self.runtime_project_allowed_paths
            .insert(path.trim().to_owned());
    }

    fn add_global_command_pattern(&mut self, bucket: CommandPatternBucket, pattern: &str) {
        if pattern.trim().is_empty() {
            return;
        }
        let value = pattern.trim().to_owned();
        match bucket {
            CommandPatternBucket::Allow => self.runtime_global_command_patterns.allow.push(value),
            CommandPatternBucket::Ask => self.runtime_global_command_patterns.ask.push(value),
            CommandPatternBucket::Deny => self.runtime_global_command_patterns.deny.push(value),
        }
    }

    fn add_project_command_pattern(&mut self, bucket: CommandPatternBucket, pattern: &str) {
        if pattern.trim().is_empty() {
            return;
        }
        let value = pattern.trim().to_owned();
        match bucket {
            CommandPatternBucket::Allow => self.runtime_project_command_patterns.allow.push(value),
            CommandPatternBucket::Ask => self.runtime_project_command_patterns.ask.push(value),
            CommandPatternBucket::Deny => self.runtime_project_command_patterns.deny.push(value),
        }
    }
}
