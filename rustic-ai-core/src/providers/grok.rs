use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::error::{Error, Result};
use crate::providers::http_client::{append_extra_headers, build_client};
use crate::providers::retry::{send_with_retry, RetryPolicy};
use crate::providers::streaming::spawn_sse_stream;
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};

#[derive(Debug, Clone)]
pub struct GrokProviderOptions {
    pub timeout_ms: u64,
    pub extra_headers: Vec<(String, String)>,
    pub retry_policy: RetryPolicy,
}

impl Default for GrokProviderOptions {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
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
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let auth = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|err| Error::Config(format!("invalid Grok authorization header: {err}")))?;
        headers.insert(AUTHORIZATION, auth);

        append_extra_headers(&mut headers, &options.extra_headers, "Grok")?;

        let client = build_client(headers, options.timeout_ms, "Grok")?;

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

        Ok(spawn_sse_stream(response))
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
