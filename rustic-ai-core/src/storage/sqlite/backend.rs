use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::config::schema::SqliteStorageConfig;
use crate::error::{Error, Result};
use crate::learning::{
    FeedbackType, MistakePattern, MistakeType, PatternCategory, PreferenceValue, SuccessPattern,
    UserFeedback, UserPreference,
};
use crate::storage::model::{Message, PendingToolState, Session, SessionConfig};
use crate::storage::StorageBackend;

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

        Ok(())
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
}
