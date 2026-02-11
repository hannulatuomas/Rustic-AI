use std::collections::HashMap;
use std::sync::Arc;

use crate::agents::behavior::Agent;
use crate::config::schema::{AgentConfig, AgentPermissionMode};

#[derive(Debug, Clone)]
pub struct AgentSuggestion {
    pub agent_name: String,
    pub score: usize,
}

#[derive(Debug, Default)]
pub struct AgentRegistry {
    agents: HashMap<String, Arc<Agent>>,
    configs: HashMap<String, AgentConfig>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, config: AgentConfig, agent: Arc<Agent>) {
        self.agents.insert(config.name.clone(), agent);
        self.configs.insert(config.name.clone(), config);
    }

    pub fn get_agent(&self, name: &str) -> Option<Arc<Agent>> {
        self.agents.get(name).cloned()
    }

    pub fn get_config(&self, name: &str) -> Option<&AgentConfig> {
        self.configs.get(name)
    }

    pub fn has_agent(&self, name: &str) -> bool {
        self.agents.contains_key(name)
    }

    pub fn list_agents(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }

    pub fn find_by_tool(&self, tool_name: &str) -> Vec<String> {
        self.configs
            .iter()
            .filter_map(|(name, config)| {
                config
                    .tools
                    .iter()
                    .any(|tool| tool == tool_name)
                    .then_some(name.clone())
            })
            .collect()
    }

    pub fn find_by_skill(&self, skill_name: &str) -> Vec<String> {
        self.configs
            .iter()
            .filter_map(|(name, config)| {
                config
                    .skills
                    .iter()
                    .any(|skill| skill == skill_name)
                    .then_some(name.clone())
            })
            .collect()
    }

    pub fn find_by_permission_mode(&self, mode: AgentPermissionMode) -> Vec<String> {
        self.configs
            .iter()
            .filter_map(|(name, config)| (config.permission_mode == mode).then_some(name.clone()))
            .collect()
    }

    pub fn suggest_for_task(&self, task_description: &str) -> Vec<AgentSuggestion> {
        let words = task_description
            .split_whitespace()
            .map(|word| word.to_ascii_lowercase())
            .collect::<Vec<_>>();

        let mut suggestions = self
            .configs
            .iter()
            .map(|(name, config)| {
                let tool_hits = config
                    .tools
                    .iter()
                    .filter(|tool| {
                        let lowered = tool.to_ascii_lowercase();
                        words
                            .iter()
                            .any(|word| lowered.contains(word) || word.contains(&lowered))
                    })
                    .count();

                let skill_hits = config
                    .skills
                    .iter()
                    .filter(|skill| {
                        let lowered = skill.to_ascii_lowercase();
                        words
                            .iter()
                            .any(|word| lowered.contains(word) || word.contains(&lowered))
                    })
                    .count();

                AgentSuggestion {
                    agent_name: name.clone(),
                    score: (tool_hits * 3) + (skill_hits * 2),
                }
            })
            .filter(|suggestion| suggestion.score > 0)
            .collect::<Vec<_>>();

        suggestions.sort_by(|left, right| right.score.cmp(&left.score));
        suggestions
    }
}
