use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::RwLock;

use crate::config::path::{ConfigPath, ConfigScope};
use crate::config::schema::{Config, RuntimeMode, StorageBackendKind};
use crate::config::validate_config;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigChange {
    pub scope: ConfigScope,
    pub path: ConfigPath,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    pub config: Config,
    pub version: u64,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigValueSource {
    ProjectExplicit,
    GlobalExplicit,
    Defaulted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigResolvedValue {
    pub effective: Value,
    pub source: ConfigValueSource,
    pub project: Option<Value>,
    pub global: Option<Value>,
}

pub struct ConfigManager {
    inner: Arc<RwLock<ConfigState>>,
}

#[derive(Debug, Clone)]
struct ConfigState {
    project_path: PathBuf,
    global_path: PathBuf,
    project_raw: Value,
    global_raw: Option<Value>,
    effective_raw: Value,
    effective_config: Config,
    version: u64,
}

impl ConfigManager {
    pub async fn load(project_path: PathBuf, global_path: PathBuf) -> Result<Self> {
        let project_raw = read_json_file_or_empty_object(&project_path)?;
        let global_raw = if global_path.exists() {
            Some(read_json_file_or_empty_object(&global_path)?)
        } else {
            None
        };

        let (effective_raw, effective_config) =
            materialize_effective(&project_raw, global_raw.as_ref())?;

        Ok(Self {
            inner: Arc::new(RwLock::new(ConfigState {
                project_path,
                global_path,
                project_raw,
                global_raw,
                effective_raw,
                effective_config,
                version: 1,
            })),
        })
    }

    pub async fn snapshot(&self) -> Result<ConfigSnapshot> {
        let state = self.inner.read().await;
        Ok(ConfigSnapshot {
            config: state.effective_config.clone(),
            version: state.version,
            path: state.project_path.clone(),
        })
    }

    pub async fn get_value(&self, scope: ConfigScope, path: &ConfigPath) -> Result<Value> {
        let state = self.inner.read().await;
        match scope {
            ConfigScope::Effective => get_effective_value(&state.effective_config, path),
            ConfigScope::Project => get_raw_value(&state.project_raw, path).ok_or_else(|| {
                Error::NotFound(format!("path '{}' not set in project scope", path))
            }),
            ConfigScope::Global => {
                let Some(global_raw) = &state.global_raw else {
                    return Err(Error::NotFound("global config is not loaded".to_owned()));
                };
                get_raw_value(global_raw, path).ok_or_else(|| {
                    Error::NotFound(format!("path '{}' not set in global scope", path))
                })
            }
            ConfigScope::Session => Err(Error::Config(
                "session-scope config reads are not implemented yet".to_owned(),
            )),
        }
    }

    pub async fn get_effective_value_with_source(
        &self,
        path: &ConfigPath,
    ) -> Result<ConfigResolvedValue> {
        let state = self.inner.read().await;
        let effective = get_effective_value(&state.effective_config, path)?;
        let project = get_raw_value(&state.project_raw, path);
        let global = state
            .global_raw
            .as_ref()
            .and_then(|global_raw| get_raw_value(global_raw, path));

        let source = if project.is_some() {
            ConfigValueSource::ProjectExplicit
        } else if global.is_some() {
            ConfigValueSource::GlobalExplicit
        } else {
            ConfigValueSource::Defaulted
        };

        Ok(ConfigResolvedValue {
            effective,
            source,
            project,
            global,
        })
    }

    pub async fn set_value(
        &self,
        scope: ConfigScope,
        path: ConfigPath,
        value: Value,
        expected_version: Option<u64>,
    ) -> Result<ConfigSnapshot> {
        self.patch(vec![ConfigChange { scope, path, value }], expected_version)
            .await
    }

    pub async fn unset_value(
        &self,
        scope: ConfigScope,
        path: ConfigPath,
        expected_version: Option<u64>,
    ) -> Result<ConfigSnapshot> {
        let mut state = self.inner.write().await;
        ensure_expected_version(&state, expected_version)?;

        let changed = match scope {
            ConfigScope::Project => unset_raw_value(&mut state.project_raw, &path)?,
            ConfigScope::Global => {
                if state.global_raw.is_none() {
                    state.global_raw = Some(empty_object());
                }
                let global_raw = state
                    .global_raw
                    .as_mut()
                    .ok_or_else(|| Error::Config("global config is unavailable".to_owned()))?;
                unset_raw_value(global_raw, &path)?
            }
            ConfigScope::Effective => {
                return Err(Error::Config(
                    "cannot write to effective scope; use project or global".to_owned(),
                ))
            }
            ConfigScope::Session => {
                return Err(Error::Config(
                    "session-scope config writes are not implemented yet".to_owned(),
                ))
            }
        };

        if !changed {
            return Err(Error::NotFound(format!(
                "path '{}' was not set in target scope",
                path
            )));
        }

        commit_state(
            &mut state,
            matches!(scope, ConfigScope::Project),
            matches!(scope, ConfigScope::Global),
        )
    }

    pub async fn patch(
        &self,
        changes: Vec<ConfigChange>,
        expected_version: Option<u64>,
    ) -> Result<ConfigSnapshot> {
        let mut state = self.inner.write().await;
        ensure_expected_version(&state, expected_version)?;

        let mut next_project = state.project_raw.clone();
        let mut next_global = state.global_raw.clone();
        let mut project_dirty = false;
        let mut global_dirty = false;

        for change in &changes {
            match change.scope {
                ConfigScope::Project => {
                    set_raw_value(&mut next_project, &change.path, change.value.clone())?;
                    project_dirty = true;
                }
                ConfigScope::Global => {
                    if next_global.is_none() {
                        next_global = Some(empty_object());
                    }
                    let target = next_global
                        .as_mut()
                        .ok_or_else(|| Error::Config("global config is unavailable".to_owned()))?;
                    set_raw_value(target, &change.path, change.value.clone())?;
                    global_dirty = true;
                }
                ConfigScope::Effective => {
                    return Err(Error::Config(
                        "cannot write to effective scope; use project or global".to_owned(),
                    ))
                }
                ConfigScope::Session => {
                    return Err(Error::Config(
                        "session-scope config writes are not implemented yet".to_owned(),
                    ))
                }
            }
        }

        state.project_raw = next_project;
        state.global_raw = next_global;

        commit_state(&mut state, project_dirty, global_dirty)
    }

    pub async fn reload(&self) -> Result<ConfigSnapshot> {
        let mut state = self.inner.write().await;
        state.project_raw = read_json_file_or_empty_object(&state.project_path)?;
        state.global_raw = if state.global_path.exists() {
            Some(read_json_file_or_empty_object(&state.global_path)?)
        } else {
            None
        };

        let (effective_raw, effective_config) =
            materialize_effective(&state.project_raw, state.global_raw.as_ref())?;
        state.effective_raw = effective_raw;
        state.effective_config = effective_config;
        state.version += 1;

        Ok(ConfigSnapshot {
            config: state.effective_config.clone(),
            version: state.version,
            path: state.project_path.clone(),
        })
    }
}

fn ensure_expected_version(state: &ConfigState, expected_version: Option<u64>) -> Result<()> {
    if let Some(expected_version) = expected_version {
        if expected_version != state.version {
            return Err(Error::Validation(format!(
                "config version conflict: expected {expected_version}, current {}",
                state.version
            )));
        }
    }
    Ok(())
}

fn commit_state(
    state: &mut ConfigState,
    project_dirty: bool,
    global_dirty: bool,
) -> Result<ConfigSnapshot> {
    let (effective_raw, effective_config) =
        materialize_effective(&state.project_raw, state.global_raw.as_ref())?;

    if global_dirty {
        if let Some(global_raw) = &state.global_raw {
            write_json_atomic_value(&state.global_path, global_raw)?;
        }
    }
    if project_dirty {
        write_json_atomic_value(&state.project_path, &state.project_raw)?;
    }

    state.effective_raw = effective_raw;
    state.effective_config = effective_config;
    state.version += 1;

    Ok(ConfigSnapshot {
        config: state.effective_config.clone(),
        version: state.version,
        path: state.project_path.clone(),
    })
}

fn materialize_effective(
    project_raw: &Value,
    global_raw: Option<&Value>,
) -> Result<(Value, Config)> {
    let base = global_raw.cloned().unwrap_or_else(empty_object);
    let merged = merge_json(&base, project_raw);
    let typed: Config = serde_json::from_value(merged.clone())?;
    validate_config(&typed)?;
    Ok((merged, typed))
}

fn merge_json(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(base_obj), Value::Object(overlay_obj)) => {
            let mut merged = base_obj.clone();
            for (key, value) in overlay_obj {
                let merged_value = if let Some(existing) = merged.get(key) {
                    merge_json(existing, value)
                } else {
                    value.clone()
                };
                merged.insert(key.clone(), merged_value);
            }
            Value::Object(merged)
        }
        (_, other) => other.clone(),
    }
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

fn read_json_file_or_empty_object(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(empty_object());
    }

    let content = std::fs::read_to_string(path)?;
    let parsed: Value = serde_json::from_str(&content)?;
    if parsed.is_object() {
        Ok(parsed)
    } else {
        Err(Error::Config(format!(
            "config file '{}' must contain a JSON object at root",
            path.display()
        )))
    }
}

