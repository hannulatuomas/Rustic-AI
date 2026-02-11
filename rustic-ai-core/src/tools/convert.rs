use async_trait::async_trait;
use csv::{ReaderBuilder, WriterBuilder};
use pulldown_cmark::{html::push_html, Options, Parser};
use quick_xml::{de::from_str as xml_from_str, se::to_string as xml_to_string};
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

#[derive(Debug, Clone)]
pub struct ConvertTool {
    config: ToolConfig,
    schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataFormat {
    Json,
    Yaml,
    Xml,
    Csv,
    Markdown,
    Html,
}

impl DataFormat {
    fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "yaml" | "yml" => Ok(Self::Yaml),
            "xml" => Ok(Self::Xml),
            "csv" => Ok(Self::Csv),
            "markdown" | "md" => Ok(Self::Markdown),
            "html" => Ok(Self::Html),
            other => Err(Error::Tool(format!(
                "unsupported format '{other}' (expected json|yaml|xml|csv|markdown|html)"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Xml => "xml",
            Self::Csv => "csv",
            Self::Markdown => "markdown",
            Self::Html => "html",
        }
    }
}

impl ConvertTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "input": { "type": "string" },
                "from": { "type": "string", "enum": ["json", "yaml", "xml", "csv", "markdown", "html"] },
                "to": { "type": "string", "enum": ["json", "yaml", "xml", "csv", "markdown", "html"] }
            },
            "required": ["input", "from", "to"]
        });
        Self { config, schema }
    }

    fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool(format!("missing '{key}' argument")))
    }

    fn parse_structured(from: DataFormat, input: &str) -> Result<Value> {
        match from {
            DataFormat::Json => serde_json::from_str::<Value>(input)
                .map_err(|err| Error::Tool(format!("invalid JSON input: {err}"))),
            DataFormat::Yaml => serde_yaml::from_str::<Value>(input)
                .map_err(|err| Error::Tool(format!("invalid YAML input: {err}"))),
            DataFormat::Xml => xml_from_str::<Value>(input)
                .map_err(|err| Error::Tool(format!("invalid XML input: {err}"))),
            DataFormat::Csv => {
                let mut reader = ReaderBuilder::new()
                    .has_headers(true)
                    .from_reader(input.as_bytes());
                let headers = reader
                    .headers()
                    .map_err(|err| Error::Tool(format!("invalid CSV headers: {err}")))?
                    .iter()
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();

                let mut rows = Vec::new();
                for record in reader.records() {
                    let record =
                        record.map_err(|err| Error::Tool(format!("invalid CSV row: {err}")))?;
                    let mut row = Map::new();
                    for (idx, value) in record.iter().enumerate() {
                        let key = headers
                            .get(idx)
                            .cloned()
                            .unwrap_or_else(|| format!("col_{idx}"));
                        row.insert(key, Value::String(value.to_owned()));
                    }
                    rows.push(Value::Object(row));
                }
                Ok(Value::Array(rows))
            }
            DataFormat::Markdown | DataFormat::Html => Err(Error::Tool(
                "markdown/html input should be converted directly without structured parse"
                    .to_owned(),
            )),
        }
    }

    fn render_structured(to: DataFormat, value: &Value) -> Result<String> {
        match to {
            DataFormat::Json => serde_json::to_string_pretty(value)
                .map_err(|err| Error::Tool(format!("failed to encode JSON output: {err}"))),
            DataFormat::Yaml => serde_yaml::to_string(value)
                .map_err(|err| Error::Tool(format!("failed to encode YAML output: {err}"))),
            DataFormat::Xml => xml_to_string(value)
                .map_err(|err| Error::Tool(format!("failed to encode XML output: {err}"))),
            DataFormat::Csv => {
                let rows = value.as_array().ok_or_else(|| {
                    Error::Tool("CSV output requires structured array input".to_owned())
                })?;
                let mut field_order = Vec::new();
                let mut seen = std::collections::HashSet::new();
                for row in rows {
                    let Some(obj) = row.as_object() else {
                        return Err(Error::Tool(
                            "CSV output requires each row to be an object".to_owned(),
                        ));
                    };
                    for key in obj.keys() {
                        if seen.insert(key.clone()) {
                            field_order.push(key.clone());
                        }
                    }
                }

                let mut writer = WriterBuilder::new().from_writer(Vec::new());
                writer
                    .write_record(&field_order)
                    .map_err(|err| Error::Tool(format!("failed to write CSV header: {err}")))?;
                for row in rows {
                    let Some(obj) = row.as_object() else {
                        return Err(Error::Tool(
                            "CSV output requires each row to be an object".to_owned(),
                        ));
                    };
                    let record = field_order
                        .iter()
                        .map(|key| {
                            obj.get(key)
                                .map(|v| {
                                    if let Some(s) = v.as_str() {
                                        s.to_owned()
                                    } else {
                                        v.to_string()
                                    }
                                })
                                .unwrap_or_default()
                        })
                        .collect::<Vec<_>>();
                    writer
                        .write_record(&record)
                        .map_err(|err| Error::Tool(format!("failed to write CSV record: {err}")))?;
                }
                let bytes = writer
                    .into_inner()
                    .map_err(|err| Error::Tool(format!("failed to finalize CSV output: {err}")))?;
                String::from_utf8(bytes)
                    .map_err(|err| Error::Tool(format!("CSV output was not utf-8: {err}")))
            }
            DataFormat::Markdown | DataFormat::Html => Err(Error::Tool(
                "markdown/html output should be converted directly without structured renderer"
                    .to_owned(),
            )),
        }
    }

    fn convert_direct(input: &str, from: DataFormat, to: DataFormat) -> Result<String> {
        match (from, to) {
            (DataFormat::Markdown, DataFormat::Html) => {
                let parser = Parser::new_ext(input, Options::all());
                let mut html = String::new();
                push_html(&mut html, parser);
                Ok(html)
            }
            (DataFormat::Html, DataFormat::Markdown) => Ok(html2md::parse_html(input)),
            _ => Err(Error::Tool(format!(
                "direct conversion from {} to {} is not supported",
                from.as_str(),
                to.as_str()
            ))),
        }
    }

    fn run_conversion(&self, args: &Value) -> Result<Value> {
        let input = Self::required_string(args, "input")?;
        let from = DataFormat::parse(Self::required_string(args, "from")?)?;
        let to = DataFormat::parse(Self::required_string(args, "to")?)?;

        let output = if from == to {
            input.to_owned()
        } else if matches!(
            (from, to),
            (DataFormat::Markdown, DataFormat::Html) | (DataFormat::Html, DataFormat::Markdown)
        ) {
            Self::convert_direct(input, from, to)?
        } else {
            let structured = Self::parse_structured(from, input)?;
            Self::render_structured(to, &structured)?
        };

        Ok(json!({
            "from": from.as_str(),
            "to": to.as_str(),
            "input_length": input.len(),
            "output_length": output.len(),
            "output": output,
        }))
    }

    async fn execute_operation(&self, args: Value) -> Result<ToolResult> {
        let payload = self.run_conversion(&args)?;
        Ok(ToolResult {
            success: true,
            exit_code: Some(0),
            output: payload.to_string(),
        })
    }
}

#[async_trait]
impl Tool for ConvertTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Convert data across json/yaml/xml/csv/markdown/html formats"
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
                let _ = tx.try_send(Event::Error(format!("convert tool failed: {err}")));
            }
        }

        result
    }
}
