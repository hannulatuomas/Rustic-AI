use std::path::Path;

use crate::config::schema::Config;
use crate::error::{Error, Result};

pub fn load_from_file(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path).map_err(|err| {
        Error::Config(format!("failed to read config '{}': {err}", path.display()))
    })?;

    toml::from_str(&content).map_err(|err| {
        Error::Config(format!(
            "failed to parse config '{}': {err}",
            path.display()
        ))
    })
}
