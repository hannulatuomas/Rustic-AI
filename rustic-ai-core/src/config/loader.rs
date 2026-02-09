use std::env;
use std::path::{Path, PathBuf};

use crate::config::schema::{Config, RuntimeMode};
use crate::error::{Error, Result};

pub fn load_from_file(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path).map_err(|err| {
        Error::Config(format!("failed to read config '{}': {err}", path.display()))
    })?;

    let config: Config = toml::from_str(&content).map_err(|err| {
        Error::Config(format!(
            "failed to parse config '{}': {err}",
            path.display()
        ))
    })?;

    Ok(config)
}

pub fn load_from_env() -> Result<Config> {
    let mut config = Config::default();

    if let Ok(mode) = env::var("RUSTIC_AI_MODE") {
        config.mode = parse_mode(&mode)?;
    }

    config.features.mcp_enabled =
        parse_bool_env("RUSTIC_AI_ENABLE_MCP", config.features.mcp_enabled)?;
    config.features.skills_enabled =
        parse_bool_env("RUSTIC_AI_ENABLE_SKILLS", config.features.skills_enabled)?;
    config.features.plugins_enabled =
        parse_bool_env("RUSTIC_AI_ENABLE_PLUGINS", config.features.plugins_enabled)?;
    config.features.workflows_enabled = parse_bool_env(
        "RUSTIC_AI_ENABLE_WORKFLOWS",
        config.features.workflows_enabled,
    )?;
    config.features.triggers_enabled = parse_bool_env(
        "RUSTIC_AI_ENABLE_TRIGGERS",
        config.features.triggers_enabled,
    )?;

    Ok(config)
}

pub fn merge(base: Config, override_config: Config) -> Config {
    Config {
        mode: override_config.mode,
        features: override_config.features,
        project: override_config.project.or(base.project),
        rules: merge_rules(base.rules, override_config.rules),
        providers: merge_vec(base.providers, override_config.providers),
        agents: merge_vec(base.agents, override_config.agents),
        tools: merge_vec(base.tools, override_config.tools),
        taxonomy: if override_config.taxonomy.baskets.is_empty() {
            base.taxonomy
        } else {
            override_config.taxonomy
        },
    }
}

pub fn load(path: Option<&Path>) -> Result<Config> {
    let file_config = if let Some(path) = path {
        load_from_file(path)?
    } else {
        let default_path = PathBuf::from("config.toml");
        if default_path.exists() {
            load_from_file(&default_path)?
        } else {
            Config::default()
        }
    };

    let env_config = load_from_env()?;
    Ok(merge(file_config, env_config))
}

fn merge_vec<T>(base: Vec<T>, override_values: Vec<T>) -> Vec<T> {
    if override_values.is_empty() {
        base
    } else {
        override_values
    }
}

fn merge_rules(
    base: crate::config::schema::RuleConfig,
    override_values: crate::config::schema::RuleConfig,
) -> crate::config::schema::RuleConfig {
    crate::config::schema::RuleConfig {
        global_files: merge_vec(base.global_files, override_values.global_files),
        project_files: merge_vec(base.project_files, override_values.project_files),
        topic_files: merge_vec(base.topic_files, override_values.topic_files),
        context_files: merge_vec(base.context_files, override_values.context_files),
    }
}

fn parse_bool_env(name: &str, default: bool) -> Result<bool> {
    match env::var(name) {
        Ok(value) => parse_bool(name, &value),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(Error::Config(format!(
            "environment variable {name} is not valid UTF-8"
        ))),
    }
}

fn parse_bool(name: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(Error::Config(format!(
            "environment variable {name} has invalid boolean value '{value}'"
        ))),
    }
}

fn parse_mode(value: &str) -> Result<RuntimeMode> {
    match value.to_ascii_lowercase().as_str() {
        "direct" => Ok(RuntimeMode::Direct),
        "project" => Ok(RuntimeMode::Project),
        _ => Err(Error::Config(format!(
            "environment variable RUSTIC_AI_MODE must be 'direct' or 'project', got '{value}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::merge;
    use crate::config::schema::{
        AgentConfig, AuthMode, Config, ProviderConfig, ProviderType, RuntimeMode,
    };

    #[test]
    fn merge_prefers_non_empty_override_vectors() {
        let mut base = Config::default();
        base.providers.push(ProviderConfig {
            name: "base".to_owned(),
            provider_type: ProviderType::OpenAi,
            model: Some("gpt-4o-mini".to_owned()),
            auth_mode: AuthMode::ApiKey,
            api_key_env: Some("OPENAI_API_KEY".to_owned()),
            base_url: None,
        });

        let mut override_config = Config {
            mode: RuntimeMode::Project,
            ..Config::default()
        };
        override_config.agents.push(AgentConfig {
            name: "planner".to_owned(),
            provider: "base".to_owned(),
            tools: Vec::new(),
            skills: Vec::new(),
        });

        let merged = merge(base, override_config);
        assert_eq!(merged.mode, RuntimeMode::Project);
        assert_eq!(merged.providers.len(), 1);
        assert_eq!(merged.providers[0].name, "base");
        assert_eq!(merged.agents.len(), 1);
    }
}
