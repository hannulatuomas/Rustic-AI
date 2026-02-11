use async_trait::async_trait;
use base64::Engine;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct EncodingTool {
    config: ToolConfig,
    schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EncodingOperation {
    Base64Encode,
    Base64Decode,
    UrlEncode,
    UrlDecode,
    HtmlEscape,
    HtmlUnescape,
    ValidateUtf8,
}

impl EncodingOperation {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "base64_encode" => Ok(Self::Base64Encode),
            "base64_decode" => Ok(Self::Base64Decode),
            "url_encode" => Ok(Self::UrlEncode),
            "url_decode" => Ok(Self::UrlDecode),
            "html_escape" => Ok(Self::HtmlEscape),
            "html_unescape" => Ok(Self::HtmlUnescape),
            "validate_utf8" => Ok(Self::ValidateUtf8),
            other => Err(Error::Tool(format!(
                "unsupported encoding operation '{other}' (expected base64_encode|base64_decode|url_encode|url_decode|html_escape|html_unescape|validate_utf8)"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Base64Encode => "base64_encode",
            Self::Base64Decode => "base64_decode",
            Self::UrlEncode => "url_encode",
            Self::UrlDecode => "url_decode",
            Self::HtmlEscape => "html_escape",
            Self::HtmlUnescape => "html_unescape",
            Self::ValidateUtf8 => "validate_utf8",
        }
    }
}

impl EncodingTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "base64_encode", "base64_decode", "url_encode", "url_decode", "html_escape", "html_unescape", "validate_utf8"
                    ]
                },
                "input": { "type": "string" }
            },
            "required": ["operation", "input"]
        });
        Self { config, schema }
    }

    fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool(format!("missing '{key}' argument")))
    }

    fn run_operation(&self, args: &Value) -> Result<Value> {
        let operation = EncodingOperation::parse(Self::required_string(args, "operation")?)?;
        let input = Self::required_string(args, "input")?;

        let output = match operation {
            EncodingOperation::Base64Encode => {
                base64::engine::general_purpose::STANDARD.encode(input.as_bytes())
            }
            EncodingOperation::Base64Decode => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(input)
                    .map_err(|err| Error::Tool(format!("invalid base64 input: {err}")))?;
                String::from_utf8(bytes)
                    .map_err(|err| Error::Tool(format!("decoded base64 is not UTF-8: {err}")))?
            }
            EncodingOperation::UrlEncode => {
                utf8_percent_encode(input, NON_ALPHANUMERIC).to_string()
            }
            EncodingOperation::UrlDecode => percent_encoding::percent_decode_str(input)
                .decode_utf8()
                .map_err(|err| Error::Tool(format!("invalid percent-encoded input: {err}")))?
                .to_string(),
            EncodingOperation::HtmlEscape => html_escape::encode_text(input).to_string(),
            EncodingOperation::HtmlUnescape => html_escape::decode_html_entities(input).to_string(),
            EncodingOperation::ValidateUtf8 => {
                let validation = String::from_utf8(input.as_bytes().to_vec());
                return Ok(json!({
                    "operation": operation.as_str(),
                    "valid": validation.is_ok(),
                    "error": validation.err().map(|err| err.to_string()),
                    "output": input,
                }));
            }
        };

        Ok(json!({
            "operation": operation.as_str(),
            "input_length": input.len(),
            "output_length": output.len(),
            "output": output,
        }))
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
impl Tool for EncodingTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Encoding/decoding utilities for base64, URL, HTML, and UTF-8"
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
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: tool_name,
                    exit_code: tool_result.exit_code.unwrap_or(0),
                });
            }
            Err(err) => {
                let _ = tx.try_send(Event::Error(format!("encoding tool failed: {err}")));
            }
        }

        result
    }
}
