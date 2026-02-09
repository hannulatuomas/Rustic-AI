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

        if provider
            .model
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define a non-empty model"
            )));
        }

        if provider
            .api_key_env
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define api_key_env"
            )));
        }

        if provider
            .base_url
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define base_url"
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

    if config.rules.max_discovery_depth > 32 {
        return Err(Error::Validation(
            "rules.max_discovery_depth must be 32 or less".to_owned(),
        ));
    }

    if !(0.0..=1.0).contains(&config.rules.topic_similarity_threshold)
        && config.rules.topic_similarity_threshold != 0.0
    {
        return Err(Error::Validation(
            "rules.topic_similarity_threshold must be between 0.0 and 1.0".to_owned(),
        ));
    }

    if config.storage.pool_size == 0 {
        return Err(Error::Validation(
            "storage.pool_size must be greater than zero".to_owned(),
        ));
    }

    if config.storage.default_root_dir_name.trim().is_empty() {
        return Err(Error::Validation(
            "storage.default_root_dir_name must be non-empty".to_owned(),
        ));
    }

    if config.storage.project_database_file.trim().is_empty() {
        return Err(Error::Validation(
            "storage.project_database_file must be non-empty".to_owned(),
        ));
    }

    if config.storage.connection_string_prefix.trim().is_empty() {
        return Err(Error::Validation(
            "storage.connection_string_prefix must be non-empty".to_owned(),
        ));
    }

    if config.storage.global_settings_file.trim().is_empty() {
        return Err(Error::Validation(
            "storage.global_settings_file must be non-empty".to_owned(),
        ));
    }

    if config.storage.global_data_subdir.trim().is_empty() {
        return Err(Error::Validation(
            "storage.global_data_subdir must be non-empty".to_owned(),
        ));
    }

    if config.summarization.max_context_tokens == 0 {
        return Err(Error::Validation(
            "summarization.max_context_tokens must be greater than zero".to_owned(),
        ));
    }

    if config.summarization.summary_max_tokens == 0 {
        return Err(Error::Validation(
            "summarization.summary_max_tokens must be greater than zero".to_owned(),
        ));
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
            model: Some("configured-model".to_owned()),
            auth_mode: AuthMode::ApiKey,
            api_key_env: Some("TEST_PROVIDER_API_KEY_ENV".to_owned()),
            base_url: Some("https://api.openai.com/v1".to_owned()),
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
