use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

const DEFAULT_MAX_BYTES: usize = 2 * 1024 * 1024;
const ABSOLUTE_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct WebFetchTool {
    config: ToolConfig,
    schema: Value,
    client: reqwest::Client,
}

impl WebFetchTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Target URL to fetch" },
                "format": { "type": "string", "enum": ["text", "markdown", "html"], "description": "Output format" },
                "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 120 },
                "max_bytes": { "type": "integer", "minimum": 1, "maximum": 8388608 }
            },
            "required": ["url"]
        });

        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("Rustic-AI-WebFetch/0.1 (+https://github.com)"),
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            config,
            schema,
            client,
        }
    }

    fn parse_url(args: &Value) -> Result<String> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::Tool("missing 'url' argument".to_owned()))?;

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(format!("https://{url}"));
        }
        Ok(url.to_owned())
    }

    fn parse_timeout(&self, args: &Value) -> u64 {
        args.get("timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(self.config.timeout_seconds)
            .clamp(1, 120)
    }

    fn parse_max_bytes(args: &Value) -> Result<usize> {
        let value = args
            .get("max_bytes")
            .and_then(Value::as_u64)
            .map(|size| {
                usize::try_from(size).map_err(|_| Error::Tool("max_bytes is too large".to_owned()))
            })
            .transpose()?
            .unwrap_or(DEFAULT_MAX_BYTES)
            .min(ABSOLUTE_MAX_BYTES);

        if value == 0 {
            return Err(Error::Tool(
                "max_bytes must be greater than zero".to_owned(),
            ));
        }

        Ok(value)
    }

    fn parse_format(args: &Value) -> Result<String> {
        let format = args
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("markdown")
            .trim()
            .to_ascii_lowercase();
        match format.as_str() {
            "text" | "markdown" | "html" => Ok(format),
            other => Err(Error::Tool(format!(
                "unsupported format '{other}' (expected text|markdown|html)"
            ))),
        }
    }

    fn html_to_text(html: &str) -> String {
        let mut text = String::with_capacity(html.len());
        let mut in_tag = false;
        for ch in html.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => text.push(ch),
                _ => {}
            }
        }
        text.replace("&nbsp;", " ")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .trim()
            .to_owned()
    }
}

#[async_trait::async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Fetch a single URL as text/markdown/html"
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
        _context: &ToolExecutionContext,
    ) -> Result<ToolResult> {
        let url = Self::parse_url(&args)?;
        let timeout_seconds = self.parse_timeout(&args);
        let max_bytes = Self::parse_max_bytes(&args)?;
        let output_format = Self::parse_format(&args)?;

        let response = timeout(
            Duration::from_secs(timeout_seconds),
            self.client.get(&url).send(),
        )
        .await
        .map_err(|_| {
            Error::Timeout(format!(
                "web_fetch timed out after {timeout_seconds} seconds"
            ))
        })?
        .map_err(|err| Error::Tool(format!("web_fetch request failed: {err}")))?;

        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = response
            .bytes()
            .await
            .map_err(|err| Error::Tool(format!("failed to read web_fetch body: {err}")))?;

        if body.len() > max_bytes {
            return Err(Error::Tool(format!(
                "web_fetch response exceeded max_bytes ({max_bytes})"
            )));
        }

        let raw_html = String::from_utf8_lossy(&body).into_owned();
        let rendered = match output_format.as_str() {
            "html" => raw_html.clone(),
            "text" | "markdown" => Self::html_to_text(&raw_html),
            _ => raw_html.clone(),
        };

        if self.config.stream_output {
            let preview: String = rendered.chars().take(512).collect();
            let _ = tx.try_send(Event::ToolOutput {
                tool: self.config.name.clone(),
                stdout_chunk: preview,
                stderr_chunk: String::new(),
            });
        }

        Ok(ToolResult {
            success: status.is_success(),
            exit_code: Some(if status.is_success() { 0 } else { 1 }),
            output: json!({
                "url": url,
                "status": status.as_u16(),
                "ok": status.is_success(),
                "content_type": content_type,
                "format": output_format,
                "content": rendered,
            })
            .to_string(),
        })
    }
}
