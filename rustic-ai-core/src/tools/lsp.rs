use async_trait::async_trait;
use lsp_types::{
    GotoDefinitionResponse, Hover, ReferenceParams, SymbolInformation, SymbolKind,
    WorkspaceSymbolResponse,
};
use reqwest::Url;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct LspTool {
    config: ToolConfig,
    schema: Value,
    servers: Arc<Mutex<HashMap<String, Arc<Mutex<LspServerHandle>>>>>,
}

#[derive(Debug)]
struct LspServerHandle {
    server_id: String,
    command: String,
    workspace_root: PathBuf,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    symbol_cache: HashMap<String, Vec<Value>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LspOperation {
    StartServer,
    StopServer,
    SymbolSearch,
    Definition,
    References,
    Hover,
}

impl LspOperation {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "start_server" => Ok(Self::StartServer),
            "stop_server" => Ok(Self::StopServer),
            "symbol_search" => Ok(Self::SymbolSearch),
            "definition" => Ok(Self::Definition),
            "references" => Ok(Self::References),
            "hover" => Ok(Self::Hover),
            other => Err(Error::Tool(format!(
                "unsupported lsp operation '{other}' (expected start_server|stop_server|symbol_search|definition|references|hover)"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::StartServer => "start_server",
            Self::StopServer => "stop_server",
            Self::SymbolSearch => "symbol_search",
            Self::Definition => "definition",
            Self::References => "references",
            Self::Hover => "hover",
        }
    }
}

impl LspTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "operation": {"type": "string", "enum": ["start_server", "stop_server", "symbol_search", "definition", "references", "hover"]},
                "server_id": {"type": "string"},
                "server_command": {"type": "string"},
                "server_args": {"type": "array", "items": {"type": "string"}},
                "workspace_root": {"type": "string"},
                "query": {"type": "string"},
                "file_path": {"type": "string"},
                "line": {"type": "integer", "minimum": 0},
                "character": {"type": "integer", "minimum": 0},
                "include_declaration": {"type": "boolean"},
                "language_id": {"type": "string"},
                "max_results": {"type": "integer", "minimum": 1, "maximum": 500},
                "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 300}
            },
            "required": ["operation"]
        });

        Self {
            config,
            schema,
            servers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Tool(format!("missing '{key}' argument")))
    }

    fn optional_string<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }

    fn timeout_seconds(&self, args: &Value) -> u64 {
        args.get("timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(self.config.timeout_seconds)
            .clamp(1, 300)
    }

    fn canonicalize(path: &Path) -> Result<PathBuf> {
        std::fs::canonicalize(path).map_err(|err| {
            Error::Tool(format!(
                "failed to resolve path '{}': {err}",
                path.display()
            ))
        })
    }

    fn resolve_workspace_root(
        &self,
        context: &ToolExecutionContext,
        raw: Option<&str>,
    ) -> Result<PathBuf> {
        let base = Self::canonicalize(&context.working_directory)?;
        let candidate = raw
            .map(PathBuf::from)
            .map(|p| if p.is_absolute() { p } else { base.join(p) })
            .unwrap_or_else(|| base.clone());
        let resolved = Self::canonicalize(&candidate)?;
        if !resolved.starts_with(&base) {
            return Err(Error::Tool(format!(
                "workspace_root '{}' is outside tool working directory '{}'; use a path within the workspace",
                resolved.display(),
                base.display()
            )));
        }
        Ok(resolved)
    }

    fn resolve_file_path(&self, context: &ToolExecutionContext, raw_file: &str) -> Result<PathBuf> {
        let base = Self::canonicalize(&context.working_directory)?;
        let candidate = {
            let p = PathBuf::from(raw_file);
            if p.is_absolute() {
                p
            } else {
                base.join(p)
            }
        };
        let resolved = Self::canonicalize(&candidate)?;
        if !resolved.starts_with(&base) {
            return Err(Error::Tool(format!(
                "file_path '{}' is outside tool working directory '{}'; use a path within the workspace",
                resolved.display(),
                base.display()
            )));
        }
        Ok(resolved)
    }

    fn symbol_kind_name(kind: SymbolKind) -> &'static str {
        match kind {
            SymbolKind::FILE => "file",
            SymbolKind::MODULE => "module",
            SymbolKind::NAMESPACE => "namespace",
            SymbolKind::PACKAGE => "package",
            SymbolKind::CLASS => "class",
            SymbolKind::METHOD => "method",
            SymbolKind::PROPERTY => "property",
            SymbolKind::FIELD => "field",
            SymbolKind::CONSTRUCTOR => "constructor",
            SymbolKind::ENUM => "enum",
            SymbolKind::INTERFACE => "interface",
            SymbolKind::FUNCTION => "function",
            SymbolKind::VARIABLE => "variable",
            SymbolKind::CONSTANT => "constant",
            SymbolKind::STRING => "string",
            SymbolKind::NUMBER => "number",
            SymbolKind::BOOLEAN => "boolean",
            SymbolKind::ARRAY => "array",
            SymbolKind::OBJECT => "object",
            SymbolKind::KEY => "key",
            SymbolKind::NULL => "null",
            SymbolKind::ENUM_MEMBER => "enum_member",
            SymbolKind::STRUCT => "struct",
            SymbolKind::EVENT => "event",
            SymbolKind::OPERATOR => "operator",
            SymbolKind::TYPE_PARAMETER => "type_parameter",
            _ => "unknown",
        }
    }

    async fn write_message(stdin: &mut ChildStdin, payload: &Value) -> Result<()> {
        let body = payload.to_string();
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|err| Error::Tool(format!("failed writing LSP header: {err}")))?;
        stdin
            .write_all(body.as_bytes())
            .await
            .map_err(|err| Error::Tool(format!("failed writing LSP body: {err}")))?;
        stdin
            .flush()
            .await
            .map_err(|err| Error::Tool(format!("failed flushing LSP stdin: {err}")))?;
        Ok(())
    }

    async fn read_message(stdout: &mut BufReader<ChildStdout>) -> Result<Value> {
        let mut content_length = None::<usize>;
        loop {
            let mut line = String::new();
            let bytes = stdout
                .read_line(&mut line)
                .await
                .map_err(|err| Error::Tool(format!("failed reading LSP header: {err}")))?;
            if bytes == 0 {
                return Err(Error::Tool("LSP server closed stdout".to_owned()));
            }
            let line_trimmed = line.trim_end_matches(['\r', '\n']);
            if line_trimmed.is_empty() {
                break;
            }
            if let Some(rest) = line_trimmed.strip_prefix("Content-Length:") {
                let length = rest.trim().parse::<usize>().map_err(|err| {
                    Error::Tool(format!("invalid LSP Content-Length header: {err}"))
                })?;
                content_length = Some(length);
            }
        }

        let length = content_length.ok_or_else(|| {
            Error::Tool("missing Content-Length header in LSP response".to_owned())
        })?;
        let mut body = vec![0u8; length];
        stdout
            .read_exact(&mut body)
            .await
            .map_err(|err| Error::Tool(format!("failed reading LSP body: {err}")))?;

        serde_json::from_slice(&body)
            .map_err(|err| Error::Tool(format!("invalid JSON from LSP server: {err}")))
    }

    async fn send_request(
        handle: &mut LspServerHandle,
        method: &str,
        params: Value,
        timeout_seconds: u64,
    ) -> Result<Value> {
        let id = handle.next_id;
        handle.next_id = handle.next_id.saturating_add(1);
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        Self::write_message(&mut handle.stdin, &req).await?;

        loop {
            let response = timeout(
                Duration::from_secs(timeout_seconds),
                Self::read_message(&mut handle.stdout),
            )
            .await
            .map_err(|_| {
                Error::Timeout(format!(
                    "LSP request '{}' timed out after {} seconds",
                    method, timeout_seconds
                ))
            })??;

            if response.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(err) = response.get("error") {
                    return Err(Error::Tool(format!("LSP '{}' error: {}", method, err)));
                }
                return Ok(response.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    async fn send_notification(
        handle: &mut LspServerHandle,
        method: &str,
        params: Value,
    ) -> Result<()> {
        let req = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        Self::write_message(&mut handle.stdin, &req).await
    }

    fn server_id_for(command: &str, workspace_root: &Path) -> String {
        format!("{}::{}", command, workspace_root.display())
    }

    async fn start_server_internal(
        &self,
        command: &str,
        args: &[String],
        workspace_root: &Path,
        timeout_seconds: u64,
    ) -> Result<String> {
        let server_id = Self::server_id_for(command, workspace_root);
        {
            let servers = self.servers.lock().await;
            if servers.contains_key(&server_id) {
                return Ok(server_id);
            }
        }

        let mut cmd = Command::new(command);
        cmd.args(args)
            .current_dir(workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = cmd.spawn().map_err(|err| {
            Error::Tool(format!("failed to spawn LSP server '{}': {err}", command))
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Tool("LSP child missing stdin".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Tool("LSP child missing stdout".to_owned()))?;

        let mut handle = LspServerHandle {
            server_id: server_id.clone(),
            command: command.to_owned(),
            workspace_root: workspace_root.to_path_buf(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            symbol_cache: HashMap::new(),
        };

        let root_uri = Self::path_to_lsp_uri(workspace_root, true)?;

        let _init = Self::send_request(
            &mut handle,
            "initialize",
            json!({
                "processId": null,
                "rootUri": root_uri,
                "capabilities": {},
                "trace": "off",
                "workspaceFolders": [{
                    "uri": root_uri,
                    "name": workspace_root.file_name().and_then(|v| v.to_str()).unwrap_or("workspace")
                }]
            }),
            timeout_seconds,
        )
        .await?;
        Self::send_notification(&mut handle, "initialized", json!({})).await?;

        let mut servers = self.servers.lock().await;
        servers.insert(server_id.clone(), Arc::new(Mutex::new(handle)));
        Ok(server_id)
    }

    async fn stop_server_internal(&self, server_id: &str, timeout_seconds: u64) -> Result<Value> {
        let server = {
            let mut servers = self.servers.lock().await;
            servers.remove(server_id)
        }
        .ok_or_else(|| Error::Tool(format!("lsp server '{}' not found", server_id)))?;

        let mut handle = server.lock().await;
        let _ = Self::send_request(&mut handle, "shutdown", json!({}), timeout_seconds).await;
        let _ = Self::send_notification(&mut handle, "exit", json!({})).await;
        let _ = handle.child.start_kill();

        Ok(json!({
            "operation": "stop_server",
            "server_id": server_id,
            "stopped": true,
        }))
    }

    async fn get_handle_for_operation(
        &self,
        args: &Value,
        context: &ToolExecutionContext,
        timeout_seconds: u64,
    ) -> Result<Arc<Mutex<LspServerHandle>>> {
        if let Some(server_id) = Self::optional_string(args, "server_id") {
            let servers = self.servers.lock().await;
            return servers
                .get(server_id)
                .cloned()
                .ok_or_else(|| Error::Tool(format!("lsp server '{}' not found", server_id)));
        }

        let command = Self::required_string(args, "server_command")?;
        let server_args = args
            .get("server_args")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let workspace_root =
            self.resolve_workspace_root(context, Self::optional_string(args, "workspace_root"))?;
        let server_id = self
            .start_server_internal(command, &server_args, &workspace_root, timeout_seconds)
            .await?;
        let servers = self.servers.lock().await;
        servers
            .get(&server_id)
            .cloned()
            .ok_or_else(|| Error::Tool(format!("lsp server '{}' not found after start", server_id)))
    }

    async fn did_open_document(
        &self,
        handle: &mut LspServerHandle,
        file_path: &Path,
        language_id: Option<&str>,
    ) -> Result<()> {
        let text = std::fs::read_to_string(file_path).map_err(|err| {
            Error::Tool(format!("failed to read '{}': {err}", file_path.display()))
        })?;
        let uri = Self::path_to_lsp_uri(file_path, false)?;
        let lang = language_id
            .map(ToOwned::to_owned)
            .or_else(|| {
                file_path
                    .extension()
                    .and_then(|v| v.to_str())
                    .map(|ext| ext.to_ascii_lowercase())
            })
            .unwrap_or_else(|| "text".to_owned());

        Self::send_notification(
            handle,
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": lang,
                    "version": 1,
                    "text": text,
                }
            }),
        )
        .await
    }

    fn location_to_json(location: &lsp_types::Location) -> Value {
        json!({
            "uri": location.uri,
            "range": {
                "start": {
                    "line": location.range.start.line,
                    "character": location.range.start.character
                },
                "end": {
                    "line": location.range.end.line,
                    "character": location.range.end.character
                }
            }
        })
    }

    fn symbol_info_to_json(symbol: &SymbolInformation) -> Value {
        json!({
            "name": symbol.name,
            "kind": Self::symbol_kind_name(symbol.kind),
            "tags": symbol.tags,
            "container_name": symbol.container_name,
            "location": Self::location_to_json(&symbol.location),
        })
    }

    fn parse_workspace_symbols(value: Value) -> Result<Vec<Value>> {
        let response: WorkspaceSymbolResponse = serde_json::from_value(value)
            .map_err(|err| Error::Tool(format!("invalid workspace/symbol response: {err}")))?;
        let items = match response {
            WorkspaceSymbolResponse::Flat(items) => items
                .iter()
                .map(Self::symbol_info_to_json)
                .collect::<Vec<_>>(),
            WorkspaceSymbolResponse::Nested(items) => items
                .iter()
                .map(|item| {
                    json!({
                        "name": item.name,
                        "kind": Self::symbol_kind_name(item.kind),
                        "tags": item.tags,
                        "container_name": item.container_name,
                        "location": item.location,
                    })
                })
                .collect::<Vec<_>>(),
        };
        Ok(items)
    }

    async fn execute_operation(
        &self,
        args: Value,
        context: &ToolExecutionContext,
        tx: Option<mpsc::Sender<Event>>,
    ) -> Result<ToolResult> {
        let operation = LspOperation::parse(Self::required_string(&args, "operation")?)?;
        let timeout_seconds = self.timeout_seconds(&args);

        let payload = match operation {
            LspOperation::StartServer => {
                let command = Self::required_string(&args, "server_command")?;
                let server_args = args
                    .get("server_args")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(ToOwned::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let workspace_root = self.resolve_workspace_root(
                    context,
                    Self::optional_string(&args, "workspace_root"),
                )?;
                let server_id = self
                    .start_server_internal(command, &server_args, &workspace_root, timeout_seconds)
                    .await?;
                json!({
                    "operation": operation.as_str(),
                    "server_id": server_id,
                    "command": command,
                    "workspace_root": workspace_root,
                    "started": true,
                })
            }
            LspOperation::StopServer => {
                let server_id = Self::required_string(&args, "server_id")?;
                self.stop_server_internal(server_id, timeout_seconds)
                    .await?
            }
            LspOperation::SymbolSearch => {
                let query = Self::required_string(&args, "query")?;
                let max_results = args
                    .get("max_results")
                    .and_then(Value::as_u64)
                    .unwrap_or(100)
                    .clamp(1, 500) as usize;

                let server = self
                    .get_handle_for_operation(&args, context, timeout_seconds)
                    .await?;
                let mut handle = server.lock().await;

                if let Some(cached) = handle.symbol_cache.get(query) {
                    let mut items = cached.clone();
                    items.truncate(max_results);
                    json!({
                        "operation": operation.as_str(),
                        "server_id": handle.server_id,
                        "server_command": handle.command,
                        "workspace_root": handle.workspace_root,
                        "cached": true,
                        "count": items.len(),
                        "symbols": items,
                    })
                } else {
                    let result = Self::send_request(
                        &mut handle,
                        "workspace/symbol",
                        json!({ "query": query }),
                        timeout_seconds,
                    )
                    .await?;
                    let mut symbols = Self::parse_workspace_symbols(result)?;
                    symbols.truncate(max_results);
                    handle
                        .symbol_cache
                        .insert(query.to_owned(), symbols.clone());
                    json!({
                        "operation": operation.as_str(),
                        "server_id": handle.server_id,
                        "server_command": handle.command,
                        "workspace_root": handle.workspace_root,
                        "cached": false,
                        "count": symbols.len(),
                        "symbols": symbols,
                    })
                }
            }
            LspOperation::Definition | LspOperation::References | LspOperation::Hover => {
                let file_path =
                    self.resolve_file_path(context, Self::required_string(&args, "file_path")?)?;
                let line = args.get("line").and_then(Value::as_u64).unwrap_or(0) as u32;
                let character = args.get("character").and_then(Value::as_u64).unwrap_or(0) as u32;
                let language_id = Self::optional_string(&args, "language_id");

                let server = self
                    .get_handle_for_operation(&args, context, timeout_seconds)
                    .await?;
                let mut handle = server.lock().await;
                self.did_open_document(&mut handle, &file_path, language_id)
                    .await?;

                let uri = Self::path_to_lsp_uri(&file_path, false)?;

                match operation {
                    LspOperation::Definition => {
                        let result = Self::send_request(
                            &mut handle,
                            "textDocument/definition",
                            json!({
                                "textDocument": { "uri": uri },
                                "position": { "line": line, "character": character }
                            }),
                            timeout_seconds,
                        )
                        .await?;
                        let parsed: GotoDefinitionResponse = serde_json::from_value(result)
                            .map_err(|err| {
                                Error::Tool(format!("invalid definition response: {err}"))
                            })?;
                        let locations = match parsed {
                            GotoDefinitionResponse::Scalar(loc) => {
                                vec![Self::location_to_json(&loc)]
                            }
                            GotoDefinitionResponse::Array(locs) => {
                                locs.iter().map(Self::location_to_json).collect::<Vec<_>>()
                            }
                            GotoDefinitionResponse::Link(links) => links
                                .iter()
                                .map(|link| {
                                    json!({
                                        "target_uri": link.target_uri,
                                        "target_range": link.target_range,
                                        "target_selection_range": link.target_selection_range,
                                        "origin_selection_range": link.origin_selection_range,
                                    })
                                })
                                .collect::<Vec<_>>(),
                        };
                        json!({
                            "operation": operation.as_str(),
                            "server_id": handle.server_id,
                            "server_command": handle.command,
                            "workspace_root": handle.workspace_root,
                            "count": locations.len(),
                            "definitions": locations,
                        })
                    }
                    LspOperation::References => {
                        let include_declaration = args
                            .get("include_declaration")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let params = ReferenceParams {
                            text_document_position: lsp_types::TextDocumentPositionParams {
                                text_document: lsp_types::TextDocumentIdentifier {
                                    uri: uri.clone(),
                                },
                                position: lsp_types::Position { line, character },
                            },
                            context: lsp_types::ReferenceContext {
                                include_declaration,
                            },
                            work_done_progress_params: Default::default(),
                            partial_result_params: Default::default(),
                        };
                        let result = Self::send_request(
                            &mut handle,
                            "textDocument/references",
                            serde_json::to_value(&params).map_err(|err| {
                                Error::Tool(format!("failed to encode references params: {err}"))
                            })?,
                            timeout_seconds,
                        )
                        .await?;
                        let parsed: Option<Vec<lsp_types::Location>> =
                            serde_json::from_value(result).map_err(|err| {
                                Error::Tool(format!("invalid references response: {err}"))
                            })?;
                        let refs = parsed
                            .unwrap_or_default()
                            .iter()
                            .map(Self::location_to_json)
                            .collect::<Vec<_>>();
                        json!({
                            "operation": operation.as_str(),
                            "server_id": handle.server_id,
                            "server_command": handle.command,
                            "workspace_root": handle.workspace_root,
                            "count": refs.len(),
                            "references": refs,
                        })
                    }
                    LspOperation::Hover => {
                        let result = Self::send_request(
                            &mut handle,
                            "textDocument/hover",
                            json!({
                                "textDocument": { "uri": uri },
                                "position": { "line": line, "character": character }
                            }),
                            timeout_seconds,
                        )
                        .await?;
                        let parsed: Option<Hover> = serde_json::from_value(result)
                            .map_err(|err| Error::Tool(format!("invalid hover response: {err}")))?;
                        json!({
                            "operation": operation.as_str(),
                            "server_id": handle.server_id,
                            "server_command": handle.command,
                            "workspace_root": handle.workspace_root,
                            "hover": parsed,
                        })
                    }
                    _ => unreachable!(),
                }
            }
        };

        if let Some(tx) = tx {
            let _ = tx.try_send(Event::ToolOutput {
                tool: self.config.name.clone(),
                stdout_chunk: format!("lsp {} completed\n", operation.as_str()),
                stderr_chunk: String::new(),
            });
        }

        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: payload.to_string(),
        })
    }

    fn path_to_lsp_uri(path: &Path, is_dir: bool) -> Result<lsp_types::Uri> {
        let as_url = if is_dir {
            Url::from_directory_path(path)
        } else {
            Url::from_file_path(path)
        }
        .map_err(|_| {
            Error::Tool(format!(
                "failed to convert path '{}' to file uri",
                path.display()
            ))
        })?;
        lsp_types::Uri::from_str(as_url.as_str())
            .map_err(|err| Error::Tool(format!("invalid LSP uri '{}': {err}", as_url.as_str())))
    }
}

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "LSP navigation and symbol search over running language servers"
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
                let _ = tx.try_send(Event::Error(format!("lsp tool failed: {err}")));
            }
        }

        result
    }
}
