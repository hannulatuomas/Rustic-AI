use async_trait::async_trait;
use base64::Engine;
use futures::TryStreamExt;
use serde_json::{json, Map, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Column, Row};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

const MAX_QUERY_ROWS: usize = 10_000;

#[derive(Debug, Clone)]
pub struct DatabaseTool {
    config: ToolConfig,
    schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabaseCommand {
    Connect,
    Query,
    ListTables,
    DescribeTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabaseType {
    Sqlite,
    Postgres,
    Mysql,
}

impl DatabaseCommand {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "connect" => Ok(Self::Connect),
            "query" => Ok(Self::Query),
            "list_tables" => Ok(Self::ListTables),
            "describe_table" => Ok(Self::DescribeTable),
            other => Err(Error::Tool(format!(
                "unsupported database command '{other}' (expected connect|query|list_tables|describe_table)"
            ))),
        }
    }
}

impl DatabaseType {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sqlite" => Ok(Self::Sqlite),
            "postgres" | "postgresql" => Ok(Self::Postgres),
            "mysql" => Ok(Self::Mysql),
            other => Err(Error::Tool(format!(
                "unsupported database_type '{other}' (expected sqlite|postgres|mysql)"
            ))),
        }
    }
}

impl DatabaseTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "enum": ["connect", "query", "list_tables", "describe_table"]},
                "database_type": {"type": "string", "enum": ["sqlite", "postgres", "mysql"]},
                "connection_url": {"type": "string"},
                "sql": {"type": "string"},
                "table": {"type": "string"},
                "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 600},
                "max_rows": {"type": "integer", "minimum": 1, "maximum": 10000}
            },
            "required": ["command", "database_type", "connection_url"]
        });
        Self { config, schema }
    }

    fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::Tool(format!("missing '{key}' argument")))
    }

    fn timeout_seconds(&self, args: &Value) -> u64 {
        args.get("timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(self.config.timeout_seconds)
            .clamp(1, 600)
    }

    fn max_rows(args: &Value) -> usize {
        args.get("max_rows")
            .and_then(Value::as_u64)
            .unwrap_or(200)
            .clamp(1, MAX_QUERY_ROWS as u64) as usize
    }

    fn split_table_name(table: &str) -> (String, String) {
        let trimmed = table.trim();
        if let Some((schema, name)) = trimmed.split_once('.') {
            (schema.trim().to_owned(), name.trim().to_owned())
        } else {
            ("public".to_owned(), trimmed.to_owned())
        }
    }

    fn validate_identifier(value: &str) -> Result<()> {
        if value.is_empty() {
            return Err(Error::Tool(
                "table/schema identifier cannot be empty".to_owned(),
            ));
        }
        if value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Ok(());
        }
        Err(Error::Tool(format!(
            "identifier '{value}' must use only [A-Za-z0-9_] characters"
        )))
    }

    fn decode_sqlite_cell(row: &sqlx::sqlite::SqliteRow, index: usize) -> Value {
        if let Ok(value) = row.try_get::<Option<String>, _>(index) {
            return value.map(Value::String).unwrap_or(Value::Null);
        }
        if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
            return value.map_or(Value::Null, |v| json!(v));
        }
        if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
            return value.map_or(Value::Null, |v| json!(v));
        }
        if let Ok(value) = row.try_get::<Option<bool>, _>(index) {
            return value.map_or(Value::Null, |v| json!(v));
        }
        if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
            return value.map_or(Value::Null, |v| {
                Value::String(base64::engine::general_purpose::STANDARD.encode(v))
            });
        }
        Value::String("<unrenderable>".to_owned())
    }

    fn decode_postgres_cell(row: &sqlx::postgres::PgRow, index: usize) -> Value {
        if let Ok(value) = row.try_get::<Option<String>, _>(index) {
            return value.map(Value::String).unwrap_or(Value::Null);
        }
        if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
            return value.map_or(Value::Null, |v| json!(v));
        }
        if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
            return value.map_or(Value::Null, |v| json!(v));
        }
        if let Ok(value) = row.try_get::<Option<bool>, _>(index) {
            return value.map_or(Value::Null, |v| json!(v));
        }
        if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
            return value.map_or(Value::Null, |v| {
                Value::String(base64::engine::general_purpose::STANDARD.encode(v))
            });
        }
        Value::String("<unrenderable>".to_owned())
    }

    fn sqlite_row_to_json(row: &sqlx::sqlite::SqliteRow) -> Value {
        let mut map = Map::new();
        for (index, column) in row.columns().iter().enumerate() {
            map.insert(
                column.name().to_owned(),
                Self::decode_sqlite_cell(row, index),
            );
        }
        Value::Object(map)
    }

    fn postgres_row_to_json(row: &sqlx::postgres::PgRow) -> Value {
        let mut map = Map::new();
        for (index, column) in row.columns().iter().enumerate() {
            map.insert(
                column.name().to_owned(),
                Self::decode_postgres_cell(row, index),
            );
        }
        Value::Object(map)
    }

    async fn run_sqlite(
        &self,
        command: DatabaseCommand,
        connection_url: &str,
        args: &Value,
        tx: Option<mpsc::Sender<Event>>,
    ) -> Result<Value> {
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect(connection_url)
            .await
            .map_err(|err| Error::Tool(format!("failed to connect sqlite database: {err}")))?;

        let result = match command {
            DatabaseCommand::Connect => {
                let row: (i64,) = sqlx::query_as("SELECT 1")
                    .fetch_one(&pool)
                    .await
                    .map_err(|err| Error::Tool(format!("sqlite ping failed: {err}")))?;
                json!({
                    "command": "connect",
                    "database_type": "sqlite",
                    "ok": row.0 == 1
                })
            }
            DatabaseCommand::ListTables => {
                let rows = sqlx::query(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )
                .fetch_all(&pool)
                .await
                .map_err(|err| Error::Tool(format!("sqlite list_tables failed: {err}")))?;
                let tables = rows
                    .iter()
                    .filter_map(|row| row.try_get::<String, _>("name").ok())
                    .collect::<Vec<_>>();
                json!({
                    "command": "list_tables",
                    "database_type": "sqlite",
                    "count": tables.len(),
                    "tables": tables
                })
            }
            DatabaseCommand::DescribeTable => {
                let table = Self::required_string(args, "table")?;
                Self::validate_identifier(table)?;
                let pragma = format!("PRAGMA table_info({table})");
                let rows = sqlx::query(&pragma)
                    .fetch_all(&pool)
                    .await
                    .map_err(|err| Error::Tool(format!("sqlite describe_table failed: {err}")))?;
                let columns = rows
                    .iter()
                    .map(Self::sqlite_row_to_json)
                    .collect::<Vec<_>>();
                json!({
                    "command": "describe_table",
                    "database_type": "sqlite",
                    "table": table,
                    "columns": columns
                })
            }
            DatabaseCommand::Query => {
                let sql = Self::required_string(args, "sql")?;
                let max_rows = Self::max_rows(args);
                let mut stream = sqlx::query(sql).fetch(&pool);
                let mut rows = Vec::new();
                while let Some(row) = stream
                    .try_next()
                    .await
                    .map_err(|err| Error::Tool(format!("sqlite query execution failed: {err}")))?
                {
                    rows.push(Self::sqlite_row_to_json(&row));
                    if rows.len() >= max_rows {
                        break;
                    }
                    if let Some(tx) = tx.as_ref() {
                        let _ = tx.try_send(Event::ToolOutput {
                            tool: self.config.name.clone(),
                            stdout_chunk: format!("streamed {} rows\n", rows.len()),
                            stderr_chunk: String::new(),
                        });
                    }
                }
                json!({
                    "command": "query",
                    "database_type": "sqlite",
                    "row_count": rows.len(),
                    "rows": rows
                })
            }
        };

        pool.close().await;
        Ok(result)
    }

    async fn run_postgres(
        &self,
        command: DatabaseCommand,
        connection_url: &str,
        args: &Value,
        tx: Option<mpsc::Sender<Event>>,
    ) -> Result<Value> {
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(connection_url)
            .await
            .map_err(|err| Error::Tool(format!("failed to connect postgres database: {err}")))?;

        let result = match command {
            DatabaseCommand::Connect => {
                let row: (i64,) = sqlx::query_as("SELECT 1")
                    .fetch_one(&pool)
                    .await
                    .map_err(|err| Error::Tool(format!("postgres ping failed: {err}")))?;
                json!({
                    "command": "connect",
                    "database_type": "postgres",
                    "ok": row.0 == 1
                })
            }
            DatabaseCommand::ListTables => {
                let rows = sqlx::query(
                    "SELECT table_schema, table_name FROM information_schema.tables WHERE table_type = 'BASE TABLE' AND table_schema NOT IN ('pg_catalog', 'information_schema') ORDER BY table_schema, table_name",
                )
                .fetch_all(&pool)
                .await
                .map_err(|err| Error::Tool(format!("postgres list_tables failed: {err}")))?;
                let tables = rows
                    .iter()
                    .filter_map(|row| {
                        let schema = row.try_get::<String, _>("table_schema").ok()?;
                        let table = row.try_get::<String, _>("table_name").ok()?;
                        Some(format!("{schema}.{table}"))
                    })
                    .collect::<Vec<_>>();
                json!({
                    "command": "list_tables",
                    "database_type": "postgres",
                    "count": tables.len(),
                    "tables": tables
                })
            }
            DatabaseCommand::DescribeTable => {
                let table = Self::required_string(args, "table")?;
                let (schema_name, table_name) = Self::split_table_name(table);
                Self::validate_identifier(&schema_name)?;
                Self::validate_identifier(&table_name)?;
                let rows = sqlx::query(
                    "SELECT column_name, data_type, is_nullable, column_default FROM information_schema.columns WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
                )
                .bind(&schema_name)
                .bind(&table_name)
                .fetch_all(&pool)
                .await
                .map_err(|err| Error::Tool(format!("postgres describe_table failed: {err}")))?;
                let columns = rows
                    .iter()
                    .map(Self::postgres_row_to_json)
                    .collect::<Vec<_>>();
                json!({
                    "command": "describe_table",
                    "database_type": "postgres",
                    "table": format!("{schema_name}.{table_name}"),
                    "columns": columns
                })
            }
            DatabaseCommand::Query => {
                let sql = Self::required_string(args, "sql")?;
                let max_rows = Self::max_rows(args);
                let mut stream = sqlx::query(sql).fetch(&pool);
                let mut rows = Vec::new();
                while let Some(row) = stream
                    .try_next()
                    .await
                    .map_err(|err| Error::Tool(format!("postgres query execution failed: {err}")))?
                {
                    rows.push(Self::postgres_row_to_json(&row));
                    if rows.len() >= max_rows {
                        break;
                    }
                    if let Some(tx) = tx.as_ref() {
                        let _ = tx.try_send(Event::ToolOutput {
                            tool: self.config.name.clone(),
                            stdout_chunk: format!("streamed {} rows\n", rows.len()),
                            stderr_chunk: String::new(),
                        });
                    }
                }
                json!({
                    "command": "query",
                    "database_type": "postgres",
                    "row_count": rows.len(),
                    "rows": rows
                })
            }
        };

        pool.close().await;
        Ok(result)
    }

    async fn run_with_controls<F>(
        &self,
        timeout_seconds: u64,
        cancellation_token: Option<tokio_util::sync::CancellationToken>,
        operation: F,
    ) -> Result<Value>
    where
        F: std::future::Future<Output = Result<Value>>,
    {
        if let Some(token) = cancellation_token {
            tokio::select! {
                _ = token.cancelled() => Err(Error::Timeout("database operation cancelled".to_owned())),
                result = timeout(Duration::from_secs(timeout_seconds), operation) => {
                    match result {
                        Ok(inner) => inner,
                        Err(_) => Err(Error::Timeout(format!("database operation timed out after {timeout_seconds} seconds"))),
                    }
                }
            }
        } else {
            match timeout(Duration::from_secs(timeout_seconds), operation).await {
                Ok(inner) => inner,
                Err(_) => Err(Error::Timeout(format!(
                    "database operation timed out after {timeout_seconds} seconds"
                ))),
            }
        }
    }

    async fn execute_operation(
        &self,
        args: Value,
        context: &ToolExecutionContext,
        tx: Option<mpsc::Sender<Event>>,
    ) -> Result<ToolResult> {
        let command = DatabaseCommand::parse(Self::required_string(&args, "command")?)?;
        let database_type = DatabaseType::parse(Self::required_string(&args, "database_type")?)?;
        let connection_url = Self::required_string(&args, "connection_url")?.to_owned();
        let timeout_seconds = self.timeout_seconds(&args);

        let payload = match database_type {
            DatabaseType::Sqlite => {
                self.run_with_controls(
                    timeout_seconds,
                    context.cancellation_token.clone(),
                    self.run_sqlite(command, &connection_url, &args, tx),
                )
                .await?
            }
            DatabaseType::Postgres => {
                self.run_with_controls(
                    timeout_seconds,
                    context.cancellation_token.clone(),
                    self.run_postgres(command, &connection_url, &args, tx),
                )
                .await?
            }
            DatabaseType::Mysql => {
                return Err(Error::Tool(
                    "database_type 'mysql' is not enabled in this build; enable MySQL sqlx feature to support it"
                        .to_owned(),
                ));
            }
        };

        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: payload.to_string(),
        })
    }
}

#[async_trait]
impl Tool for DatabaseTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Database operations for sqlite/postgres with timeouts and streaming"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        self.execute_operation(args, context, None).await
    }

    async fn stream_execute(
        &self,
        args: Value,
        tx: mpsc::Sender<Event>,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let tool_name = self.name().to_owned();
        let _ = tx.try_send(Event::ToolStarted {
            tool: tool_name.clone(),
            args: args.clone(),
        });

        let result = self
            .execute_operation(args, context, Some(tx.clone()))
            .await;

        match &result {
            Ok(tool_result) => {
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: tool_name,
                    exit_code: tool_result.exit_code.unwrap_or(0),
                });
            }
            Err(err) => {
                let _ = tx.try_send(Event::Error(format!("database tool failed: {err}")));
            }
        }

        result
    }
}
