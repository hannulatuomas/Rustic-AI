use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigScope {
    Effective,
    Global,
    Project,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigPath {
    Mode,
    StorageBackend,
    StoragePoolSize,
    StorageProjectDataPath,
    StorageGlobalRootPath,
    StorageProjectDatabaseFile,
    StorageConnectionStringPrefix,
    StorageSqliteBusyTimeoutMs,
    StorageSqliteJournalMode,
    StorageSqliteSynchronous,
    StorageSqliteForeignKeys,
    StoragePostgresConnectionUrl,
    StoragePostgresSchemaName,
    SummarizationEnabled,
    SummarizationProviderName,
    SummarizationMaxContextTokens,
    SummarizationSummaryMaxTokens,
    RuleProjectRulesFolder,
    RuleRecursiveDiscovery,
    RuleMaxDiscoveryDepth,
    RuleUseGitignore,
    RuleTopicDebounceIntervalSecs,
    RuleTopicSimilarityThreshold,
    ProviderModel { provider_name: String },
    ProviderBaseUrl { provider_name: String },
    ProviderApiKeyEnv { provider_name: String },
    ProviderSettings { provider_name: String },
    AgentProvider { agent_name: String },
    ProjectSummarizationEnabled,
    ProjectSummarizationProviderName,
    ProjectSummaryMaxTokens,
}

impl FromStr for ConfigPath {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let trimmed = value.trim();
        match trimmed {
            "mode" => return Ok(Self::Mode),
            "storage.backend" => return Ok(Self::StorageBackend),
            "storage.pool_size" => return Ok(Self::StoragePoolSize),
            "storage.project_data_path" => return Ok(Self::StorageProjectDataPath),
            "storage.global_root_path" => return Ok(Self::StorageGlobalRootPath),
            "storage.project_database_file" => return Ok(Self::StorageProjectDatabaseFile),
            "storage.connection_string_prefix" => return Ok(Self::StorageConnectionStringPrefix),
            "storage.sqlite.busy_timeout_ms" => return Ok(Self::StorageSqliteBusyTimeoutMs),
            "storage.sqlite.journal_mode" => return Ok(Self::StorageSqliteJournalMode),
            "storage.sqlite.synchronous" => return Ok(Self::StorageSqliteSynchronous),
            "storage.sqlite.foreign_keys" => return Ok(Self::StorageSqliteForeignKeys),
            "storage.postgres.connection_url" => return Ok(Self::StoragePostgresConnectionUrl),
            "storage.postgres.schema_name" => return Ok(Self::StoragePostgresSchemaName),
            "summarization.enabled" => return Ok(Self::SummarizationEnabled),
            "summarization.provider_name" => return Ok(Self::SummarizationProviderName),
            "summarization.max_context_tokens" => return Ok(Self::SummarizationMaxContextTokens),
            "summarization.summary_max_tokens" => return Ok(Self::SummarizationSummaryMaxTokens),
            "rules.project_rules_folder" => return Ok(Self::RuleProjectRulesFolder),
            "rules.recursive_discovery" => return Ok(Self::RuleRecursiveDiscovery),
            "rules.max_discovery_depth" => return Ok(Self::RuleMaxDiscoveryDepth),
            "rules.use_gitignore" => return Ok(Self::RuleUseGitignore),
            "rules.topic_debounce_interval_secs" => return Ok(Self::RuleTopicDebounceIntervalSecs),
            "rules.topic_similarity_threshold" => return Ok(Self::RuleTopicSimilarityThreshold),
            "project.summarization_enabled" => return Ok(Self::ProjectSummarizationEnabled),
            "project.summarization_provider_name" => {
                return Ok(Self::ProjectSummarizationProviderName)
            }
            "project.summary_max_tokens" => return Ok(Self::ProjectSummaryMaxTokens),
            _ => {}
        }

        if let Some((name, suffix)) = parse_indexed(trimmed, "providers") {
            return match suffix {
                "model" => Ok(Self::ProviderModel {
                    provider_name: name,
                }),
                "base_url" => Ok(Self::ProviderBaseUrl {
                    provider_name: name,
                }),
                "api_key_env" => Ok(Self::ProviderApiKeyEnv {
                    provider_name: name,
                }),
                "settings" => Ok(Self::ProviderSettings {
                    provider_name: name,
                }),
                _ => Err(Error::Config(format!(
                    "unknown provider config path '{trimmed}'"
                ))),
            };
        }

        if let Some((name, suffix)) = parse_indexed(trimmed, "agents") {
            return match suffix {
                "provider" => Ok(Self::AgentProvider { agent_name: name }),
                _ => Err(Error::Config(format!(
                    "unknown agent config path '{trimmed}'"
                ))),
            };
        }

        Err(Error::Config(format!("unknown config path '{trimmed}'")))
    }
}

