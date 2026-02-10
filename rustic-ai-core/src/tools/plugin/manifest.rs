use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginManifest {
    pub api_version: String,
    #[serde(alias = "name")]
    pub plugin_name: String,
    pub version: String,
    pub tool_name: String,
    pub description: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub schema: Value,
    pub timeout_seconds: Option<u64>,
    pub working_directory: Option<String>,
    pub enabled: bool,
}

impl Default for PluginManifest {
    fn default() -> Self {
        Self {
            api_version: "rustic-ai-plugin/v1".to_owned(),
            plugin_name: String::new(),
            version: "0.0.0".to_owned(),
            tool_name: String::new(),
            description: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            schema: Value::Object(serde_json::Map::new()),
            timeout_seconds: None,
            working_directory: None,
            enabled: true,
        }
    }
}
