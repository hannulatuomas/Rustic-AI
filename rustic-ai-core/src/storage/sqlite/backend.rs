use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqlitePoolOptions, SqliteRow};
use sqlx::Row;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::config::schema::SqliteStorageConfig;
use crate::error::{Error, Result};
use crate::indexing::{
    CallEdge, IndexedCallEdgeRecord, IndexedFileRecord, IndexedSymbolRecord, SymbolIndex,
    SymbolType,
};
use crate::learning::{
    FeedbackType, MistakePattern, MistakeType, PatternCategory, PreferenceValue, SuccessPattern,
    UserFeedback, UserPreference,
};
use crate::storage::model::{
    Message, PendingToolState, RoutingTrace, RoutingTraceFilter, Session, SessionConfig,
    SubAgentOutput, SubAgentOutputFilter, Todo, TodoFilter, TodoPriority, TodoStatus, TodoUpdate,
};
use crate::storage::StorageBackend;
use crate::vector::StoredVector;

const SCHEMA_V1: [&str; 12] = [
    "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL PRIMARY KEY)",
    "INSERT OR IGNORE INTO schema_version(version) VALUES (0)",
    "CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, agent_name TEXT NOT NULL, created_at TEXT NOT NULL, config_json TEXT)",
    "CREATE TABLE IF NOT EXISTS messages (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, created_at TEXT NOT NULL, FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE)",
    "CREATE TABLE IF NOT EXISTS context_files (path TEXT PRIMARY KEY, content TEXT NOT NULL, metadata TEXT, loaded_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS session_topics (session_id TEXT PRIMARY KEY, topics_json TEXT NOT NULL, updated_at TEXT NOT NULL, FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE)",
    "CREATE TABLE IF NOT EXISTS manual_rule_invocations (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, rule_path TEXT NOT NULL, invoked_at TEXT NOT NULL, FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE)",
    "CREATE INDEX IF NOT EXISTS idx_messages_session_created_at ON messages(session_id, created_at)",
    "CREATE INDEX IF NOT EXISTS idx_context_files_loaded_at ON context_files(loaded_at)",
    "CREATE INDEX IF NOT EXISTS idx_manual_rule_invocations_session_id ON manual_rule_invocations(session_id)",
    "CREATE INDEX IF NOT EXISTS idx_manual_rule_invocations_rule_path ON manual_rule_invocations(rule_path)",
    "UPDATE schema_version SET version = 1",
];

const SCHEMA_V2_MIGRATION: [&str; 2] = [
    "CREATE TABLE IF NOT EXISTS pending_tools (session_id TEXT PRIMARY KEY, tool_name TEXT NOT NULL, args_json TEXT NOT NULL, round_index INTEGER NOT NULL, tool_messages_json TEXT NOT NULL, context_snapshot_json TEXT NOT NULL, created_at TEXT NOT NULL, FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE)",
    "UPDATE schema_version SET version = 2",
];

const SCHEMA_V3_MIGRATION: [&str; 10] = [
    "CREATE TABLE IF NOT EXISTS user_feedback (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, agent_name TEXT NOT NULL, feedback_type TEXT NOT NULL, rating INTEGER NOT NULL, comment TEXT, context_json TEXT NOT NULL, created_at TEXT NOT NULL, FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE)",
    "CREATE INDEX IF NOT EXISTS idx_user_feedback_session_created_at ON user_feedback(session_id, created_at)",
    "CREATE TABLE IF NOT EXISTS mistake_patterns (id TEXT PRIMARY KEY, agent_name TEXT NOT NULL, mistake_type TEXT NOT NULL, trigger TEXT NOT NULL, frequency INTEGER NOT NULL, last_seen TEXT NOT NULL, suggested_fix TEXT, UNIQUE(agent_name, mistake_type, trigger))",
    "CREATE INDEX IF NOT EXISTS idx_mistake_patterns_agent_frequency ON mistake_patterns(agent_name, frequency DESC)",
    "CREATE TABLE IF NOT EXISTS user_preferences (session_id TEXT NOT NULL, preference_key TEXT NOT NULL, value_json TEXT NOT NULL, value_kind TEXT NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY(session_id, preference_key), FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE)",
    "CREATE INDEX IF NOT EXISTS idx_user_preferences_session ON user_preferences(session_id)",
    "CREATE TABLE IF NOT EXISTS success_patterns (id TEXT PRIMARY KEY, agent_name TEXT NOT NULL, name TEXT NOT NULL, category TEXT NOT NULL, description TEXT NOT NULL, template TEXT NOT NULL, frequency INTEGER NOT NULL, last_used TEXT NOT NULL, success_rate REAL NOT NULL, created_at TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS idx_success_patterns_agent_last_used ON success_patterns(agent_name, last_used DESC)",
    "CREATE INDEX IF NOT EXISTS idx_success_patterns_agent_category ON success_patterns(agent_name, category)",
    "UPDATE schema_version SET version = 3",
];

const SCHEMA_V4_MIGRATION: [&str; 9] = [
    "CREATE TABLE IF NOT EXISTS code_index_metadata (workspace TEXT PRIMARY KEY, updated_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS code_file_indexes (workspace TEXT NOT NULL, path TEXT NOT NULL, language TEXT NOT NULL, functions_json TEXT NOT NULL, classes_json TEXT NOT NULL, imports_json TEXT NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY(workspace, path))",
    "CREATE INDEX IF NOT EXISTS idx_code_file_indexes_workspace ON code_file_indexes(workspace)",
    "CREATE TABLE IF NOT EXISTS code_symbol_indexes (workspace TEXT NOT NULL, file_path TEXT NOT NULL, name TEXT NOT NULL, symbol_type TEXT NOT NULL, line INTEGER NOT NULL, column_number INTEGER NOT NULL, docstring TEXT, signature TEXT, updated_at TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS idx_code_symbol_indexes_workspace_name ON code_symbol_indexes(workspace, name)",
    "CREATE INDEX IF NOT EXISTS idx_code_symbol_indexes_workspace_file ON code_symbol_indexes(workspace, file_path)",
    "CREATE TABLE IF NOT EXISTS code_call_edges (workspace TEXT NOT NULL, file_path TEXT NOT NULL, caller_symbol TEXT NOT NULL, callee_symbol TEXT NOT NULL, line INTEGER NOT NULL, column_number INTEGER NOT NULL, updated_at TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS idx_code_call_edges_workspace_file ON code_call_edges(workspace, file_path)",
    "UPDATE schema_version SET version = 4",
];

const SCHEMA_V5_MIGRATION: [&str; 3] = [
    "CREATE TABLE IF NOT EXISTS vector_embeddings (workspace TEXT NOT NULL, id TEXT NOT NULL, vector_json TEXT NOT NULL, metadata_json TEXT NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY(workspace, id))",
    "CREATE INDEX IF NOT EXISTS idx_vector_embeddings_workspace ON vector_embeddings(workspace)",
    "UPDATE schema_version SET version = 5",
];

const SCHEMA_V6_MIGRATION: [&str; 5] = [
    "CREATE TABLE IF NOT EXISTS todos (id TEXT PRIMARY KEY, project_id TEXT, session_id TEXT NOT NULL, parent_id TEXT, title TEXT NOT NULL, description TEXT, status TEXT NOT NULL, priority TEXT NOT NULL, tags_json TEXT NOT NULL, metadata_json TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, completed_at TEXT, FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE, FOREIGN KEY(parent_id) REFERENCES todos(id) ON DELETE CASCADE)",
    "CREATE INDEX IF NOT EXISTS idx_todos_session_id ON todos(session_id)",
    "CREATE INDEX IF NOT EXISTS idx_todos_project_id ON todos(project_id)",
    "CREATE INDEX IF NOT EXISTS idx_todos_parent_id ON todos(parent_id)",
    "UPDATE schema_version SET version = 6",
];

