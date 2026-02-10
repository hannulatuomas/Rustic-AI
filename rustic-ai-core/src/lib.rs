pub mod agents;
pub mod catalog;
pub mod commands;
pub mod config;
pub mod conversation;
pub mod error;
pub mod events;
pub mod logging;
pub mod permissions;
pub mod project;
pub mod providers;
pub mod rules;
pub mod runtime;
pub mod skills;
pub mod storage;
pub mod tools;
pub mod workflows;

pub use agents::Agent;
pub use config::Config;
pub use error::{Error, Result};
pub use storage::model::Session;
pub use tools::ToolManager;

pub struct RusticAI {
    config: Config,
    runtime: runtime::Runtime,
    session_manager: std::sync::Arc<conversation::session_manager::SessionManager>,
    topic_inference: rules::TopicInferenceService,
    work_dir: std::path::PathBuf,
}

impl RusticAI {
    pub fn new(mut config: Config) -> Result<Self> {
        config::validate_config(&config)?;
        let work_dir = std::env::current_dir()
            .map_err(|err| Error::Config(format!("failed to read current directory: {err}")))?;
        let storage_paths = storage::paths::StoragePaths::resolve(&work_dir, &config);
        std::fs::create_dir_all(&storage_paths.project_data_dir)?;
        std::fs::create_dir_all(&storage_paths.global_data_dir)?;
        if let Some(parent) = storage_paths.global_settings.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !storage_paths.global_settings.exists() {
            std::fs::write(&storage_paths.global_settings, "{}")?;
        }

        // Set default permissions if not present
        if config.permissions.default_tool_permission == crate::config::schema::PermissionMode::Ask
            && config.permissions.ask_decisions_persist_scope
                == crate::config::schema::DecisionScope::Session
        {
            // Use default values
            config.permissions = crate::config::schema::PermissionConfig::default();
        }

        let storage_backend = storage::create_storage_backend(&config, &storage_paths)?;
        let session_manager =
            std::sync::Arc::new(conversation::session_manager::SessionManager::new(
                storage_backend,
                config.rules.discovered_rules.clone(),
                work_dir.clone(),
            ));

        let runtime = runtime::Runtime::new(config.clone(), session_manager.clone())?;
        let inference_provider = config.summarization.provider_name.clone().ok_or_else(|| {
            Error::Config(
                "summarization.provider_name must be set (no implicit provider fallback)"
                    .to_owned(),
            )
        })?;
        let topic_inference = rules::TopicInferenceService::new(inference_provider);

        Ok(Self {
            config,
            runtime,
            session_manager,
            topic_inference,
            work_dir,
        })
    }

    pub fn from_config_path(path: &std::path::Path) -> Result<Self> {
        let config = config::load(Some(path))?;
        Self::new(config)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn runtime(&self) -> &runtime::Runtime {
        &self.runtime
    }

    pub fn session_manager(
        &self,
    ) -> &std::sync::Arc<conversation::session_manager::SessionManager> {
        &self.session_manager
    }

    pub fn topic_inference(&self) -> &rules::TopicInferenceService {
        &self.topic_inference
    }

    pub fn work_dir(&self) -> &std::path::Path {
        &self.work_dir
    }
}
