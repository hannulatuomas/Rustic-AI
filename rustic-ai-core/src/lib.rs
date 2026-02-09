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

pub struct RusticAI {
    config: Config,
    runtime: runtime::Runtime,
}

impl RusticAI {
    pub fn new(config: Config) -> Result<Self> {
        config::validate_config(&config)?;
        let runtime = runtime::Runtime::new(config.clone());
        Ok(Self { config, runtime })
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
}
