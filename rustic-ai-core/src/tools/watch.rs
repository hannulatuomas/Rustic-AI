use globset::Glob;
use ignore::WalkBuilder;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tokio::sync::{mpsc, Mutex};

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
struct FileFingerprint {
    size: u64,
    modified_epoch_millis: u128,
}

#[derive(Debug, Clone)]
struct WatchArgs {
    operation: String,
    key: String,
    path: Option<String>,
    glob: Option<String>,
    max_files: usize,
    max_changes: usize,
    update_snapshot: bool,
}

#[derive(Debug, Clone)]
pub struct WatchTool {
    config: ToolConfig,
    schema: Value,
    snapshots: std::sync::Arc<Mutex<HashMap<String, HashMap<String, FileFingerprint>>>>,
}

impl WatchTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["snapshot", "changes", "clear"]
                },
                "key": { "type": "string", "description": "Logical snapshot key" },
                "path": { "type": "string" },
                "glob": { "type": "string" },
                "max_files": { "type": "integer", "minimum": 1, "maximum": 200000 },
                "max_changes": { "type": "integer", "minimum": 1, "maximum": 10000 },
                "update_snapshot": { "type": "boolean" }
            },
            "required": ["operation", "key"]
        });

        Self {
            config,
            schema,
            snapshots: std::sync::Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn parse_args(&self, args: &Value) -> Result<WatchArgs> {
        let operation = args
            .get("operation")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::Tool("missing 'operation' argument".to_owned()))?
            .to_ascii_lowercase();
        if !matches!(operation.as_str(), "snapshot" | "changes" | "clear") {
            return Err(Error::Tool(format!(
                "unsupported watch operation '{}'",
                operation
            )));
        }

        let key = args
            .get("key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::Tool("missing 'key' argument".to_owned()))?
            .to_owned();

        let path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let glob = args
            .get("glob")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let max_files = args
            .get("max_files")
            .and_then(Value::as_u64)
            .unwrap_or(20_000)
            .clamp(1, 200_000) as usize;
        let max_changes = args
            .get("max_changes")
            .and_then(Value::as_u64)
            .unwrap_or(1_000)
            .clamp(1, 10_000) as usize;
        let update_snapshot = args
            .get("update_snapshot")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        Ok(WatchArgs {
            operation,
            key,
            path,
            glob,
            max_files,
            max_changes,
            update_snapshot,
        })
    }

    fn canonicalize(path: &Path) -> Result<PathBuf> {
        fs::canonicalize(path).map_err(|err| {
            Error::Tool(format!(
                "failed to resolve path '{}': {err}",
                path.display()
            ))
        })
    }

    fn resolve_root(
        &self,
        context: &ToolExecutionContext,
        raw_path: Option<&str>,
    ) -> Result<PathBuf> {
        let workspace = Self::canonicalize(&context.working_directory)?;
        let requested = match raw_path {
            Some(path) => {
                let candidate = PathBuf::from(path);
                if candidate.is_absolute() {
                    candidate
                } else {
                    workspace.join(candidate)
                }
            }
            None => workspace.clone(),
        };
        let resolved = Self::canonicalize(&requested)?;
        if !resolved.starts_with(&workspace) {
            return Err(Error::Tool(format!(
                "watch path '{}' is outside tool working directory '{}'",
                resolved.display(),
                workspace.display()
            )));
        }
        Ok(resolved)
    }

    fn collect_snapshot(
        &self,
        context: &ToolExecutionContext,
        path: Option<&str>,
        glob: Option<&str>,
        max_files: usize,
    ) -> Result<HashMap<String, FileFingerprint>> {
        let root = self.resolve_root(context, path)?;
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .ignore(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .parents(true);

        let glob_matcher = if let Some(glob) = glob {
            Some(
                Glob::new(glob)
                    .map_err(|err| Error::Tool(format!("invalid glob '{glob}': {err}")))?
                    .compile_matcher(),
            )
        } else {
            None
        };

        let mut snapshot = HashMap::new();
        for entry in builder.build() {
            if snapshot.len() >= max_files {
                break;
            }
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(matcher) = glob_matcher.as_ref() {
                let rel = path.strip_prefix(&root).unwrap_or(path);
                if !matcher.is_match(rel) {
                    continue;
                }
            }

            let Ok(metadata) = fs::metadata(path) else {
                continue;
            };
            let modified_epoch_millis = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                .map(|value| value.as_millis())
                .unwrap_or(0);
            snapshot.insert(
                path.to_string_lossy().to_string(),
                FileFingerprint {
                    size: metadata.len(),
                    modified_epoch_millis,
                },
            );
        }

        Ok(snapshot)
    }

    fn diff_snapshots(
        before: &HashMap<String, FileFingerprint>,
        after: &HashMap<String, FileFingerprint>,
        max_changes: usize,
    ) -> (Vec<String>, Vec<String>, Vec<String>) {
        let before_paths = before.keys().cloned().collect::<HashSet<_>>();
        let after_paths = after.keys().cloned().collect::<HashSet<_>>();

        let mut created = after_paths
            .difference(&before_paths)
            .take(max_changes)
            .cloned()
            .collect::<Vec<_>>();
        created.sort();

        let mut deleted = before_paths
            .difference(&after_paths)
            .take(max_changes)
            .cloned()
            .collect::<Vec<_>>();
        deleted.sort();

        let mut modified = before_paths
            .intersection(&after_paths)
            .filter_map(|path| {
                let old = before.get(path)?;
                let new = after.get(path)?;
                ((old.size != new.size) || (old.modified_epoch_millis != new.modified_epoch_millis))
                    .then_some(path.clone())
            })
            .take(max_changes)
            .collect::<Vec<_>>();
        modified.sort();

        (created, modified, deleted)
    }
}

#[async_trait::async_trait]
impl Tool for WatchTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Capture filesystem snapshots and report changes"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let parsed = self.parse_args(&args)?;
        let context = context.clone();
        let tool = self.clone();
        let current_snapshot = tokio::task::spawn_blocking(move || {
            tool.collect_snapshot(
                &context,
                parsed.path.as_deref(),
                parsed.glob.as_deref(),
                parsed.max_files,
            )
            .map(|snapshot| (parsed, snapshot))
        })
        .await
        .map_err(|err| Error::Tool(format!("watch task failed: {err}")))??;

        let (parsed, snapshot) = current_snapshot;
        let mut snapshots = self.snapshots.lock().await;

        let payload = match parsed.operation.as_str() {
            "snapshot" => {
                snapshots.insert(parsed.key.clone(), snapshot.clone());
                json!({
                    "operation": "snapshot",
                    "key": parsed.key,
                    "files": snapshot.len(),
                })
            }
            "changes" => {
                let previous = snapshots.get(&parsed.key).cloned().unwrap_or_default();
                let (created, modified, deleted) =
                    Self::diff_snapshots(&previous, &snapshot, parsed.max_changes);
                if parsed.update_snapshot {
                    snapshots.insert(parsed.key.clone(), snapshot.clone());
                }
                json!({
                    "operation": "changes",
                    "key": parsed.key,
                    "baseline_files": previous.len(),
                    "current_files": snapshot.len(),
                    "created": created,
                    "modified": modified,
                    "deleted": deleted,
                })
            }
            "clear" => {
                snapshots.remove(&parsed.key);
                json!({
                    "operation": "clear",
                    "key": parsed.key,
                    "cleared": true,
                })
            }
            _ => {
                return Err(Error::Tool(format!(
                    "unsupported watch operation '{}'",
                    parsed.operation
                )));
            }
        };

        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: payload.to_string(),
        })
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

        let result = self.execute(args, context).await;
        match &result {
            Ok(payload) => {
                let _ = tx.try_send(Event::ToolOutput {
                    tool: tool_name.clone(),
                    stdout_chunk: format!("watch completed: {}\n", payload.output),
                    stderr_chunk: String::new(),
                });
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: tool_name,
                    exit_code: payload.exit_code.unwrap_or(0),
                });
            }
            Err(err) => {
                let _ = tx.try_send(Event::Error(format!("watch failed: {err}")));
            }
        }
        result
    }
}