const SCHEMA_V7_MIGRATION: [&str; 5] = [
    "CREATE TABLE IF NOT EXISTS sub_agent_outputs (id TEXT PRIMARY KEY, caller_agent TEXT NOT NULL, target_agent TEXT NOT NULL, task_key TEXT NOT NULL, task_type TEXT, output TEXT NOT NULL, created_at TEXT NOT NULL, expires_at TEXT, metadata_json TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS idx_sub_agent_outputs_task_key ON sub_agent_outputs(task_key)",
    "CREATE INDEX IF NOT EXISTS idx_sub_agent_outputs_caller_target ON sub_agent_outputs(caller_agent, target_agent)",
    "CREATE INDEX IF NOT EXISTS idx_sub_agent_outputs_expires_at ON sub_agent_outputs(expires_at)",
    "UPDATE schema_version SET version = 7",
];

const SCHEMA_V8_MIGRATION: [&str; 3] = [
    "CREATE TABLE IF NOT EXISTS routing_traces (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, task TEXT NOT NULL, selected_agent TEXT NOT NULL, reason TEXT NOT NULL, confidence REAL NOT NULL, policy TEXT NOT NULL, alternatives_json TEXT NOT NULL, fallback_used INTEGER NOT NULL, context_pressure REAL, created_at TEXT NOT NULL, FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE)",
    "CREATE INDEX IF NOT EXISTS idx_routing_traces_session_id ON routing_traces(session_id)",
    "UPDATE schema_version SET version = 8",
];

#[derive(Debug, Clone)]
pub struct SqliteStorage {
    pool: sqlx::SqlitePool,
    initialized: std::sync::Arc<OnceCell<()>>,
    options: SqliteStorageConfig,
}

impl SqliteStorage {
    pub fn new(
        connection_string: &str,
        pool_size: usize,
        options: SqliteStorageConfig,
    ) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(pool_size as u32)
            .connect_lazy(connection_string)
            .map_err(|err| Error::Storage(format!("failed to create SQLite pool: {err}")));

        let pool = pool?;
        Ok(Self {
            pool,
            initialized: std::sync::Arc::new(OnceCell::new()),
            options,
        })
    }

    async fn ensure_initialized(&self) -> Result<()> {
        self.initialized
            .get_or_try_init(|| async {
                self.apply_runtime_settings().await?;

                let current_version = {
                    let row = sqlx::query("SELECT version FROM schema_version LIMIT 1")
                        .fetch_optional(&self.pool)
                        .await?;
                    row.map(|r| r.get::<i64, _>("version") as u32).unwrap_or(0)
                };

                if current_version < 1 {
                    for statement in SCHEMA_V1 {
                        sqlx::query(statement).execute(&self.pool).await?;
                    }
                }

                if current_version < 2 {
                    for statement in SCHEMA_V2_MIGRATION {
                        sqlx::query(statement).execute(&self.pool).await?;
                    }
                }

                if current_version < 3 {
                    for statement in SCHEMA_V3_MIGRATION {
                        sqlx::query(statement).execute(&self.pool).await?;
                    }
                }

                if current_version < 4 {
                    for statement in SCHEMA_V4_MIGRATION {
                        sqlx::query(statement).execute(&self.pool).await?;
                    }
                }

                if current_version < 5 {
                    for statement in SCHEMA_V5_MIGRATION {
                        sqlx::query(statement).execute(&self.pool).await?;
                    }
                }

                if current_version < 6 {
                    for statement in SCHEMA_V6_MIGRATION {
                        sqlx::query(statement).execute(&self.pool).await?;
                    }
                }

                if current_version < 7 {
                    for statement in SCHEMA_V7_MIGRATION {
                        sqlx::query(statement).execute(&self.pool).await?;
                    }
                }

                if current_version < 8 {
                    for statement in SCHEMA_V8_MIGRATION {
                        sqlx::query(statement).execute(&self.pool).await?;
                    }
                }

                Ok::<(), sqlx::Error>(())
            })
            .await
            .map_err(Error::from)
            .map(|_| ())
    }

    async fn apply_runtime_settings(&self) -> std::result::Result<(), sqlx::Error> {
        let foreign_keys = if self.options.foreign_keys {
            "ON"
        } else {
            "OFF"
        };
        let journal_mode = self.options.journal_mode.trim().to_ascii_uppercase();
        let synchronous = self.options.synchronous.trim().to_ascii_uppercase();

        sqlx::query(&format!("PRAGMA foreign_keys = {foreign_keys}"))
            .execute(&self.pool)
            .await?;
        sqlx::query(&format!("PRAGMA journal_mode = {journal_mode}"))
            .execute(&self.pool)
            .await?;
        sqlx::query(&format!("PRAGMA synchronous = {synchronous}"))
            .execute(&self.pool)
            .await?;
        sqlx::query(&format!(
            "PRAGMA busy_timeout = {}",
            self.options.busy_timeout_ms
        ))
        .execute(&self.pool)
        .await?;

        self.maybe_load_vector_extension().await?;

        Ok(())
    }

    async fn maybe_load_vector_extension(&self) -> std::result::Result<(), sqlx::Error> {
        if !self.options.vector_extension_enabled {
            return Ok(());
        }

        let extension_path = self
            .options
            .vector_extension_path
            .as_deref()
            .unwrap_or_default()
            .trim();
        if extension_path.is_empty() {
            if self.options.vector_extension_strict {
                return Err(sqlx::Error::Protocol(
                    "vector extension enabled but vector_extension_path is empty".to_owned(),
                ));
            }
            return Ok(());
        }

        let load_result = if let Some(entrypoint) = self
            .options
            .vector_extension_entrypoint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            sqlx::query("SELECT load_extension(?, ?)")
                .bind(extension_path)
                .bind(entrypoint)
                .execute(&self.pool)
                .await
        } else {
            sqlx::query("SELECT load_extension(?)")
                .bind(extension_path)
                .execute(&self.pool)
                .await
        };

        match load_result {
            Ok(_) => Ok(()),
            Err(err) if self.options.vector_extension_strict => Err(sqlx::Error::Protocol(
                format!("failed to load sqlite vector extension '{extension_path}': {err}"),
            )),
            Err(_) => Ok(()),
        }
    }

    fn parse_timestamp(value: &str) -> Result<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(value)
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .map_err(|err| Error::Storage(format!("failed to parse timestamp '{value}': {err}")))
    }

    fn parse_feedback_type(value: &str) -> FeedbackType {
        match value {
            "explicit" => FeedbackType::Explicit,
            "implicit_success" => FeedbackType::ImplicitSuccess,
            "implicit_permission_denied" => FeedbackType::ImplicitPermissionDenied,
            _ => FeedbackType::ImplicitError,
        }
    }

    fn parse_mistake_type(value: &str) -> MistakeType {
        match value {
            "permission_denied" => MistakeType::PermissionDenied,
            "tool_timeout" => MistakeType::ToolTimeout,
            "file_not_found" => MistakeType::FileNotFound,
            "compilation_error" => MistakeType::CompilationError,
            "test_failure" => MistakeType::TestFailure,
            _ => MistakeType::WrongApproach,
        }
    }

    fn parse_pattern_category(value: &str) -> PatternCategory {
        match value {
            "error_fixing" => PatternCategory::ErrorFixing,
            "refactoring" => PatternCategory::Refactoring,
            "debugging" => PatternCategory::Debugging,
            "testing" => PatternCategory::Testing,
            _ => PatternCategory::FeatureImplementation,
        }
    }

    fn parse_todo_status(value: &str) -> TodoStatus {
        match value {
            "todo" => TodoStatus::Todo,
            "in_progress" => TodoStatus::InProgress,
            "blocked" => TodoStatus::Blocked,
            "completed" => TodoStatus::Completed,
            "cancelled" => TodoStatus::Cancelled,
            _ => TodoStatus::Todo,
        }
    }

    fn parse_todo_priority(value: &str) -> TodoPriority {
        match value {
            "low" => TodoPriority::Low,
            "medium" => TodoPriority::Medium,
            "high" => TodoPriority::High,
            "critical" => TodoPriority::Critical,
            _ => TodoPriority::Medium,
        }
    }

    fn todo_status_as_str(status: TodoStatus) -> &'static str {
        match status {
            TodoStatus::Todo => "todo",
            TodoStatus::InProgress => "in_progress",
            TodoStatus::Blocked => "blocked",
            TodoStatus::Completed => "completed",
            TodoStatus::Cancelled => "cancelled",
        }
    }

    fn todo_priority_as_str(priority: TodoPriority) -> &'static str {
        match priority {
            TodoPriority::Low => "low",
            TodoPriority::Medium => "medium",
            TodoPriority::High => "high",
            TodoPriority::Critical => "critical",
        }
    }

    fn parse_sub_agent_output(&self, row: SqliteRow) -> Result<SubAgentOutput> {
        let id = row.get::<String, _>("id");
        let created_at = row.get::<String, _>("created_at");
        let expires_at: Option<String> = row.get("expires_at");
        let metadata_json = row.get::<String, _>("metadata_json");

        Ok(SubAgentOutput {
            id: Uuid::parse_str(&id).map_err(|err| {
                Error::Storage(format!("invalid sub-agent output uuid '{id}': {err}"))
            })?,
            caller_agent: row.get("caller_agent"),
            target_agent: row.get("target_agent"),
            task_key: row.get("task_key"),
            task_type: row.get("task_type"),
            output: row.get("output"),
            created_at: Self::parse_timestamp(&created_at)?,
            expires_at: expires_at
                .as_deref()
                .map(Self::parse_timestamp)
                .transpose()?,
            metadata: serde_json::from_str(&metadata_json)
                .unwrap_or_else(|_| serde_json::json!({})),
        })
    }
}

