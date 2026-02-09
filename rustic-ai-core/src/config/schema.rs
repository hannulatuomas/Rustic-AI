use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    Direct,
    Project,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureConfig {
    pub mcp_enabled: bool,
    pub skills_enabled: bool,
    pub plugins_enabled: bool,
    pub workflows_enabled: bool,
    pub triggers_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub root_path: String,
    pub tech_stack: Vec<String>,
    pub goals: Vec<String>,
    pub preferences: Vec<String>,
    pub style_guidelines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConfig {
    pub global_files: Vec<String>,
    pub project_files: Vec<String>,
    pub topic_files: Vec<String>,
    pub context_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub provider_type: String,
    pub model: String,
    pub auth_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    pub provider: String,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaxonomyConfig {
    pub baskets: Vec<BasketConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasketConfig {
    pub name: String,
    pub sub_baskets: Vec<String>,
}
