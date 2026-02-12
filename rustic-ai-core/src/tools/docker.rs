use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::config::schema::{AgentPermissionMode, ToolConfig};
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct DockerTool {
    config: ToolConfig,
    schema: Value,
}

impl DockerTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["ps", "images", "logs", "inspect", "run", "stop", "rm", "pull"]
                },
                "args": { "type": "array", "items": { "type": "string" } },
                "engine": {
                    "type": "string",
                    "enum": ["auto", "docker", "podman"],
                    "description": "Container CLI engine"
                },
                "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 300 }
            },
            "required": ["action"]
        });

        Self { config, schema }
    }

    fn parse_action(args: &Value) -> Result<String> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::Tool("missing 'action' argument".to_owned()))?
            .to_ascii_lowercase();

        match action.as_str() {
            "ps" | "images" | "logs" | "inspect" | "run" | "stop" | "rm" | "pull" => Ok(action),
            _ => Err(Error::Tool(format!(
                "unsupported docker action '{}' (expected ps|images|logs|inspect|run|stop|rm|pull)",
                action
            ))),
        }
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

    fn parse_timeout(&self, args: &Value) -> u64 {
        args.get("timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(self.config.timeout_seconds)
            .clamp(1, 300)
    }

    fn parse_engine(args: &Value) -> Result<String> {
        let engine = args
            .get("engine")
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .trim()
            .to_ascii_lowercase();
        match engine.as_str() {
            "auto" | "docker" | "podman" => Ok(engine),
            _ => Err(Error::Tool(format!(
                "unsupported engine '{}' (expected auto|docker|podman)",
                engine
            ))),
        }
    }

    fn enforce_permission(action: &str, context: &ToolExecutionContext) -> Result<()> {
        let write_action = matches!(action, "run" | "stop" | "rm" | "pull");
        if write_action && context.agent_permission_mode == AgentPermissionMode::ReadOnly {
            return Err(Error::Tool(format!(
                "docker action '{}' is blocked in read-only agent mode",
                action
            )));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Tool for DockerTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Run bounded docker operations"
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
        let action_args = Self::parse_args(&args);
        let engine = Self::parse_engine(&args)?;
        let timeout_seconds = self.parse_timeout(&args);
        Self::enforce_permission(&action, context)?;

        let selected_engine = match engine.as_str() {
            "docker" => "docker",
            "podman" => "podman",
            _ => "docker",
        };

        let run_output = async |engine_binary: &str| -> Result<std::process::Output> {
            let mut command = Command::new(engine_binary);
            command.arg(&action);
            command.args(&action_args);
            command.current_dir(&context.working_directory);

            timeout(Duration::from_secs(timeout_seconds), command.output())
                .await
                .map_err(|_| {
                    Error::Timeout(format!(
                        "{engine_binary} command timed out after {timeout_seconds} seconds"
                    ))
                })?
                .map_err(|err| {
                    Error::Tool(format!("failed to execute {engine_binary} command: {err}"))
                })
        };

        let (output, used_engine) = match run_output(selected_engine).await {
            Ok(result) => (result, selected_engine),
            Err(err) if engine == "auto" => {
                let not_found = matches!(
                    &err,
                    Error::Tool(message)
                        if message.contains("failed to execute docker command")
                            && message.to_ascii_lowercase().contains("no such file")
                );
                if not_found {
                    (run_output("podman").await?, "podman")
                } else {
                    return Err(err);
                }
            }
            Err(err) => return Err(err),
        };

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if self.config.stream_output {
            if !stdout.is_empty() {
                let _ = tx.try_send(Event::ToolOutput {
                    tool: self.config.name.clone(),
                    stdout_chunk: stdout.clone(),
                    stderr_chunk: String::new(),
                });
            }
            if !stderr.is_empty() {
                let _ = tx.try_send(Event::ToolOutput {
                    tool: self.config.name.clone(),
                    stdout_chunk: String::new(),
                    stderr_chunk: stderr.clone(),
                });
            }
        }

        Ok(ToolResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            output: json!({
                "action": action,
                "args": action_args,
                "engine": used_engine,
                "success": output.status.success(),
                "stdout": stdout,
                "stderr": stderr,
            })
            .to_string(),
        })
    }
}
