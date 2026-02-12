use globset::Glob;
use ignore::WalkBuilder;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct CodeSearchTool {
    config: ToolConfig,
    schema: Value,
}

#[derive(Debug, Clone)]
struct CodeSearchArgs {
    query: String,
    path: Option<String>,
    glob: Option<String>,
    max_results: usize,
    max_file_bytes: u64,
    max_snippet_chars: usize,
}

#[derive(Debug, Clone)]
struct CodeSearchMatch {
    file: String,
    score: f64,
    snippet: String,
}

impl CodeSearchTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "path": { "type": "string" },
                "glob": { "type": "string" },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 500 },
                "max_file_bytes": { "type": "integer", "minimum": 1024, "maximum": 8388608 },
                "max_snippet_chars": { "type": "integer", "minimum": 40, "maximum": 800 }
            },
            "required": ["query"]
        });

        Self { config, schema }
    }

    fn parse_args(&self, args: &Value) -> Result<CodeSearchArgs> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::Tool("missing 'query' argument".to_owned()))?
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
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(40)
            .clamp(1, 500) as usize;
        let max_file_bytes = args
            .get("max_file_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(512 * 1024)
            .clamp(1024, 8 * 1024 * 1024);
        let max_snippet_chars = args
            .get("max_snippet_chars")
            .and_then(Value::as_u64)
            .unwrap_or(220)
            .clamp(40, 800) as usize;

        Ok(CodeSearchArgs {
            query,
            path,
            glob,
            max_results,
            max_file_bytes,
            max_snippet_chars,
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
                "search path '{}' is outside tool working directory '{}'",
                resolved.display(),
                workspace.display()
            )));
        }
        Ok(resolved)
    }

    fn tokenize(query: &str) -> Vec<String> {
        query
            .split_whitespace()
            .map(|token| token.trim().to_ascii_lowercase())
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>()
    }

    fn score_content(path: &Path, content: &str, tokens: &[String]) -> (f64, Option<usize>) {
        if tokens.is_empty() {
            return (0.0, None);
        }

        let lower = content.to_ascii_lowercase();
        let lower_path = path.to_string_lossy().to_ascii_lowercase();
        let mut score = 0.0;
        let mut first_index = None;

        for token in tokens {
            let mut search_from = 0usize;
            let mut token_hits = 0usize;
            while let Some(position) = lower[search_from..].find(token) {
                token_hits += 1;
                let absolute = search_from + position;
                if first_index.is_none() {
                    first_index = Some(absolute);
                }
                search_from = absolute.saturating_add(token.len());
                if token_hits >= 32 {
                    break;
                }
            }

            if token_hits > 0 {
                score += (token_hits as f64).min(8.0);
            }
            if lower_path.contains(token) {
                score += 3.0;
            }
        }

        let lines = content.lines().count().max(1);
        let density_penalty = (lines as f64).log10().max(1.0);
        (score / density_penalty, first_index)
    }

    fn snippet_around(content: &str, index: usize, max_chars: usize) -> String {
        let chars = content.chars().collect::<Vec<_>>();
        if chars.is_empty() {
            return String::new();
        }
        let center = index.min(chars.len().saturating_sub(1));
        let half = max_chars / 2;
        let start = center.saturating_sub(half);
        let end = usize::min(chars.len(), start + max_chars);
        chars[start..end].iter().collect::<String>()
    }

    fn run_search(
        &self,
        args: CodeSearchArgs,
        context: &ToolExecutionContext,
    ) -> Result<Vec<CodeSearchMatch>> {
        let root = self.resolve_search_root(context, args.path.as_deref())?;
        let tokens = Self::tokenize(&args.query);
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .ignore(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .parents(true)
            .max_filesize(Some(args.max_file_bytes));

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

            let Ok(content) = fs::read_to_string(path) else {
                continue;
            };

            let (score, first_index) = Self::score_content(path, &content, &tokens);
            if score <= 0.0 {
                continue;
            }

            let snippet = first_index
                .map(|idx| Self::snippet_around(&content, idx, args.max_snippet_chars))
                .unwrap_or_default();
            results.push(CodeSearchMatch {
                file: path.to_string_lossy().to_string(),
                score,
                snippet,
            });
        }

        results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.file.cmp(&right.file))
        });
        results.truncate(args.max_results);
        Ok(results)
    }
}

#[async_trait::async_trait]
impl Tool for CodeSearchTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Search code with relevance-ranked snippets"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let parsed = self.parse_args(&args)?;
        let tool = self.clone();
        let context = context.clone();
        let matches = tokio::task::spawn_blocking(move || tool.run_search(parsed, &context))
            .await
            .map_err(|err| Error::Tool(format!("code_search task failed: {err}")))??;

        let payload = json!({
            "count": matches.len(),
            "matches": matches.iter().map(|m| json!({
                "file": m.file,
                "score": m.score,
                "snippet": m.snippet,
            })).collect::<Vec<_>>(),
        });
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
                let parsed = serde_json::from_str::<Value>(&payload.output).ok();
                if let Some(count) = parsed
                    .as_ref()
                    .and_then(|value| value.get("count"))
                    .and_then(Value::as_u64)
                {
                    let _ = tx.try_send(Event::ToolOutput {
                        tool: tool_name.clone(),
                        stdout_chunk: format!("code_search matches: {count}\n"),
                        stderr_chunk: String::new(),
                    });
                }
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: tool_name,
                    exit_code: payload.exit_code.unwrap_or(0),
                });
            }
            Err(err) => {
                let _ = tx.try_send(Event::Error(format!("code_search failed: {err}")));
            }
        }

        result
    }
}