fn write_json_atomic_value(path: &Path, payload: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let rendered = serde_json::to_string_pretty(payload)?;
    let tmp_path = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp_path)?;
        use std::io::Write;
        file.write_all(rendered.as_bytes())?;
        file.sync_all()?;
    }

    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn get_effective_value(config: &Config, path: &ConfigPath) -> Result<Value> {
    match path {
        ConfigPath::Mode => Ok(Value::String(match config.mode {
            RuntimeMode::Direct => "direct".to_owned(),
            RuntimeMode::Project => "project".to_owned(),
        })),
        ConfigPath::StorageBackend => Ok(Value::String(match config.storage.backend {
            StorageBackendKind::Sqlite => "sqlite".to_owned(),
            StorageBackendKind::Postgres => "postgres".to_owned(),
            StorageBackendKind::Custom => "custom".to_owned(),
        })),
        ConfigPath::StoragePoolSize => Ok(Value::from(config.storage.pool_size as u64)),
        ConfigPath::StorageProjectDataPath => {
            option_to_json(config.storage.project_data_path.clone())
        }
        ConfigPath::StorageGlobalRootPath => {
            option_to_json(config.storage.global_root_path.clone())
        }
        ConfigPath::StorageProjectDatabaseFile => {
            Ok(Value::String(config.storage.project_database_file.clone()))
        }
        ConfigPath::StorageConnectionStringPrefix => Ok(Value::String(
            config.storage.connection_string_prefix.clone(),
        )),
        ConfigPath::StorageSqliteBusyTimeoutMs => {
            Ok(Value::from(config.storage.sqlite.busy_timeout_ms))
        }
        ConfigPath::StorageSqliteJournalMode => {
            Ok(Value::String(config.storage.sqlite.journal_mode.clone()))
        }
        ConfigPath::StorageSqliteSynchronous => {
            Ok(Value::String(config.storage.sqlite.synchronous.clone()))
        }
        ConfigPath::StorageSqliteForeignKeys => Ok(Value::Bool(config.storage.sqlite.foreign_keys)),
        ConfigPath::StoragePostgresConnectionUrl => {
            option_to_json(config.storage.postgres.connection_url.clone())
        }
        ConfigPath::StoragePostgresSchemaName => {
            option_to_json(config.storage.postgres.schema_name.clone())
        }
        ConfigPath::SummarizationEnabled => Ok(Value::Bool(config.summarization.enabled)),
        ConfigPath::SummarizationProviderName => {
            option_to_json(config.summarization.provider_name.clone())
        }
        ConfigPath::SummarizationMaxContextTokens => {
            Ok(Value::from(config.summarization.max_context_tokens as u64))
        }
        ConfigPath::SummarizationSummaryMaxTokens => {
            Ok(Value::from(config.summarization.summary_max_tokens as u64))
        }
        ConfigPath::RuleProjectRulesFolder => {
            Ok(Value::String(config.rules.project_rules_folder.clone()))
        }
        ConfigPath::RuleRecursiveDiscovery => Ok(Value::Bool(config.rules.recursive_discovery)),
        ConfigPath::RuleMaxDiscoveryDepth => {
            Ok(Value::from(config.rules.max_discovery_depth as u64))
        }
        ConfigPath::RuleUseGitignore => Ok(Value::Bool(config.rules.use_gitignore)),
        ConfigPath::RuleTopicDebounceIntervalSecs => {
            Ok(Value::from(config.rules.topic_debounce_interval_secs))
        }
        ConfigPath::RuleTopicSimilarityThreshold => {
            Ok(Value::from(config.rules.topic_similarity_threshold))
        }
        ConfigPath::ProviderModel { provider_name } => {
            let provider = find_provider(config, provider_name)?;
            option_to_json(provider.model.clone())
        }
        ConfigPath::ProviderBaseUrl { provider_name } => {
            let provider = find_provider(config, provider_name)?;
            option_to_json(provider.base_url.clone())
        }
        ConfigPath::ProviderApiKeyEnv { provider_name } => {
            let provider = find_provider(config, provider_name)?;
            option_to_json(provider.api_key_env.clone())
        }
        ConfigPath::ProviderSettings { provider_name } => {
            let provider = find_provider(config, provider_name)?;
            Ok(provider.settings.clone().unwrap_or(Value::Null))
        }
        ConfigPath::AgentProvider { agent_name } => {
            let agent = find_agent(config, agent_name)?;
            Ok(Value::String(agent.provider.clone()))
        }
        ConfigPath::ProjectSummarizationEnabled => Ok(config
            .project
            .as_ref()
            .and_then(|project| project.summarization_enabled)
            .map(Value::Bool)
            .unwrap_or(Value::Null)),
        ConfigPath::ProjectSummarizationProviderName => option_to_json(
            config
                .project
                .as_ref()
                .and_then(|project| project.summarization_provider_name.clone()),
        ),
        ConfigPath::ProjectSummaryMaxTokens => Ok(config
            .project
            .as_ref()
            .and_then(|project| project.summary_max_tokens)
            .map(|value| Value::from(value as u64))
            .unwrap_or(Value::Null)),
    }
}

