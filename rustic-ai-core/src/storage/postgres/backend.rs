use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::storage::model::{Message, PendingToolState, Session, SessionConfig};
use crate::storage::StorageBackend;

const SCHEMA_V1: [&str; 12] = [
    "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL PRIMARY KEY)",
    "INSERT INTO schema_version(version) VALUES (0) ON CONFLICT (version) DO NOTHING",
    "CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, agent_name TEXT NOT NULL, created_at TEXT NOT NULL, config_json TEXT)",
    "CREATE TABLE IF NOT EXISTS messages (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, created_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS context_files (path TEXT PRIMARY KEY, content TEXT NOT NULL, metadata TEXT, loaded_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS session_topics (session_id TEXT PRIMARY KEY, topics_json TEXT NOT NULL, updated_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS manual_rule_invocations (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, rule_path TEXT NOT NULL, invoked_at TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS idx_messages_session_created_at ON messages(session_id, created_at)",
    "CREATE INDEX IF NOT EXISTS idx_context_files_loaded_at ON context_files(loaded_at)",
    "CREATE INDEX IF NOT EXISTS idx_manual_rule_invocations_session_id ON manual_rule_invocations(session_id)",
    "CREATE INDEX IF NOT EXISTS idx_manual_rule_invocations_rule_path ON manual_rule_invocations(rule_path)",
    "UPDATE schema_version SET version = 1",
];

const SCHEMA_V2_MIGRATION: [&str; 2] = [
    "CREATE TABLE IF NOT EXISTS pending_tools (session_id TEXT PRIMARY KEY, tool_name TEXT NOT NULL, args_json TEXT NOT NULL, round_index INTEGER NOT NULL, tool_messages_json TEXT NOT NULL, context_snapshot_json TEXT NOT NULL, created_at TEXT NOT NULL)",
    "UPDATE schema_version SET version = 2",
];

#[derive(Debug, Clone)]
pub struct PostgresStorage {
    pool: sqlx::PgPool,
    initialized: std::sync::Arc<OnceCell<()>>,
    schema_name: Option<String>,
}

impl PostgresStorage {
    pub fn new(
        connection_url: &str,
        pool_size: usize,
        schema_name: Option<String>,
    ) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(pool_size as u32)
            .connect_lazy(connection_url)
            .map_err(|err| Error::Storage(format!("failed to create Postgres pool: {err}")))?;

        Ok(Self {
            pool,
            initialized: std::sync::Arc::new(OnceCell::new()),
            schema_name,
        })
    }

    async fn ensure_initialized(&self) -> Result<()> {
        self.initialized
            .get_or_try_init(|| async {
                if let Some(schema_name) = &self.schema_name {
                    let schema_name = schema_name.trim();
                    if !schema_name.is_empty() {
                        let create_schema =
                            format!("CREATE SCHEMA IF NOT EXISTS \"{}\"", schema_name);
                        sqlx::query(&create_schema).execute(&self.pool).await?;
                        let set_search_path =
                            format!("SET search_path TO \"{}\", public", schema_name);
                        sqlx::query(&set_search_path).execute(&self.pool).await?;
                    }
                }

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

                Ok::<(), sqlx::Error>(())
            })
            .await
            .map_err(Error::from)
            .map(|_| ())
    }

    fn parse_timestamp(value: &str) -> Result<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(value)
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .map_err(|err| Error::Storage(format!("failed to parse timestamp '{value}': {err}")))
    }
}

