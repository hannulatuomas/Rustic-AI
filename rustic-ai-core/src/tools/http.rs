use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::config::schema::ToolConfig;
use crate::error::{Error, Result};
use crate::events::Event;
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

const MAX_RESPONSE_BYTES_DEFAULT: usize = 2 * 1024 * 1024;
const MAX_RESPONSE_BYTES_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HttpTool {
    config: ToolConfig,
    schema: Value,
    client: reqwest::Client,
}

impl HttpTool {
    pub fn new(config: ToolConfig) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "method": { "type": "string", "description": "HTTP method (GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS)" },
                "url": { "type": "string", "description": "Target URL" },
                "headers": { "type": "object", "additionalProperties": { "type": "string" } },
                "query": { "type": "object", "additionalProperties": { "type": "string" } },
                "body_json": { "type": ["object", "array", "string", "number", "boolean", "null"] },
                "body_text": { "type": "string" },
                "timeout_seconds": { "type": "integer", "minimum": 1 },
                "max_response_bytes": { "type": "integer", "minimum": 1 }
            },
            "required": ["method", "url"]
        });

        let client = reqwest::Client::builder()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            config,
            schema,
            client,
        }
    }

    fn method_from_args(&self, args: &Value) -> Result<Method> {
        let method = args
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool("missing 'method' argument".to_owned()))?
            .trim()
            .to_ascii_uppercase();

        Method::from_bytes(method.as_bytes())
            .map_err(|err| Error::Tool(format!("invalid HTTP method '{method}': {err}")))
    }

    fn url_from_args<'a>(&self, args: &'a Value) -> Result<&'a str> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Tool("missing 'url' argument".to_owned()))?;
        if url.trim().is_empty() {
            return Err(Error::Tool("url must be non-empty".to_owned()));
        }
        Ok(url)
    }

    fn parse_headers(&self, args: &Value) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let Some(raw_headers) = args.get("headers") else {
            return Ok(headers);
        };

        let Some(map) = raw_headers.as_object() else {
            return Err(Error::Tool("'headers' must be an object".to_owned()));
        };

        for (name, value) in map {
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|err| Error::Tool(format!("invalid header name '{name}': {err}")))?;
            let raw_value = value
                .as_str()
                .ok_or_else(|| Error::Tool(format!("header '{name}' value must be a string")))?;
            let header_value = HeaderValue::from_str(raw_value)
                .map_err(|err| Error::Tool(format!("invalid value for header '{name}': {err}")))?;
            headers.insert(header_name, header_value);
        }

        Ok(headers)
    }

    fn parse_query_pairs(&self, args: &Value) -> Result<Vec<(String, String)>> {
        let Some(raw_query) = args.get("query") else {
            return Ok(Vec::new());
        };

        let Some(map) = raw_query.as_object() else {
            return Err(Error::Tool("'query' must be an object".to_owned()));
        };

        let mut pairs = Vec::with_capacity(map.len());
        for (key, value) in map {
            let query_value = match value {
                Value::String(text) => text.clone(),
                _ => value.to_string(),
            };
            pairs.push((key.clone(), query_value));
        }
        Ok(pairs)
    }

    fn timeout_seconds_from_args(&self, args: &Value) -> Result<u64> {
        let configured = args
            .get("timeout_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(self.config.timeout_seconds);
        if configured == 0 {
            return Err(Error::Tool(
                "timeout_seconds must be greater than zero".to_owned(),
            ));
        }
        Ok(configured)
    }

    fn max_response_bytes_from_args(&self, args: &Value) -> Result<usize> {
        let configured = args
            .get("max_response_bytes")
            .and_then(Value::as_u64)
            .map(|value| {
                usize::try_from(value)
                    .map_err(|_| Error::Tool("max_response_bytes is too large".to_owned()))
            })
            .transpose()?
            .unwrap_or(MAX_RESPONSE_BYTES_DEFAULT)
            .min(MAX_RESPONSE_BYTES_LIMIT);

        if configured == 0 {
            return Err(Error::Tool(
                "max_response_bytes must be greater than zero".to_owned(),
            ));
        }

        Ok(configured)
    }

    fn apply_body(
        &self,
        mut request: reqwest::RequestBuilder,
        args: &Value,
    ) -> Result<reqwest::RequestBuilder> {
        if let Some(body_json) = args.get("body_json") {
            request = request.json(body_json);
            return Ok(request);
        }

        if let Some(body_text) = args.get("body_text").and_then(Value::as_str) {
            request = request.body(body_text.to_owned());
        }

        Ok(request)
    }

    async fn execute_request(&self, args: Value, tx: mpsc::Sender<Event>) -> Result<ToolResult> {
        let method = self.method_from_args(&args)?;
        let url = self.url_from_args(&args)?.to_owned();
        let headers = self.parse_headers(&args)?;
        let query = self.parse_query_pairs(&args)?;
        let timeout_seconds = self.timeout_seconds_from_args(&args)?;
        let max_response_bytes = self.max_response_bytes_from_args(&args)?;

        let mut request = self.client.request(method.clone(), &url);
        if !headers.is_empty() {
            request = request.headers(headers);
        }
        if !query.is_empty() {
            request = request.query(&query);
        }
        request = self.apply_body(request, &args)?;

        let response = tokio::time::timeout(Duration::from_secs(timeout_seconds), request.send())
            .await
            .map_err(|_| {
                Error::Tool(format!(
                    "HTTP request timed out after {timeout_seconds} seconds"
                ))
            })
            .and_then(|result| {
                result.map_err(|err| Error::Tool(format!("HTTP request failed: {err}")))
            })?;

        let status = response.status();
        let response_headers = response.headers().clone();

        let mut received = 0usize;
        let mut body_bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|err| Error::Tool(format!("failed reading response body: {err}")))?;
            received += chunk.len();
            if received > max_response_bytes {
                return Err(Error::Tool(format!(
                    "response body exceeded size limit ({max_response_bytes} bytes)"
                )));
            }

            if self.config.stream_output {
                let preview = String::from_utf8_lossy(&chunk);
                let _ = tx.try_send(Event::ToolOutput {
                    tool: self.config.name.clone(),
                    stdout_chunk: preview.into_owned(),
                    stderr_chunk: String::new(),
                });
            }

            body_bytes.extend_from_slice(&chunk);
        }

        let body_text = String::from_utf8_lossy(&body_bytes).into_owned();
        let content_type = response_headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();

        let parsed_json = if content_type
            .to_ascii_lowercase()
            .contains("application/json")
        {
            serde_json::from_slice::<Value>(&body_bytes).ok()
        } else {
            None
        };

        let mut response_header_map = serde_json::Map::new();
        for (name, value) in &response_headers {
            let key = name.as_str().to_ascii_lowercase();
            if key == "set-cookie" || key == "authorization" || key == "proxy-authorization" {
                continue;
            }
            if let Ok(text) = value.to_str() {
                response_header_map.insert(name.to_string(), Value::String(text.to_owned()));
            }
        }

        let output = json!({
            "method": method.as_str(),
            "url": url,
            "status": status.as_u16(),
            "ok": status.is_success(),
            "response_headers": response_header_map,
            "content_type": content_type,
            "body": body_text,
            "json": parsed_json
        });

        Ok(ToolResult {
            success: status.is_success(),
            exit_code: Some(if status.is_success() { 0 } else { 1 }),
            output: output.to_string(),
        })
    }
}

#[async_trait]
impl Tool for HttpTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        "HTTP client tool with bounded responses"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    async fn execute(&self, args: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let (dummy_tx, _) = mpsc::channel(1);
        self.execute_request(args, dummy_tx).await
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

        let result = self.execute_request(args, tx.clone()).await;

        match &result {
            Ok(tool_result) => {
                let _ = tx.try_send(Event::ToolCompleted {
                    tool: tool_name,
                    exit_code: tool_result.exit_code.unwrap_or(0),
                });
            }
            Err(err) => {
                let _ = tx.try_send(Event::Error(format!("http tool failed: {err}")));
            }
        }

        result
    }
}
