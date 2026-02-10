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
    pub project: Option<ProjectConfig>,
    pub rules: RuleConfig,
    pub providers: Vec<ProviderConfig>,
    pub agents: Vec<AgentConfig>,
    pub tools: Vec<ToolConfig>,
    pub taxonomy: TaxonomyConfig,
    pub storage: StorageConfig,
    pub summarization: SummarizationConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: RuntimeMode::Direct,
            features: FeatureConfig::default(),
            project: None,
            rules: RuleConfig::default(),
            providers: Vec::new(),
            agents: Vec::new(),
            tools: Vec::new(),
            taxonomy: TaxonomyConfig::default(),
            storage: StorageConfig::default(),
            summarization: SummarizationConfig::default(),
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
    Ollama,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    ApiKey,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentConfig {
    pub name: String,
    pub provider: String,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolConfig {
    pub name: String,
    pub enabled: bool,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
        }
    }
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
