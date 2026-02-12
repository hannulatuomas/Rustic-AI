use std::env;
use std::path::{Path, PathBuf};

use crate::config::schema::{
    Config, PostgresStorageConfig, RuntimeMode, SqliteStorageConfig, StorageConfig,
    SummarizationConfig,
};
use crate::error::{Error, Result};
use crate::rules::discover_rule_and_context_files;
use serde_json::Value;

pub fn load_from_file(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path).map_err(|err| {
        Error::Config(format!(
            "failed to read config file '{}': {err}",
            path.display()
        ))
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
    config.features.learning_enabled = parse_bool_env(
        "RUSTIC_AI_ENABLE_LEARNING",
        config.features.learning_enabled,
    )?;
    config.features.indexing_enabled = parse_bool_env(
        "RUSTIC_AI_ENABLE_INDEXING",
        config.features.indexing_enabled,
    )?;
    config.features.vector_enabled =
        parse_bool_env("RUSTIC_AI_ENABLE_VECTOR", config.features.vector_enabled)?;
    config.features.rag_enabled =
        parse_bool_env("RUSTIC_AI_ENABLE_RAG", config.features.rag_enabled)?;
    config.features.aggressive_summary_enabled = parse_bool_env(
        "RUSTIC_AI_ENABLE_AGGRESSIVE_SUMMARY",
        config.features.aggressive_summary_enabled,
    )?;
    config.features.todo_tracking_enabled = parse_bool_env(
        "RUSTIC_AI_ENABLE_TODO_TRACKING",
        config.features.todo_tracking_enabled,
    )?;
    config.features.sub_agent_parallel_enabled = parse_bool_env(
        "RUSTIC_AI_ENABLE_SUB_AGENT_PARALLEL",
        config.features.sub_agent_parallel_enabled,
    )?;
    config.features.sub_agent_output_caching_enabled = parse_bool_env(
        "RUSTIC_AI_ENABLE_SUB_AGENT_OUTPUT_CACHING",
        config.features.sub_agent_output_caching_enabled,
    )?;
    config.features.dynamic_routing_enabled = parse_bool_env(
        "RUSTIC_AI_ENABLE_DYNAMIC_ROUTING",
        config.features.dynamic_routing_enabled,
    )?;
    config.retrieval.enabled =
        parse_bool_env("RUSTIC_AI_ENABLE_RETRIEVAL", config.retrieval.enabled)?;

    Ok(config)
}

pub fn merge(base: Config, override_config: Config) -> Config {
    Config {
        mode: override_config.mode,
        features: override_config.features,
        retrieval: merge_retrieval(base.retrieval, override_config.retrieval),
        dynamic_routing: merge_dynamic_routing(
            base.dynamic_routing,
            override_config.dynamic_routing,
        ),
        mcp: if override_config.mcp.servers.is_empty() {
            base.mcp
        } else {
            override_config.mcp
        },
        plugins: if override_config.plugins.directories.is_empty()
            && override_config.plugins.manifest_file_name.trim().is_empty()
        {
            base.plugins
        } else {
            override_config.plugins
        },
        skills: if override_config.skills.directories.is_empty() {
            base.skills
        } else {
            override_config.skills
        },
        workflows: if override_config.workflows.directories.is_empty() {
            base.workflows
        } else {
            override_config.workflows
        },
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
        permissions: override_config.permissions,
    }
}

pub fn load(path: Option<&Path>) -> Result<Config> {
    let base_config = if let Some(path) = path {
        load_from_file(path)?
    } else {
        let default_path = PathBuf::from("config.json");
        if default_path.exists() {
            load_from_file(&default_path)?
        } else {
            Config::default()
        }
    };

    // Load and merge fragments
    let mut merged = load_with_fragments(base_config)?;

    // Apply environment variable overrides
    apply_env_overrides(&mut merged)?;
    apply_rule_defaults(&mut merged.rules);

    // Discover rule and context files
    let work_dir = env::current_dir().map_err(|err| {
        Error::Config(format!(
            "failed to resolve current working directory: {err}"
        ))
    })?;
    merged.rules = discover_rule_and_context_files(&work_dir, &merged.rules)?;

    Ok(merged)
}

pub fn load_with_fragments(base_config: Config) -> Result<Config> {
    // Convert base config to JSON value for merging
    let mut merged_value = serde_json::to_value(&base_config)
        .map_err(|err| Error::Config(format!("failed to serialize base config: {err}")))?;

    // Resolve fragment directories
    let global_fragment_dir = resolve_global_fragment_dir(&merged_value);
    let project_fragment_dir = resolve_project_fragment_dir(&merged_value);

    // Load and merge global fragments (sorted by filename)
    if let Some(dir) = &global_fragment_dir {
        if dir.exists() {
            let global_fragments = load_fragments_from_dir(dir)?;
            for fragment_value in global_fragments {
                merged_value = merge_json_values(merged_value, fragment_value);
            }
        }
    }

    // Load and merge project fragments (sorted by filename)
    if let Some(dir) = &project_fragment_dir {
        if dir.exists() {
            let project_fragments = load_fragments_from_dir(dir)?;
            for fragment_value in project_fragments {
                merged_value = merge_json_values(merged_value, fragment_value);
            }
        }
    }

    // Convert merged JSON back to Config
    let merged_config: Config = serde_json::from_value(merged_value)
        .map_err(|err| Error::Config(format!("failed to parse merged config: {err}")))?;

    Ok(merged_config)
}

fn resolve_global_fragment_dir(config_value: &Value) -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    let default_root = PathBuf::from(home).join(".rustic-ai");

    // Check if storage.global_root_path is configured
    let global_root_path = config_value
        .get("storage")
        .and_then(|s| s.get("global_root_path"))
        .and_then(|v| v.as_str());

    if let Some(path) = global_root_path {
        if !path.is_empty() {
            return Some(PathBuf::from(path).join("config"));
        }
    }

    // Use default ~/.rustic-ai/config
    Some(default_root.join("config"))
}

