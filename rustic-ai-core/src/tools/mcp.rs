use crate::config::schema::{McpConfig, McpServerConfig, ToolConfig};
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

#[derive(Debug, Clone)]
pub struct McpToolAdapter {
    config: ToolConfig,
    schema: Value,
    mcp_config: Arc<McpConfig>,
}

impl McpToolAdapter {
    pub fn new(config: ToolConfig, mcp_config: Arc<McpConfig>) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["list_servers", "list_tools", "call_tool"]
                },
                "server": {
                    "type": "string",
                    "description": "Configured MCP server name"
                },
                "tool": {
                    "type": "string",
                    "description": "Remote MCP tool name"
                },
                "arguments": {
                    "type": "object",
                    "description": "Arguments object for remote MCP tool call"
                }
            },
            "required": ["operation"]
        });

        Self {
            config,
            schema,
            mcp_config,
        }
    }

    fn required_string<'a>(&self, args: &'a Value, key: &str) -> Result<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::Tool(format!("missing '{key}' argument")))
    }

    fn server_by_name(&self, name: &str) -> Result<McpServerConfig> {
        self.mcp_config
            .servers
            .iter()
            .find(|server| server.name == name)
            .cloned()
            .ok_or_else(|| Error::Tool(format!("mcp server '{name}' is not configured")))
    }

    fn resolve_path(raw: &str, work_dir: &Path) -> PathBuf {
        let path = Path::new(raw);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            work_dir.join(path)
        }
    }

    fn write_framed_message(writer: &mut dyn Write, message: &Value) -> Result<()> {
        let payload = serde_json::to_vec(message)
            .map_err(|err| Error::Tool(format!("failed to serialize mcp message: {err}")))?;
        let header = format!("Content-Length: {}\r\n\r\n", payload.len());
        writer
            .write_all(header.as_bytes())
            .map_err(|err| Error::Tool(format!("failed writing mcp header: {err}")))?;
        writer
            .write_all(&payload)
            .map_err(|err| Error::Tool(format!("failed writing mcp payload: {err}")))?;
        writer
            .flush()
            .map_err(|err| Error::Tool(format!("failed flushing mcp payload: {err}")))?;
        Ok(())
    }

    fn read_framed_message(reader: &mut BufReader<impl Read>) -> Result<Value> {
        let mut content_length: Option<usize> = None;

        loop {
            let mut line = String::new();
            let read = reader
                .read_line(&mut line)
                .map_err(|err| Error::Tool(format!("failed reading mcp header: {err}")))?;
            if read == 0 {
                return Err(Error::Tool(
                    "mcp server closed stream unexpectedly".to_owned(),
                ));
            }

            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }

            let Some((name, value)) = trimmed.split_once(':') else {
                continue;
            };
            if name.eq_ignore_ascii_case("content-length") {
                let parsed = value.trim().parse::<usize>().map_err(|err| {
                    Error::Tool(format!("invalid mcp Content-Length header: {err}"))
                })?;
                content_length = Some(parsed);
            }
        }

        let len = content_length
            .ok_or_else(|| Error::Tool("missing mcp Content-Length header".to_owned()))?;
        let mut body = vec![0u8; len];
        reader
            .read_exact(&mut body)
            .map_err(|err| Error::Tool(format!("failed reading mcp message body: {err}")))?;
        serde_json::from_slice::<Value>(&body)
            .map_err(|err| Error::Tool(format!("failed parsing mcp message json: {err}")))
    }

    fn read_response_for_id(
        reader: &mut BufReader<impl Read>,
        id: i64,
    ) -> Result<(Option<Value>, Option<Value>)> {
        loop {
            let message = Self::read_framed_message(reader)?;
            let message_id = message.get("id").and_then(Value::as_i64);
            if message_id != Some(id) {
                continue;
            }

            let result = message.get("result").cloned();
            let error = message.get("error").cloned();
            return Ok((result, error));
        }
    }

    fn spawn_server_process(
        server: &McpServerConfig,
        context: &ToolExecutionContext,
    ) -> Result<std::process::Child> {
        let mut command = StdCommand::new(&server.command);
        command.args(&server.args);

        let working_dir = if let Some(raw) = &server.working_directory {
            Self::resolve_path(raw, &context.working_directory)
        } else {
            context.working_directory.clone()
        };
        command.current_dir(&working_dir);

        if !server.env.is_empty() {
            command.envs(&server.env);
        }

        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                Error::Tool(format!(
                    "failed to spawn mcp server '{}' using '{}': {err}",
                    server.name, server.command
                ))
            })
    }

    fn initialize_and_open(
        server: &McpServerConfig,
        context: &ToolExecutionContext,
    ) -> Result<(
        std::process::Child,
        std::process::ChildStdin,
        BufReader<std::process::ChildStdout>,
    )> {
        let mut child = Self::spawn_server_process(server, context)?;
        let mut child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Tool("failed to capture mcp server stdin".to_owned()))?;
        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Tool("failed to capture mcp server stdout".to_owned()))?;
        let mut reader = BufReader::new(child_stdout);

        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": server.protocol_version,
                "capabilities": {},
                "clientInfo": {
                    "name": "rustic-ai",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        });
        Self::write_framed_message(&mut child_stdin, &initialize)?;

        let (_, init_error) = Self::read_response_for_id(&mut reader, 1)?;
        if let Some(err) = init_error {
            let _ = child.kill();
            return Err(Error::Tool(format!(
                "mcp initialize failed for server '{}': {err}",
                server.name
            )));
        }

        let initialized = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        Self::write_framed_message(&mut child_stdin, &initialized)?;

        Ok((child, child_stdin, reader))
    }

    fn list_tools_blocking(
        server: McpServerConfig,
        context: ToolExecutionContext,
    ) -> Result<Value> {
        let (mut child, mut child_stdin, mut reader) =
            Self::initialize_and_open(&server, &context)?;
        let request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });
        Self::write_framed_message(&mut child_stdin, &request)?;

        let (result, error) = Self::read_response_for_id(&mut reader, 2)?;
        let _ = child.kill();

        if let Some(err) = error {
            return Err(Error::Tool(format!(
                "mcp tools/list failed for server '{}': {err}",
                server.name
            )));
        }

        Ok(result.unwrap_or_else(|| json!({})))
    }

    fn call_tool_blocking(
        server: McpServerConfig,
        context: ToolExecutionContext,
        tool_name: String,
        arguments: Value,
    ) -> Result<Value> {
        let (mut child, mut child_stdin, mut reader) =
            Self::initialize_and_open(&server, &context)?;
        let request = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            }
        });
        Self::write_framed_message(&mut child_stdin, &request)?;

        let (result, error) = Self::read_response_for_id(&mut reader, 3)?;
        let _ = child.kill();

        if let Some(err) = error {
            return Err(Error::Tool(format!(
                "mcp tools/call failed for server '{}': {err}",
                server.name
            )));
        }

        Ok(result.unwrap_or_else(|| json!({})))
    }

    async fn list_tools_for_server(
        &self,
        server: McpServerConfig,
        context: &ToolExecutionContext,
    ) -> Result<Value> {
        let ctx = context.clone();
        let timeout_secs = server.startup_timeout_seconds.max(1);
        let handle = tokio::task::spawn_blocking(move || Self::list_tools_blocking(server, ctx));
        let joined = timeout(Duration::from_secs(timeout_secs), handle)
            .await
            .map_err(|_| Error::Tool("mcp tools/list timed out".to_owned()))?;
        joined.map_err(|err| Error::Tool(format!("mcp task join failed: {err}")))?
    }

    async fn call_tool_on_server(
        &self,
        server: McpServerConfig,
        context: &ToolExecutionContext,
        tool_name: String,
        arguments: Value,
    ) -> Result<Value> {
        let ctx = context.clone();
        let timeout_secs = server.startup_timeout_seconds.max(1);
        let handle = tokio::task::spawn_blocking(move || {
            Self::call_tool_blocking(server, ctx, tool_name, arguments)
        });
        let joined = timeout(Duration::from_secs(timeout_secs), handle)
            .await
            .map_err(|_| Error::Tool("mcp tools/call timed out".to_owned()))?;
        joined.map_err(|err| Error::Tool(format!("mcp task join failed: {err}")))?
    }

    fn server_summary(&self) -> Vec<Value> {
        self.mcp_config
            .servers
            .iter()
            .map(|server| {
                json!({
                    "name": server.name,
                    "command": server.command,
                    "args": server.args,
                    "startup_timeout_seconds": server.startup_timeout_seconds,
                    "protocol_version": server.protocol_version
                })
            })
            .collect()
    }
}

