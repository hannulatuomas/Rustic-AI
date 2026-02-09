use std::path::{Path, PathBuf};

use crate::config::schema::Config;

#[derive(Debug, Clone)]
pub struct StoragePaths {
    pub global_settings: PathBuf,
    pub global_data_dir: PathBuf,
    pub project_data_dir: PathBuf,
    pub project_database: PathBuf,
}

impl StoragePaths {
    pub fn resolve(work_dir: &Path, config: &Config) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
        let global_root = config
            .storage
            .global_root_path
            .as_ref()
            .map(PathBuf::from)
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    work_dir.join(path)
                }
            })
            .unwrap_or_else(|| PathBuf::from(home).join(&config.storage.default_root_dir_name));

        let configured_project_dir = config
            .storage
            .project_data_path
            .as_ref()
            .map(PathBuf::from)
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    work_dir.join(path)
                }
            })
            .unwrap_or_else(|| work_dir.join(&config.storage.default_root_dir_name));

        Self {
            global_settings: global_root.join(&config.storage.global_settings_file),
            global_data_dir: global_root.join(&config.storage.global_data_subdir),
            project_data_dir: configured_project_dir.clone(),
            project_database: configured_project_dir.join(&config.storage.project_database_file),
        }
    }
}
