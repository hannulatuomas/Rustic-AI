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

pub use config::schema::Config;
pub use error::{Error, Result};

#[derive(Debug, Clone)]
pub struct RusticAI {
    pub config: Config,
}

impl RusticAI {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}
