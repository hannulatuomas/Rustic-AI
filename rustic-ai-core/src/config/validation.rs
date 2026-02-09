use std::collections::HashSet;

use crate::config::schema::{Config, RuntimeMode};
use crate::error::{Error, Result};

pub fn validate_config(config: &Config) -> Result<()> {
    if config.providers.is_empty() {
        return Err(Error::Validation(
            "at least one provider must be configured".to_owned(),
        ));
    }

    if config.agents.is_empty() {
        return Err(Error::Validation(
            "at least one agent must be configured".to_owned(),
        ));
    }

    if matches!(config.mode, RuntimeMode::Project) && config.project.is_none() {
        return Err(Error::Validation(
            "project mode requires a project profile".to_owned(),
        ));
    }

    let mut provider_names = HashSet::new();
    for provider in &config.providers {
        let name = provider.name.trim();
        if name.is_empty() {
            return Err(Error::Validation(
                "provider name cannot be empty".to_owned(),
            ));
        }

        if !provider_names.insert(name.to_owned()) {
            return Err(Error::Validation(format!(
                "duplicate provider name '{name}'"
            )));
        }
    }

    let mut tool_names = HashSet::new();
    for tool in &config.tools {
        let name = tool.name.trim();
        if name.is_empty() {
            return Err(Error::Validation("tool name cannot be empty".to_owned()));
        }

        if !tool_names.insert(name.to_owned()) {
            return Err(Error::Validation(format!("duplicate tool name '{name}'")));
        }
    }

    for agent in &config.agents {
        let name = agent.name.trim();
        if name.is_empty() {
            return Err(Error::Validation("agent name cannot be empty".to_owned()));
        }

        if !provider_names.contains(agent.provider.trim()) {
            return Err(Error::Validation(format!(
                "agent '{name}' references missing provider '{}'",
                agent.provider
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_config;
    use crate::config::schema::{
        AgentConfig, AuthMode, Config, ProviderConfig, ProviderType, ToolConfig,
    };

    fn valid_config() -> Config {
        let mut config = Config::default();
        config.providers.push(ProviderConfig {
            name: "openai".to_owned(),
            provider_type: ProviderType::OpenAi,
            model: Some("gpt-4o-mini".to_owned()),
            auth_mode: AuthMode::ApiKey,
            api_key_env: Some("OPENAI_API_KEY".to_owned()),
            base_url: None,
        });
        config.agents.push(AgentConfig {
            name: "orchestrator".to_owned(),
            provider: "openai".to_owned(),
            tools: vec!["shell".to_owned()],
            skills: Vec::new(),
        });
        config.tools.push(ToolConfig {
            name: "shell".to_owned(),
            enabled: true,
        });
        config
    }

    #[test]
    fn accepts_minimal_valid_config() {
        let config = valid_config();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn rejects_agent_with_unknown_provider() {
        let mut config = valid_config();
        config.agents[0].provider = "missing".to_owned();

        let error = validate_config(&config).expect_err("validation should fail");
        let message = error.to_string();
        assert!(message.contains("references missing provider"));
    }
}
