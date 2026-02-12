use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct BracketValidatorTool {
    config: ToolConfig,
    schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Json,
    Text,
    Lsp,
}

impl OutputFormat {
    fn parse(value: Option<&str>) -> Result<Self> {
        match value.unwrap_or("json").trim().to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "text" => Ok(Self::Text),
            "lsp" => Ok(Self::Lsp),
            other => Err(Error::Tool(format!(
                "unsupported format '{other}' (expected json|text|lsp)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum DetailLevel {
    Summary,
    Detailed,
}

impl DetailLevel {
    fn parse(value: Option<&str>) -> Result<Self> {
        match value
            .unwrap_or("detailed")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "summary" => Ok(Self::Summary),
            "detailed" => Ok(Self::Detailed),
            other => Err(Error::Tool(format!(
                "unsupported detail_level '{other}' (expected summary|detailed)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ValidationError {
    kind: String,
    line: usize,
    column: usize,
    found: Option<String>,
    expected: Option<String>,
    message: String,
}

#[derive(Debug, Clone)]
struct StackEntry {
    ch: char,
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, Serialize)]
struct ValidationResult {
    valid: bool,
    error_count: usize,
    truncated: bool,
    errors: Vec<ValidationError>,
    diagnostics: Vec<LspDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
struct LspDiagnostic {
    line: usize,
    column: usize,
    severity: String,
    code: String,
    message: String,
}

#[derive(Debug, Clone)]
struct LanguageRules {
    line_comments: Vec<&'static str>,
    block_comment: Option<(&'static str, &'static str)>,
    string_delimiters: Vec<char>,
}

impl BracketValidatorTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "content": { "type": "string" },
                "language": { "type": "string", "default": "auto" },
                "path": { "type": "string" },
                "max_errors": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 50 },
                "detail_level": { "type": "string", "enum": ["summary", "detailed"], "default": "detailed" },
                "format": { "type": "string", "enum": ["json", "text", "lsp"], "default": "json" },
                "angle_brackets": { "type": "boolean", "default": true }
            },
            "required": ["content"]
        });

        Self { config, schema }
    }

    fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool(format!("missing '{key}' argument")))
    }

    fn language_rules(language: &str) -> LanguageRules {
        match language.to_ascii_lowercase().as_str() {
            "rust" | "c" | "cpp" | "c++" | "java" | "javascript" | "js" | "typescript" | "ts"
            | "go" => LanguageRules {
                line_comments: vec!["//"],
                block_comment: Some(("/*", "*/")),
                string_delimiters: vec!['"', '\'', '`'],
            },
            "python" | "bash" | "shell" | "ruby" => LanguageRules {
                line_comments: vec!["#"],
                block_comment: None,
                string_delimiters: vec!['"', '\''],
            },
            "html" | "xml" => LanguageRules {
                line_comments: vec![],
                block_comment: Some(("<!--", "-->")),
                string_delimiters: vec!['"', '\''],
            },
            _ => LanguageRules {
                line_comments: vec!["//", "#"],
                block_comment: Some(("/*", "*/")),
                string_delimiters: vec!['"', '\'', '`'],
            },
        }
    }

    fn starts_with(chars: &[char], index: usize, needle: &str) -> bool {
        let mut i = index;
        for expected in needle.chars() {
            if i >= chars.len() || chars[i] != expected {
                return false;
            }
            i += 1;
        }
        true
    }

    fn closing_for(opening: char) -> Option<char> {
        match opening {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '<' => Some('>'),
            _ => None,
        }
    }

    fn is_opening(ch: char, angle_enabled: bool) -> bool {
        matches!(ch, '(' | '[' | '{') || (angle_enabled && ch == '<')
    }

    fn is_closing(ch: char, angle_enabled: bool) -> bool {
        matches!(ch, ')' | ']' | '}') || (angle_enabled && ch == '>')
    }

    fn validate_content(
        &self,
        content: &str,
        language: &str,
        max_errors: usize,
        angle_enabled: bool,
    ) -> ValidationResult {
        let rules = Self::language_rules(language);
        let chars = content.chars().collect::<Vec<_>>();
        let mut errors = Vec::new();
        let mut stack = Vec::<StackEntry>::new();

        let mut idx = 0usize;
        let mut line = 1usize;
        let mut col = 1usize;

        let mut in_line_comment = false;
        let mut in_block_comment = false;
        let mut in_string: Option<char> = None;
        let mut escaped = false;

        while idx < chars.len() {
            let ch = chars[idx];

            if in_line_comment {
                if ch == '\n' {
                    in_line_comment = false;
                }
            } else if in_block_comment {
                if let Some((_, end)) = rules.block_comment {
                    if Self::starts_with(&chars, idx, end) {
                        for _ in 0..end.chars().count() {
                            if chars[idx] == '\n' {
                                line += 1;
                                col = 0;
                            }
                            idx += 1;
                            col += 1;
                        }
                        in_block_comment = false;
                        continue;
                    }
                }
            } else if let Some(delim) = in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == delim {
                    in_string = None;
                }
            } else {
                let mut entered_comment = false;
                for marker in &rules.line_comments {
                    if Self::starts_with(&chars, idx, marker) {
                        in_line_comment = true;
                        entered_comment = true;
                        break;
                    }
                }
                if entered_comment {
                    idx += 1;
                    col += 1;
                    continue;
                }

                if let Some((start, _)) = rules.block_comment {
                    if Self::starts_with(&chars, idx, start) {
                        in_block_comment = true;
                        idx += 1;
                        col += 1;
                        continue;
                    }
                }

                if rules.string_delimiters.contains(&ch) {
                    in_string = Some(ch);
                } else if Self::is_opening(ch, angle_enabled) {
                    stack.push(StackEntry {
                        ch,
                        line,
                        column: col,
                    });
                } else if Self::is_closing(ch, angle_enabled) {
                    if let Some(opening) = stack.pop() {
                        let expected = Self::closing_for(opening.ch).unwrap_or(ch);
                        if ch != expected {
                            errors.push(ValidationError {
                                kind: "mismatched_pair".to_string(),
                                line,
                                column: col,
                                found: Some(ch.to_string()),
                                expected: Some(expected.to_string()),
                                message: format!(
                                    "mismatched closing bracket '{ch}', expected '{expected}' for '{}' opened at {}:{}",
                                    opening.ch, opening.line, opening.column
                                ),
                            });
                        }
                    } else {
                        errors.push(ValidationError {
                            kind: "missing_opening".to_string(),
                            line,
                            column: col,
                            found: Some(ch.to_string()),
                            expected: None,
                            message: format!(
                                "closing bracket '{ch}' has no matching opening bracket"
                            ),
                        });
                    }
                }
            }

            if errors.len() >= max_errors {
                break;
            }

            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            idx += 1;
        }

