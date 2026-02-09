use async_trait::async_trait;

use crate::error::Result;
use crate::storage::model::{Message, Session};
use crate::storage::StorageBackend;

#[derive(Debug, Clone)]
pub struct SqliteStorage {
    pub connection_string: String,
}

#[async_trait]
impl StorageBackend for SqliteStorage {
    async fn create_session(&self, _session: Session) -> Result<()> {
        Ok(())
    }

    async fn append_message(&self, _message: Message) -> Result<()> {
        Ok(())
    }
}