fn get_raw_value(raw: &Value, path: &ConfigPath) -> Option<Value> {
    match path {
        ConfigPath::Mode => get_path_value(raw, &["mode"]),
        ConfigPath::StorageBackend => get_path_value(raw, &["storage", "backend"]),
        ConfigPath::StoragePoolSize => get_path_value(raw, &["storage", "pool_size"]),
        ConfigPath::StorageProjectDataPath => {
            get_path_value(raw, &["storage", "project_data_path"])
        }
        ConfigPath::StorageGlobalRootPath => get_path_value(raw, &["storage", "global_root_path"]),
        ConfigPath::StorageProjectDatabaseFile => {
            get_path_value(raw, &["storage", "project_database_file"])
        }
        ConfigPath::StorageConnectionStringPrefix => {
            get_path_value(raw, &["storage", "connection_string_prefix"])
        }
        ConfigPath::StorageSqliteBusyTimeoutMs => {
            get_path_value(raw, &["storage", "sqlite", "busy_timeout_ms"])
        }
        ConfigPath::StorageSqliteJournalMode => {
            get_path_value(raw, &["storage", "sqlite", "journal_mode"])
        }
        ConfigPath::StorageSqliteSynchronous => {
            get_path_value(raw, &["storage", "sqlite", "synchronous"])
        }
        ConfigPath::StorageSqliteForeignKeys => {
            get_path_value(raw, &["storage", "sqlite", "foreign_keys"])
        }
        ConfigPath::StoragePostgresConnectionUrl => {
            get_path_value(raw, &["storage", "postgres", "connection_url"])
        }
        ConfigPath::StoragePostgresSchemaName => {
            get_path_value(raw, &["storage", "postgres", "schema_name"])
        }
        ConfigPath::SummarizationEnabled => get_path_value(raw, &["summarization", "enabled"]),
        ConfigPath::SummarizationProviderName => {
            get_path_value(raw, &["summarization", "provider_name"])
        }
        ConfigPath::SummarizationMaxContextTokens => {
            get_path_value(raw, &["summarization", "max_context_tokens"])
        }
        ConfigPath::SummarizationSummaryMaxTokens => {
            get_path_value(raw, &["summarization", "summary_max_tokens"])
        }
        ConfigPath::RuleProjectRulesFolder => {
            get_path_value(raw, &["rules", "project_rules_folder"])
        }
        ConfigPath::RuleRecursiveDiscovery => {
            get_path_value(raw, &["rules", "recursive_discovery"])
        }
        ConfigPath::RuleMaxDiscoveryDepth => get_path_value(raw, &["rules", "max_discovery_depth"]),
        ConfigPath::RuleUseGitignore => get_path_value(raw, &["rules", "use_gitignore"]),
        ConfigPath::RuleTopicDebounceIntervalSecs => {
            get_path_value(raw, &["rules", "topic_debounce_interval_secs"])
        }
        ConfigPath::RuleTopicSimilarityThreshold => {
            get_path_value(raw, &["rules", "topic_similarity_threshold"])
        }
        ConfigPath::ProviderModel { provider_name } => {
            get_provider_field(raw, provider_name, "model")
        }
        ConfigPath::ProviderBaseUrl { provider_name } => {
            get_provider_field(raw, provider_name, "base_url")
        }
        ConfigPath::ProviderApiKeyEnv { provider_name } => {
            get_provider_field(raw, provider_name, "api_key_env")
        }
        ConfigPath::ProviderSettings { provider_name } => {
            get_provider_field(raw, provider_name, "settings")
        }
        ConfigPath::AgentProvider { agent_name } => get_agent_field(raw, agent_name, "provider"),
        ConfigPath::ProjectSummarizationEnabled => {
            get_path_value(raw, &["project", "summarization_enabled"])
        }
        ConfigPath::ProjectSummarizationProviderName => {
            get_path_value(raw, &["project", "summarization_provider_name"])
        }
        ConfigPath::ProjectSummaryMaxTokens => {
            get_path_value(raw, &["project", "summary_max_tokens"])
        }
    }
}

