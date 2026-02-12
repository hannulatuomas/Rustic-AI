pub mod factory;
pub mod model;
pub mod paths;
pub mod postgres;
pub mod sqlite;

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::indexing::{
    CallEdge, IndexedCallEdgeRecord, IndexedFileRecord, IndexedSymbolRecord, SymbolIndex,
};
use crate::learning::{
    MistakePattern, PatternCategory, PreferenceValue, SuccessPattern, UserFeedback, UserPreference,
};
use crate::vector::StoredVector;

pub use factory::create_storage_backend;
pub use model::{
    Message, PendingToolState, RoutingTrace, RoutingTraceFilter, Session, SessionConfig,
    SubAgentOutput, SubAgentOutputFilter, Todo, TodoFilter, TodoMetadata, TodoPriority, TodoStatus,
    TodoUpdate,
};

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

    // Pending tool execution state
    async fn set_pending_tool(&self, state: &PendingToolState) -> Result<()>;
    async fn get_and_clear_pending_tool(
        &self,
        session_id: Uuid,
    ) -> Result<Option<PendingToolState>>;
    async fn delete_stale_pending_tools(&self, older_than_secs: u64) -> Result<usize>;
    async fn has_pending_tool(&self, session_id: Uuid) -> Result<bool>;

    // Learning feedback and adaptation
    async fn store_user_feedback(&self, feedback: &UserFeedback) -> Result<()>;
    async fn list_user_feedback(&self, session_id: Uuid, limit: usize)
        -> Result<Vec<UserFeedback>>;

    async fn upsert_mistake_pattern(&self, pattern: &MistakePattern) -> Result<()>;
    async fn list_mistake_patterns(
        &self,
        agent_name: &str,
        min_frequency: u32,
        limit: usize,
    ) -> Result<Vec<MistakePattern>>;

    async fn upsert_user_preference(
        &self,
        session_id: Uuid,
        key: &str,
        value: &PreferenceValue,
    ) -> Result<()>;
    async fn get_user_preference(
        &self,
        session_id: Uuid,
        key: &str,
    ) -> Result<Option<PreferenceValue>>;
    async fn list_user_preferences(&self, session_id: Uuid) -> Result<Vec<UserPreference>>;

    async fn upsert_success_pattern(&self, pattern: &SuccessPattern) -> Result<()>;
    async fn find_success_patterns(
        &self,
        agent_name: &str,
        category: Option<PatternCategory>,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SuccessPattern>>;

    // Code indexing and symbol search
    async fn upsert_code_index_metadata(
        &self,
        workspace: &str,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()>;
    async fn get_code_index_metadata(
        &self,
        workspace: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>>;
    async fn upsert_code_file_index(
        &self,
        workspace: &str,
        path: &str,
        language: &str,
        functions: &[String],
        classes: &[String],
        imports: &[String],
    ) -> Result<()>;
    async fn list_code_file_indexes(&self, workspace: &str) -> Result<Vec<IndexedFileRecord>>;
    async fn replace_code_symbols_for_file(
        &self,
        workspace: &str,
        file_path: &str,
        symbols: &[SymbolIndex],
    ) -> Result<()>;
    async fn list_code_symbols(&self, workspace: &str) -> Result<Vec<IndexedSymbolRecord>>;
    async fn search_code_symbols(
        &self,
        workspace: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SymbolIndex>>;
    async fn replace_code_call_edges_for_file(
        &self,
        workspace: &str,
        file_path: &str,
        edges: &[CallEdge],
    ) -> Result<()>;
    async fn list_code_call_edges(&self, workspace: &str) -> Result<Vec<IndexedCallEdgeRecord>>;

    // Vector storage
    async fn upsert_vector_embedding(
        &self,
        workspace: &str,
        id: &str,
        vector: &[f32],
        metadata: &serde_json::Value,
    ) -> Result<()>;
    async fn list_vector_embeddings(&self, workspace: &str) -> Result<Vec<StoredVector>>;

    // TODO tracking
    async fn create_todo(&self, todo: &Todo) -> Result<()>;
    async fn list_todos(&self, filter: &TodoFilter) -> Result<Vec<Todo>>;
    async fn update_todo(&self, id: Uuid, update: &TodoUpdate) -> Result<()>;
    async fn delete_todo(&self, id: Uuid) -> Result<()>;
    async fn get_todo(&self, id: Uuid) -> Result<Option<Todo>>;
    async fn complete_todo_chain(&self, id: Uuid) -> Result<()>;

    // Sub-agent output caching
    async fn upsert_sub_agent_output(&self, output: &SubAgentOutput) -> Result<()>;
    async fn get_sub_agent_output_exact(&self, task_key: &str) -> Result<Option<SubAgentOutput>>;
    async fn get_sub_agent_output_semantic(
        &self,
        task_type: &str,
        caller_agent: &str,
        target_agent: &str,
    ) -> Result<Vec<SubAgentOutput>>;
    async fn list_sub_agent_outputs(
        &self,
        filter: &SubAgentOutputFilter,
    ) -> Result<Vec<SubAgentOutput>>;
    async fn delete_expired_sub_agent_outputs(&self) -> Result<usize>;

    // Routing traces
    async fn create_routing_trace(&self, trace: &RoutingTrace) -> Result<()>;
    async fn list_routing_traces(&self, filter: &RoutingTraceFilter) -> Result<Vec<RoutingTrace>>;
}
