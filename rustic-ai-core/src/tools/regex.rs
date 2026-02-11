use async_trait::async_trait;
use regex::{Captures, Regex, RegexBuilder};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct RegexTool {
    config: ToolConfig,
    schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegexOperation {
    Match,
    Replace,
    FindAll,
}

impl RegexOperation {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "match" => Ok(Self::Match),
            "replace" => Ok(Self::Replace),
            "find_all" => Ok(Self::FindAll),
            other => Err(Error::Tool(format!(
                "unsupported regex operation '{other}' (expected match|replace|find_all)"
            ))),
        }
    }
}

impl RegexTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "enum": ["match", "replace", "find_all"] },
                "pattern": { "type": "string" },
                "input": { "type": "string" },
                "flags": { "type": "string", "description": "Regex flags: i,m,s,U,u,x" },
                "replacement": { "type": "string" },
                "max_matches": { "type": "integer", "minimum": 1, "maximum": 10000 }
            },
            "required": ["operation", "pattern", "input"]
        });

        Self { config, schema }
    }

    fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool(format!("missing '{key}' argument")))
    }

    fn build_regex(pattern: &str, flags: &str) -> Result<Regex> {
        let mut builder = RegexBuilder::new(pattern);
        for flag in flags.chars() {
            match flag {
                'i' => {
                    builder.case_insensitive(true);
                }
                'm' => {
                    builder.multi_line(true);
                }
                's' => {
                    builder.dot_matches_new_line(true);
                }
                'U' => {
                    builder.swap_greed(true);
                }
                'u' => {
                    builder.unicode(true);
                }
                'x' => {
                    builder.ignore_whitespace(true);
                }
                _ => {
                    return Err(Error::Tool(format!(
                        "unsupported regex flag '{}' (supported: i,m,s,U,u,x)",
                        flag
                    )));
                }
            }
        }

        builder
            .build()
            .map_err(|err| Error::Tool(format!("invalid regex pattern '{pattern}': {err}")))
    }

    fn captures_to_json(regex: &Regex, caps: &Captures<'_>) -> Value {
        let mut groups = Vec::new();
        for (index, maybe_name) in regex.capture_names().enumerate() {
            if let Some(matched) = caps.get(index) {
                groups.push(json!({
                    "index": index,
                    "name": maybe_name,
                    "start": matched.start(),
                    "end": matched.end(),
                    "text": matched.as_str(),
                }));
            } else {
                groups.push(json!({
                    "index": index,
                    "name": maybe_name,
                    "start": Value::Null,
                    "end": Value::Null,
                    "text": Value::Null,
                }));
            }
        }
        Value::Array(groups)
    }

    fn run_operation(&self, args: &Value) -> Result<Value> {
        let operation = RegexOperation::parse(Self::required_string(args, "operation")?)?;
        let pattern = Self::required_string(args, "pattern")?;
        let input = Self::required_string(args, "input")?;
        let flags = args.get("flags").and_then(Value::as_str).unwrap_or("");
        let regex = Self::build_regex(pattern, flags)?;

        match operation {
            RegexOperation::Match => {
                if let Some(caps) = regex.captures(input) {
                    let m = caps.get(0);
                    Ok(json!({
                        "operation": "match",
                        "matched": true,
                        "match": {
                            "start": m.map(|m| m.start()).unwrap_or_default(),
                            "end": m.map(|m| m.end()).unwrap_or_default(),
                            "text": m.map(|m| m.as_str()).unwrap_or_default(),
                            "groups": Self::captures_to_json(&regex, &caps),
                        }
                    }))
                } else {
                    Ok(json!({
                        "operation": "match",
                        "matched": false,
                        "match": Value::Null,
                    }))
                }
            }
            RegexOperation::FindAll => {
                let max_matches = args
                    .get("max_matches")
                    .and_then(Value::as_u64)
                    .unwrap_or(1000)
                    .clamp(1, 10_000) as usize;
                let mut matches = Vec::new();
                for caps in regex.captures_iter(input).take(max_matches) {
                    let m = caps.get(0);
                    matches.push(json!({
                        "start": m.map(|m| m.start()).unwrap_or_default(),
                        "end": m.map(|m| m.end()).unwrap_or_default(),
                        "text": m.map(|m| m.as_str()).unwrap_or_default(),
                        "groups": Self::captures_to_json(&regex, &caps),
                    }));
                }
                Ok(json!({
                    "operation": "find_all",
                    "count": matches.len(),
                    "matches": matches,
                }))
            }
            RegexOperation::Replace => {
                let replacement = Self::required_string(args, "replacement")?;
                let replacements = regex.find_iter(input).count();
                let output = regex.replace_all(input, replacement).to_string();
                Ok(json!({
                    "operation": "replace",
                    "replacement": replacement,
                    "replacements": replacements,
                    "output": output,
                }))
            }
        }
    }

    async fn execute_operation(&self, args: Value) -> Result<ToolResult> {
        let payload = self.run_operation(&args)?;
        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: payload.to_string(),
        })
    }
}

#[async_trait]
impl Tool for RegexTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Regex matching, replacement, and structured capture extraction"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        self.execute_operation(args).await
    }

    async fn stream_execute(
        &self,
        args: Value,
        tx: mpsc::Sender<Event>,
        _context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let tool_name = self.name().to_owned();
        let _ = tx.try_send(Event::ToolStarted {
            tool: tool_name.clone(),
            args: args.clone(),
        });

        let result = self.execute_operation(args).await;
        match &result {
            Ok(tool_result) => {
                let _ = tx.try_send(Event::ToolOutput {
                    tool: tool_name.clone(),
                    stdout_chunk: "regex operation completed\n".to_owned(),
                    stderr_chunk: String::new(),
                });
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: tool_name,
                    exit_code: tool_result.exit_code.unwrap_or(0),
                });
            }
            Err(err) => {
                let _ = tx.try_send(Event::Error(format!("regex tool failed: {err}")));
            }
        }

        result
    }
}