fn set_raw_value(raw: &mut Value, path: &ConfigPath, value: Value) -> Result<()> {
    match path {
        ConfigPath::Mode => set_path_value(raw, &["mode"], value),
        ConfigPath::StorageBackend => set_path_value(raw, &["storage", "backend"], value),
        ConfigPath::StoragePoolSize => set_path_value(raw, &["storage", "pool_size"], value),
        ConfigPath::StorageProjectDataPath => {
            set_path_value(raw, &["storage", "project_data_path"], value)
        }
        ConfigPath::StorageGlobalRootPath => {
            set_path_value(raw, &["storage", "global_root_path"], value)
        }
        ConfigPath::StorageProjectDatabaseFile => {
            set_path_value(raw, &["storage", "project_database_file"], value)
        }
        ConfigPath::StorageConnectionStringPrefix => {
            set_path_value(raw, &["storage", "connection_string_prefix"], value)
        }
        ConfigPath::StorageSqliteBusyTimeoutMs => {
            set_path_value(raw, &["storage", "sqlite", "busy_timeout_ms"], value)
        }
        ConfigPath::StorageSqliteJournalMode => {
            set_path_value(raw, &["storage", "sqlite", "journal_mode"], value)
        }
        ConfigPath::StorageSqliteSynchronous => {
            set_path_value(raw, &["storage", "sqlite", "synchronous"], value)
        }
        ConfigPath::StorageSqliteForeignKeys => {
            set_path_value(raw, &["storage", "sqlite", "foreign_keys"], value)
        }
        ConfigPath::StoragePostgresConnectionUrl => {
            set_path_value(raw, &["storage", "postgres", "connection_url"], value)
        }
        ConfigPath::StoragePostgresSchemaName => {
            set_path_value(raw, &["storage", "postgres", "schema_name"], value)
        }
        ConfigPath::SummarizationEnabled => {
            set_path_value(raw, &["summarization", "enabled"], value)
        }
        ConfigPath::SummarizationProviderName => {
            set_path_value(raw, &["summarization", "provider_name"], value)
        }
        ConfigPath::SummarizationMaxContextTokens => {
            set_path_value(raw, &["summarization", "max_context_tokens"], value)
        }
        ConfigPath::SummarizationSummaryMaxTokens => {
            set_path_value(raw, &["summarization", "summary_max_tokens"], value)
        }
        ConfigPath::RuleProjectRulesFolder => {
            set_path_value(raw, &["rules", "project_rules_folder"], value)
        }
        ConfigPath::RuleRecursiveDiscovery => {
            set_path_value(raw, &["rules", "recursive_discovery"], value)
        }
        ConfigPath::RuleMaxDiscoveryDepth => {
            set_path_value(raw, &["rules", "max_discovery_depth"], value)
        }
        ConfigPath::RuleUseGitignore => set_path_value(raw, &["rules", "use_gitignore"], value),
        ConfigPath::RuleTopicDebounceIntervalSecs => {
            set_path_value(raw, &["rules", "topic_debounce_interval_secs"], value)
        }
        ConfigPath::RuleTopicSimilarityThreshold => {
            set_path_value(raw, &["rules", "topic_similarity_threshold"], value)
        }
        ConfigPath::ProviderModel { provider_name } => {
            set_provider_field(raw, provider_name, "model", value)
        }
        ConfigPath::ProviderBaseUrl { provider_name } => {
            set_provider_field(raw, provider_name, "base_url", value)
        }
        ConfigPath::ProviderApiKeyEnv { provider_name } => {
            set_provider_field(raw, provider_name, "api_key_env", value)
        }
        ConfigPath::ProviderSettings { provider_name } => {
            set_provider_field(raw, provider_name, "settings", value)
        }
        ConfigPath::AgentProvider { agent_name } => {
            set_agent_field(raw, agent_name, "provider", value)
        }
        ConfigPath::ProjectSummarizationEnabled => {
            set_path_value(raw, &["project", "summarization_enabled"], value)
        }
        ConfigPath::ProjectSummarizationProviderName => {
            set_path_value(raw, &["project", "summarization_provider_name"], value)
        }
        ConfigPath::ProjectSummaryMaxTokens => {
            set_path_value(raw, &["project", "summary_max_tokens"], value)
        }
    }
}

