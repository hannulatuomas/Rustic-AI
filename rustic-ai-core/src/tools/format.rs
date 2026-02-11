use async_trait::async_trait;
use quick_xml::events::Event as XmlEvent;
use quick_xml::{Reader, Writer};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Cursor;
use tokio::sync::mpsc;

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct FormatTool {
    config: ToolConfig,
    schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormatOperation {
    JsonPretty,
    JsonMinify,
    XmlPretty,
    XmlMinify,
}

impl FormatOperation {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "json" | "json_pretty" => Ok(Self::JsonPretty),
            "json_minify" | "minify_json" => Ok(Self::JsonMinify),
            "xml" | "xml_pretty" => Ok(Self::XmlPretty),
            "xml_minify" | "minify_xml" => Ok(Self::XmlMinify),
            other => Err(Error::Tool(format!(
                "unsupported format operation '{other}' (expected json|json_minify|xml|xml_minify)"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::JsonPretty => "json_pretty",
            Self::JsonMinify => "json_minify",
            Self::XmlPretty => "xml_pretty",
            Self::XmlMinify => "xml_minify",
        }
    }
}

impl FormatTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["json", "json_pretty", "json_minify", "xml", "xml_pretty", "xml_minify"]
                },
                "input": { "type": "string" },
                "indent": { "type": "integer", "minimum": 0, "maximum": 8 }
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

    fn json_pretty(input: &str, indent: usize) -> Result<String> {
        let value: Value = serde_json::from_str(input)
            .map_err(|err| Error::Tool(format!("invalid JSON input: {err}")))?;
        if indent == 2 {
            serde_json::to_string_pretty(&value)
                .map_err(|err| Error::Tool(format!("failed to format JSON: {err}")))
        } else {
            let spacer = " ".repeat(indent.max(1));
            let formatter = serde_json::ser::PrettyFormatter::with_indent(spacer.as_bytes());
            let mut out = Vec::new();
            let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
            value
                .serialize(&mut serializer)
                .map_err(|err| Error::Tool(format!("failed to format JSON: {err}")))?;
            String::from_utf8(out)
                .map_err(|err| Error::Tool(format!("formatted JSON was not utf-8: {err}")))
        }
    }

    fn json_minify(input: &str) -> Result<String> {
        let value: Value = serde_json::from_str(input)
            .map_err(|err| Error::Tool(format!("invalid JSON input: {err}")))?;
        serde_json::to_string(&value)
            .map_err(|err| Error::Tool(format!("failed to minify JSON: {err}")))
    }

    fn rewrite_xml(input: &str, pretty: bool, indent: usize) -> Result<String> {
        let mut reader = Reader::from_str(input);
        reader.config_mut().trim_text(false);
        let mut writer = if pretty {
            Writer::new_with_indent(
                Cursor::new(Vec::with_capacity(input.len() + 32)),
                b' ',
                indent.clamp(1, 8),
            )
        } else {
            Writer::new(Cursor::new(Vec::with_capacity(input.len())))
        };

        loop {
            match reader.read_event() {
                Ok(XmlEvent::Eof) => break,
                Ok(event) => writer
                    .write_event(event.into_owned())
                    .map_err(|err| Error::Tool(format!("failed writing XML output: {err}")))?,
                Err(err) => {
                    return Err(Error::Tool(format!("invalid XML input: {err}")));
                }
            }
        }

        let bytes = writer.into_inner().into_inner();
        String::from_utf8(bytes)
            .map_err(|err| Error::Tool(format!("formatted XML was not utf-8: {err}")))
    }

    fn run_operation(&self, args: &Value) -> Result<Value> {
        let operation = FormatOperation::parse(Self::required_string(args, "operation")?)?;
        let input = Self::required_string(args, "input")?;
        let indent = args
            .get("indent")
            .and_then(Value::as_u64)
            .unwrap_or(2)
            .clamp(0, 8) as usize;

        let output = match operation {
            FormatOperation::JsonPretty => Self::json_pretty(input, indent)?,
            FormatOperation::JsonMinify => Self::json_minify(input)?,
            FormatOperation::XmlPretty => Self::rewrite_xml(input, true, indent)?,
            FormatOperation::XmlMinify => Self::rewrite_xml(input, false, indent)?,
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
impl Tool for FormatTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Format and minify JSON/XML payloads"
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
                    stdout_chunk: "format operation completed\n".to_owned(),
                    stderr_chunk: String::new(),
                });
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: tool_name,
                    exit_code: tool_result.exit_code.unwrap_or(0),
                });
            }
            Err(err) => {
                let _ = tx.try_send(Event::Error(format!("format tool failed: {err}")));
            }
        }

        result
    }
}
