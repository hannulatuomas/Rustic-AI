use std::sync::Arc;

use crate::config::schema::{Config, StorageBackendKind};
use crate::error::{Error, Result};
use crate::storage::paths::StoragePaths;
use crate::storage::postgres::PostgresStorage;
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
        StorageBackendKind::Postgres => {
            let connection_url = config
                .storage
                .postgres
                .connection_url
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    Error::Storage(
                        "storage backend 'postgres' requires storage.postgres.connection_url"
                            .to_owned(),
                    )
                })?;
            let backend = PostgresStorage::new(
                connection_url,
                config.storage.pool_size,
                config.storage.postgres.schema_name.clone(),
            )?;
            Ok(Arc::new(backend))
        }
        StorageBackendKind::Custom => {
            let custom_url = std::env::var("RUSTIC_AI_CUSTOM_STORAGE_URL").ok();
            let custom_url = custom_url
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty());

            if let Some(url) = custom_url {
                if url.starts_with("postgres://") || url.starts_with("postgresql://") {
                    let backend = PostgresStorage::new(
                        url,
                        config.storage.pool_size,
                        config.storage.postgres.schema_name.clone(),
                    )?;
                    return Ok(Arc::new(backend));
                }

                let backend = SqliteStorage::new(
                    url,
                    config.storage.pool_size,
                    config.storage.sqlite.clone(),
                )?;
                return Ok(Arc::new(backend));
            }

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
    }
}