fn unset_raw_value(raw: &mut Value, path: &ConfigPath) -> Result<bool> {
    match path {
        ConfigPath::Mode => Ok(remove_path_value(raw, &["mode"])),
        ConfigPath::StorageBackend => Ok(remove_path_value(raw, &["storage", "backend"])),
        ConfigPath::StoragePoolSize => Ok(remove_path_value(raw, &["storage", "pool_size"])),
        ConfigPath::StorageProjectDataPath => {
            Ok(remove_path_value(raw, &["storage", "project_data_path"]))
        }
        ConfigPath::StorageGlobalRootPath => {
            Ok(remove_path_value(raw, &["storage", "global_root_path"]))
        }
        ConfigPath::StorageProjectDatabaseFile => Ok(remove_path_value(
            raw,
            &["storage", "project_database_file"],
        )),
        ConfigPath::StorageConnectionStringPrefix => Ok(remove_path_value(
            raw,
            &["storage", "connection_string_prefix"],
        )),
        ConfigPath::StorageSqliteBusyTimeoutMs => Ok(remove_path_value(
            raw,
            &["storage", "sqlite", "busy_timeout_ms"],
        )),
        ConfigPath::StorageSqliteJournalMode => Ok(remove_path_value(
            raw,
            &["storage", "sqlite", "journal_mode"],
        )),
        ConfigPath::StorageSqliteSynchronous => Ok(remove_path_value(
            raw,
            &["storage", "sqlite", "synchronous"],
        )),
        ConfigPath::StorageSqliteForeignKeys => Ok(remove_path_value(
            raw,
            &["storage", "sqlite", "foreign_keys"],
        )),
        ConfigPath::StoragePostgresConnectionUrl => Ok(remove_path_value(
            raw,
            &["storage", "postgres", "connection_url"],
        )),
        ConfigPath::StoragePostgresSchemaName => Ok(remove_path_value(
            raw,
            &["storage", "postgres", "schema_name"],
        )),
        ConfigPath::SummarizationEnabled => {
            Ok(remove_path_value(raw, &["summarization", "enabled"]))
        }
        ConfigPath::SummarizationProviderName => {
            Ok(remove_path_value(raw, &["summarization", "provider_name"]))
        }
        ConfigPath::SummarizationMaxContextTokens => Ok(remove_path_value(
            raw,
            &["summarization", "max_context_tokens"],
        )),
        ConfigPath::SummarizationSummaryMaxTokens => Ok(remove_path_value(
            raw,
            &["summarization", "summary_max_tokens"],
        )),
        ConfigPath::RuleProjectRulesFolder => {
            Ok(remove_path_value(raw, &["rules", "project_rules_folder"]))
        }
        ConfigPath::RuleRecursiveDiscovery => {
            Ok(remove_path_value(raw, &["rules", "recursive_discovery"]))
        }
        ConfigPath::RuleMaxDiscoveryDepth => {
            Ok(remove_path_value(raw, &["rules", "max_discovery_depth"]))
        }
        ConfigPath::RuleUseGitignore => Ok(remove_path_value(raw, &["rules", "use_gitignore"])),
        ConfigPath::RuleTopicDebounceIntervalSecs => Ok(remove_path_value(
            raw,
            &["rules", "topic_debounce_interval_secs"],
        )),
        ConfigPath::RuleTopicSimilarityThreshold => Ok(remove_path_value(
            raw,
            &["rules", "topic_similarity_threshold"],
        )),
        ConfigPath::ProviderModel { provider_name } => {
            remove_provider_field(raw, provider_name, "model")
        }
        ConfigPath::ProviderBaseUrl { provider_name } => {
            remove_provider_field(raw, provider_name, "base_url")
        }
        ConfigPath::ProviderApiKeyEnv { provider_name } => {
            remove_provider_field(raw, provider_name, "api_key_env")
        }
        ConfigPath::ProviderSettings { provider_name } => {
            remove_provider_field(raw, provider_name, "settings")
        }
        ConfigPath::AgentProvider { agent_name } => remove_agent_field(raw, agent_name, "provider"),
        ConfigPath::ProjectSummarizationEnabled => Ok(remove_path_value(
            raw,
            &["project", "summarization_enabled"],
        )),
        ConfigPath::ProjectSummarizationProviderName => Ok(remove_path_value(
            raw,
            &["project", "summarization_provider_name"],
        )),
        ConfigPath::ProjectSummaryMaxTokens => {
            Ok(remove_path_value(raw, &["project", "summary_max_tokens"]))
        }
    }
}

