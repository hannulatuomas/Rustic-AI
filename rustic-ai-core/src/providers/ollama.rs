use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::Duration;

use crate::error::{Error, Result};
use crate::providers::retry::{send_with_retry, RetryPolicy};
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone)]
pub struct OllamaProviderOptions {
    pub timeout_ms: u64,
    pub extra_headers: Vec<(String, String)>,
    pub retry_policy: RetryPolicy,
}

impl Default for OllamaProviderOptions {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            extra_headers: Vec::new(),
            retry_policy: RetryPolicy::default(),
        }
    }
}

#[derive(Clone)]
pub struct OllamaProvider {
    name: String,
    model: String,
    chat_endpoint: String,
    tokenize_endpoint: String,
    client: reqwest::Client,
    retry_policy: RetryPolicy,
}

impl std::fmt::Debug for OllamaProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaProvider")
            .field("name", &self.name)
            .field("model", &self.model)
            .field("chat_endpoint", &self.chat_endpoint)
            .field("tokenize_endpoint", &self.tokenize_endpoint)
            .field("client", &"<reqwest::Client>")
            .finish()
    }
}

impl OllamaProvider {
    pub fn new(
        name: String,
        model: String,
        api_key: String,
        base_url: String,
        options: OllamaProviderOptions,
    ) -> Result<Self> {
        let timeout_ms = if options.timeout_ms == 0 {
            DEFAULT_TIMEOUT_MS
        } else {
            options.timeout_ms
        };

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let auth = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|err| Error::Config(format!("invalid Ollama authorization header: {err}")))?;
        headers.insert(AUTHORIZATION, auth);

        for (name, value) in options.extra_headers {
            let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
                Error::Config(format!("invalid Ollama custom header name '{name}': {err}"))
            })?;
            let header_value = HeaderValue::from_str(&value).map_err(|err| {
                Error::Config(format!(
                    "invalid Ollama custom header value for '{name}': {err}"
                ))
            })?;
            headers.insert(header_name, header_value);
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|err| Error::Provider(format!("failed to build Ollama client: {err}")))?;

        let base = base_url.trim_end_matches('/');
        Ok(Self {
            name,
            model,
            chat_endpoint: format!("{base}/api/chat"),
            tokenize_endpoint: format!("{base}/api/tokenize"),
            client,
            retry_policy: options.retry_policy,
        })
    }

    fn payload(&self, messages: &[ChatMessage], options: &GenerateOptions, stream: bool) -> Value {
        json!({
            "model": self.model,
            "messages": messages,
            "stream": stream,
            "options": {
                "temperature": options.temperature,
                "num_predict": options.max_tokens,
                "top_p": options.top_p,
                "top_k": options.top_k,
                "repeat_penalty": options.frequency_penalty,
                "presence_penalty": options.presence_penalty,
                "stop": options.stop_sequences,
            }
        })
    }
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    content: String,
}

#[async_trait]
impl ModelProvider for OllamaProvider {
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
                .post(&self.chat_endpoint)
                .json(&self.payload(messages, options, false)),
            &self.retry_policy,
            "Ollama request",
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Ollama request failed with status {status}: {body}"
            )));
        }

        let payload: OllamaChatResponse = response
            .json()
            .await
            .map_err(|err| Error::Provider(format!("failed to parse Ollama response: {err}")))?;

        Ok(payload.message.content)
    }

    async fn stream_generate(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
    ) -> Result<mpsc::Receiver<String>> {
        let response = send_with_retry(
            self.client
                .post(&self.chat_endpoint)
                .json(&self.payload(messages, options, true)),
            &self.retry_policy,
            "Ollama stream request",
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Ollama stream request failed with status {status}: {body}"
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
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    match serde_json::from_str::<Value>(trimmed) {
                        Ok(value) => {
                            if value.get("done").and_then(Value::as_bool) == Some(true) {
                                return;
                            }

                            if let Some(text) = value
                                .pointer("/message/content")
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned)
                            {
                                if tx.send(text).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Err(err) => {
                            let _ = tx
                                .send(format!(
                                    "[stream error] failed to parse stream JSON line: {err}"
                                ))
                                .await;
                            return;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn count_tokens(&self, messages: &[ChatMessage]) -> Result<usize> {
        let prompt = messages
            .iter()
            .map(|message| format!("{}: {}", message.role, message.content))
            .collect::<Vec<_>>()
            .join("\n\n");

        let response = send_with_retry(
            self.client.post(&self.tokenize_endpoint).json(&json!({
                "model": self.model,
                "prompt": prompt,
            })),
            &self.retry_policy,
            "Ollama count_tokens request",
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Ollama count_tokens request failed with status {status}: {body}"
            )));
        }

        let payload: Value = response.json().await.map_err(|err| {
            Error::Provider(format!(
                "failed to parse Ollama count_tokens response: {err}"
            ))
        })?;

        if let Some(count) = payload
            .get("tokens")
            .and_then(Value::as_array)
            .map(std::vec::Vec::len)
        {
            return Ok(count);
        }

        if let Some(count) = payload.get("count").and_then(Value::as_u64) {
            return Ok(count as usize);
        }

        Err(Error::Provider(
            "Ollama count_tokens response missing token count fields".to_owned(),
        ))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_functions(&self) -> bool {
        false
    }
}