impl std::fmt::Display for ConfigPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mode => write!(f, "mode"),
            Self::StorageBackend => write!(f, "storage.backend"),
            Self::StoragePoolSize => write!(f, "storage.pool_size"),
            Self::StorageProjectDataPath => write!(f, "storage.project_data_path"),
            Self::StorageGlobalRootPath => write!(f, "storage.global_root_path"),
            Self::StorageProjectDatabaseFile => write!(f, "storage.project_database_file"),
            Self::StorageConnectionStringPrefix => write!(f, "storage.connection_string_prefix"),
            Self::StorageSqliteBusyTimeoutMs => write!(f, "storage.sqlite.busy_timeout_ms"),
            Self::StorageSqliteJournalMode => write!(f, "storage.sqlite.journal_mode"),
            Self::StorageSqliteSynchronous => write!(f, "storage.sqlite.synchronous"),
            Self::StorageSqliteForeignKeys => write!(f, "storage.sqlite.foreign_keys"),
            Self::StoragePostgresConnectionUrl => write!(f, "storage.postgres.connection_url"),
            Self::StoragePostgresSchemaName => write!(f, "storage.postgres.schema_name"),
            Self::SummarizationEnabled => write!(f, "summarization.enabled"),
            Self::SummarizationProviderName => write!(f, "summarization.provider_name"),
            Self::SummarizationMaxContextTokens => write!(f, "summarization.max_context_tokens"),
            Self::SummarizationSummaryMaxTokens => write!(f, "summarization.summary_max_tokens"),
            Self::RuleProjectRulesFolder => write!(f, "rules.project_rules_folder"),
            Self::RuleRecursiveDiscovery => write!(f, "rules.recursive_discovery"),
            Self::RuleMaxDiscoveryDepth => write!(f, "rules.max_discovery_depth"),
            Self::RuleUseGitignore => write!(f, "rules.use_gitignore"),
            Self::RuleTopicDebounceIntervalSecs => write!(f, "rules.topic_debounce_interval_secs"),
            Self::RuleTopicSimilarityThreshold => write!(f, "rules.topic_similarity_threshold"),
            Self::ProviderModel { provider_name } => {
                write!(f, "providers[{provider_name}].model")
            }
            Self::ProviderBaseUrl { provider_name } => {
                write!(f, "providers[{provider_name}].base_url")
            }
            Self::ProviderApiKeyEnv { provider_name } => {
                write!(f, "providers[{provider_name}].api_key_env")
            }
            Self::ProviderSettings { provider_name } => {
                write!(f, "providers[{provider_name}].settings")
            }
            Self::AgentProvider { agent_name } => write!(f, "agents[{agent_name}].provider"),
            Self::ProjectSummarizationEnabled => write!(f, "project.summarization_enabled"),
            Self::ProjectSummarizationProviderName => {
                write!(f, "project.summarization_provider_name")
            }
            Self::ProjectSummaryMaxTokens => write!(f, "project.summary_max_tokens"),
        }
    }
}

fn parse_indexed<'a>(value: &'a str, prefix: &str) -> Option<(String, &'a str)> {
    if !value.starts_with(prefix) {
        return None;
    }

    let remainder = value.strip_prefix(prefix)?;
    let remainder = remainder.strip_prefix('[')?;
    let bracket_index = remainder.find(']')?;
    let name = remainder[..bracket_index].trim();
    if name.is_empty() {
        return None;
    }
    let rest = &remainder[(bracket_index + 1)..];
    let suffix = rest.strip_prefix('.')?;
    Some((name.to_owned(), suffix))
}
