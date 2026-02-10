pub mod factory;
pub mod model;
pub mod paths;
pub mod sqlite;

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::storage::model::{Message, Session, SessionConfig};

pub use factory::create_storage_backend;

#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn get_schema_version(&self) -> Result<Option<u32>>;
    async fn set_schema_version(&self, version: u32) -> Result<()>;

    async fn create_session(&self, session: Session) -> Result<()>;
    async fn get_session(&self, id: Uuid) -> Result<Option<Session>>;
    async fn list_sessions(&self, limit: Option<usize>) -> Result<Vec<Session>>;
    async fn delete_session(&self, id: Uuid) -> Result<()>;
    async fn append_message(&self, message: Message) -> Result<()>;

    async fn get_session_messages(&self, session_id: Uuid) -> Result<Vec<Message>>;
    async fn get_recent_messages(&self, session_id: Uuid, limit: usize) -> Result<Vec<Message>>;

    async fn get_session_config(&self, session_id: Uuid) -> Result<Option<SessionConfig>>;
    async fn update_session_config(&self, session_id: Uuid, config: &SessionConfig) -> Result<()>;

    async fn get_or_load_context_file(&self, path: &str) -> Result<Option<String>>;
    async fn cache_context_file(&self, path: &str, content: &str, metadata: &str) -> Result<()>;

    async fn update_session_topics(&self, session_id: Uuid, topics: &[String]) -> Result<()>;
    async fn get_session_topics(&self, session_id: Uuid) -> Result<Option<Vec<String>>>;

    async fn track_manual_invocation(&self, session_id: Uuid, rule_path: &str) -> Result<()>;
    async fn get_manual_invocations(&self, session_id: Uuid) -> Result<Vec<String>>;
}
