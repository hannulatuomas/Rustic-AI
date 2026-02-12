use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use reqwest::Url;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

const DEFAULT_MAX_LINKS: usize = 32;
const ABSOLUTE_MAX_LINKS: usize = 256;

#[derive(Debug, Clone)]
pub struct CrawlerTool {
    config: ToolConfig,
    schema: Value,
    client: reqwest::Client,
}

impl CrawlerTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Seed URL to crawl" },
                "max_links": { "type": "integer", "minimum": 1, "maximum": 256 },
                "same_host_only": { "type": "boolean", "description": "Keep only links on the same host" },
                "timeout_seconds": { "type": "integer", "minimum": 1, "maximum": 120 }
            },
            "required": ["url"]
        });

        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("Rustic-AI-Crawler/0.1 (+https://github.com)"),
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

    fn parse_url(args: &Value) -> Result<Url> {
        let raw = args
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::Tool("missing 'url' argument".to_owned()))?;
        let normalized = if raw.starts_with("http://") || raw.starts_with("https://") {
            raw.to_owned()
        } else {
            format!("https://{raw}")
        };
        Url::parse(&normalized).map_err(|err| Error::Tool(format!("invalid url '{raw}': {err}")))
    }

    fn parse_max_links(args: &Value) -> usize {
        args.get("max_links")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_MAX_LINKS as u64)
            .clamp(1, ABSOLUTE_MAX_LINKS as u64) as usize
    }

    fn parse_timeout(&self, args: &Value) -> u64 {
        args.get("timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(self.config.timeout_seconds)
            .clamp(1, 120)
    }

    fn parse_same_host_only(args: &Value) -> bool {
        args.get("same_host_only")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    fn extract_links(
        base: &Url,
        html: &str,
        same_host_only: bool,
        max_links: usize,
    ) -> Vec<String> {
        let mut links = BTreeSet::new();
        let mut index = 0usize;
        while let Some(found) = html[index..].find("href=") {
            let start = index + found + 5;
            let bytes = html.as_bytes();
            if start >= bytes.len() {
                break;
            }

            let quote = bytes[start] as char;
            if quote != '"' && quote != '\'' {
                index = start;
                continue;
            }

            let value_start = start + 1;
            if value_start >= bytes.len() {
                break;
            }

            let remainder = &html[value_start..];
            let Some(end_offset) = remainder.find(quote) else {
                break;
            };

            let raw = &remainder[..end_offset];
            index = value_start + end_offset + 1;

            if raw.is_empty() || raw.starts_with('#') || raw.starts_with("javascript:") {
                continue;
            }

            let resolved = if let Ok(url) = Url::parse(raw) {
                url
            } else if let Ok(url) = base.join(raw) {
                url
            } else {
                continue;
            };

            if same_host_only && resolved.host_str() != base.host_str() {
                continue;
            }

            links.insert(resolved.to_string());
            if links.len() >= max_links {
                break;
            }
        }

        links.into_iter().collect()
    }
}

#[async_trait::async_trait]
impl Tool for CrawlerTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "Crawl a page and extract bounded links"
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
        let max_links = Self::parse_max_links(&args);
        let same_host_only = Self::parse_same_host_only(&args);
        let timeout_seconds = self.parse_timeout(&args);

        let response = timeout(
            Duration::from_secs(timeout_seconds),
            self.client.get(url.clone()).send(),
        )
        .await
        .map_err(|_| Error::Timeout(format!("crawler timed out after {timeout_seconds} seconds")))?
        .map_err(|err| Error::Tool(format!("crawler request failed: {err}")))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| Error::Tool(format!("failed to read crawler response: {err}")))?;
        let links = Self::extract_links(&url, &body, same_host_only, max_links);

        if self.config.stream_output {
            let _ = tx.try_send(Event::ToolOutput {
                tool: self.config.name.clone(),
                stdout_chunk: format!("discovered {} links\n", links.len()),
                stderr_chunk: String::new(),
            });
        }

        Ok(ToolResult {
            success: status.is_success(),
            exit_code: Some(if status.is_success() { 0 } else { 1 }),
            output: json!({
                "url": url.to_string(),
                "status": status.as_u16(),
                "ok": status.is_success(),
                "same_host_only": same_host_only,
                "links": links,
            })
            .to_string(),
        })
    }
}