fn get_path_value(raw: &Value, keys: &[&str]) -> Option<Value> {
    let mut cursor = raw;
    for key in keys {
        let object = cursor.as_object()?;
        cursor = object.get(*key)?;
    }
    Some(cursor.clone())
}

fn set_path_value(raw: &mut Value, keys: &[&str], value: Value) -> Result<()> {
    let object = raw
        .as_object_mut()
        .ok_or_else(|| Error::Config("config root must be an object".to_owned()))?;
    set_path_value_in_object(object, keys, value);
    Ok(())
}

fn set_path_value_in_object(object: &mut Map<String, Value>, keys: &[&str], value: Value) {
    if keys.is_empty() {
        return;
    }
    let key = keys[0];
    if keys.len() == 1 {
        object.insert(key.to_owned(), value);
        return;
    }

    let entry = object
        .entry(key.to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    let child = entry.as_object_mut().expect("entry must be object");
    set_path_value_in_object(child, &keys[1..], value);
}

fn remove_path_value(raw: &mut Value, keys: &[&str]) -> bool {
    let Some(object) = raw.as_object_mut() else {
        return false;
    };
    remove_path_value_in_object(object, keys)
}

fn remove_path_value_in_object(object: &mut Map<String, Value>, keys: &[&str]) -> bool {
    if keys.is_empty() {
        return false;
    }
    if keys.len() == 1 {
        return object.remove(keys[0]).is_some();
    }

    let Some(child) = object
        .get_mut(keys[0])
        .and_then(|value| value.as_object_mut())
    else {
        return false;
    };
    remove_path_value_in_object(child, &keys[1..])
}

fn get_provider_field(raw: &Value, provider_name: &str, field: &str) -> Option<Value> {
    let providers = raw.get("providers")?.as_array()?;
    let provider = providers.iter().find(|provider| {
        provider
            .get("name")
            .and_then(|name| name.as_str())
            .map(|name| name == provider_name)
            .unwrap_or(false)
    })?;
    provider.get(field).cloned()
}

fn set_provider_field(
    raw: &mut Value,
    provider_name: &str,
    field: &str,
    value: Value,
) -> Result<()> {
    let provider = find_provider_raw_mut(raw, provider_name)?;
    let provider_obj = provider
        .as_object_mut()
        .ok_or_else(|| Error::Config(format!("provider '{provider_name}' must be an object")))?;
    provider_obj.insert(field.to_owned(), value);
    Ok(())
}

fn remove_provider_field(raw: &mut Value, provider_name: &str, field: &str) -> Result<bool> {
    let provider = find_provider_raw_mut(raw, provider_name)?;
    let provider_obj = provider
        .as_object_mut()
        .ok_or_else(|| Error::Config(format!("provider '{provider_name}' must be an object")))?;
    Ok(provider_obj.remove(field).is_some())
}

fn find_provider_raw_mut<'a>(raw: &'a mut Value, provider_name: &str) -> Result<&'a mut Value> {
    let root = raw
        .as_object_mut()
        .ok_or_else(|| Error::Config("config root must be an object".to_owned()))?;
    let providers = root
        .get_mut("providers")
        .and_then(|value| value.as_array_mut())
        .ok_or_else(|| Error::NotFound("providers array not found".to_owned()))?;

    providers
        .iter_mut()
        .find(|provider| {
            provider
                .get("name")
                .and_then(|name| name.as_str())
                .map(|name| name == provider_name)
                .unwrap_or(false)
        })
        .ok_or_else(|| Error::NotFound(format!("provider '{provider_name}' not found")))
}

