use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::Duration;

use crate::error::{Error, Result};
use crate::providers::retry::{send_with_retry, RetryPolicy};
use crate::providers::streaming::{parse_sse_line, StreamEvent};
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone)]
pub struct GrokProviderOptions {
    pub timeout_ms: u64,
    pub extra_headers: Vec<(String, String)>,
    pub retry_policy: RetryPolicy,
}

impl Default for GrokProviderOptions {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            extra_headers: Vec::new(),
            retry_policy: RetryPolicy::default(),
        }
    }
}

#[derive(Clone)]
pub struct GrokProvider {
    name: String,
    model: String,
    endpoint: String,
    tokenize_endpoint: String,
    client: reqwest::Client,
    retry_policy: RetryPolicy,
}

impl std::fmt::Debug for GrokProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrokProvider")
            .field("name", &self.name)
            .field("model", &self.model)
            .field("endpoint", &self.endpoint)
            .field("tokenize_endpoint", &self.tokenize_endpoint)
            .field("client", &"<reqwest::Client>")
            .finish()
    }
}

impl GrokProvider {
    pub fn new(
        name: String,
        model: String,
        api_key: String,
        base_url: String,
        options: GrokProviderOptions,
    ) -> Result<Self> {
        let timeout_ms = if options.timeout_ms == 0 {
            DEFAULT_TIMEOUT_MS
        } else {
            options.timeout_ms
        };

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let auth = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|err| Error::Config(format!("invalid Grok authorization header: {err}")))?;
        headers.insert(AUTHORIZATION, auth);

        for (name, value) in options.extra_headers {
            let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
                Error::Config(format!("invalid Grok custom header name '{name}': {err}"))
            })?;
            let header_value = HeaderValue::from_str(&value).map_err(|err| {
                Error::Config(format!(
                    "invalid Grok custom header value for '{name}': {err}"
                ))
            })?;
            headers.insert(header_name, header_value);
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|err| Error::Provider(format!("failed to build Grok client: {err}")))?;

        let base = base_url.trim_end_matches('/');
        Ok(Self {
            name,
            model,
            endpoint: format!("{base}/chat/completions"),
            tokenize_endpoint: format!("{base}/tokenize-text"),
            client,
            retry_policy: options.retry_policy,
        })
    }

    fn build_payload(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
        stream: bool,
    ) -> Value {
        let mut payload = json!({
            "model": self.model,
            "messages": messages,
            "temperature": options.temperature,
            "max_tokens": options.max_tokens,
            "stream": stream,
        });

        if let Some(top_p) = options.top_p {
            payload["top_p"] = json!(top_p);
        }
        if let Some(top_k) = options.top_k {
            payload["top_k"] = json!(top_k);
        }
        if let Some(stop_sequences) = &options.stop_sequences {
            payload["stop"] = json!(stop_sequences);
        }
        if let Some(presence_penalty) = options.presence_penalty {
            payload["presence_penalty"] = json!(presence_penalty);
        }
        if let Some(frequency_penalty) = options.frequency_penalty {
            payload["frequency_penalty"] = json!(frequency_penalty);
        }

        payload
    }
}

#[derive(Debug, Deserialize)]
struct GrokResponse {
    choices: Vec<GrokChoice>,
}

#[derive(Debug, Deserialize)]
struct GrokChoice {
    message: ChatMessage,
}

#[async_trait]
impl ModelProvider for GrokProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn generate(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
    ) -> Result<String> {
        let response = send_with_retry(
            self.client
                .post(&self.endpoint)
                .json(&self.build_payload(messages, options, false)),
            &self.retry_policy,
            "Grok request",
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Grok request failed with status {status}: {body}"
            )));
        }

        let payload: GrokResponse = response
            .json()
            .await
            .map_err(|err| Error::Provider(format!("failed to parse Grok response: {err}")))?;

        payload
            .choices
            .first()
            .map(|choice| choice.message.content.clone())
            .ok_or_else(|| Error::Provider("Grok returned no choices".to_owned()))
    }

    async fn stream_generate(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
    ) -> Result<mpsc::Receiver<String>> {
        let response = send_with_retry(
            self.client
                .post(&self.endpoint)
                .json(&self.build_payload(messages, options, true)),
            &self.retry_policy,
            "Grok stream request",
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Grok stream request failed with status {status}: {body}"
            )));
        }

        let (tx, rx) = mpsc::channel(256);
        let mut stream = response.bytes_stream();

        tokio::spawn(async move {
            let mut line_buffer = String::new();

            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        let _ = tx
                            .send(format!("[stream error] failed to read stream chunk: {err}"))
                            .await;
                        break;
                    }
                };

                let decoded = match std::str::from_utf8(&chunk) {
                    Ok(text) => text,
                    Err(err) => {
                        let _ = tx
                            .send(format!(
                                "[stream error] invalid UTF-8 in stream chunk: {err}"
                            ))
                            .await;
                        break;
                    }
                };

                line_buffer.push_str(decoded);

                while let Some(idx) = line_buffer.find('\n') {
                    let line = line_buffer[..idx].trim_end_matches('\r').to_owned();
                    line_buffer.drain(..=idx);

                    match parse_sse_line(&line) {
                        Some(StreamEvent::Text(text)) => {
                            if tx.send(text).await.is_err() {
                                return;
                            }
                        }
                        Some(StreamEvent::Error(err)) => {
                            let _ = tx.send(format!("[stream error] {err}")).await;
                            return;
                        }
                        Some(StreamEvent::Done) => return,
                        None => {}
                    }
                }
            }

            if !line_buffer.is_empty() {
                match parse_sse_line(line_buffer.trim_end_matches('\r')) {
                    Some(StreamEvent::Text(text)) => {
                        let _ = tx.send(text).await;
                    }
                    Some(StreamEvent::Error(err)) => {
                        let _ = tx.send(format!("[stream error] {err}")).await;
                    }
                    Some(StreamEvent::Done) | None => {}
                }
            }
        });

        Ok(rx)
    }

    async fn count_tokens(&self, messages: &[ChatMessage]) -> Result<usize> {
        let text = messages
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n\n");

        let response = send_with_retry(
            self.client
                .post(&self.tokenize_endpoint)
                .json(&json!({ "text": text })),
            &self.retry_policy,
            "Grok count_tokens request",
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Grok count_tokens request failed with status {status}: {body}"
            )));
        }

        let payload: Value = response.json().await.map_err(|err| {
            Error::Provider(format!("failed to parse Grok count_tokens response: {err}"))
        })?;

        if let Some(count) = payload.get("total_tokens").and_then(Value::as_u64) {
            return Ok(count as usize);
        }
        if let Some(count) = payload.get("token_count").and_then(Value::as_u64) {
            return Ok(count as usize);
        }
        if let Some(count) = payload.get("num_tokens").and_then(Value::as_u64) {
            return Ok(count as usize);
        }
        if let Some(count) = payload
            .get("tokens")
            .and_then(Value::as_array)
            .map(std::vec::Vec::len)
        {
            return Ok(count);
        }

        Err(Error::Provider(
            "Grok count_tokens response missing token count fields".to_owned(),
        ))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_functions(&self) -> bool {
        false
    }
}