#[async_trait]
impl StorageBackend for SqliteStorage {
    async fn get_schema_version(&self) -> Result<Option<u32>> {
        self.ensure_initialized().await?;

        let row = sqlx::query("SELECT version FROM schema_version LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|value| value.get::<i64, _>("version") as u32))
    }

    async fn set_schema_version(&self, version: u32) -> Result<()> {
        self.ensure_initialized().await?;

        sqlx::query("UPDATE schema_version SET version = ?")
            .bind(version as i64)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn create_session(&self, session: Session) -> Result<()> {
        self.ensure_initialized().await?;

        sqlx::query(
            "INSERT INTO sessions(id, agent_name, created_at, config_json) VALUES(?, ?, ?, NULL)",
        )
        .bind(session.id.to_string())
        .bind(session.agent_name)
        .bind(session.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_session(&self, id: Uuid) -> Result<Option<Session>> {
        self.ensure_initialized().await?;

        let row = sqlx::query("SELECT id, agent_name, created_at FROM sessions WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let id_value = row.get::<String, _>("id");
        let created_at = row.get::<String, _>("created_at");
        Ok(Some(Session {
            id: Uuid::parse_str(&id_value).map_err(|err| {
                Error::Storage(format!("invalid session uuid '{id_value}': {err}"))
            })?,
            agent_name: row.get::<String, _>("agent_name"),
            created_at: Self::parse_timestamp(&created_at)?,
        }))
    }

    async fn list_sessions(&self, limit: Option<usize>) -> Result<Vec<Session>> {
        self.ensure_initialized().await?;

        let query = if let Some(limit) = limit {
            sqlx::query(
                "SELECT id, agent_name, created_at FROM sessions ORDER BY created_at DESC LIMIT ?",
            )
            .bind(limit as i64)
        } else {
            sqlx::query("SELECT id, agent_name, created_at FROM sessions ORDER BY created_at DESC")
        };

        let rows = query.fetch_all(&self.pool).await?;
        let mut sessions = Vec::with_capacity(rows.len());

        for row in rows {
            let id_value = row.get::<String, _>("id");
            let created_at = row.get::<String, _>("created_at");
            sessions.push(Session {
                id: Uuid::parse_str(&id_value).map_err(|err| {
                    Error::Storage(format!("invalid session uuid '{id_value}': {err}"))
                })?,
                agent_name: row.get::<String, _>("agent_name"),
                created_at: Self::parse_timestamp(&created_at)?,
            });
        }

        Ok(sessions)
    }

    async fn delete_session(&self, id: Uuid) -> Result<()> {
        self.ensure_initialized().await?;

        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn append_message(&self, message: Message) -> Result<()> {
        self.ensure_initialized().await?;

        sqlx::query(
            "INSERT INTO messages(id, session_id, role, content, created_at) VALUES(?, ?, ?, ?, ?)",
        )
        .bind(message.id.to_string())
        .bind(message.session_id.to_string())
        .bind(message.role)
        .bind(message.content)
        .bind(message.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_session_messages(&self, session_id: Uuid) -> Result<Vec<Message>> {
        self.ensure_initialized().await?;

        let rows = sqlx::query(
            "SELECT id, session_id, role, content, created_at FROM messages WHERE session_id = ? ORDER BY created_at ASC",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let id_value = row.get::<String, _>("id");
            let sid = row.get::<String, _>("session_id");
            let created_at = row.get::<String, _>("created_at");
            messages.push(Message {
                id: Uuid::parse_str(&id_value).map_err(|err| {
                    Error::Storage(format!("invalid message uuid '{id_value}': {err}"))
                })?,
                session_id: Uuid::parse_str(&sid).map_err(|err| {
                    Error::Storage(format!("invalid session uuid '{sid}': {err}"))
                })?,
                role: row.get::<String, _>("role"),
                content: row.get::<String, _>("content"),
                created_at: Self::parse_timestamp(&created_at)?,
            });
        }

        Ok(messages)
    }

    async fn get_recent_messages(&self, session_id: Uuid, limit: usize) -> Result<Vec<Message>> {
        self.ensure_initialized().await?;

        let rows = sqlx::query(
            "SELECT id, session_id, role, content, created_at FROM messages WHERE session_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(session_id.to_string())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let id_value = row.get::<String, _>("id");
            let sid = row.get::<String, _>("session_id");
            let created_at = row.get::<String, _>("created_at");
            messages.push(Message {
                id: Uuid::parse_str(&id_value).map_err(|err| {
                    Error::Storage(format!("invalid message uuid '{id_value}': {err}"))
                })?,
                session_id: Uuid::parse_str(&sid).map_err(|err| {
                    Error::Storage(format!("invalid session uuid '{sid}': {err}"))
                })?,
                role: row.get::<String, _>("role"),
                content: row.get::<String, _>("content"),
                created_at: Self::parse_timestamp(&created_at)?,
            });
        }

        messages.reverse();
        Ok(messages)
    }

    async fn get_session_config(&self, session_id: Uuid) -> Result<Option<SessionConfig>> {
        self.ensure_initialized().await?;

        let row = sqlx::query("SELECT config_json FROM sessions WHERE id = ?")
            .bind(session_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let value: Option<String> = row.get("config_json");
        let Some(value) = value else {
            return Ok(Some(SessionConfig::default()));
        };

        Ok(Some(serde_json::from_str(&value)?))
    }

    async fn update_session_config(&self, session_id: Uuid, config: &SessionConfig) -> Result<()> {
        self.ensure_initialized().await?;

        let payload = serde_json::to_string(config)?;
        sqlx::query("UPDATE sessions SET config_json = ? WHERE id = ?")
            .bind(payload)
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_or_load_context_file(&self, path: &str) -> Result<Option<String>> {
        self.ensure_initialized().await?;

        let row = sqlx::query("SELECT content FROM context_files WHERE path = ?")
            .bind(path)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|value| value.get::<String, _>("content")))
    }

    async fn cache_context_file(&self, path: &str, content: &str, metadata: &str) -> Result<()> {
        self.ensure_initialized().await?;

        sqlx::query(
            "INSERT INTO context_files(path, content, metadata, loaded_at) VALUES(?, ?, ?, ?) ON CONFLICT(path) DO UPDATE SET content = excluded.content, metadata = excluded.metadata, loaded_at = excluded.loaded_at",
        )
        .bind(path)
        .bind(content)
        .bind(metadata)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn update_session_topics(&self, session_id: Uuid, topics: &[String]) -> Result<()> {
        self.ensure_initialized().await?;

        let payload = serde_json::to_string(topics)?;
        sqlx::query(
            "INSERT INTO session_topics(session_id, topics_json, updated_at) VALUES(?, ?, ?) ON CONFLICT(session_id) DO UPDATE SET topics_json = excluded.topics_json, updated_at = excluded.updated_at",
        )
        .bind(session_id.to_string())
        .bind(payload)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_session_topics(&self, session_id: Uuid) -> Result<Option<Vec<String>>> {
        self.ensure_initialized().await?;

        let row = sqlx::query("SELECT topics_json FROM session_topics WHERE session_id = ?")
            .bind(session_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let payload = row.get::<String, _>("topics_json");
        let topics = serde_json::from_str(&payload)?;
        Ok(Some(topics))
    }

    async fn track_manual_invocation(&self, session_id: Uuid, rule_path: &str) -> Result<()> {
        self.ensure_initialized().await?;

        sqlx::query(
            "INSERT INTO manual_rule_invocations(id, session_id, rule_path, invoked_at) VALUES(?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(session_id.to_string())
        .bind(rule_path)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_manual_invocations(&self, session_id: Uuid) -> Result<Vec<String>> {
        self.ensure_initialized().await?;

        let rows = sqlx::query(
            "SELECT rule_path FROM manual_rule_invocations WHERE session_id = ? ORDER BY invoked_at ASC",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| row.get::<String, _>("rule_path"))
            .collect())
    }

    async fn set_pending_tool(&self, state: &PendingToolState) -> Result<()> {
        self.ensure_initialized().await?;

        let tool_messages_json = serde_json::to_string(&state.tool_messages)?;
        let context_snapshot_json = serde_json::to_string(&state.context_snapshot)?;
        let args_json = serde_json::to_string(&state.args)?;

        sqlx::query(
            "INSERT OR REPLACE INTO pending_tools(session_id, tool_name, args_json, round_index, tool_messages_json, context_snapshot_json, created_at) VALUES(?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(state.session_id.to_string())
        .bind(&state.tool_name)
        .bind(args_json)
        .bind(state.round_index as i64)
        .bind(tool_messages_json)
        .bind(context_snapshot_json)
        .bind(state.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_and_clear_pending_tool(
        &self,
        session_id: Uuid,
    ) -> Result<Option<PendingToolState>> {
        self.ensure_initialized().await?;

        let row = sqlx::query(
            "SELECT session_id, tool_name, args_json, round_index, tool_messages_json, context_snapshot_json, created_at FROM pending_tools WHERE session_id = ?",
        )
        .bind(session_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        // Delete the pending tool state
        sqlx::query("DELETE FROM pending_tools WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;

        let tool_messages_json = row.get::<String, _>("tool_messages_json");
        let context_snapshot_json = row.get::<String, _>("context_snapshot_json");
        let args_json = row.get::<String, _>("args_json");
        let session_id_str = row.get::<String, _>("session_id");
        let created_at_str = row.get::<String, _>("created_at");

        Ok(Some(PendingToolState {
            session_id: Uuid::parse_str(&session_id_str).map_err(|err| {
                Error::Storage(format!(
                    "invalid session uuid in pending tool state '{session_id_str}': {err}"
                ))
            })?,
            tool_name: row.get::<String, _>("tool_name"),
            args: serde_json::from_str(&args_json)?,
            round_index: row.get::<i64, _>("round_index") as usize,
            tool_messages: serde_json::from_str(&tool_messages_json)?,
            context_snapshot: serde_json::from_str(&context_snapshot_json)?,
            created_at: Self::parse_timestamp(&created_at_str)?,
        }))
    }

    async fn delete_stale_pending_tools(&self, older_than_secs: u64) -> Result<usize> {
        self.ensure_initialized().await?;

        let cutoff_time = Utc::now() - chrono::Duration::seconds(older_than_secs as i64);
        let cutoff_rfc3339 = cutoff_time.to_rfc3339();

        let result = sqlx::query("DELETE FROM pending_tools WHERE created_at < ?")
            .bind(cutoff_rfc3339)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() as usize)
    }

    async fn has_pending_tool(&self, session_id: Uuid) -> Result<bool> {
        self.ensure_initialized().await?;

        let row = sqlx::query("SELECT 1 FROM pending_tools WHERE session_id = ?")
            .bind(session_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.is_some())
    }

    async fn store_user_feedback(&self, feedback: &UserFeedback) -> Result<()> {
        self.ensure_initialized().await?;

        sqlx::query("INSERT OR REPLACE INTO user_feedback(id, session_id, agent_name, feedback_type, rating, comment, context_json, created_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(feedback.id.to_string())
            .bind(feedback.session_id.to_string())
            .bind(&feedback.agent_name)
            .bind(feedback.feedback_type.as_str())
            .bind(feedback.rating as i64)
            .bind(&feedback.comment)
            .bind(serde_json::to_string(&feedback.context)?)
            .bind(feedback.created_at.to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn list_user_feedback(
        &self,
        session_id: Uuid,
        limit: usize,
    ) -> Result<Vec<UserFeedback>> {
        self.ensure_initialized().await?;

        let rows = sqlx::query("SELECT id, session_id, agent_name, feedback_type, rating, comment, context_json, created_at FROM user_feedback WHERE session_id = ? ORDER BY created_at DESC LIMIT ?")
            .bind(session_id.to_string())
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;

        let mut feedback = Vec::with_capacity(rows.len());
        for row in rows {
            let id = row.get::<String, _>("id");
            let sid = row.get::<String, _>("session_id");
            let context_json = row.get::<String, _>("context_json");
            let created_at = row.get::<String, _>("created_at");
            feedback.push(UserFeedback {
                id: Uuid::parse_str(&id)
                    .map_err(|err| Error::Storage(format!("invalid feedback id '{id}': {err}")))?,
                session_id: Uuid::parse_str(&sid).map_err(|err| {
                    Error::Storage(format!("invalid feedback session id '{sid}': {err}"))
                })?,
                agent_name: row.get::<String, _>("agent_name"),
                feedback_type: Self::parse_feedback_type(
                    row.get::<String, _>("feedback_type").as_str(),
                ),
                rating: row.get::<i64, _>("rating") as i8,
                comment: row.get::<Option<String>, _>("comment"),
                context: serde_json::from_str(&context_json)?,
                created_at: Self::parse_timestamp(&created_at)?,
            });
        }
        feedback.reverse();
        Ok(feedback)
    }

    async fn upsert_mistake_pattern(&self, pattern: &MistakePattern) -> Result<()> {
        self.ensure_initialized().await?;

        sqlx::query("INSERT INTO mistake_patterns(id, agent_name, mistake_type, trigger, frequency, last_seen, suggested_fix) VALUES(?, ?, ?, ?, ?, ?, ?) ON CONFLICT(agent_name, mistake_type, trigger) DO UPDATE SET frequency = excluded.frequency, last_seen = excluded.last_seen, suggested_fix = excluded.suggested_fix")
            .bind(pattern.id.to_string())
            .bind(&pattern.agent_name)
            .bind(pattern.mistake_type.as_str())
            .bind(&pattern.trigger)
            .bind(pattern.frequency as i64)
            .bind(pattern.last_seen.to_rfc3339())
            .bind(&pattern.suggested_fix)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn list_mistake_patterns(
        &self,
        agent_name: &str,
        min_frequency: u32,
        limit: usize,
    ) -> Result<Vec<MistakePattern>> {
        self.ensure_initialized().await?;

        let rows = sqlx::query("SELECT id, agent_name, mistake_type, trigger, frequency, last_seen, suggested_fix FROM mistake_patterns WHERE agent_name = ? AND frequency >= ? ORDER BY frequency DESC, last_seen DESC LIMIT ?")
            .bind(agent_name)
            .bind(min_frequency as i64)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;

        let mut patterns = Vec::with_capacity(rows.len());
        for row in rows {
            let id = row.get::<String, _>("id");
            let last_seen = row.get::<String, _>("last_seen");
            let mistake_type = row.get::<String, _>("mistake_type");
            patterns.push(MistakePattern {
                id: Uuid::parse_str(&id).map_err(|err| {
                    Error::Storage(format!("invalid mistake pattern id '{id}': {err}"))
                })?,
                agent_name: row.get::<String, _>("agent_name"),
                mistake_type: Self::parse_mistake_type(&mistake_type),
                trigger: row.get::<String, _>("trigger"),
                frequency: row.get::<i64, _>("frequency") as u32,
                last_seen: Self::parse_timestamp(&last_seen)?,
                suggested_fix: row.get::<Option<String>, _>("suggested_fix"),
            });
        }
        Ok(patterns)
    }

    async fn upsert_user_preference(
        &self,
        session_id: Uuid,
        key: &str,
        value: &PreferenceValue,
    ) -> Result<()> {
        self.ensure_initialized().await?;

        sqlx::query("INSERT INTO user_preferences(session_id, preference_key, value_json, value_kind, updated_at) VALUES(?, ?, ?, ?, ?) ON CONFLICT(session_id, preference_key) DO UPDATE SET value_json = excluded.value_json, value_kind = excluded.value_kind, updated_at = excluded.updated_at")
            .bind(session_id.to_string())
            .bind(key)
            .bind(serde_json::to_string(value)?)
            .bind(value.kind())
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn get_user_preference(
        &self,
        session_id: Uuid,
        key: &str,
    ) -> Result<Option<PreferenceValue>> {
        self.ensure_initialized().await?;

        let row = sqlx::query(
            "SELECT value_json FROM user_preferences WHERE session_id = ? AND preference_key = ?",
        )
        .bind(session_id.to_string())
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let value_json = row.get::<String, _>("value_json");
        Ok(Some(serde_json::from_str(&value_json)?))
    }

    async fn list_user_preferences(&self, session_id: Uuid) -> Result<Vec<UserPreference>> {
        self.ensure_initialized().await?;

        let rows = sqlx::query(
            "SELECT preference_key, value_json, updated_at FROM user_preferences WHERE session_id = ? ORDER BY preference_key ASC",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut preferences = Vec::with_capacity(rows.len());
        for row in rows {
            let value_json = row.get::<String, _>("value_json");
            let updated_at = row.get::<String, _>("updated_at");
            preferences.push(UserPreference {
                session_id,
                key: row.get::<String, _>("preference_key"),
                value: serde_json::from_str(&value_json)?,
                updated_at: Self::parse_timestamp(&updated_at)?,
            });
        }

        Ok(preferences)
    }

    async fn upsert_success_pattern(&self, pattern: &SuccessPattern) -> Result<()> {
        self.ensure_initialized().await?;

        sqlx::query("INSERT INTO success_patterns(id, agent_name, name, category, description, template, frequency, last_used, success_rate, created_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name, category = excluded.category, description = excluded.description, template = excluded.template, frequency = excluded.frequency, last_used = excluded.last_used, success_rate = excluded.success_rate")
            .bind(pattern.id.to_string())
            .bind(&pattern.agent_name)
            .bind(&pattern.name)
            .bind(pattern.category.as_str())
            .bind(&pattern.description)
            .bind(&pattern.template)
            .bind(pattern.frequency as i64)
            .bind(pattern.last_used.to_rfc3339())
            .bind(pattern.success_rate)
            .bind(pattern.created_at.to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn find_success_patterns(
        &self,
        agent_name: &str,
        category: Option<PatternCategory>,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SuccessPattern>> {
        self.ensure_initialized().await?;

        let rows = if let Some(category) = category {
            let search = query.map(|value| format!("%{}%", value.to_ascii_lowercase()));
            if let Some(search) = search {
                sqlx::query("SELECT id, agent_name, name, category, description, template, frequency, last_used, success_rate, created_at FROM success_patterns WHERE agent_name = ? AND category = ? AND (LOWER(name) LIKE ? OR LOWER(description) LIKE ? OR LOWER(template) LIKE ?) ORDER BY frequency DESC, last_used DESC LIMIT ?")
                    .bind(agent_name)
                    .bind(category.as_str())
                    .bind(&search)
                    .bind(&search)
                    .bind(&search)
                    .bind(limit as i64)
                    .fetch_all(&self.pool)
                    .await?
            } else {
                sqlx::query("SELECT id, agent_name, name, category, description, template, frequency, last_used, success_rate, created_at FROM success_patterns WHERE agent_name = ? AND category = ? ORDER BY frequency DESC, last_used DESC LIMIT ?")
                    .bind(agent_name)
                    .bind(category.as_str())
                    .bind(limit as i64)
                    .fetch_all(&self.pool)
                    .await?
            }
        } else {
            let search = query.map(|value| format!("%{}%", value.to_ascii_lowercase()));
            if let Some(search) = search {
                sqlx::query("SELECT id, agent_name, name, category, description, template, frequency, last_used, success_rate, created_at FROM success_patterns WHERE agent_name = ? AND (LOWER(name) LIKE ? OR LOWER(description) LIKE ? OR LOWER(template) LIKE ?) ORDER BY frequency DESC, last_used DESC LIMIT ?")
                    .bind(agent_name)
                    .bind(&search)
                    .bind(&search)
                    .bind(&search)
                    .bind(limit as i64)
                    .fetch_all(&self.pool)
                    .await?
            } else {
                sqlx::query("SELECT id, agent_name, name, category, description, template, frequency, last_used, success_rate, created_at FROM success_patterns WHERE agent_name = ? ORDER BY frequency DESC, last_used DESC LIMIT ?")
                    .bind(agent_name)
                    .bind(limit as i64)
                    .fetch_all(&self.pool)
                    .await?
            }
        };

        let mut patterns = Vec::with_capacity(rows.len());
        for row in rows {
            let id = row.get::<String, _>("id");
            let category = row.get::<String, _>("category");
            let created_at = row.get::<String, _>("created_at");
            let last_used = row.get::<String, _>("last_used");
            patterns.push(SuccessPattern {
                id: Uuid::parse_str(&id).map_err(|err| {
                    Error::Storage(format!("invalid success pattern id '{id}': {err}"))
                })?,
                agent_name: row.get::<String, _>("agent_name"),
                name: row.get::<String, _>("name"),
                category: Self::parse_pattern_category(&category),
                description: row.get::<String, _>("description"),
                template: row.get::<String, _>("template"),
                frequency: row.get::<i64, _>("frequency") as u32,
                last_used: Self::parse_timestamp(&last_used)?,
                success_rate: row.get::<f64, _>("success_rate"),
                created_at: Self::parse_timestamp(&created_at)?,
            });
        }

        Ok(patterns)
    }

    async fn upsert_code_index_metadata(
        &self,
        workspace: &str,
        updated_at: DateTime<Utc>,
    ) -> Result<()> {
        self.ensure_initialized().await?;
        sqlx::query(
            "INSERT INTO code_index_metadata(workspace, updated_at) VALUES(?, ?) ON CONFLICT(workspace) DO UPDATE SET updated_at = excluded.updated_at",
        )
        .bind(workspace)
        .bind(updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_code_index_metadata(&self, workspace: &str) -> Result<Option<DateTime<Utc>>> {
        self.ensure_initialized().await?;
        let row = sqlx::query("SELECT updated_at FROM code_index_metadata WHERE workspace = ?")
            .bind(workspace)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let updated_at = row.get::<String, _>("updated_at");
        Ok(Some(Self::parse_timestamp(&updated_at)?))
    }

    async fn upsert_code_file_index(
        &self,
        workspace: &str,
        path: &str,
        language: &str,
        functions: &[String],
        classes: &[String],
        imports: &[String],
    ) -> Result<()> {
        self.ensure_initialized().await?;
        sqlx::query(
            "INSERT INTO code_file_indexes(workspace, path, language, functions_json, classes_json, imports_json, updated_at) VALUES(?, ?, ?, ?, ?, ?, ?) ON CONFLICT(workspace, path) DO UPDATE SET language = excluded.language, functions_json = excluded.functions_json, classes_json = excluded.classes_json, imports_json = excluded.imports_json, updated_at = excluded.updated_at",
        )
        .bind(workspace)
        .bind(path)
        .bind(language)
        .bind(serde_json::to_string(functions)?)
        .bind(serde_json::to_string(classes)?)
        .bind(serde_json::to_string(imports)?)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_code_file_indexes(&self, workspace: &str) -> Result<Vec<IndexedFileRecord>> {
        self.ensure_initialized().await?;
        let rows = sqlx::query("SELECT path, language, functions_json, classes_json, imports_json, updated_at FROM code_file_indexes WHERE workspace = ? ORDER BY path ASC")
            .bind(workspace)
            .fetch_all(&self.pool)
            .await?;
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let functions_json = row.get::<String, _>("functions_json");
            let classes_json = row.get::<String, _>("classes_json");
            let imports_json = row.get::<String, _>("imports_json");
            let updated_at = row.get::<String, _>("updated_at");
            records.push(IndexedFileRecord {
                path: row.get::<String, _>("path"),
                language: row.get::<String, _>("language"),
                functions: serde_json::from_str(&functions_json)?,
                classes: serde_json::from_str(&classes_json)?,
                imports: serde_json::from_str(&imports_json)?,
                updated_at: Self::parse_timestamp(&updated_at)?,
            });
        }
        Ok(records)
    }

    async fn replace_code_symbols_for_file(
        &self,
        workspace: &str,
        file_path: &str,
        symbols: &[SymbolIndex],
    ) -> Result<()> {
        self.ensure_initialized().await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM code_symbol_indexes WHERE workspace = ? AND file_path = ?")
            .bind(workspace)
            .bind(file_path)
            .execute(&mut *tx)
            .await?;

        for symbol in symbols {
            sqlx::query("INSERT INTO code_symbol_indexes(workspace, file_path, name, symbol_type, line, column_number, docstring, signature, updated_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(workspace)
                .bind(&symbol.file_path)
                .bind(&symbol.name)
                .bind(symbol.symbol_type.as_str())
                .bind(symbol.line as i64)
                .bind(symbol.column as i64)
                .bind(&symbol.docstring)
                .bind(&symbol.signature)
                .bind(Utc::now().to_rfc3339())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn list_code_symbols(&self, workspace: &str) -> Result<Vec<IndexedSymbolRecord>> {
        self.ensure_initialized().await?;
        let rows = sqlx::query("SELECT name, symbol_type, file_path, line, column_number, docstring, signature, updated_at FROM code_symbol_indexes WHERE workspace = ? ORDER BY file_path ASC, line ASC")
            .bind(workspace)
            .fetch_all(&self.pool)
            .await?;
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let updated_at = row.get::<String, _>("updated_at");
            let symbol_type = row.get::<String, _>("symbol_type");
            records.push(IndexedSymbolRecord {
                name: row.get::<String, _>("name"),
                symbol_type: SymbolType::from_storage_value(&symbol_type),
                file_path: row.get::<String, _>("file_path"),
                line: row.get::<i64, _>("line") as usize,
                column: row.get::<i64, _>("column_number") as usize,
                docstring: row.get::<Option<String>, _>("docstring"),
                signature: row.get::<Option<String>, _>("signature"),
                updated_at: Self::parse_timestamp(&updated_at)?,
            });
        }
        Ok(records)
    }

    async fn search_code_symbols(
        &self,
        workspace: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SymbolIndex>> {
        self.ensure_initialized().await?;
        let pattern = format!("%{}%", query.to_ascii_lowercase());
        let rows = sqlx::query("SELECT name, symbol_type, file_path, line, column_number, docstring, signature FROM code_symbol_indexes WHERE workspace = ? AND LOWER(name) LIKE ? ORDER BY name ASC LIMIT ?")
            .bind(workspace)
            .bind(pattern)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;
        let mut symbols = Vec::with_capacity(rows.len());
        for row in rows {
            let symbol_type = row.get::<String, _>("symbol_type");
            symbols.push(SymbolIndex {
                name: row.get::<String, _>("name"),
                symbol_type: SymbolType::from_storage_value(&symbol_type),
                file_path: row.get::<String, _>("file_path"),
                line: row.get::<i64, _>("line") as usize,
                column: row.get::<i64, _>("column_number") as usize,
                docstring: row.get::<Option<String>, _>("docstring"),
                signature: row.get::<Option<String>, _>("signature"),
            });
        }
        Ok(symbols)
    }

    async fn replace_code_call_edges_for_file(
        &self,
        workspace: &str,
        file_path: &str,
        edges: &[CallEdge],
    ) -> Result<()> {
        self.ensure_initialized().await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM code_call_edges WHERE workspace = ? AND file_path = ?")
            .bind(workspace)
            .bind(file_path)
            .execute(&mut *tx)
            .await?;

        for edge in edges {
            sqlx::query("INSERT INTO code_call_edges(workspace, file_path, caller_symbol, callee_symbol, line, column_number, updated_at) VALUES(?, ?, ?, ?, ?, ?, ?)")
                .bind(workspace)
                .bind(&edge.file_path)
                .bind(&edge.caller_symbol)
                .bind(&edge.callee_symbol)
                .bind(edge.line as i64)
                .bind(edge.column as i64)
                .bind(Utc::now().to_rfc3339())
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn list_code_call_edges(&self, workspace: &str) -> Result<Vec<IndexedCallEdgeRecord>> {
        self.ensure_initialized().await?;
        let rows = sqlx::query("SELECT caller_symbol, callee_symbol, file_path, line, column_number, updated_at FROM code_call_edges WHERE workspace = ? ORDER BY file_path ASC, line ASC")
            .bind(workspace)
            .fetch_all(&self.pool)
            .await?;

        let mut edges = Vec::with_capacity(rows.len());
        for row in rows {
            let updated_at = row.get::<String, _>("updated_at");
            edges.push(IndexedCallEdgeRecord {
                caller_symbol: row.get::<String, _>("caller_symbol"),
                callee_symbol: row.get::<String, _>("callee_symbol"),
                file_path: row.get::<String, _>("file_path"),
                line: row.get::<i64, _>("line") as usize,
                column: row.get::<i64, _>("column_number") as usize,
                updated_at: Self::parse_timestamp(&updated_at)?,
            });
        }

        Ok(edges)
    }

    async fn upsert_vector_embedding(
        &self,
        workspace: &str,
        id: &str,
        vector: &[f32],
        metadata: &serde_json::Value,
    ) -> Result<()> {
        self.ensure_initialized().await?;
        sqlx::query("INSERT INTO vector_embeddings(workspace, id, vector_json, metadata_json, updated_at) VALUES(?, ?, ?, ?, ?) ON CONFLICT(workspace, id) DO UPDATE SET vector_json = excluded.vector_json, metadata_json = excluded.metadata_json, updated_at = excluded.updated_at")
            .bind(workspace)
            .bind(id)
            .bind(serde_json::to_string(vector)?)
            .bind(serde_json::to_string(metadata)?)
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_vector_embeddings(&self, workspace: &str) -> Result<Vec<StoredVector>> {
        self.ensure_initialized().await?;
        let rows = sqlx::query(
            "SELECT id, vector_json, metadata_json FROM vector_embeddings WHERE workspace = ?",
        )
        .bind(workspace)
        .fetch_all(&self.pool)
        .await?;

        let mut vectors = Vec::with_capacity(rows.len());
        for row in rows {
            let id = row.get::<String, _>("id");
            let vector_json = row.get::<String, _>("vector_json");
            let metadata_json = row.get::<String, _>("metadata_json");
            vectors.push(StoredVector {
                id,
                vector: serde_json::from_str(&vector_json)?,
                metadata: serde_json::from_str(&metadata_json)?,
            });
        }

        Ok(vectors)
    }

    async fn create_todo(&self, todo: &Todo) -> Result<()> {
        self.ensure_initialized().await?;
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO todos(id, project_id, session_id, parent_id, title, description, status, priority, tags_json, metadata_json, created_at, updated_at, completed_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(todo.id.to_string())
        .bind(todo.project_id.as_deref())
        .bind(todo.session_id.to_string())
        .bind(todo.parent_id.map(|id| id.to_string()))
        .bind(&todo.title)
        .bind(&todo.description)
        .bind(Self::todo_status_as_str(todo.status))
        .bind(Self::todo_priority_as_str(todo.priority))
        .bind(serde_json::to_string(&todo.tags)?)
        .bind(serde_json::to_string(&todo.metadata)?)
        .bind(todo.created_at.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(todo.completed_at.map(|dt| dt.to_rfc3339()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_todos(&self, filter: &TodoFilter) -> Result<Vec<Todo>> {
        self.ensure_initialized().await?;

        // Build dynamic query based on filter
        let mut where_clauses = Vec::new();
        let mut binds: Vec<String> = Vec::new();

        if filter.session_id.is_some() {
            where_clauses.push("session_id = ?");
            if let Some(session_id) = filter.session_id {
                binds.push(session_id.to_string());
            }
        }
        if let Some(ref project_id) = filter.project_id {
            where_clauses.push("project_id = ?");
            binds.push(project_id.clone());
        }
        if filter.parent_id.is_some() {
            where_clauses.push("parent_id = ?");
            if let Some(parent_id) = filter.parent_id {
                binds.push(parent_id.to_string());
            }
        }
        if filter.status.is_some() {
            where_clauses.push("status = ?");
            if let Some(status) = filter.status {
                binds.push(Self::todo_status_as_str(status).to_string());
            }
        }
        if filter.priority.is_some() {
            where_clauses.push("priority = ?");
            if let Some(priority) = filter.priority {
                binds.push(Self::todo_priority_as_str(priority).to_string());
            }
        }

        let mut query = String::from(
            "SELECT id, project_id, session_id, parent_id, title, description, status, priority, tags_json, metadata_json, created_at, updated_at, completed_at FROM todos",
        );

        if !where_clauses.is_empty() {
            query.push_str(" WHERE ");
            query.push_str(&where_clauses.join(" AND "));
        }

        query.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = filter.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        let mut q = sqlx::query(&query);
        for bind in &binds {
            q = q.bind(bind);
        }

        let rows = q.fetch_all(&self.pool).await?;
        let mut todos = Vec::with_capacity(rows.len());

        for row in rows {
            let id = row.get::<String, _>("id");
            let parent_id: Option<String> = row.get("parent_id");
            let created_at = row.get::<String, _>("created_at");
            let updated_at = row.get::<String, _>("updated_at");
            let completed_at: Option<String> = row.get("completed_at");

            todos.push(Todo {
                id: Uuid::parse_str(&id)
                    .map_err(|err| Error::Storage(format!("invalid todo uuid '{id}': {err}")))?,
                project_id: row.get("project_id"),
                session_id: Uuid::parse_str(&row.get::<String, _>("session_id"))
                    .map_err(|err| Error::Storage(format!("invalid todo session uuid: {err}")))?,
                parent_id: parent_id.and_then(|value| Uuid::parse_str(&value).ok()),
                title: row.get("title"),
                description: row.get("description"),
                status: Self::parse_todo_status(row.get("status")),
                priority: Self::parse_todo_priority(row.get("priority")),
                tags: serde_json::from_str(row.get("tags_json"))?,
                metadata: serde_json::from_str(row.get("metadata_json"))?,
                created_at: Self::parse_timestamp(&created_at)?,
                updated_at: Self::parse_timestamp(&updated_at)?,
                completed_at: completed_at
                    .as_ref()
                    .and_then(|value| Self::parse_timestamp(value).ok()),
            });
        }

        Ok(todos)
    }

    async fn update_todo(&self, id: Uuid, update: &TodoUpdate) -> Result<()> {
        self.ensure_initialized().await?;
        let mut set_clauses = Vec::new();
        let mut binds: Vec<String> = Vec::new();

        if let Some(ref title) = update.title {
            set_clauses.push("title = ?");
            binds.push(title.clone());
        }
        if update.description.is_some() {
            set_clauses.push("description = ?");
            if let Some(ref desc) = update.description {
                binds.push(desc.as_deref().unwrap_or("").to_string());
            }
        }
        if let Some(status) = update.status {
            set_clauses.push("status = ?");
            binds.push(Self::todo_status_as_str(status).to_string());
            if status == TodoStatus::Completed {
                set_clauses.push("completed_at = ?");
                binds.push(Utc::now().to_rfc3339());
            }
        }
        if let Some(priority) = update.priority {
            set_clauses.push("priority = ?");
            binds.push(Self::todo_priority_as_str(priority).to_string());
        }
        if let Some(tags) = update.tags.as_ref() {
            set_clauses.push("tags_json = ?");
            binds.push(serde_json::to_string(tags)?);
        }
        if let Some(metadata) = update.metadata.as_ref() {
            set_clauses.push("metadata_json = ?");
            binds.push(serde_json::to_string(metadata)?);
        }

        if set_clauses.is_empty() {
            return Ok(());
        }

        set_clauses.push("updated_at = ?");
        binds.push(Utc::now().to_rfc3339());

        let query = format!("UPDATE todos SET {} WHERE id = ?", set_clauses.join(", "));

        let mut q = sqlx::query(&query);
        for bind in &binds {
            q = q.bind(bind);
        }
        q = q.bind(id.to_string());

        q.execute(&self.pool).await?;
        Ok(())
    }

    async fn delete_todo(&self, id: Uuid) -> Result<()> {
        self.ensure_initialized().await?;
        sqlx::query("DELETE FROM todos WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_todo(&self, id: Uuid) -> Result<Option<Todo>> {
        self.ensure_initialized().await?;
        let row = sqlx::query(
            "SELECT id, project_id, session_id, parent_id, title, description, status, priority, tags_json, metadata_json, created_at, updated_at, completed_at FROM todos WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let id = row.get::<String, _>("id");
        let parent_id: Option<String> = row.get("parent_id");
        let created_at = row.get::<String, _>("created_at");
        let updated_at = row.get::<String, _>("updated_at");
        let completed_at: Option<String> = row.get("completed_at");

        Ok(Some(Todo {
            id: Uuid::parse_str(&id)
                .map_err(|err| Error::Storage(format!("invalid todo uuid '{id}': {err}")))?,
            project_id: row.get("project_id"),
            session_id: Uuid::parse_str(&row.get::<String, _>("session_id"))
                .map_err(|err| Error::Storage(format!("invalid todo session uuid: {err}")))?,
            parent_id: parent_id.and_then(|value| Uuid::parse_str(&value).ok()),
            title: row.get("title"),
            description: row.get("description"),
            status: Self::parse_todo_status(row.get("status")),
            priority: Self::parse_todo_priority(row.get("priority")),
            tags: serde_json::from_str(row.get("tags_json"))?,
            metadata: serde_json::from_str(row.get("metadata_json"))?,
            created_at: Self::parse_timestamp(&created_at)?,
            updated_at: Self::parse_timestamp(&updated_at)?,
            completed_at: completed_at
                .as_ref()
                .and_then(|value| Self::parse_timestamp(value).ok()),
        }))
    }

    async fn complete_todo_chain(&self, id: Uuid) -> Result<()> {
        self.ensure_initialized().await?;
        let mut current_id = Some(id);

        while let Some(todo_id) = current_id {
            // Get the current TODO
            let todo = self.get_todo(todo_id).await?;
            let Some(ref todo) = todo else {
                break;
            };

            // Update to completed status
            self.update_todo(
                todo_id,
                &TodoUpdate {
                    status: Some(TodoStatus::Completed),
                    ..Default::default()
                },
            )
            .await?;

            // Check if this TODO has a parent, and if all siblings are completed
            if let Some(parent_id) = todo.parent_id {
                // Get all siblings of this TODO (children of the same parent)
                let siblings = self
                    .list_todos(&TodoFilter {
                        parent_id: Some(parent_id),
                        ..Default::default()
                    })
                    .await?;

                // Check if all siblings are completed
                let all_completed = siblings
                    .iter()
                    .all(|sib| sib.status == TodoStatus::Completed);

                if all_completed {
                    current_id = Some(parent_id);
                    continue;
                }
            }

            break;
        }

        Ok(())
    }

    async fn upsert_sub_agent_output(&self, output: &SubAgentOutput) -> Result<()> {
        self.ensure_initialized().await?;

        let expires_at = output.expires_at.map(|dt| dt.to_rfc3339());

        sqlx::query(
            "INSERT OR REPLACE INTO sub_agent_outputs(id, caller_agent, target_agent, task_key, task_type, output, created_at, expires_at, metadata_json) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(output.id.to_string())
        .bind(&output.caller_agent)
        .bind(&output.target_agent)
        .bind(&output.task_key)
        .bind(&output.task_type)
        .bind(&output.output)
        .bind(output.created_at.to_rfc3339())
        .bind(expires_at)
        .bind(serde_json::to_string(&output.metadata)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_sub_agent_output_exact(&self, task_key: &str) -> Result<Option<SubAgentOutput>> {
        self.ensure_initialized().await?;

        let row = sqlx::query(
            "SELECT id, caller_agent, target_agent, task_key, task_type, output, created_at, expires_at, metadata_json FROM sub_agent_outputs WHERE task_key = ? AND (expires_at IS NULL OR expires_at > datetime('now')) ORDER BY created_at DESC LIMIT 1",
        )
        .bind(task_key)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(self.parse_sub_agent_output(row)?))
    }

    async fn get_sub_agent_output_semantic(
        &self,
        task_type: &str,
        caller_agent: &str,
        target_agent: &str,
    ) -> Result<Vec<SubAgentOutput>> {
        self.ensure_initialized().await?;

        let rows = sqlx::query(
            "SELECT id, caller_agent, target_agent, task_key, task_type, output, created_at, expires_at, metadata_json FROM sub_agent_outputs WHERE task_type = ? AND caller_agent = ? AND target_agent = ? AND (expires_at IS NULL OR expires_at > datetime('now')) ORDER BY created_at DESC LIMIT 10",
        )
        .bind(task_type)
        .bind(caller_agent)
        .bind(target_agent)
        .fetch_all(&self.pool)
        .await?;

        let mut outputs = Vec::with_capacity(rows.len());
        for row in rows {
            outputs.push(self.parse_sub_agent_output(row)?);
        }
        Ok(outputs)
    }

    async fn list_sub_agent_outputs(
        &self,
        filter: &SubAgentOutputFilter,
    ) -> Result<Vec<SubAgentOutput>> {
        self.ensure_initialized().await?;

        let mut query = String::from(
            "SELECT id, caller_agent, target_agent, task_key, task_type, output, created_at, expires_at, metadata_json FROM sub_agent_outputs WHERE 1=1",
        );
        if filter.exclude_expired {
            query.push_str(" AND (expires_at IS NULL OR expires_at > datetime('now'))");
        }
        if filter.caller_agent.is_some() {
            query.push_str(" AND caller_agent = ?");
        }
        if filter.target_agent.is_some() {
            query.push_str(" AND target_agent = ?");
        }
        if filter.task_type.is_some() {
            query.push_str(" AND task_type = ?");
        }
        query.push_str(" ORDER BY created_at DESC LIMIT ?");

        let mut qb = sqlx::query(&query);
        if let Some(ref caller) = filter.caller_agent {
            qb = qb.bind(caller);
        }
        if let Some(ref target) = filter.target_agent {
            qb = qb.bind(target);
        }
        if let Some(ref task_type) = filter.task_type {
            qb = qb.bind(task_type);
        }
        qb = qb.bind(filter.limit.unwrap_or(100) as i64);

        let rows = qb.fetch_all(&self.pool).await?;
        let mut outputs = Vec::with_capacity(rows.len());
        for row in rows {
            outputs.push(self.parse_sub_agent_output(row)?);
        }
        Ok(outputs)
    }

    async fn delete_expired_sub_agent_outputs(&self) -> Result<usize> {
        self.ensure_initialized().await?;

        let result = sqlx::query("DELETE FROM sub_agent_outputs WHERE expires_at IS NOT NULL AND expires_at <= datetime('now')")
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() as usize)
    }

    async fn create_routing_trace(&self, trace: &RoutingTrace) -> Result<()> {
        self.ensure_initialized().await?;

        let context_pressure = trace.context_pressure.map(|p| p.to_string());

        sqlx::query(
            "INSERT INTO routing_traces(id, session_id, task, selected_agent, reason, confidence, policy, alternatives_json, fallback_used, context_pressure, created_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(trace.id.to_string())
        .bind(trace.session_id.to_string())
        .bind(&trace.task)
        .bind(&trace.selected_agent)
        .bind(&trace.reason)
        .bind(trace.confidence as f64)
        .bind(&trace.policy)
        .bind(serde_json::to_string(&trace.alternatives)?)
        .bind(trace.fallback_used)
        .bind(context_pressure)
        .bind(trace.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_routing_traces(&self, filter: &RoutingTraceFilter) -> Result<Vec<RoutingTrace>> {
        self.ensure_initialized().await?;

        let mut query = String::from(
            "SELECT id, session_id, task, selected_agent, reason, confidence, policy, alternatives_json, fallback_used, context_pressure, created_at FROM routing_traces WHERE 1=1",
        );

        if filter.session_id.is_some() {
            query.push_str(" AND session_id = ?");
        }
        if filter.selected_agent.is_some() {
            query.push_str(" AND selected_agent = ?");
        }
        if filter.min_confidence.is_some() {
            query.push_str(" AND confidence >= ?");
        }
        if filter.fallback_only {
            query.push_str(" AND fallback_used = 1");
        }

        query.push_str(" ORDER BY created_at DESC LIMIT ?");

        let mut query_builder = sqlx::query(&query);
        if let Some(session_id) = filter.session_id {
            query_builder = query_builder.bind(session_id.to_string());
        }
        if let Some(ref selected_agent) = filter.selected_agent {
            query_builder = query_builder.bind(selected_agent);
        }
        if let Some(min_confidence) = filter.min_confidence {
            query_builder = query_builder.bind(min_confidence as f64);
        }
        query_builder = query_builder.bind(filter.limit.unwrap_or(100) as i64);

        let rows = query_builder.fetch_all(&self.pool).await?;
        let mut traces = Vec::with_capacity(rows.len());
        for row in rows {
            let id = row.get::<String, _>("id");
            let session_id_str = row.get::<String, _>("session_id");
            let context_pressure: Option<String> = row.get("context_pressure");
            let created_at = row.get::<String, _>("created_at");

            traces.push(RoutingTrace {
                id: Uuid::parse_str(&id)
                    .map_err(|err| Error::Storage(format!("invalid trace uuid '{id}': {err}")))?,
                session_id: Uuid::parse_str(&session_id_str).map_err(|err| {
                    Error::Storage(format!(
                        "invalid trace session uuid '{session_id_str}': {err}"
                    ))
                })?,
                task: row.get("task"),
                selected_agent: row.get("selected_agent"),
                reason: row.get("reason"),
                confidence: row.get::<f64, _>("confidence") as f32,
                policy: row.get("policy"),
                alternatives: serde_json::from_str(row.get("alternatives_json"))?,
                fallback_used: row.get::<i64, _>("fallback_used") != 0,
                context_pressure: context_pressure
                    .as_ref()
                    .and_then(|v| v.parse::<f32>().ok()),
                created_at: Self::parse_timestamp(&created_at)?,
            });
        }
        Ok(traces)
    }
}