fn resolve_project_fragment_dir(config_value: &Value) -> Option<PathBuf> {
    let work_dir = env::current_dir().ok()?;

    // Check if storage.default_root_dir_name is configured
    let default_root_dir_name = config_value
        .get("storage")
        .and_then(|s| s.get("default_root_dir_name"))
        .and_then(|v| v.as_str())
        .unwrap_or(".rustic-ai");

    // Check if storage.project_data_path is configured
    let project_data_path = config_value
        .get("storage")
        .and_then(|s| s.get("project_data_path"))
        .and_then(|v| v.as_str());

    let project_dir = if let Some(path) = project_data_path {
        if !path.is_empty() {
            if PathBuf::from(path).is_absolute() {
                PathBuf::from(path)
            } else {
                work_dir.join(path)
            }
        } else {
            work_dir.join(default_root_dir_name)
        }
    } else {
        work_dir.join(default_root_dir_name)
    };

    Some(project_dir.join("config"))
}

fn load_fragments_from_dir(dir: &Path) -> Result<Vec<Value>> {
    let entries = std::fs::read_dir(dir).map_err(|err| {
        Error::Config(format!(
            "failed to read config fragment directory '{}': {err}",
            dir.display()
        ))
    })?;

    let mut fragment_files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            Error::Config(format!(
                "failed to read entry in '{}': {err}",
                dir.display()
            ))
        })?;

        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "json" {
                    fragment_files.push(path);
                }
            }
        }
    }

    // Sort files by filename for deterministic merge order
    fragment_files.sort_by_key(|p| {
        p.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });

    let mut fragments = Vec::new();
    for fragment_path in &fragment_files {
        let content = std::fs::read_to_string(fragment_path).map_err(|err| {
            Error::Config(format!(
                "failed to read config fragment '{}': {err}",
                fragment_path.display()
            ))
        })?;

        let value: Value = serde_json::from_str(&content).map_err(|err| {
            Error::Config(format!(
                "failed to parse config fragment '{}': {err}",
                fragment_path.display()
            ))
        })?;

        // Each fragment must be a JSON object
        if !value.is_object() {
            return Err(Error::Config(format!(
                "config fragment '{}' must be a JSON object",
                fragment_path.display()
            )));
        }

        fragments.push(value);
    }

    Ok(fragments)
}

fn merge_json_values(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(base_obj), Value::Object(overlay_obj)) => {
            let mut merged = base_obj;
            for (key, overlay_value) in overlay_obj {
                let merged_value = if let Some(base_value) = merged.get(&key) {
                    merge_json_values(base_value.clone(), overlay_value)
                } else {
                    overlay_value
                };
                merged.insert(key, merged_value);
            }
            Value::Object(merged)
        }
        (_, overlay) => overlay,
    }
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
    config.features.learning_enabled = parse_bool_env(
        "RUSTIC_AI_ENABLE_LEARNING",
        config.features.learning_enabled,
    )?;
    config.features.indexing_enabled = parse_bool_env(
        "RUSTIC_AI_ENABLE_INDEXING",
        config.features.indexing_enabled,
    )?;
    config.features.vector_enabled =
        parse_bool_env("RUSTIC_AI_ENABLE_VECTOR", config.features.vector_enabled)?;
    config.features.rag_enabled =
        parse_bool_env("RUSTIC_AI_ENABLE_RAG", config.features.rag_enabled)?;
    config.retrieval.enabled =
        parse_bool_env("RUSTIC_AI_ENABLE_RETRIEVAL", config.retrieval.enabled)?;

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
        trigger_mode: override_values.trigger_mode,
        message_window_threshold: override_values
            .message_window_threshold
            .or(base.message_window_threshold),
        token_threshold_percent: override_values
            .token_threshold_percent
            .or(base.token_threshold_percent),
        include_user_task: override_values.include_user_task,
        include_completion_summary: override_values.include_completion_summary,
        quality_tracking_enabled: override_values.quality_tracking_enabled,
        user_rating_prompt: override_values.user_rating_prompt,
    }
}

