use crate::config::schema::{ToolConfig, WorkingDirMode};
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::ToolExecutionContext;
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;

const SAFE_PASSTHROUGH_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "USER", "SHELL", "TMP", "TMPDIR", "TEMP", "LANG", "LC_ALL",
];
const OUTPUT_CAPTURE_LIMIT_BYTES: usize = 10 * 1024;

#[derive(Debug, Clone)]
struct SudoPasswordEntry {
    password: String,
    expires_at: SystemTime,
}

#[derive(Debug, Clone)]
pub struct ShellTool {
    config: ToolConfig,
    schema: serde_json::Value,
    sudo_cache_ttl_secs: u64,
    sudo_password_cache: Arc<Mutex<HashMap<String, SudoPasswordEntry>>>,
}

impl ShellTool {
    pub fn new(config: ToolConfig, sudo_cache_ttl_secs: u64) -> Self {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                }
            },
            "required": ["command"]
        });

        if config.enabled {
            schema["properties"]["working_directory"] = json!({
                "type": "string",
                "description": "Working directory for command execution (overrides config)"
            });
        }

        Self {
            config,
            schema,
            sudo_cache_ttl_secs,
            sudo_password_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn resolve_working_dir(
        &self,
        work_dir: &Path,
        per_call_override: Option<&Path>,
    ) -> Result<PathBuf> {
        let resolved = if let Some(override_path) = per_call_override {
            if override_path.is_absolute() {
                override_path.to_path_buf()
            } else {
                work_dir.join(override_path)
            }
        } else {
            match &self.config.working_dir {
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
            }
        };

        if !resolved.exists() {
            return Err(Error::Tool(format!(
                "working directory '{}' does not exist",
                resolved.display()
            )));
        }

        let metadata = std::fs::metadata(&resolved).map_err(|err| {
            Error::Tool(format!(
                "failed to read metadata for working directory '{}': {err}",
                resolved.display()
            ))
        })?;
        if !metadata.is_dir() {
            return Err(Error::Tool(format!(
                "working directory '{}' is not a directory",
                resolved.display()
            )));
        }

        std::fs::canonicalize(&resolved).map_err(|err| {
            Error::Tool(format!(
                "failed to canonicalize working directory '{}': {err}",
                resolved.display()
            ))
        })
    }

    fn command_program(command: &str) -> Option<String> {
        command
            .split_whitespace()
            .next()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn command_matches_policy_entry(program: &str, policy_entry: &str) -> bool {
        if program == policy_entry {
            return true;
        }

        let program_name = std::path::Path::new(program)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(program);
        let entry_name = std::path::Path::new(policy_entry)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(policy_entry);
        program_name == entry_name
    }

    fn validate_command(&self, command: &str) -> Result<()> {
        let program = Self::command_program(command)
            .ok_or_else(|| Error::Tool("command cannot be empty".to_owned()))?;

        for denied in &self.config.denied_commands {
            if Self::command_matches_policy_entry(&program, denied) {
                return Err(Error::Tool(format!(
                    "command denied by policy: '{program}'"
                )));
            }
        }

        if !self.config.allowed_commands.is_empty() {
            let allowed = self
                .config
                .allowed_commands
                .iter()
                .any(|allowed| Self::command_matches_policy_entry(&program, allowed));
            if !allowed {
                return Err(Error::Tool(format!(
                    "command not in allowed list: '{program}'"
                )));
            }
        }

        Ok(())
    }

    fn command_requires_sudo(&self, command: &str) -> bool {
        if self.config.require_sudo {
            return true;
        }

        if let Some(program) = Self::command_program(command) {
            for pattern in &self.config.privileged_command_patterns {
                if let Some(pattern_program) = Self::command_program(pattern) {
                    if Self::command_matches_policy_entry(&program, &pattern_program) {
                        return true;
                    }
                }
            }
        }

        let lower_command = command.to_ascii_lowercase();
        lower_command.starts_with("sudo ") || lower_command.contains(" sudo ")
    }

    async fn cleanup_expired_sudo_passwords(&self) {
        let now = SystemTime::now();
        let mut cache = self.sudo_password_cache.lock().await;
        cache.retain(|_, entry| {
            let keep = entry.expires_at > now;
            if !keep {
                let mut password = entry.password.clone();
                password.clear();
            }
            keep
        });
    }

    fn apply_environment(&self, cmd: &mut Command) {
        if self.config.env_passthrough {
            return;
        }

        cmd.env_clear();
        for key in SAFE_PASSTHROUGH_ENV_VARS {
            if let Ok(value) = std::env::var(key) {
                cmd.env(key, value);
            }
        }
    }

    fn append_bounded(buffer: &mut String, chunk: &str) {
        let remaining = OUTPUT_CAPTURE_LIMIT_BYTES.saturating_sub(buffer.len());
        if remaining == 0 {
            return;
        }
        if chunk.len() <= remaining {
            buffer.push_str(chunk);
            return;
        }
        let mut consumed = 0usize;
        for ch in chunk.chars() {
            let width = ch.len_utf8();
            if consumed + width > remaining {
                break;
            }
            buffer.push(ch);
            consumed += width;
        }
    }

    async fn stream_command(
        &self,
        command: &str,
        work_dir: &Path,
        per_call_override: Option<&Path>,
        tx: mpsc::Sender<Event>,
        tool_name: &str,
    ) -> Result<(String, String, i32)> {
        self.cleanup_expired_sudo_passwords().await;
        let working_dir = self.resolve_working_dir(work_dir, per_call_override)?;

        if self.command_requires_sudo(command) {
            let ttl_secs = self.sudo_cache_ttl_secs;
            let _ = tx.try_send(Event::SudoSecretPrompt {
                session_id: "unknown".to_owned(),
                command: command.to_owned(),
                reason: "sudo command detected".to_owned(),
            });
            return Err(Error::Tool(format!(
                "sudo command execution is not wired yet; prompt event emitted (configured cache ttl: {ttl_secs}s)"
            )));
        }

        let shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
        let shell_flag = if cfg!(windows) { "/C" } else { "-c" };

        let mut cmd = Command::new(shell);
        cmd.arg(shell_flag).arg(command);
        cmd.current_dir(&working_dir);
        self.apply_environment(&mut cmd);

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

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

        let tool_name_stdout = tool_name.to_owned();
        let tx_stdout = tx.clone();
        let stdout_task = tokio::spawn(async move {
            let mut captured = String::new();
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let chunk = format!("{line}\n");
                let _ = tx_stdout.try_send(Event::ToolOutput {
                    tool: tool_name_stdout.clone(),
                    stdout_chunk: chunk.clone(),
                    stderr_chunk: String::new(),
                });
                Self::append_bounded(&mut captured, &chunk);
            }
            captured
        });

        let tool_name_stderr = tool_name.to_owned();
        let tx_stderr = tx.clone();
        let stderr_task = tokio::spawn(async move {
            let mut captured = String::new();
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let chunk = format!("{line}\n");
                let _ = tx_stderr.try_send(Event::ToolOutput {
                    tool: tool_name_stderr.clone(),
                    stdout_chunk: String::new(),
                    stderr_chunk: chunk.clone(),
                });
                Self::append_bounded(&mut captured, &chunk);
            }
            captured
        });

        let timeout_duration = Duration::from_secs(self.config.timeout_seconds);
        let wait_result = timeout(timeout_duration, child.wait()).await;
        let status = match wait_result {
            Ok(result) => {
                result.map_err(|err| Error::Tool(format!("failed waiting for command: {err}")))?
            }
            Err(_) => {
                let _ = child.start_kill();
                return Err(Error::Tool(format!(
                    "command timed out after {} seconds",
                    self.config.timeout_seconds
                )));
            }
        };

        let stdout_captured = stdout_task
            .await
            .map_err(|err| Error::Tool(format!("stdout task join error: {err}")))?;
        let stderr_captured = stderr_task
            .await
            .map_err(|err| Error::Tool(format!("stderr task join error: {err}")))?;

        Ok((
            stdout_captured,
            stderr_captured,
            status.code().unwrap_or(-1),
        ))
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

    async fn execute(
        &self,
        args: serde_json::Value,
        context: &ToolExecutionContext,
    ) -> Result<super::ToolResult> {
        let (dummy_tx, _) = mpsc::channel(1);
        self.stream_execute(args, dummy_tx, context).await
    }

    async fn stream_execute(
        &self,
        args: serde_json::Value,
        tx: mpsc::Sender<Event>,
        context: &ToolExecutionContext,
    ) -> Result<super::ToolResult> {
        let tool_name = self.name().to_owned();
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("missing 'command' argument".to_owned()))?;

        let per_call_working_dir = args
            .get("working_directory")
            .and_then(|value| value.as_str())
            .map(PathBuf::from);

        self.validate_command(command)?;

        let _ = tx.try_send(Event::ToolStarted {
            tool: tool_name.clone(),
            args: args.clone(),
        });

        let (stdout, stderr, exit_code) = self
            .stream_command(
                command,
                &context.working_directory,
                per_call_working_dir.as_deref(),
                tx.clone(),
                &tool_name,
            )
            .await?;

        let success = exit_code == 0;
        let _ = tx.try_send(Event::ToolCompleted {
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
