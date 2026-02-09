pub mod behavior;
pub mod coordinator;
pub mod memory;
pub mod registry;
pub mod state;

use crate::config::schema::AgentConfig;

#[derive(Debug, Clone)]
pub struct Agent {
    pub name: String,
    pub provider: String,
    pub tools: Vec<String>,
}

impl From<&AgentConfig> for Agent {
    fn from(config: &AgentConfig) -> Self {
        Self {
            name: config.name.clone(),
            provider: config.provider.clone(),
            tools: config.tools.clone(),
        }
    }
}
