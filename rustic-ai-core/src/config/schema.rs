use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RuleConfig {
    pub global_files: Vec<String>,
    pub project_files: Vec<String>,
    pub topic_files: Vec<String>,
    pub context_files: Vec<String>,
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