        if errors.len() < max_errors {
            for opening in stack {
                if let Some(expected) = Self::closing_for(opening.ch) {
                    errors.push(ValidationError {
                        kind: "missing_closing".to_string(),
                        line: opening.line,
                        column: opening.column,
                        found: Some(opening.ch.to_string()),
                        expected: Some(expected.to_string()),
                        message: format!(
                            "opening bracket '{}' at {}:{} is missing closing '{}'",
                            opening.ch, opening.line, opening.column, expected
                        ),
                    });
                }
                if errors.len() >= max_errors {
                    break;
                }
            }
        }

        let diagnostics = errors
            .iter()
            .map(|error| LspDiagnostic {
                line: error.line,
                column: error.column,
                severity: "error".to_string(),
                code: error.kind.clone(),
                message: error.message.clone(),
            })
            .collect::<Vec<_>>();

        ValidationResult {
            valid: errors.is_empty(),
            error_count: errors.len(),
            truncated: errors.len() >= max_errors,
            errors,
            diagnostics,
        }
    }

    fn render_text(path: Option<&str>, result: &ValidationResult, detailed: bool) -> String {
        if result.valid {
            return "OK: bracket validation passed".to_string();
        }

        let mut lines = vec![format!("FAIL: {} bracket issue(s)", result.error_count)];
        if detailed {
            for error in &result.errors {
                lines.push(format!(
                    "{}:{}: {}",
                    error.line, error.column, error.message
                ));
            }
        }

        if let Some(path) = path {
            lines.insert(0, format!("file: {path}"));
        }

        lines.join("\n")
    }

    fn render_lsp(path: Option<&str>, result: &ValidationResult) -> String {
        if result.valid {
            return String::new();
        }

        let file = path.unwrap_or("<input>");
        result
            .diagnostics
            .iter()
            .map(|d| format!("{file}:{}:{}: {} [{}]", d.line, d.column, d.message, d.code))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn run_operation(&self, args: &Value) -> Result<Value> {
        let content = Self::required_string(args, "content")?;
        let language = args
            .get("language")
            .and_then(Value::as_str)
            .unwrap_or("auto");
        let path = args.get("path").and_then(Value::as_str);
        let max_errors = args
            .get("max_errors")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .clamp(1, 1000) as usize;
        let format = OutputFormat::parse(args.get("format").and_then(Value::as_str))?;
        let detail_level = DetailLevel::parse(args.get("detail_level").and_then(Value::as_str))?;
        let angle_enabled = args
            .get("angle_brackets")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let result = self.validate_content(content, language, max_errors, angle_enabled);

        let rendered = match format {
            OutputFormat::Json => None,
            OutputFormat::Text => Some(Self::render_text(
                path,
                &result,
                matches!(detail_level, DetailLevel::Detailed),
            )),
            OutputFormat::Lsp => Some(Self::render_lsp(path, &result)),
        };

        Ok(json!({
            "valid": result.valid,
            "error_count": result.error_count,
            "truncated": result.truncated,
            "errors": if matches!(detail_level, DetailLevel::Detailed) {
                json!(result.errors)
            } else {
                json!([])
            },
            "diagnostics": result.diagnostics,
            "format": match format {
                OutputFormat::Json => "json",
                OutputFormat::Text => "text",
                OutputFormat::Lsp => "lsp",
            },
            "output": rendered,
        }))
    }

    async fn execute_operation(&self, args: Value) -> Result<ToolResult> {
        let payload = self.run_operation(&args)?;
        Ok(ToolResult {
            success: true,
            exit_code: Some(if payload["valid"].as_bool().unwrap_or(false) {
                0
            } else {
                1
            }),
            output: payload.to_string(),
        })
    }
}

#[async_trait]
impl Tool for BracketValidatorTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Validate (), {}, [], <> nesting with language-aware comment/string handling"
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
                    stdout_chunk: "bracket validation completed\n".to_string(),
                    stderr_chunk: String::new(),
                });
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: tool_name,
                    exit_code: tool_result.exit_code.unwrap_or(0),
                });
            }
            Err(err) => {
                let _ = tx.try_send(Event::Error(format!("bracket validator failed: {err}")));
            }
        }

        result
    }
}
