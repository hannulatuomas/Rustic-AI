use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::LazyLock;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::time::{timeout, Duration};

use crate::config::schema::{AgentPermissionMode, ToolConfig};
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

static PROCESS_TABLE: LazyLock<Mutex<HashMap<u32, Child>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
pub struct ProcessTool {
    config: ToolConfig,
    schema: Value,
}

impl ProcessTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["start", "status", "stop", "list"] },
                "command": { "type": "string" },
                "args": { "type": "array", "items": { "type": "string" } },
                "working_dir": { "type": "string" },
                "pid": { "type": "integer", "minimum": 1 },
                "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 120 }
            },
            "required": ["action"]
        });

        Self { config, schema }
    }

    fn parse_action(args: &Value) -> Result<String> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool("missing 'action' argument".to_owned()))?
            .trim()
            .to_ascii_lowercase();
        match action.as_str() {
            "start" | "status" | "stop" | "list" => Ok(action),
            other => Err(Error::Tool(format!(
                "unsupported process action '{other}' (expected start|status|stop|list)"
            ))),
        }
    }

    fn parse_timeout(&self, args: &Value) -> u64 {
        args.get("timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(self.config.timeout_seconds)
            .clamp(1, 120)
    }

    fn parse_pid(args: &Value) -> Result<u32> {
        let pid = args
            .get("pid")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::Tool("missing 'pid' argument".to_owned()))?;
        u32::try_from(pid).map_err(|_| Error::Tool("pid is out of range".to_owned()))
    }

    fn parse_command(args: &Value) -> Result<String> {
        args.get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(|| Error::Tool("missing 'command' argument".to_owned()))
    }

    fn parse_args(args: &Value) -> Vec<String> {
        args.get("args")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn enforce_write_permissions(action: &str, context: &ToolExecutionContext) -> Result<()> {
        if matches!(action, "start" | "stop")
            && context.agent_permission_mode == AgentPermissionMode::ReadOnly
        {
            return Err(Error::Tool(format!(
                "process action '{action}' is blocked in read-only agent mode"
            )));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Tool for ProcessTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Start, inspect, and stop background processes"
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
        let action = Self::parse_action(&args)?;
        Self::enforce_write_permissions(action.as_str(), context)?;

        match action.as_str() {
            "start" => {
                let command = Self::parse_command(&args)?;
                let command_args = Self::parse_args(&args);
                let mut cmd = Command::new(&command);
                cmd.args(&command_args);
                cmd.stdin(Stdio::null());
                cmd.stdout(Stdio::null());
                cmd.stderr(Stdio::null());

                if let Some(custom_dir) = args.get("working_dir").and_then(Value::as_str) {
                    if !custom_dir.trim().is_empty() {
                        cmd.current_dir(custom_dir);
                    }
                } else {
                    cmd.current_dir(&context.working_directory);
                }

                let child = cmd.spawn().map_err(|err| {
                    Error::Tool(format!("failed to start process '{command}': {err}"))
                })?;
                let pid = child
                    .id()
                    .ok_or_else(|| Error::Tool("started process has no pid".to_owned()))?;
                PROCESS_TABLE.lock().await.insert(pid, child);

                let _ = tx.try_send(Event::ToolOutput {
                    tool: self.config.name.clone(),
                    stdout_chunk: format!("started process pid={pid}\n"),
                    stderr_chunk: String::new(),
                });

                Ok(ToolResult {
                    success: true,
                    exit_code: Some(0),
                    output: json!({
                        "action": "start",
                        "pid": pid,
                        "command": command,
                        "args": command_args,
                    })
                    .to_string(),
                })
            }
            "status" => {
                let pid = Self::parse_pid(&args)?;
                let mut table = PROCESS_TABLE.lock().await;
                let Some(child) = table.get_mut(&pid) else {
                    return Ok(ToolResult {
                        success: false,
                        exit_code: Some(1),
                        output: json!({"action":"status","pid":pid,"running":false,"known":false})
                            .to_string(),
                    });
                };

                let status = child
                    .try_wait()
                    .map_err(|err| Error::Tool(format!("failed to query process status: {err}")))?;
                let running = status.is_none();
                let exit_code = status.and_then(|value| value.code());
                if !running {
                    table.remove(&pid);
                }

                Ok(ToolResult {
                    success: true,
                    exit_code: Some(0),
                    output: json!({
                        "action": "status",
                        "pid": pid,
                        "running": running,
                        "known": true,
                        "exit_code": exit_code,
                    })
                    .to_string(),
                })
            }
            "stop" => {
                let pid = Self::parse_pid(&args)?;
                let timeout_seconds = self.parse_timeout(&args);

                let mut table = PROCESS_TABLE.lock().await;
                let Some(mut child) = table.remove(&pid) else {
                    return Ok(ToolResult {
                        success: false,
                        exit_code: Some(1),
                        output: json!({"action":"stop","pid":pid,"stopped":false,"known":false})
                            .to_string(),
                    });
                };

                child
                    .kill()
                    .await
                    .map_err(|err| Error::Tool(format!("failed to stop process {pid}: {err}")))?;

                let _ = timeout(Duration::from_secs(timeout_seconds), child.wait())
                    .await
                    .map_err(|_| {
                        Error::Timeout(format!("waiting for process {pid} exceeded timeout"))
                    })
                    .and_then(|result| {
                        result.map_err(|err| {
                            Error::Tool(format!("failed while waiting for process {pid}: {err}"))
                        })
                    })?;

                Ok(ToolResult {
                    success: true,
                    exit_code: Some(0),
                    output: json!({"action":"stop","pid":pid,"stopped":true,"known":true})
                        .to_string(),
                })
            }
            "list" => {
                let mut table = PROCESS_TABLE.lock().await;
                let mut entries = Vec::new();
                let mut finished = Vec::new();
                for (pid, child) in table.iter_mut() {
                    let status = child.try_wait().map_err(|err| {
                        Error::Tool(format!("failed to query process status: {err}"))
                    })?;
                    let running = status.is_none();
                    let exit_code = status.and_then(|value| value.code());
                    if !running {
                        finished.push(*pid);
                    }
                    entries.push(json!({
                        "pid": pid,
                        "running": running,
                        "exit_code": exit_code,
                    }));
                }
                for pid in finished {
                    table.remove(&pid);
                }

                Ok(ToolResult {
                    success: true,
                    exit_code: Some(0),
                    output: json!({"action":"list","processes":entries}).to_string(),
                })
            }
            _ => Err(Error::Tool("unsupported process action".to_owned())),
        }
    }
}
