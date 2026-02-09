use std::env;
use std::path::{Path, PathBuf};

use crate::config::schema::{Config, RuntimeMode, StorageConfig, SummarizationConfig};
use crate::error::{Error, Result};
use crate::rules::discover_rule_and_context_files;

pub fn load_from_file(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path).map_err(|err| {
        Error::Config(format!("failed to read config '{}': {err}", path.display()))
    })?;

    let config: Config = serde_json::from_str(&content).map_err(|err| {
        Error::Config(format!(
            "failed to parse JSON config '{}': {err}",
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
        storage: merge_storage(base.storage, override_config.storage),
        summarization: merge_summarization(base.summarization, override_config.summarization),
    }
}

pub fn load(path: Option<&Path>) -> Result<Config> {
    let file_config = if let Some(path) = path {
        load_from_file(path)?
    } else {
        let default_path = PathBuf::from("config.json");
        if default_path.exists() {
            load_from_file(&default_path)?
        } else {
            Config::default()
        }
    };

    let mut merged = file_config;
    apply_env_overrides(&mut merged)?;
    apply_rule_defaults(&mut merged.rules);

    let work_dir = env::current_dir().map_err(|err| {
        Error::Config(format!(
            "failed to resolve current working directory: {err}"
        ))
    })?;
    merged.rules = discover_rule_and_context_files(&work_dir, &merged.rules)?;

    Ok(merged)
}

fn apply_env_overrides(config: &mut Config) -> Result<()> {
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

    Ok(())
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
        global_rules_path: merge_string(base.global_rules_path, override_values.global_rules_path),
        project_rules_folder: merge_string(
            base.project_rules_folder,
            override_values.project_rules_folder,
        ),
        additional_search_paths: merge_vec(
            base.additional_search_paths,
            override_values.additional_search_paths,
        ),
        recursive_discovery: merge_bool(
            base.recursive_discovery,
            override_values.recursive_discovery,
        ),
        max_discovery_depth: merge_usize(
            base.max_discovery_depth,
            override_values.max_discovery_depth,
        ),
        use_gitignore: merge_bool(base.use_gitignore, override_values.use_gitignore),
        rule_extensions: merge_vec(base.rule_extensions, override_values.rule_extensions),
        rule_file_names: merge_vec(base.rule_file_names, override_values.rule_file_names),
        context_file_patterns: merge_vec(
            base.context_file_patterns,
            override_values.context_file_patterns,
        ),
        context_extensions: merge_vec(base.context_extensions, override_values.context_extensions),
        topic_debounce_interval_secs: merge_u64(
            base.topic_debounce_interval_secs,
            override_values.topic_debounce_interval_secs,
        ),
        topic_similarity_threshold: if override_values.topic_similarity_threshold > 0.0 {
            override_values.topic_similarity_threshold
        } else {
            base.topic_similarity_threshold
        },
        global_files: merge_vec(base.global_files, override_values.global_files),
        project_files: merge_vec(base.project_files, override_values.project_files),
        topic_files: merge_vec(base.topic_files, override_values.topic_files),
        context_files: merge_vec(base.context_files, override_values.context_files),
        discovered_rules: merge_vec(base.discovered_rules, override_values.discovered_rules),
    }
}

fn merge_string(base: String, override_value: String) -> String {
    if override_value.trim().is_empty() {
        base
    } else {
        override_value
    }
}

fn merge_bool(base: bool, override_value: bool) -> bool {
    if override_value {
        true
    } else {
        base
    }
}

fn merge_usize(base: usize, override_value: usize) -> usize {
    if override_value == 0 {
        base
    } else {
        override_value
    }
}

fn merge_u64(base: u64, override_value: u64) -> u64 {
    if override_value == 0 {
        base
    } else {
        override_value
    }
}

fn merge_summarization(
    base: SummarizationConfig,
    override_values: SummarizationConfig,
) -> SummarizationConfig {
    SummarizationConfig {
        enabled: merge_bool(base.enabled, override_values.enabled),
        provider_name: override_values.provider_name.or(base.provider_name),
        max_context_tokens: merge_usize(
            base.max_context_tokens,
            override_values.max_context_tokens,
        ),
        summary_max_tokens: merge_usize(
            base.summary_max_tokens,
            override_values.summary_max_tokens,
        ),
    }
}

fn merge_storage(base: StorageConfig, override_values: StorageConfig) -> StorageConfig {
    StorageConfig {
        default_root_dir_name: merge_string(
            base.default_root_dir_name,
            override_values.default_root_dir_name,
        ),
        project_data_path: override_values.project_data_path.or(base.project_data_path),
        project_database_file: merge_string(
            base.project_database_file,
            override_values.project_database_file,
        ),
        connection_string_prefix: merge_string(
            base.connection_string_prefix,
            override_values.connection_string_prefix,
        ),
        global_root_path: override_values.global_root_path.or(base.global_root_path),
        global_settings_file: merge_string(
            base.global_settings_file,
            override_values.global_settings_file,
        ),
        global_data_subdir: merge_string(
            base.global_data_subdir,
            override_values.global_data_subdir,
        ),
        pool_size: merge_usize(base.pool_size, override_values.pool_size),
    }
}

fn apply_rule_defaults(rule_config: &mut crate::config::schema::RuleConfig) {
    if rule_config.global_rules_path.trim().is_empty() {
        rule_config.global_rules_path = "~/.rustic-ai/rules".to_owned();
    }
    if rule_config.project_rules_folder.trim().is_empty() {
        rule_config.project_rules_folder = ".agents".to_owned();
    }
    if rule_config.max_discovery_depth == 0 {
        rule_config.max_discovery_depth = 5;
    }
    if rule_config.rule_extensions.is_empty() {
        rule_config.rule_extensions = vec!["md".to_owned(), "rules".to_owned(), "txt".to_owned()];
    }
    if rule_config.rule_file_names.is_empty() {
        rule_config.rule_file_names = vec![".cursorrules".to_owned(), ".windsurfrules".to_owned()];
    }
    if rule_config.context_extensions.is_empty() {
        rule_config.context_extensions = vec!["md".to_owned(), "txt".to_owned()];
    }
    if rule_config.topic_debounce_interval_secs == 0 {
        rule_config.topic_debounce_interval_secs = 30;
    }
    if rule_config.topic_similarity_threshold <= 0.0 || rule_config.topic_similarity_threshold > 1.0
    {
        rule_config.topic_similarity_threshold = 0.5;
    }
    if !rule_config.recursive_discovery && rule_config.max_discovery_depth == 0 {
        rule_config.max_discovery_depth = 1;
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
            model: Some("configured-model".to_owned()),
            auth_mode: AuthMode::ApiKey,
            api_key_env: Some("TEST_PROVIDER_API_KEY_ENV".to_owned()),
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
