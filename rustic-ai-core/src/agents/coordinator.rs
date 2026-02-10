use crate::agents::behavior::Agent;
use crate::config::schema::AgentConfig;
use crate::conversation::session_manager::SessionManager;
use crate::error::{Error, Result};
use crate::providers::registry::ProviderRegistry;
use crate::ToolManager;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct AgentCoordinator {
    agents: HashMap<String, Arc<Agent>>,
    default_agent: String,
}

impl AgentCoordinator {
    pub fn new(
        agent_configs: Vec<AgentConfig>,
        provider_registry: &ProviderRegistry,
        tool_manager: Arc<ToolManager>,
        session_manager: Arc<SessionManager>,
    ) -> Result<Self> {
        let mut agents = HashMap::new();
        let mut default_agent = String::new();

        for config in agent_configs {
            let provider = provider_registry
                .get_provider(&config.provider)
                .ok_or_else(|| {
                    Error::Config(format!(
                        "agent '{}' references unknown provider '{}'",
                        config.name, config.provider
                    ))
                })?;

            let agent = Arc::new(Agent::new(
                config.clone(),
                provider,
                tool_manager.clone(),
                session_manager.clone(),
            ));

            agents.insert(config.name.clone(), agent);

            // Use first agent as default if not specified
            if default_agent.is_empty() {
                default_agent = config.name.clone();
            }
        }

        Ok(Self {
            agents,
            default_agent,
        })
    }

    pub fn get_agent(&self, name: Option<&str>) -> Result<Arc<Agent>> {
        let name = name.unwrap_or(&self.default_agent);
        self.agents
            .get(name)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("agent '{}' not found", name)))
    }

    pub fn has_agent(&self, name: &str) -> bool {
        self.agents.contains_key(name)
    }

    pub fn list_agents(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }
}
