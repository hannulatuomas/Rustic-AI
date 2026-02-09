use crate::config::schema::Config;
use crate::config::schema::RuntimeMode;
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

    Ok(())
}
