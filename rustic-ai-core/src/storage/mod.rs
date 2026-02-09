pub mod model;
pub mod sqlite;

use async_trait::async_trait;

use crate::error::Result;
use crate::storage::model::{Message, Session};

#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn create_session(&self, session: Session) -> Result<()>;
    async fn append_message(&self, message: Message) -> Result<()>;
}
