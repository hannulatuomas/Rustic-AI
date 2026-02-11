use super::manifest::PluginManifest;
use crate::config::schema::{PermissionMode, PluginConfig, ToolConfig};
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::timeout;

const OUTPUT_CAPTURE_LIMIT_BYTES: usize = 10 * 1024;

pub struct LoadedPlugin {
    pub name: String,
    pub tool: Arc<dyn Tool>,
    pub config: ToolConfig,
}

#[derive(Debug, Clone)]
struct ExternalPluginTool {
    manifest: PluginManifest,
    manifest_path: PathBuf,
}

impl ExternalPluginTool {
    fn effective_timeout(&self) -> u64 {
        self.manifest.timeout_seconds.unwrap_or(60).max(1)
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

    fn resolve_path(raw: &str, base: &Path) -> PathBuf {
        let path = Path::new(raw);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        }
    }

    fn resolve_command_path(&self) -> PathBuf {
        let manifest_dir = self
            .manifest_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::resolve_path(&self.manifest.command, &manifest_dir)
    }

    fn resolve_work_dir(&self, context: &ToolExecutionContext) -> PathBuf {
        let manifest_dir = self
            .manifest_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| context.working_directory.clone());
        if let Some(raw) = &self.manifest.working_directory {
            Self::resolve_path(raw, &manifest_dir)
        } else {
            manifest_dir
        }
    }

    async fn run_plugin(
        &self,
        args: Value,
        tx: mpsc::Sender<Event>,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let command_path = self.resolve_command_path();
        let work_dir = self.resolve_work_dir(context);

        let mut cmd = Command::new(&command_path);
        cmd.kill_on_drop(true);
        cmd.args(&self.manifest.args)
            .current_dir(&work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if !self.manifest.env.is_empty() {
            cmd.envs(&self.manifest.env);
        }

        let mut child = cmd.spawn().map_err(|err| {
            Error::Tool(format!(
                "failed to spawn plugin '{}' command '{}': {err}",
                self.manifest.tool_name,
                command_path.display()
            ))
        })?;

        if let Some(mut stdin) = child.stdin.take() {
            let request = json!({
                "tool": self.manifest.tool_name,
                "args": args
            });
            let payload = serde_json::to_vec(&request)
                .map_err(|err| Error::Tool(format!("failed to serialize plugin input: {err}")))?;
            stdin
                .write_all(&payload)
                .await
                .map_err(|err| Error::Tool(format!("failed writing plugin stdin: {err}")))?;
            stdin.write_all(b"\n").await.map_err(|err| {
                Error::Tool(format!("failed writing plugin stdin newline: {err}"))
            })?;
            stdin
                .flush()
                .await
                .map_err(|err| Error::Tool(format!("failed flushing plugin stdin: {err}")))?;
        }

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Tool("failed to capture plugin stdout".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::Tool("failed to capture plugin stderr".to_owned()))?;

        let tool_name_stdout = self.manifest.tool_name.clone();
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

        let tool_name_stderr = self.manifest.tool_name.clone();
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

        let timeout_secs = self.effective_timeout();
        let wait_result = if let Some(cancellation_token) = context.cancellation_token.clone() {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    let _ = child.start_kill();
                    stdout_task.abort();
                    stderr_task.abort();
                    return Err(Error::Timeout(format!(
                        "plugin '{}' cancelled by workflow timeout",
                        self.manifest.tool_name
                    )));
                }
                result = timeout(std::time::Duration::from_secs(timeout_secs), child.wait()) => result,
            }
        } else {
            timeout(std::time::Duration::from_secs(timeout_secs), child.wait()).await
        };
        let wait_result = wait_result
            .map_err(|_| {
                let _ = child.start_kill();
                stdout_task.abort();
                stderr_task.abort();
                Error::Tool(format!(
                    "plugin '{}' timed out after {timeout_secs} seconds",
                    self.manifest.tool_name
                ))
            })?
            .map_err(|err| Error::Tool(format!("failed waiting for plugin process: {err}")))?;

        let stdout_captured = stdout_task
            .await
            .map_err(|err| Error::Tool(format!("plugin stdout task join error: {err}")))?;
        let stderr_captured = stderr_task
            .await
            .map_err(|err| Error::Tool(format!("plugin stderr task join error: {err}")))?;

        let exit_code = wait_result.code().unwrap_or(-1);
        Ok(ToolResult {
            success: exit_code == 0,
            exit_code: Some(exit_code),
            output: if exit_code == 0 {
                stdout_captured
            } else {
                stderr_captured
            },
        })
    }
}

#[async_trait]
impl Tool for ExternalPluginTool {
    fn name(&self) -> &str {
        &self.manifest.tool_name
    }

    fn description(&self) -> &str {
        &self.manifest.description
    }

