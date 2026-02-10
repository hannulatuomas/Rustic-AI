use std::sync::Arc;

use crate::config::schema::{Config, StorageBackendKind};
use crate::error::{Error, Result};
use crate::storage::paths::StoragePaths;
use crate::storage::sqlite::SqliteStorage;
use crate::storage::StorageBackend;

pub fn create_storage_backend(
    config: &Config,
    storage_paths: &StoragePaths,
) -> Result<Arc<dyn StorageBackend>> {
    match config.storage.backend {
        StorageBackendKind::Sqlite => {
            let connection_string = format!(
                "{}{}",
                config.storage.connection_string_prefix,
                storage_paths.project_database.display()
            );

            let backend = SqliteStorage::new(
                &connection_string,
                config.storage.pool_size,
                config.storage.sqlite.clone(),
            )?;
            Ok(Arc::new(backend))
        }
        StorageBackendKind::Postgres => Err(Error::Storage(
            "storage backend 'postgres' is not implemented yet".to_owned(),
        )),
        StorageBackendKind::Custom => Err(Error::Storage(
            "storage backend 'custom' is not implemented yet".to_owned(),
        )),
    }
}
