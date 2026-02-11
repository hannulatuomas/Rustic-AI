use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleScopeConfig {
    Global,
    #[default]
    Project,
    Topic,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleApplicability {
    #[default]
    General,
    ContextSpecific,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mode: RuntimeMode,
    pub features: FeatureConfig,
    pub mcp: McpConfig,
    pub plugins: PluginConfig,
    pub skills: SkillsConfig,
    pub workflows: WorkflowsConfig,
    pub project: Option<ProjectConfig>,
    pub rules: RuleConfig,
    pub providers: Vec<ProviderConfig>,
    pub agents: Vec<AgentConfig>,
    pub tools: Vec<ToolConfig>,
    pub taxonomy: TaxonomyConfig,
    pub storage: StorageConfig,
    pub summarization: SummarizationConfig,
    pub permissions: PermissionConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: RuntimeMode::Direct,
            features: FeatureConfig::default(),
            mcp: McpConfig::default(),
            plugins: PluginConfig::default(),
            skills: SkillsConfig::default(),
            workflows: WorkflowsConfig::default(),
            project: None,
            rules: RuleConfig::default(),
            providers: Vec::new(),
            agents: Vec::new(),
            tools: Vec::new(),
            taxonomy: TaxonomyConfig::default(),
            storage: StorageConfig::default(),
            summarization: SummarizationConfig::default(),
            permissions: PermissionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct McpConfig {
    pub servers: Vec<McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
    pub working_directory: Option<String>,
    pub startup_timeout_seconds: u64,
    pub protocol_version: String,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: std::collections::BTreeMap::new(),
            working_directory: None,
            startup_timeout_seconds: 20,
            protocol_version: "2024-11-05".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginConfig {
    pub directories: Vec<String>,
    pub manifest_file_name: String,
    pub max_discovery_depth: usize,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            directories: vec![
                "~/.rustic-ai/plugins".to_owned(),
                ".rustic-ai/plugins".to_owned(),
            ],
            manifest_file_name: "plugin.json".to_owned(),
            max_discovery_depth: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillsConfig {
    pub directories: Vec<String>,
    pub max_discovery_depth: usize,
    pub script_execution_mode: ScriptExecutionMode,
    pub default_timeout_seconds: u64,
    pub sandbox: SkillSandboxConfig,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            directories: vec![
                "~/.rustic-ai/skills".to_owned(),
                ".rustic-ai/skills".to_owned(),
                ".agents/skills".to_owned(),
            ],
            max_discovery_depth: 4,
            script_execution_mode: ScriptExecutionMode::Disabled,
            default_timeout_seconds: 60,
            sandbox: SkillSandboxConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScriptExecutionMode {
    #[default]
    Disabled,
    Host,
    Sandbox,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillSandboxConfig {
    pub enabled: bool,
    pub sandbox_type: String,
}

impl Default for SkillSandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sandbox_type: "none".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkflowsConfig {
    pub directories: Vec<String>,
    pub max_discovery_depth: usize,
    pub default_timeout_seconds: u64,
    pub compatibility_preset: WorkflowCompatibilityPreset,
    pub switch_case_sensitive_default: Option<bool>,
    pub switch_pattern_priority: Option<String>,
    pub loop_continue_on_iteration_error_default: Option<bool>,
    pub wait_timeout_succeeds: Option<bool>,
    pub condition_missing_path_as_false: Option<bool>,
    pub default_continue_on_error: Option<bool>,
    pub continue_on_error_routing: Option<String>,
    pub execution_error_policy: Option<String>,
    pub timeout_error_policy: Option<String>,
    pub default_retry_count: Option<u32>,
    pub default_retry_backoff_ms: Option<u64>,
    pub default_retry_backoff_multiplier: Option<f64>,
    pub default_retry_backoff_max_ms: Option<u64>,
    pub max_recursion_depth: Option<usize>,
    pub max_steps_per_run: Option<usize>,
    pub condition_group_max_depth: usize,
    pub expression_max_length: usize,
    pub expression_max_depth: usize,
    pub loop_default_max_iterations: u64,
    pub loop_default_max_parallelism: u64,
    pub loop_hard_max_parallelism: u64,
    pub wait_default_poll_interval_ms: u64,
    pub wait_default_timeout_seconds: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCompatibilityPreset {
    #[default]
    Rustic,
    OpenCode,
    ClaudeCode,
    N8n,
}

impl Default for WorkflowsConfig {
    fn default() -> Self {
        Self {
            directories: vec![
                "~/.rustic-ai/workflows".to_owned(),
                ".rustic-ai/workflows".to_owned(),
                ".agents/workflows".to_owned(),
            ],
            max_discovery_depth: 4,
            default_timeout_seconds: 300,
            compatibility_preset: WorkflowCompatibilityPreset::Rustic,
            switch_case_sensitive_default: None,
            switch_pattern_priority: None,
            loop_continue_on_iteration_error_default: None,
            wait_timeout_succeeds: None,
            condition_missing_path_as_false: None,
            default_continue_on_error: None,
            continue_on_error_routing: None,
            execution_error_policy: None,
            timeout_error_policy: None,
            default_retry_count: None,
            default_retry_backoff_ms: None,
            default_retry_backoff_multiplier: None,
            default_retry_backoff_max_ms: None,
            max_recursion_depth: Some(16),
            max_steps_per_run: Some(256),
            condition_group_max_depth: 8,
            expression_max_length: 8_192,
            expression_max_depth: 64,
            loop_default_max_iterations: 1_000,
            loop_default_max_parallelism: 8,
            loop_hard_max_parallelism: 256,
            wait_default_poll_interval_ms: 250,
            wait_default_timeout_seconds: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub backend: StorageBackendKind,
    pub default_root_dir_name: String,
    pub project_data_path: Option<String>,
    pub project_database_file: String,
    pub connection_string_prefix: String,
    pub global_root_path: Option<String>,
    pub global_settings_file: String,
    pub global_data_subdir: String,
    pub pool_size: usize,
    pub sqlite: SqliteStorageConfig,
    pub postgres: PostgresStorageConfig,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: StorageBackendKind::Sqlite,
            default_root_dir_name: ".rustic-ai".to_owned(),
            project_data_path: None,
            project_database_file: "sessions.db".to_owned(),
            connection_string_prefix: "sqlite://".to_owned(),
            global_root_path: None,
            global_settings_file: "settings.json".to_owned(),
            global_data_subdir: "data".to_owned(),
            pool_size: 5,
            sqlite: SqliteStorageConfig::default(),
            postgres: PostgresStorageConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageBackendKind {
    #[default]
    Sqlite,
    Postgres,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SqliteStorageConfig {
    pub busy_timeout_ms: u64,
    pub journal_mode: String,
    pub synchronous: String,
    pub foreign_keys: bool,
}

impl Default for SqliteStorageConfig {
    fn default() -> Self {
        Self {
            busy_timeout_ms: 5_000,
            journal_mode: "WAL".to_owned(),
            synchronous: "NORMAL".to_owned(),
            foreign_keys: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PostgresStorageConfig {
    pub connection_url: Option<String>,
    pub schema_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SummarizationConfig {
    pub enabled: bool,
    pub provider_name: Option<String>,
    pub max_context_tokens: usize,
    pub summary_max_tokens: usize,
}

impl Default for SummarizationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider_name: None,
            max_context_tokens: 16_000,
            summary_max_tokens: 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionConfig {
    pub default_tool_permission: PermissionMode,
    pub ask_decisions_persist_scope: DecisionScope,
    pub remember_denied_duration_secs: u64,
    pub globally_allowed_paths: Vec<String>,
    pub project_allowed_paths: Vec<String>,
    pub global_command_patterns: CommandPatternConfig,
    pub project_command_patterns: CommandPatternConfig,
    /// Timeout in seconds for pending tool execution states (default: 300 = 5 minutes)
    pub pending_tool_timeout_secs: u64,
    /// Time-to-live for sudo password cache in RAM (default: 300 = 5 minutes)
    /// Never persisted to disk/session history/logs; only in-memory for security
    pub sudo_cache_ttl_secs: u64,
    /// Maximum number of cached allow/deny permission decisions in memory.
    /// Prevents unbounded cache growth in long-running sessions.
    pub permission_cache_max_entries: usize,
    /// Patterns considered write-like for shell commands in read-only agent mode.
    /// Operators can tune this list for stricter or looser behavior.
    pub read_only_shell_write_patterns: Vec<String>,
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            default_tool_permission: PermissionMode::Ask,
            ask_decisions_persist_scope: DecisionScope::Session,
            remember_denied_duration_secs: 0,
            globally_allowed_paths: Vec::new(),
            project_allowed_paths: Vec::new(),
            global_command_patterns: CommandPatternConfig::default(),
            project_command_patterns: CommandPatternConfig::default(),
            pending_tool_timeout_secs: 300,
            sudo_cache_ttl_secs: 300,
            permission_cache_max_entries: 4_096,
            read_only_shell_write_patterns: vec![
                " rm ".to_owned(),
                " mv ".to_owned(),
                " cp ".to_owned(),
                " mkdir ".to_owned(),
                " rmdir ".to_owned(),
                " touch ".to_owned(),
                " chmod ".to_owned(),
                " chown ".to_owned(),
                " tee ".to_owned(),
                " sed -i".to_owned(),
                " >".to_owned(),
                " >>".to_owned(),
                " git commit".to_owned(),
                " git push".to_owned(),
                " apt ".to_owned(),
                " apt-get ".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CommandPatternConfig {
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionScope {
    Session,
    #[default]
    Project,
    Global,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    Direct,
    Project,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureConfig {
    pub mcp_enabled: bool,
    pub skills_enabled: bool,
    pub plugins_enabled: bool,
    pub workflows_enabled: bool,
    pub triggers_enabled: bool,
}

impl Default for FeatureConfig {
    fn default() -> Self {
        Self {
            mcp_enabled: false,
            skills_enabled: true,
            plugins_enabled: false,
            workflows_enabled: true,
            triggers_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProjectConfig {
    pub name: String,
    pub root_path: String,
    pub tech_stack: Vec<String>,
    pub goals: Vec<String>,
    pub preferences: Vec<String>,
    pub style_guidelines: Vec<String>,
    pub summarization_enabled: Option<bool>,
    pub summarization_provider_name: Option<String>,
    pub summary_max_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuleConfig {
    pub global_rules_path: String,
    pub project_rules_folder: String,
    pub additional_search_paths: Vec<String>,
    pub recursive_discovery: bool,
    pub max_discovery_depth: usize,
    pub use_gitignore: bool,
    pub rule_extensions: Vec<String>,
    pub rule_file_names: Vec<String>,
    pub context_file_patterns: Vec<String>,
    pub context_extensions: Vec<String>,
    pub topic_debounce_interval_secs: u64,
    pub topic_similarity_threshold: f64,
    pub global_files: Vec<String>,
    pub project_files: Vec<String>,
    pub topic_files: Vec<String>,
    pub context_files: Vec<String>,
    pub discovered_rules: Vec<DiscoveredRuleConfig>,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            global_rules_path: "~/.rustic-ai/rules".to_owned(),
            project_rules_folder: ".agents".to_owned(),
            additional_search_paths: Vec::new(),
            recursive_discovery: true,
            max_discovery_depth: 5,
            use_gitignore: true,
            rule_extensions: vec!["md".to_owned(), "rules".to_owned(), "txt".to_owned()],
            rule_file_names: vec![".cursorrules".to_owned(), ".windsurfrules".to_owned()],
            context_file_patterns: Vec::new(),
            context_extensions: vec!["md".to_owned(), "txt".to_owned()],
            topic_debounce_interval_secs: 30,
            topic_similarity_threshold: 0.5,
            global_files: Vec::new(),
            project_files: Vec::new(),
            topic_files: Vec::new(),
            context_files: Vec::new(),
            discovered_rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveredRuleConfig {
    pub path: String,
    pub scope: RuleScopeConfig,
    pub description: Option<String>,
    pub globs: Vec<String>,
    pub always_apply: bool,
    pub applicability: RuleApplicability,
    pub topics: Vec<String>,
    pub priority: Option<i32>,
}

impl Default for DiscoveredRuleConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            scope: RuleScopeConfig::Project,
            description: None,
            globs: Vec::new(),
            always_apply: false,
            applicability: RuleApplicability::General,
            topics: Vec::new(),
            priority: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub name: String,
    pub provider_type: ProviderType,
    pub model: Option<String>,
    pub auth_mode: AuthMode,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub settings: Option<serde_json::Value>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            provider_type: ProviderType::OpenAi,
            model: None,
            auth_mode: AuthMode::ApiKey,
            api_key_env: None,
            base_url: None,
            settings: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    OpenAi,
    Anthropic,
    Grok,
    Google,
    ZAi,
    Ollama,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    ApiKey,
    Subscription,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentConfig {
    pub name: String,
    pub provider: String,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
    pub system_prompt_template: Option<String>,
    pub temperature: f32,
    pub max_tokens: usize,
    pub context_window_size: usize,
    pub max_tool_rounds: Option<usize>,
    pub max_tools_per_round: Option<usize>,
    pub max_total_tool_calls_per_turn: Option<usize>,
    pub max_turn_duration_seconds: Option<u64>,
    pub permission_mode: AgentPermissionMode,
    pub allow_sub_agent_calls: bool,
    pub max_sub_agent_depth: Option<usize>,
    pub sub_agent_context_window_size: Option<usize>,
    pub sub_agent_max_context_tokens: Option<usize>,
    pub context_summary_enabled: Option<bool>,
    pub context_summary_max_tokens: Option<usize>,
    pub context_summary_cache_entries: Option<usize>,
    pub taxonomy_membership: Vec<TaxonomyMembershipConfig>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentPermissionMode {
    ReadOnly,
    #[default]
    ReadWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolConfig {
    pub name: String,
    pub enabled: bool,
    pub permission_mode: PermissionMode,
    pub timeout_seconds: u64,
    pub allowed_commands: Vec<String>,
    pub denied_commands: Vec<String>,
    pub working_dir: WorkingDirMode,
    pub custom_working_dir: Option<String>,
    pub env_passthrough: bool,
    pub stream_output: bool,
    /// Explicit sudo requirement (optional, default false)
    /// When true, always prompt for sudo regardless of command patterns
    pub require_sudo: bool,
    /// Command patterns that always require sudo privilege (optional, default empty)
    /// Shell commands matching these patterns will always trigger sudo prompt
    pub privileged_command_patterns: Vec<String>,
    /// Shell-only: command patterns blocked in read-only agent mode.
    /// Empty uses defaults from code; set explicitly to tune behavior.
    pub read_only_blocked_patterns: Vec<String>,
    pub taxonomy_membership: Vec<TaxonomyMembershipConfig>,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            permission_mode: PermissionMode::Ask,
            timeout_seconds: 30,
            allowed_commands: Vec::new(),
            denied_commands: Vec::new(),
            working_dir: WorkingDirMode::Current,
            custom_working_dir: None,
            env_passthrough: true,
            stream_output: true,
            require_sudo: false,
            privileged_command_patterns: Vec::new(),
            read_only_blocked_patterns: vec![
                " rm ".to_owned(),
                " mv ".to_owned(),
                " cp ".to_owned(),
                " mkdir ".to_owned(),
                " rmdir ".to_owned(),
                " touch ".to_owned(),
                " chmod ".to_owned(),
                " chown ".to_owned(),
                " tee ".to_owned(),
                " sed -i".to_owned(),
                " >".to_owned(),
                " >>".to_owned(),
                " git commit".to_owned(),
                " git push".to_owned(),
            ],
            taxonomy_membership: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TaxonomyMembershipConfig {
    pub basket: String,
    pub sub_basket: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Allow,
    Deny,
    #[default]
    Ask,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkingDirMode {
    Current,
    #[default]
    ProjectRoot,
    CustomPath,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaxonomyConfig {
    pub baskets: Vec<BasketConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BasketConfig {
    pub name: String,
    pub sub_baskets: Vec<String>,
}