fn get_agent_field(raw: &Value, agent_name: &str, field: &str) -> Option<Value> {
    let agents = raw.get("agents")?.as_array()?;
    let agent = agents.iter().find(|agent| {
        agent
            .get("name")
            .and_then(|name| name.as_str())
            .map(|name| name == agent_name)
            .unwrap_or(false)
    })?;
    agent.get(field).cloned()
}

fn set_agent_field(raw: &mut Value, agent_name: &str, field: &str, value: Value) -> Result<()> {
    let agent = find_agent_raw_mut(raw, agent_name)?;
    let agent_obj = agent
        .as_object_mut()
        .ok_or_else(|| Error::Config(format!("agent '{agent_name}' must be an object")))?;
    agent_obj.insert(field.to_owned(), value);
    Ok(())
}

fn remove_agent_field(raw: &mut Value, agent_name: &str, field: &str) -> Result<bool> {
    let agent = find_agent_raw_mut(raw, agent_name)?;
    let agent_obj = agent
        .as_object_mut()
        .ok_or_else(|| Error::Config(format!("agent '{agent_name}' must be an object")))?;
    Ok(agent_obj.remove(field).is_some())
}

fn find_agent_raw_mut<'a>(raw: &'a mut Value, agent_name: &str) -> Result<&'a mut Value> {
    let root = raw
        .as_object_mut()
        .ok_or_else(|| Error::Config("config root must be an object".to_owned()))?;
    let agents = root
        .get_mut("agents")
        .and_then(|value| value.as_array_mut())
        .ok_or_else(|| Error::NotFound("agents array not found".to_owned()))?;

    agents
        .iter_mut()
        .find(|agent| {
            agent
                .get("name")
                .and_then(|name| name.as_str())
                .map(|name| name == agent_name)
                .unwrap_or(false)
        })
        .ok_or_else(|| Error::NotFound(format!("agent '{agent_name}' not found")))
}

fn find_provider<'a>(
    config: &'a Config,
    provider_name: &str,
) -> Result<&'a crate::config::schema::ProviderConfig> {
    config
        .providers
        .iter()
        .find(|provider| provider.name == provider_name)
        .ok_or_else(|| Error::NotFound(format!("provider '{provider_name}' not found")))
}

fn find_agent<'a>(
    config: &'a Config,
    agent_name: &str,
) -> Result<&'a crate::config::schema::AgentConfig> {
    config
        .agents
        .iter()
        .find(|agent| agent.name == agent_name)
        .ok_or_else(|| Error::NotFound(format!("agent '{agent_name}' not found")))
}

fn option_to_json(value: Option<String>) -> Result<Value> {
    Ok(value.map(Value::String).unwrap_or(Value::Null))
}
