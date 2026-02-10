use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;

const OUTPUT_CAPTURE_LIMIT_BYTES: usize = 10 * 1024;
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 15;
const DEFAULT_CONTROL_PERSIST_SECS: u64 = 600;

#[derive(Debug, Clone)]
struct SshSession {
    name: String,
    host: String,
    user: Option<String>,
    port: u16,
    control_path: PathBuf,
    identity_file: Option<PathBuf>,
    strict_host_key_checking: String,
    known_hosts_file: Option<PathBuf>,
    created_at_epoch_secs: u64,
}

#[derive(Debug, Clone)]
pub struct SshTool {
    config: ToolConfig,
    schema: Value,
    sessions: Arc<Mutex<HashMap<String, SshSession>>>,
}

impl SshTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "connect",
                        "exec",
                        "disconnect",
                        "list_sessions",
                        "close_all",
                        "scp_upload",
                        "scp_download"
                    ]
                },
                "session": { "type": "string", "description": "Logical SSH session name" },
                "host": { "type": "string", "description": "Remote host name or IP" },
                "port": { "type": "integer", "minimum": 1, "maximum": 65535 },
                "user": { "type": "string", "description": "SSH username" },
                "identity_file": { "type": "string", "description": "Private key path" },
                "strict_host_key_checking": {
                    "type": "string",
                    "enum": ["accept-new", "yes", "no"],
                    "description": "Host key verification mode"
                },
                "known_hosts_file": {
                    "type": "string",
                    "description": "Path to known_hosts file"
                },
                "connect_timeout_seconds": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Connection timeout in seconds"
                },
                "command": {
                    "type": "string",
                    "description": "Command to execute for 'exec' operation"
                },
                "pty": {
                    "type": "boolean",
                    "description": "Allocate a PTY for 'exec' operation"
                },
                "local_path": {
                    "type": "string",
                    "description": "Local filesystem path for SCP transfer"
                },
                "remote_path": {
                    "type": "string",
                    "description": "Remote path for SCP transfer"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "Enable recursive SCP transfer"
                }
            },
            "required": ["operation"]
        });

        Self {
            config,
            schema,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn required_string<'a>(&self, args: &'a Value, key: &str) -> Result<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::Tool(format!("missing '{key}' argument")))
    }

    fn optional_string(args: &Value, key: &str) -> Option<String> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    fn resolve_local_path(raw: &str, context: &ToolExecutionContext) -> PathBuf {
        let path = Path::new(raw);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            context.working_directory.join(path)
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

    fn now_epoch_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn ssh_target(user: Option<&str>, host: &str) -> String {
        if let Some(user) = user {
            format!("{user}@{host}")
        } else {
            host.to_owned()
        }
    }

    fn default_session_name(host: &str, user: Option<&str>, port: u16) -> String {
        let user_part = user.unwrap_or("default");
        format!("{user_part}@{host}:{port}")
    }

    fn ensure_ssh_available() -> Result<()> {
        let output = std::process::Command::new("ssh")
            .arg("-V")
            .output()
            .map_err(|err| Error::Tool(format!("ssh binary not available: {err}")))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::Tool(
                "ssh binary is not usable on this system".to_owned(),
            ))
        }
    }

    fn ensure_scp_available() -> Result<()> {
        let output = std::process::Command::new("scp")
            .arg("-V")
            .output()
            .map_err(|err| Error::Tool(format!("scp binary not available: {err}")))?;
        if output.status.success() || !String::from_utf8_lossy(&output.stderr).is_empty() {
            // OpenSSH scp commonly prints version/help to stderr and may return non-zero.
            Ok(())
        } else {
            Err(Error::Tool(
                "scp binary is not usable on this system".to_owned(),
            ))
        }
    }

    fn build_control_path(base_dir: &Path, session_name: &str) -> PathBuf {
        let safe = session_name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>();
        base_dir.join(format!("{safe}.sock"))
    }

    async fn run_ssh_streaming(
        &self,
        mut cmd: Command,
        tx: mpsc::Sender<Event>,
        tool_name: &str,
    ) -> Result<(String, String, i32)> {
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|err| Error::Tool(format!("failed to spawn ssh command: {err}")))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Tool("failed to capture ssh stdout".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::Tool("failed to capture ssh stderr".to_owned()))?;

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

        let wait_result = timeout(
            Duration::from_secs(self.config.timeout_seconds),
            child.wait(),
        )
        .await
        .map_err(|_| {
            let _ = child.start_kill();
            Error::Tool(format!(
                "ssh command timed out after {} seconds",
                self.config.timeout_seconds
            ))
        })?
        .map_err(|err| Error::Tool(format!("failed waiting for ssh command: {err}")))?;

        let stdout_captured = stdout_task
            .await
            .map_err(|err| Error::Tool(format!("stdout task join error: {err}")))?;
        let stderr_captured = stderr_task
            .await
            .map_err(|err| Error::Tool(format!("stderr task join error: {err}")))?;

        Ok((
            stdout_captured,
            stderr_captured,
            wait_result.code().unwrap_or(-1),
        ))
    }

    async fn connect_session(
        &self,
        args: &Value,
        context: &ToolExecutionContext,
        tx: mpsc::Sender<Event>,
    ) -> Result<ToolResult> {
        Self::ensure_ssh_available()?;

        let host = self.required_string(args, "host")?.to_owned();
        let port = args
            .get("port")
            .and_then(Value::as_u64)
            .map(|raw| {
                u16::try_from(raw)
                    .map_err(|_| Error::Tool("'port' must be between 1 and 65535".to_owned()))
            })
            .transpose()?
            .unwrap_or(22);
        let user = Self::optional_string(args, "user");
        let session_name = Self::optional_string(args, "session")
            .unwrap_or_else(|| Self::default_session_name(&host, user.as_deref(), port));

        let strict_host_key_checking = Self::optional_string(args, "strict_host_key_checking")
            .unwrap_or_else(|| "accept-new".to_owned());
        if !matches!(
            strict_host_key_checking.as_str(),
            "accept-new" | "yes" | "no"
        ) {
            return Err(Error::Tool(
                "strict_host_key_checking must be one of: accept-new, yes, no".to_owned(),
            ));
        }

        let connect_timeout = args
            .get("connect_timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_CONNECT_TIMEOUT_SECS);

        let identity_file = Self::optional_string(args, "identity_file")
            .map(|raw| Self::resolve_local_path(&raw, context));
        let known_hosts_file = Self::optional_string(args, "known_hosts_file")
            .map(|raw| Self::resolve_local_path(&raw, context));

        let mut sessions = self.sessions.lock().await;
        if sessions.contains_key(&session_name) {
            return Err(Error::Tool(format!(
                "ssh session '{session_name}' already exists"
            )));
        }

        let control_dir = context.working_directory.join(".rustic-ai").join("ssh");
        std::fs::create_dir_all(&control_dir).map_err(|err| {
            Error::Tool(format!(
                "failed to create ssh control directory '{}': {err}",
                control_dir.display()
            ))
        })?;
        let control_path = Self::build_control_path(&control_dir, &session_name);

        let target = Self::ssh_target(user.as_deref(), &host);
        let mut cmd = Command::new("ssh");
        cmd.arg("-M")
            .arg("-N")
            .arg("-f")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg(format!("ConnectTimeout={connect_timeout}"))
            .arg("-o")
            .arg(format!("ControlPersist={DEFAULT_CONTROL_PERSIST_SECS}"))
            .arg("-o")
            .arg(format!("ControlPath={}", control_path.display()))
            .arg("-o")
            .arg(format!("StrictHostKeyChecking={strict_host_key_checking}"))
            .arg("-p")
            .arg(port.to_string());

        if let Some(identity) = &identity_file {
            cmd.arg("-i").arg(identity);
        }
        if let Some(known_hosts) = &known_hosts_file {
            cmd.arg("-o")
                .arg(format!("UserKnownHostsFile={}", known_hosts.display()));
        }

        cmd.arg(&target);

        let (stdout, stderr, exit_code) =
            self.run_ssh_streaming(cmd, tx, &self.config.name).await?;
        if exit_code != 0 {
            return Err(Error::Tool(format!(
                "failed to establish ssh session '{session_name}' (exit {exit_code}): {}",
                if stderr.is_empty() { stdout } else { stderr }
            )));
        }

        let session = SshSession {
            name: session_name.clone(),
            host,
            user,
            port,
            control_path,
            identity_file,
            strict_host_key_checking,
            known_hosts_file,
            created_at_epoch_secs: Self::now_epoch_secs(),
        };
        sessions.insert(session_name.clone(), session.clone());

        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: json!({
                "session": session_name,
                "target": Self::ssh_target(session.user.as_deref(), &session.host),
                "port": session.port,
                "control_path": session.control_path,
                "created_at_epoch_secs": session.created_at_epoch_secs
            })
            .to_string(),
        })
    }

    async fn exec_command(&self, args: &Value, tx: mpsc::Sender<Event>) -> Result<ToolResult> {
        let session_name = self.required_string(args, "session")?.to_owned();
        let command = self.required_string(args, "command")?.to_owned();
        let pty = args.get("pty").and_then(Value::as_bool).unwrap_or(false);

        let session = {
            let sessions = self.sessions.lock().await;
            sessions.get(&session_name).cloned()
        }
        .ok_or_else(|| Error::Tool(format!("ssh session '{session_name}' does not exist")))?;

        let target = Self::ssh_target(session.user.as_deref(), &session.host);
        let mut cmd = Command::new("ssh");
        cmd.arg("-S")
            .arg(&session.control_path)
            .arg("-o")
            .arg("ControlMaster=auto")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg(format!(
                "StrictHostKeyChecking={}",
                session.strict_host_key_checking
            ))
            .arg("-p")
            .arg(session.port.to_string());

        if pty {
            cmd.arg("-tt");
        }

        if let Some(identity) = &session.identity_file {
            cmd.arg("-i").arg(identity);
        }
        if let Some(known_hosts) = &session.known_hosts_file {
            cmd.arg("-o")
                .arg(format!("UserKnownHostsFile={}", known_hosts.display()));
        }

        cmd.arg(&target).arg("sh").arg("-lc").arg(&command);

        let (stdout, stderr, exit_code) =
            self.run_ssh_streaming(cmd, tx, &self.config.name).await?;
        Ok(ToolResult {
            success: exit_code == 0,
            exit_code: Some(exit_code),
            output: if exit_code == 0 { stdout } else { stderr },
        })
    }

    async fn scp_upload(
        &self,
        args: &Value,
        context: &ToolExecutionContext,
        tx: mpsc::Sender<Event>,
    ) -> Result<ToolResult> {
        Self::ensure_scp_available()?;

        let session_name = self.required_string(args, "session")?.to_owned();
        let local_path_raw = self.required_string(args, "local_path")?;
        let remote_path = self.required_string(args, "remote_path")?.to_owned();
        let recursive = args
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let session = {
            let sessions = self.sessions.lock().await;
            sessions.get(&session_name).cloned()
        }
        .ok_or_else(|| Error::Tool(format!("ssh session '{session_name}' does not exist")))?;

        let local_path = Self::resolve_local_path(local_path_raw, context);
        if !local_path.exists() {
            return Err(Error::Tool(format!(
                "local_path '{}' does not exist",
                local_path.display()
            )));
        }

        let target = Self::ssh_target(session.user.as_deref(), &session.host);
        let remote_target = format!("{target}:{remote_path}");

        let mut cmd = Command::new("scp");
        cmd.arg("-B")
            .arg("-P")
            .arg(session.port.to_string())
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg(format!(
                "StrictHostKeyChecking={}",
                session.strict_host_key_checking
            ))
            .arg("-o")
            .arg(format!("ControlPath={}", session.control_path.display()));

        if recursive {
            cmd.arg("-r");
        }
        if let Some(identity) = &session.identity_file {
            cmd.arg("-i").arg(identity);
        }
        if let Some(known_hosts) = &session.known_hosts_file {
            cmd.arg("-o")
                .arg(format!("UserKnownHostsFile={}", known_hosts.display()));
        }

        cmd.arg(&local_path).arg(&remote_target);

        let (stdout, stderr, exit_code) =
            self.run_ssh_streaming(cmd, tx, &self.config.name).await?;
        Ok(ToolResult {
            success: exit_code == 0,
            exit_code: Some(exit_code),
            output: if exit_code == 0 {
                json!({
                    "session": session_name,
                    "local_path": local_path,
                    "remote_path": remote_path,
                    "recursive": recursive,
                    "stdout": stdout
                })
                .to_string()
            } else {
                stderr
            },
        })
    }

    async fn scp_download(
        &self,
        args: &Value,
        context: &ToolExecutionContext,
        tx: mpsc::Sender<Event>,
    ) -> Result<ToolResult> {
        Self::ensure_scp_available()?;

        let session_name = self.required_string(args, "session")?.to_owned();
        let remote_path = self.required_string(args, "remote_path")?.to_owned();
        let local_path_raw = self.required_string(args, "local_path")?;
        let recursive = args
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let session = {
            let sessions = self.sessions.lock().await;
            sessions.get(&session_name).cloned()
        }
        .ok_or_else(|| Error::Tool(format!("ssh session '{session_name}' does not exist")))?;

        let local_path = Self::resolve_local_path(local_path_raw, context);
        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                Error::Tool(format!(
                    "failed to create local parent directory '{}': {err}",
                    parent.display()
                ))
            })?;
        }

        let target = Self::ssh_target(session.user.as_deref(), &session.host);
        let remote_source = format!("{target}:{remote_path}");

        let mut cmd = Command::new("scp");
        cmd.arg("-B")
            .arg("-P")
            .arg(session.port.to_string())
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg(format!(
                "StrictHostKeyChecking={}",
                session.strict_host_key_checking
            ))
            .arg("-o")
            .arg(format!("ControlPath={}", session.control_path.display()));

        if recursive {
            cmd.arg("-r");
        }
        if let Some(identity) = &session.identity_file {
            cmd.arg("-i").arg(identity);
        }
        if let Some(known_hosts) = &session.known_hosts_file {
            cmd.arg("-o")
                .arg(format!("UserKnownHostsFile={}", known_hosts.display()));
        }

        cmd.arg(&remote_source).arg(&local_path);

        let (stdout, stderr, exit_code) =
            self.run_ssh_streaming(cmd, tx, &self.config.name).await?;
        Ok(ToolResult {
            success: exit_code == 0,
            exit_code: Some(exit_code),
            output: if exit_code == 0 {
                json!({
                    "session": session_name,
                    "remote_path": remote_path,
                    "local_path": local_path,
                    "recursive": recursive,
                    "stdout": stdout
                })
                .to_string()
            } else {
                stderr
            },
        })
    }

    async fn disconnect_session(
        &self,
        args: &Value,
        tx: mpsc::Sender<Event>,
    ) -> Result<ToolResult> {
        let session_name = self.required_string(args, "session")?.to_owned();
        let session = {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(&session_name)
        }
        .ok_or_else(|| Error::Tool(format!("ssh session '{session_name}' does not exist")))?;

        let target = Self::ssh_target(session.user.as_deref(), &session.host);
        let mut cmd = Command::new("ssh");
        cmd.arg("-S")
            .arg(&session.control_path)
            .arg("-O")
            .arg("exit")
            .arg("-p")
            .arg(session.port.to_string())
            .arg(&target);

        let (stdout, stderr, exit_code) =
            self.run_ssh_streaming(cmd, tx, &self.config.name).await?;
        let _ = std::fs::remove_file(&session.control_path);
        if exit_code != 0 {
            return Err(Error::Tool(format!(
                "failed to close ssh session '{session_name}' (exit {exit_code}): {}",
                if stderr.is_empty() { stdout } else { stderr }
            )));
        }

        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: json!({ "session": session_name, "closed": true }).to_string(),
        })
    }

    async fn list_sessions(&self) -> ToolResult {
        let sessions = self.sessions.lock().await;
        let items = sessions
            .values()
            .map(|session| {
                json!({
                    "session": session.name,
                    "target": Self::ssh_target(session.user.as_deref(), &session.host),
                    "port": session.port,
                    "control_path": session.control_path,
                    "created_at_epoch_secs": session.created_at_epoch_secs
                })
            })
            .collect::<Vec<_>>();

        ToolResult {
            success: true,
            exit_code: Some(0),
            output: json!({ "sessions": items, "count": items.len() }).to_string(),
        }
    }

    async fn close_all_sessions(&self, tx: mpsc::Sender<Event>) -> Result<ToolResult> {
        let names = {
            let sessions = self.sessions.lock().await;
            sessions.keys().cloned().collect::<Vec<_>>()
        };

        let mut closed = Vec::new();
        for name in names {
            let args = json!({ "session": name });
            match self.disconnect_session(&args, tx.clone()).await {
                Ok(_) => closed.push(args["session"].as_str().unwrap_or_default().to_owned()),
                Err(err) => {
                    return Err(Error::Tool(format!(
                        "failed closing ssh sessions during close_all: {err}"
                    )));
                }
            }
        }

        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: json!({ "closed_sessions": closed, "count": closed.len() }).to_string(),
        })
    }
}

#[async_trait]
impl Tool for SshTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Manage persistent SSH sessions and execute remote commands"
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
            "connect" => self.connect_session(&args, context, tx.clone()).await,
            "exec" => self.exec_command(&args, tx.clone()).await,
            "disconnect" => self.disconnect_session(&args, tx.clone()).await,
            "list_sessions" => Ok(self.list_sessions().await),
            "close_all" => self.close_all_sessions(tx.clone()).await,
            "scp_upload" => self.scp_upload(&args, context, tx.clone()).await,
            "scp_download" => self.scp_download(&args, context, tx.clone()).await,
            other => Err(Error::Tool(format!("unsupported ssh operation '{other}'"))),
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