#[async_trait]
impl StorageBackend for PostgresStorage {
    async fn get_schema_version(&self) -> Result<Option<u32>> {
        self.ensure_initialized().await?;
        let row = sqlx::query("SELECT version FROM schema_version LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|value| value.get::<i64, _>("version") as u32))
    }

    async fn set_schema_version(&self, version: u32) -> Result<()> {
        self.ensure_initialized().await?;
        sqlx::query("UPDATE schema_version SET version = $1")
            .bind(version as i64)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn create_session(&self, session: Session) -> Result<()> {
        self.ensure_initialized().await?;
        sqlx::query(
            "INSERT INTO sessions(id, agent_name, created_at, config_json) VALUES($1, $2, $3, NULL)",
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
        let row = sqlx::query("SELECT id, agent_name, created_at FROM sessions WHERE id = $1")
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

        let rows = if let Some(limit) = limit {
            sqlx::query(
                "SELECT id, agent_name, created_at FROM sessions ORDER BY created_at DESC LIMIT $1",
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query("SELECT id, agent_name, created_at FROM sessions ORDER BY created_at DESC")
                .fetch_all(&self.pool)
                .await?
        };

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
        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn append_message(&self, message: Message) -> Result<()> {
        self.ensure_initialized().await?;
        sqlx::query(
            "INSERT INTO messages(id, session_id, role, content, created_at) VALUES($1, $2, $3, $4, $5)",
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
            "SELECT id, session_id, role, content, created_at FROM messages WHERE session_id = $1 ORDER BY created_at ASC",
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
            "SELECT id, session_id, role, content, created_at FROM messages WHERE session_id = $1 ORDER BY created_at DESC LIMIT $2",
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
        let row = sqlx::query("SELECT config_json FROM sessions WHERE id = $1")
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
        sqlx::query("UPDATE sessions SET config_json = $1 WHERE id = $2")
            .bind(payload)
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_or_load_context_file(&self, path: &str) -> Result<Option<String>> {
        self.ensure_initialized().await?;
        let row = sqlx::query("SELECT content FROM context_files WHERE path = $1")
            .bind(path)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|value| value.get::<String, _>("content")))
    }

    async fn cache_context_file(&self, path: &str, content: &str, metadata: &str) -> Result<()> {
        self.ensure_initialized().await?;
        sqlx::query(
            "INSERT INTO context_files(path, content, metadata, loaded_at) VALUES($1, $2, $3, $4) ON CONFLICT(path) DO UPDATE SET content = excluded.content, metadata = excluded.metadata, loaded_at = excluded.loaded_at",
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
            "INSERT INTO session_topics(session_id, topics_json, updated_at) VALUES($1, $2, $3) ON CONFLICT(session_id) DO UPDATE SET topics_json = excluded.topics_json, updated_at = excluded.updated_at",
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
        let row = sqlx::query("SELECT topics_json FROM session_topics WHERE session_id = $1")
            .bind(session_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let payload = row.get::<String, _>("topics_json");
        Ok(Some(serde_json::from_str(&payload)?))
    }

    async fn track_manual_invocation(&self, session_id: Uuid, rule_path: &str) -> Result<()> {
        self.ensure_initialized().await?;
        sqlx::query(
            "INSERT INTO manual_rule_invocations(id, session_id, rule_path, invoked_at) VALUES($1, $2, $3, $4)",
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
            "SELECT rule_path FROM manual_rule_invocations WHERE session_id = $1 ORDER BY invoked_at ASC",
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
            "INSERT INTO pending_tools(session_id, tool_name, args_json, round_index, tool_messages_json, context_snapshot_json, created_at) VALUES($1, $2, $3, $4, $5, $6, $7) ON CONFLICT(session_id) DO UPDATE SET tool_name = excluded.tool_name, args_json = excluded.args_json, round_index = excluded.round_index, tool_messages_json = excluded.tool_messages_json, context_snapshot_json = excluded.context_snapshot_json, created_at = excluded.created_at",
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
            "SELECT session_id, tool_name, args_json, round_index, tool_messages_json, context_snapshot_json, created_at FROM pending_tools WHERE session_id = $1",
        )
        .bind(session_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        sqlx::query("DELETE FROM pending_tools WHERE session_id = $1")
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
        let result = sqlx::query("DELETE FROM pending_tools WHERE created_at < $1")
            .bind(cutoff_rfc3339)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() as usize)
    }

    async fn has_pending_tool(&self, session_id: Uuid) -> Result<bool> {
        self.ensure_initialized().await?;
        let row = sqlx::query("SELECT 1 FROM pending_tools WHERE session_id = $1")
            .bind(session_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }
}