    fn schema(&self) -> &Value {
        &self.manifest.schema
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
        let _ = tx.try_send(Event::ToolStarted {
            tool: self.manifest.tool_name.clone(),
            args: args.clone(),
        });

        let result = self.run_plugin(args, tx.clone(), context).await;
        let _ = tx.try_send(Event::ToolCompleted {
            tool: self.manifest.tool_name.clone(),
            exit_code: result
                .as_ref()
                .ok()
                .and_then(|value| value.exit_code)
                .unwrap_or(1),
        });

        result
    }
}

#[derive(Debug, Default)]
pub struct PluginLoader;

impl PluginLoader {
    fn resolve_plugin_dir(raw: &str, base: &Path) -> PathBuf {
        if let Some(rest) = raw.strip_prefix("~/") {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join(rest);
            }
        }
        let path = Path::new(raw);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        }
    }

    fn discover_manifest_files(config: &PluginConfig, base: &Path) -> Vec<PathBuf> {
        let mut discovered = Vec::new();
        for raw_dir in &config.directories {
            let start = Self::resolve_plugin_dir(raw_dir, base);
            if !start.exists() || !start.is_dir() {
                continue;
            }

            let mut queue = VecDeque::new();
            queue.push_back((start, 0usize));
            while let Some((dir, depth)) = queue.pop_front() {
                let entries = match std::fs::read_dir(&dir) {
                    Ok(entries) => entries,
                    Err(_) => continue,
                };

                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if depth < config.max_discovery_depth {
                            queue.push_back((path, depth + 1));
                        }
                        continue;
                    }
                    if !path.is_file() {
                        continue;
                    }
                    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                        continue;
                    };
                    if name == config.manifest_file_name {
                        discovered.push(path);
                    }
                }
            }
        }

        discovered
    }

    fn read_manifest(path: &Path) -> Result<PluginManifest> {
        let raw = std::fs::read_to_string(path).map_err(|err| {
            Error::Tool(format!(
                "failed reading plugin manifest '{}': {err}",
                path.display()
            ))
        })?;

        let manifest: PluginManifest = serde_json::from_str(&raw).map_err(|err| {
            Error::Tool(format!(
                "failed parsing plugin manifest '{}': {err}",
                path.display()
            ))
        })?;

        if manifest.api_version.trim() != "rustic-ai-plugin/v1" {
            return Err(Error::Tool(format!(
                "plugin manifest '{}' has unsupported api_version '{}'; expected 'rustic-ai-plugin/v1'",
                path.display(),
                manifest.api_version
            )));
        }
        if manifest.plugin_name.trim().is_empty() {
            return Err(Error::Tool(format!(
                "plugin manifest '{}' must define non-empty name",
                path.display()
            )));
        }
        if manifest.tool_name.trim().is_empty() {
            return Err(Error::Tool(format!(
                "plugin manifest '{}' must define non-empty tool_name",
                path.display()
            )));
        }
        if manifest.command.trim().is_empty() {
            return Err(Error::Tool(format!(
                "plugin manifest '{}' must define non-empty command",
                path.display()
            )));
        }

        Ok(manifest)
    }

    fn tool_config_for_manifest(
        manifest: &PluginManifest,
        default_permission: PermissionMode,
    ) -> ToolConfig {
        ToolConfig {
            name: manifest.tool_name.clone(),
            enabled: manifest.enabled,
            permission_mode: default_permission,
            timeout_seconds: manifest.timeout_seconds.unwrap_or(60).max(1),
            allowed_commands: Vec::new(),
            denied_commands: Vec::new(),
            working_dir: crate::config::schema::WorkingDirMode::ProjectRoot,
            custom_working_dir: None,
            env_passthrough: false,
            stream_output: true,
            require_sudo: false,
            privileged_command_patterns: Vec::new(),
            read_only_blocked_patterns: Vec::new(),
            taxonomy_membership: Vec::new(),
        }
    }

    pub fn load_plugins(
        config: &PluginConfig,
        execution_context: &ToolExecutionContext,
        default_permission: PermissionMode,
    ) -> Result<Vec<LoadedPlugin>> {
        let manifest_files =
            Self::discover_manifest_files(config, &execution_context.working_directory);
        let mut loaded = Vec::new();

        for manifest_path in manifest_files {
            let manifest = match Self::read_manifest(&manifest_path) {
                Ok(value) => value,
                Err(err) => {
                    tracing::warn!(%err, path = %manifest_path.display(), "skipping invalid plugin manifest");
                    continue;
                }
            };

            if !manifest.enabled {
                continue;
            }

            let name = manifest.tool_name.clone();
            let tool = Arc::new(ExternalPluginTool {
                manifest: manifest.clone(),
                manifest_path: manifest_path.clone(),
            }) as Arc<dyn Tool>;
            let tool_config = Self::tool_config_for_manifest(&manifest, default_permission);

            loaded.push(LoadedPlugin {
                name,
                tool,
                config: tool_config,
            });
        }

        Ok(loaded)
    }
}
