use crate::config::schema::{ToolConfig, WorkingDirMode};
use crate::error::{Error, Result};
use crate::events::Event;
use async_trait::async_trait;
use serde_json::json;
use std::path::Path;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone)]
pub struct ShellTool {
    config: ToolConfig,
    schema: serde_json::Value,
}

impl ShellTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                }
            },
            "required": ["command"]
        });

        Self { config, schema }
    }

    fn resolve_working_dir(&self, work_dir: &Path) -> Result<std::path::PathBuf> {
        let resolved = match &self.config.working_dir {
            WorkingDirMode::Current => work_dir.to_path_buf(),
            WorkingDirMode::ProjectRoot => work_dir.to_path_buf(),
            WorkingDirMode::CustomPath => {
                if let Some(ref custom_path) = self.config.custom_working_dir {
                    let path = Path::new(custom_path);
                    if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        work_dir.join(path)
                    }
                } else {
                    return Err(Error::Config(
                        "custom_working_dir is required when working_dir is 'custom_path'"
                            .to_owned(),
                    ));
                }
            }
        };

        // Ensure directory exists
        if !resolved.exists() {
            std::fs::create_dir_all(&resolved).map_err(|err| {
                Error::Tool(format!(
                    "failed to create working directory '{:?}': {err}",
                    resolved
                ))
            })?;
        }

        Ok(resolved)
    }

    fn validate_command(&self, command: &str) -> Result<()> {
        // Check denied commands first
        for denied in &self.config.denied_commands {
            if command.starts_with(denied) || command.contains(denied) {
                return Err(Error::Tool(format!(
                    "command denied by policy: contains '{denied}'"
                )));
            }
        }

        // If allowed_commands is not empty, check against it
        if !self.config.allowed_commands.is_empty() {
            let allowed = self
                .config
                .allowed_commands
                .iter()
                .any(|allowed| command.starts_with(allowed) || command == *allowed);
            if !allowed {
                return Err(Error::Tool(format!(
                    "command not in allowed list: '{command}'"
                )));
            }
        }

        Ok(())
    }

    async fn stream_command(
        &self,
        command: &str,
        work_dir: &Path,
        tx: mpsc::Sender<Event>,
        tool_name: &str,
    ) -> Result<(String, String, i32)> {
        let working_dir = self.resolve_working_dir(work_dir)?;

        let shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
        let shell_flag = if cfg!(windows) { "/C" } else { "-c" };

        let mut cmd = Command::new(shell);
        cmd.arg(shell_flag).arg(command).current_dir(&working_dir);

        if self.config.env_passthrough {
            for (key, value) in std::env::vars() {
                cmd.env(key, value);
            }
        }

        let mut child = cmd
            .spawn()
            .map_err(|err| Error::Tool(format!("failed to spawn command: {err}")))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Tool("failed to capture stdout".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::Tool("failed to capture stderr".to_owned()))?;

        let tx_stdout = tx.clone();
        let tool_name_stdout = tool_name.to_string();

        // Stream stdout
        let stdout_handle = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_stdout.try_send(Event::ToolOutput {
                    tool: tool_name_stdout.clone(),
                    stdout_chunk: format!("{}\n", line),
                    stderr_chunk: String::new(),
                });
            }
        });

        let tx_stderr = tx.clone();
        let tool_name_stderr = tool_name.to_string();

        // Stream stderr
        let stderr_handle = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_stderr.try_send(Event::ToolOutput {
                    tool: tool_name_stderr.clone(),
                    stdout_chunk: String::new(),
                    stderr_chunk: format!("{}\n", line),
                });
            }
        });

        // Wait for both stream tasks to complete
        let _ = tokio::try_join!(stdout_handle, stderr_handle);

        // Wait for process to finish with timeout
        let timeout_duration = Duration::from_secs(self.config.timeout_seconds);
        let result = timeout(timeout_duration, child.wait()).await;

        let status = match result {
            Ok(Ok(status)) => status,
            Ok(Err(err)) => {
                return Err(Error::Tool(format!("failed to wait for command: {err}")));
            }
            Err(_) => {
                // Timeout: kill process
                let _ = child.start_kill();
                return Err(Error::Tool(format!(
                    "command timed out after {} seconds",
                    self.config.timeout_seconds
                )));
            }
        };

        let exit_code = status.code().unwrap_or(-1);
        let _success = exit_code == 0;

        // Note: we've been streaming output, so we don't need to capture it again
        // For the batch result, we return empty strings since output was streamed
        Ok((String::new(), String::new(), exit_code))
    }
}

#[async_trait]
impl super::Tool for ShellTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Execute shell commands with streaming output and configurable permissions"
    }

    fn schema(&self) -> &serde_json::Value {
        &self.schema
    }

    async fn execute(&self, args: serde_json::Value) -> Result<super::ToolResult> {
        // For batch mode, we don't stream events
        let (dummy_tx, _) = mpsc::channel(1);
        self.stream_execute(args, dummy_tx).await
    }

    async fn stream_execute(
        &self,
        args: serde_json::Value,
        tx: mpsc::Sender<Event>,
    ) -> Result<super::ToolResult> {
        let tool_name = self.name().to_string();

        // Extract command from args
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("missing 'command' argument".to_owned()))?;

        // Validate command
        self.validate_command(command)?;

        let work_dir = std::env::current_dir()
            .map_err(|err| Error::Tool(format!("failed to get current directory: {err}")))?;

        // Emit tool started event
        let _ = tx.try_send(Event::ToolStarted {
            tool: tool_name.clone(),
            args: args.clone(),
        });

        // Execute and stream output - clone tx for use after stream_command
        let tx_for_completion = tx.clone();
        let (stdout, stderr, exit_code) = self
            .stream_command(command, &work_dir, tx, &tool_name)
            .await?;

        let success = exit_code == 0;

        // Emit tool completed event
        let _ = tx_for_completion.try_send(Event::ToolCompleted {
            tool: tool_name,
            exit_code,
        });

        Ok(super::ToolResult {
            success,
            exit_code: Some(exit_code),
            output: if success { stdout } else { stderr },
        })
    }
}