fn merge_storage(base: StorageConfig, override_values: StorageConfig) -> StorageConfig {
    StorageConfig {
        backend: override_values.backend,
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
        sqlite: merge_sqlite_storage(base.sqlite, override_values.sqlite),
        postgres: merge_postgres_storage(base.postgres, override_values.postgres),
    }
}

fn merge_retrieval(
    base: crate::config::schema::RetrievalConfig,
    override_values: crate::config::schema::RetrievalConfig,
) -> crate::config::schema::RetrievalConfig {
    crate::config::schema::RetrievalConfig {
        enabled: override_values.enabled,
        keyword_top_k: merge_usize(base.keyword_top_k, override_values.keyword_top_k),
        vector_top_k: merge_usize(base.vector_top_k, override_values.vector_top_k),
        max_snippets: merge_usize(base.max_snippets, override_values.max_snippets),
        max_snippet_chars: merge_usize(base.max_snippet_chars, override_values.max_snippet_chars),
        vector_dimension: merge_usize(base.vector_dimension, override_values.vector_dimension),
        min_vector_score: if override_values.min_vector_score > 0.0 {
            override_values.min_vector_score
        } else {
            base.min_vector_score
        },
        inject_as_system_message: override_values.inject_as_system_message,
        context_expansion_lines: merge_usize(
            base.context_expansion_lines,
            override_values.context_expansion_lines,
        ),
        ranking_recency_weight: if override_values.ranking_recency_weight > 0.0 {
            override_values.ranking_recency_weight
        } else {
            base.ranking_recency_weight
        },
        ranking_importance_weight: if override_values.ranking_importance_weight > 0.0 {
            override_values.ranking_importance_weight
        } else {
            base.ranking_importance_weight
        },
        rag_prompt_token_budget: merge_usize(
            base.rag_prompt_token_budget,
            override_values.rag_prompt_token_budget,
        ),
        embedding_backend: override_values.embedding_backend,
        embedding_model: merge_optional_string(
            base.embedding_model,
            override_values.embedding_model,
        ),
        embedding_base_url: merge_optional_string(
            base.embedding_base_url,
            override_values.embedding_base_url,
        ),
        embedding_api_key_env: merge_optional_string(
            base.embedding_api_key_env,
            override_values.embedding_api_key_env,
        ),
    }
}

fn merge_dynamic_routing(
    base: crate::config::schema::DynamicRoutingConfig,
    override_values: crate::config::schema::DynamicRoutingConfig,
) -> crate::config::schema::DynamicRoutingConfig {
    crate::config::schema::DynamicRoutingConfig {
        enabled: merge_bool(base.enabled, override_values.enabled),
        routing_policy: override_values.routing_policy,
        task_keywords: if override_values.task_keywords.is_empty() {
            base.task_keywords
        } else {
            override_values.task_keywords
        },
        fallback_agent: merge_string(base.fallback_agent, override_values.fallback_agent),
        context_pressure_threshold: if override_values.context_pressure_threshold > 0.0 {
            override_values.context_pressure_threshold
        } else {
            base.context_pressure_threshold
        },
        routing_trace_enabled: merge_bool(
            base.routing_trace_enabled,
            override_values.routing_trace_enabled,
        ),
    }
}

fn merge_optional_string(base: Option<String>, override_value: Option<String>) -> Option<String> {
    match override_value {
        Some(value) if !value.trim().is_empty() => Some(value),
        _ => base,
    }
}

fn merge_sqlite_storage(
    base: SqliteStorageConfig,
    override_values: SqliteStorageConfig,
) -> SqliteStorageConfig {
    SqliteStorageConfig {
        busy_timeout_ms: merge_u64(base.busy_timeout_ms, override_values.busy_timeout_ms),
        journal_mode: merge_string(base.journal_mode, override_values.journal_mode),
        synchronous: merge_string(base.synchronous, override_values.synchronous),
        foreign_keys: override_values.foreign_keys,
        vector_extension_enabled: override_values.vector_extension_enabled,
        vector_extension_path: merge_optional_string(
            base.vector_extension_path,
            override_values.vector_extension_path,
        ),
        vector_extension_entrypoint: merge_optional_string(
            base.vector_extension_entrypoint,
            override_values.vector_extension_entrypoint,
        ),
        vector_extension_strict: override_values.vector_extension_strict,
    }
}

fn merge_postgres_storage(
    base: PostgresStorageConfig,
    override_values: PostgresStorageConfig,
) -> PostgresStorageConfig {
    PostgresStorageConfig {
        connection_url: override_values.connection_url.or(base.connection_url),
        schema_name: override_values.schema_name.or(base.schema_name),
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