#[async_trait]
impl Tool for McpToolAdapter {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Call external MCP tools via configured stdio servers"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let (dummy_tx, _) = mpsc::channel(1);
        self.stream_execute(args, dummy_tx, context).await
    }

    async fn stream_execute(
        &self,
        args: Value,
        tx: mpsc::Sender<Event>,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let operation = self.required_string(&args, "operation")?.to_owned();

        let _ = tx.try_send(Event::ToolStarted {
            tool: self.config.name.clone(),
            args: args.clone(),
        });

        let result = match operation.as_str() {
            "list_servers" => {
                let payload = json!({
                    "servers": self.server_summary(),
                    "count": self.mcp_config.servers.len()
                });
                Ok(ToolResult {
                    success: true,
                    exit_code: Some(0),
                    output: payload.to_string(),
                })
            }
            "list_tools" => {
                let server_name = self.required_string(&args, "server")?.to_owned();
                let server = self.server_by_name(&server_name)?;
                let listed = self.list_tools_for_server(server, context).await?;
                Ok(ToolResult {
                    success: true,
                    exit_code: Some(0),
                    output: json!({"server": server_name, "result": listed}).to_string(),
                })
            }
            "call_tool" => {
                let server_name = self.required_string(&args, "server")?.to_owned();
                let tool_name = self.required_string(&args, "tool")?.to_owned();
                let arguments = args
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

                let server = self.server_by_name(&server_name)?;
                let called = self
                    .call_tool_on_server(server, context, tool_name.clone(), arguments)
                    .await?;
                Ok(ToolResult {
                    success: true,
                    exit_code: Some(0),
                    output: json!({
                        "server": server_name,
                        "tool": tool_name,
                        "result": called
                    })
                    .to_string(),
                })
            }
            other => Err(Error::Tool(format!("unsupported mcp operation '{other}'"))),
        };

        match result {
            Ok(tool_result) => {
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: self.config.name.clone(),
                    exit_code: tool_result.exit_code.unwrap_or_default(),
                });
                Ok(tool_result)
            }
            Err(err) => {
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: self.config.name.clone(),
                    exit_code: 1,
                });
                Err(err)
            }
        }
    }
}
