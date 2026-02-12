use async_trait::async_trait;
use globset::Glob;
use ignore::WalkBuilder;
use regex::RegexBuilder;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct GrepTool {
    config: ToolConfig,
    schema: Value,
}

#[derive(Debug, Clone)]
struct GrepArgs {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    ignore_case: bool,
    max_results: usize,
    before_context: usize,
    after_context: usize,
}

#[derive(Debug, Clone)]
struct GrepMatch {
    file: String,
    line: usize,
    text: String,
    before: Vec<String>,
    after: Vec<String>,
}

impl GrepTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string" },
                "glob": { "type": "string" },
                "ignore_case": { "type": "boolean" },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 10000 },
                "before_context": { "type": "integer", "minimum": 0, "maximum": 20 },
                "after_context": { "type": "integer", "minimum": 0, "maximum": 20 }
            },
            "required": ["pattern"]
        });

        Self { config, schema }
    }

    fn parse_args(&self, args: &Value) -> Result<GrepArgs> {
        let pattern = args
            .get("pattern")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::Tool("missing 'pattern' argument".to_owned()))?
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
        let ignore_case = args
            .get("ignore_case")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(200)
            .clamp(1, 10_000) as usize;
        let before_context = args
            .get("before_context")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .clamp(0, 20) as usize;
        let after_context = args
            .get("after_context")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .clamp(0, 20) as usize;

        Ok(GrepArgs {
            pattern,
            path,
            glob,
            ignore_case,
            max_results,
            before_context,
            after_context,
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

    fn resolve_search_root(
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
                "search path '{}' is outside tool working directory '{}'; use a path within the workspace",
                resolved.display(),
                workspace.display()
            )));
        }
        Ok(resolved)
    }

    fn build_regex(pattern: &str, ignore_case: bool) -> Result<regex::Regex> {
        RegexBuilder::new(pattern)
            .case_insensitive(ignore_case)
            .build()
            .map_err(|err| Error::Tool(format!("invalid regex pattern '{pattern}': {err}")))
    }

    fn collect_lines_with_context(
        file: &Path,
        regex: &regex::Regex,
        before_context: usize,
        after_context: usize,
        max_remaining: usize,
    ) -> Vec<GrepMatch> {
        let Ok(content) = fs::read_to_string(file) else {
            return Vec::new();
        };
        let lines = content.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
        let mut matches = Vec::new();

        for (idx, line) in lines.iter().enumerate() {
            if !regex.is_match(line) {
                continue;
            }

            let start = idx.saturating_sub(before_context);
            let end = usize::min(lines.len(), idx + 1 + after_context);
            let before = lines[start..idx].to_vec();
            let after = if idx + 1 < end {
                lines[idx + 1..end].to_vec()
            } else {
                Vec::new()
            };
            matches.push(GrepMatch {
                file: file.to_string_lossy().to_string(),
                line: idx + 1,
                text: line.clone(),
                before,
                after,
            });

            if matches.len() >= max_remaining {
                break;
            }
        }

        matches
    }

    fn run_search(&self, args: GrepArgs, context: &ToolExecutionContext) -> Result<Vec<GrepMatch>> {
        let root = self.resolve_search_root(context, args.path.as_deref())?;
        let regex = Self::build_regex(&args.pattern, args.ignore_case)?;

        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .ignore(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .parents(true)
            .max_filesize(Some(4 * 1024 * 1024));

        let glob_matcher = if let Some(glob) = args.glob.as_deref() {
            Some(
                Glob::new(glob)
                    .map_err(|err| Error::Tool(format!("invalid glob '{glob}': {err}")))?
                    .compile_matcher(),
            )
        } else {
            None
        };

        let mut results = Vec::new();
        for entry in builder.build() {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            if let Some(globset) = glob_matcher.as_ref() {
                let rel = path.strip_prefix(&root).unwrap_or(path);
                if !globset.is_match(rel) {
                    continue;
                }
            }

            let remaining = args.max_results.saturating_sub(results.len());
            if remaining == 0 {
                break;
            }

            let file_matches = Self::collect_lines_with_context(
                path,
                &regex,
                args.before_context,
                args.after_context,
                remaining,
            );
            if !file_matches.is_empty() {
                results.extend(file_matches);
            }
        }

        Ok(results)
    }

    async fn run_search_async(
        &self,
        args: GrepArgs,
        context: &ToolExecutionContext,
    ) -> Result<Vec<GrepMatch>> {
        let tool = self.clone();
        let context = context.clone();
        tokio::task::spawn_blocking(move || tool.run_search(args, &context))
            .await
            .map_err(|err| Error::Tool(format!("grep task failed: {err}")))?
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Search file content with regex and context lines"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let parsed = self.parse_args(&args)?;
        let matches = self.run_search_async(parsed, context).await?;
        let payload = json!({
            "count": matches.len(),
            "matches": matches
                .iter()
                .map(|m| json!({
                    "file": m.file,
                    "line": m.line,
                    "text": m.text,
                    "before": m.before,
                    "after": m.after,
                }))
                .collect::<Vec<_>>(),
        });
        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: serde_json::to_string(&payload)
                .unwrap_or_else(|_| "{\"count\":0,\"matches\":[]}".to_owned()),
        })
    }

    async fn stream_execute(
        &self,
        args: Value,
        tx: mpsc::Sender<Event>,
        context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let parsed = self.parse_args(&args)?;
        let tool_name = self.name().to_owned();

        let _ = tx.try_send(Event::ToolStarted {
            tool: tool_name.clone(),
            args: args.clone(),
        });

        let matches = self.run_search_async(parsed, context).await?;
        for matched in &matches {
            let _ = tx.try_send(Event::ToolOutput {
                tool: tool_name.clone(),
                stdout_chunk: format!("{}:{}:{}\n", matched.file, matched.line, matched.text),
                stderr_chunk: String::new(),
            });
        }

        let payload = json!({
            "count": matches.len(),
            "matches": matches
                .iter()
                .map(|m| json!({
                    "file": m.file,
                    "line": m.line,
                    "text": m.text,
                    "before": m.before,
                    "after": m.after,
                }))
                .collect::<Vec<_>>(),
        });

        let _ = tx.try_send(Event::ToolCompleted {
            tool: tool_name,
            exit_code: 0,
        });

        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: serde_json::to_string(&payload)
                .unwrap_or_else(|_| "{\"count\":0,\"matches\":[]}".to_owned()),
        })
    }
}
