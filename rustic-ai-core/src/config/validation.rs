use std::collections::HashSet;

use crate::config::schema::{AuthMode, Config, ProviderType, RuntimeMode, StorageBackendKind};
use crate::error::{Error, Result};
use crate::providers::auth_capabilities::{supported_auth_mode_names, supports_auth_mode};

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

        if matches!(provider.auth_mode, AuthMode::ApiKey)
            && provider
                .api_key_env
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define api_key_env when auth_mode is api_key"
            )));
        }

        if !supports_auth_mode(&provider.provider_type, &provider.auth_mode) {
            let modes = supported_auth_mode_names(&provider.provider_type).join(", ");
            return Err(Error::Validation(format!(
                "provider '{name}' with type '{:?}' does not support auth_mode '{}'; supported auth modes: {}",
                provider.provider_type,
                match provider.auth_mode {
                    AuthMode::ApiKey => "api_key",
                    AuthMode::Subscription => "subscription",
                    AuthMode::None => "none",
                },
                modes
            )));
        }

        if matches!(provider.provider_type, ProviderType::Anthropic)
            && provider
                .model
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define model for anthropic"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Anthropic)
            && provider
                .base_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define base_url for anthropic"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Anthropic)
            && provider
                .api_key_env
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            && matches!(provider.auth_mode, AuthMode::ApiKey)
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define api_key_env for anthropic"
            )));
        }

        if matches!(provider.provider_type, ProviderType::OpenAi)
            && provider
                .model
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define model for open_ai"
            )));
        }

        if matches!(provider.provider_type, ProviderType::OpenAi)
            && provider
                .base_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define base_url for open_ai"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Google)
            && provider
                .model
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define model for google"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Google)
            && provider
                .base_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define base_url for google"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Google)
            && provider
                .api_key_env
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            && matches!(provider.auth_mode, AuthMode::ApiKey)
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define api_key_env for google"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Grok)
            && provider
                .model
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define model for grok"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Grok)
            && provider
                .base_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define base_url for grok"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Grok)
            && provider
                .api_key_env
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            && matches!(provider.auth_mode, AuthMode::ApiKey)
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define api_key_env for grok"
            )));
        }

        if matches!(provider.provider_type, ProviderType::ZAi)
            && provider
                .model
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define model for z_ai"
            )));
        }

        if matches!(provider.provider_type, ProviderType::ZAi)
            && provider
                .api_key_env
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define api_key_env for z_ai"
            )));
        }

        if matches!(provider.provider_type, ProviderType::ZAi)
            && provider
                .settings
                .as_ref()
                .and_then(|settings| settings.get("coding_base_url"))
                .and_then(|value| value.as_str())
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define settings.coding_base_url for z_ai"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Ollama)
            && provider
                .model
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define model for ollama"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Ollama)
            && provider
                .base_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define base_url for ollama"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Ollama)
            && provider
                .api_key_env
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define api_key_env for ollama"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Custom)
            && provider
                .model
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define model for custom"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Custom)
            && provider
                .base_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define base_url for custom"
            )));
        }

        if matches!(provider.provider_type, ProviderType::Custom)
            && provider
                .api_key_env
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(Error::Validation(format!(
                "provider '{name}' must define api_key_env for custom"
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

    let mut mcp_server_names = HashSet::new();
    for server in &config.mcp.servers {
        let name = server.name.trim();
        if name.is_empty() {
            return Err(Error::Validation(
                "mcp server name cannot be empty".to_owned(),
            ));
        }
        if !mcp_server_names.insert(name.to_owned()) {
            return Err(Error::Validation(format!(
                "duplicate mcp server name '{name}'"
            )));
        }

        if server.command.trim().is_empty() {
            return Err(Error::Validation(format!(
                "mcp server '{name}' must define command"
            )));
        }
        if server.startup_timeout_seconds == 0 {
            return Err(Error::Validation(format!(
                "mcp server '{name}' startup_timeout_seconds must be greater than zero"
            )));
        }
        if server.protocol_version.trim().is_empty() {
            return Err(Error::Validation(format!(
                "mcp server '{name}' protocol_version must be non-empty"
            )));
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

    match config.storage.backend {
        StorageBackendKind::Sqlite => {
            if config.storage.sqlite.journal_mode.trim().is_empty() {
                return Err(Error::Validation(
                    "storage.sqlite.journal_mode must be non-empty".to_owned(),
                ));
            }
            if config.storage.sqlite.synchronous.trim().is_empty() {
                return Err(Error::Validation(
                    "storage.sqlite.synchronous must be non-empty".to_owned(),
                ));
            }
            if config.storage.sqlite.busy_timeout_ms == 0 {
                return Err(Error::Validation(
                    "storage.sqlite.busy_timeout_ms must be greater than zero".to_owned(),
                ));
            }
            if !is_allowed_sqlite_journal_mode(&config.storage.sqlite.journal_mode) {
                return Err(Error::Validation(
                    "storage.sqlite.journal_mode must be one of: DELETE, TRUNCATE, PERSIST, MEMORY, WAL, OFF".to_owned(),
                ));
            }
            if !is_allowed_sqlite_synchronous(&config.storage.sqlite.synchronous) {
                return Err(Error::Validation(
                    "storage.sqlite.synchronous must be one of: OFF, NORMAL, FULL, EXTRA"
                        .to_owned(),
                ));
            }
        }
        StorageBackendKind::Postgres => {
            if config
                .storage
                .postgres
                .connection_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                return Err(Error::Validation(
                    "storage.postgres.connection_url must be set for postgres backend".to_owned(),
                ));
            }
        }
        StorageBackendKind::Custom => {}
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

fn is_allowed_sqlite_journal_mode(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_uppercase().as_str(),
        "DELETE" | "TRUNCATE" | "PERSIST" | "MEMORY" | "WAL" | "OFF"
    )
}

fn is_allowed_sqlite_synchronous(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_uppercase().as_str(),
        "OFF" | "NORMAL" | "FULL" | "EXTRA"
    )
}
