use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::auth::SubscriptionAuthManager;
use crate::error::{Error, Result};
use crate::providers::http_client::{append_extra_headers, build_client};
use crate::providers::retry::{send_with_retry, RetryPolicy};
use crate::providers::streaming::spawn_sse_stream;
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};

#[derive(Debug, Clone)]
pub struct AnthropicProviderOptions {
    pub timeout_ms: u64,
    pub api_version: String,
    pub betas: Vec<String>,
    pub extra_headers: Vec<(String, String)>,
    pub retry_policy: RetryPolicy,
}

#[derive(Clone)]
pub enum AnthropicAuth {
    ApiKey {
        token: String,
    },
    Subscription {
        manager: Arc<SubscriptionAuthManager>,
    },
}

impl std::fmt::Debug for AnthropicAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey { .. } => f
                .debug_struct("ApiKey")
                .field("token", &"<redacted>")
                .finish(),
            Self::Subscription { manager } => f
                .debug_struct("Subscription")
                .field("provider", &manager.provider_name())
                .finish(),
        }
    }
}

impl Default for AnthropicProviderOptions {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            api_version: "2023-06-01".to_owned(),
            betas: Vec::new(),
            extra_headers: Vec::new(),
            retry_policy: RetryPolicy::default(),
        }
    }
}

#[derive(Clone)]
pub struct AnthropicProvider {
    name: String,
    model: String,
    messages_endpoint: String,
    count_tokens_endpoint: String,
    client: reqwest::Client,
    auth: AnthropicAuth,
    retry_policy: RetryPolicy,
}

impl std::fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("name", &self.name)
            .field("model", &self.model)
            .field("messages_endpoint", &self.messages_endpoint)
            .field("count_tokens_endpoint", &self.count_tokens_endpoint)
            .field("auth", &self.auth)
            .field("client", &"<reqwest::Client>")
            .finish()
    }
}

impl AnthropicProvider {
    pub fn new(
        name: String,
        model: String,
        auth: AnthropicAuth,
        base_url: String,
        options: AnthropicProviderOptions,
    ) -> Result<Self> {
        let headers = Self::build_headers(&options)?;
        let client = build_client(headers, options.timeout_ms, "Anthropic")?;

        let base = base_url.trim_end_matches('/');
        Ok(Self {
            name,
            model,
            messages_endpoint: format!("{base}/messages"),
            count_tokens_endpoint: format!("{base}/messages/count_tokens"),
            client,
            auth,
            retry_policy: options.retry_policy,
        })
    }

    fn build_headers(options: &AnthropicProviderOptions) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_str(&options.api_version).map_err(|err| {
                Error::Config(format!("invalid anthropic-version header value: {err}"))
            })?,
        );

        if !options.betas.is_empty() {
            headers.insert(
                HeaderName::from_static("anthropic-beta"),
                HeaderValue::from_str(&options.betas.join(",")).map_err(|err| {
                    Error::Config(format!("invalid anthropic-beta header value: {err}"))
                })?,
            );
        }

        append_extra_headers(&mut headers, &options.extra_headers, "Anthropic")?;

        Ok(headers)
    }

    async fn apply_auth(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder> {
        match &self.auth {
            AnthropicAuth::ApiKey { token } => {
                let value = HeaderValue::from_str(token).map_err(|err| {
                    Error::Provider(format!("invalid anthropic x-api-key header value: {err}"))
                })?;
                Ok(builder.header(HeaderName::from_static("x-api-key"), value))
            }
            AnthropicAuth::Subscription { manager } => {
                let token = manager.ensure_access_token().await?;
                let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|err| {
                    Error::Provider(format!(
                        "invalid anthropic subscription authorization header value: {err}"
                    ))
                })?;
                Ok(builder.header(HeaderName::from_static("authorization"), value))
            }
        }
    }

    fn build_messages_payload(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
        stream: bool,
    ) -> Value {
        let (system, anthropic_messages) = split_messages(messages);
        let mut payload = json!({
            "model": self.model,
            "messages": anthropic_messages,
            "max_tokens": options.max_tokens,
            "stream": stream,
        });

        if let Some(system) = system {
            payload["system"] = json!(system);
        }

        payload["temperature"] = json!(options.temperature);
        if let Some(top_p) = options.top_p {
            payload["top_p"] = json!(top_p);
        }
        if let Some(top_k) = options.top_k {
            payload["top_k"] = json!(top_k);
        }
        if let Some(stop_sequences) = &options.stop_sequences {
            payload["stop_sequences"] = json!(stop_sequences);
        }

        payload
    }

    fn build_count_tokens_payload(&self, messages: &[ChatMessage]) -> Value {
        let (system, anthropic_messages) = split_messages(messages);
        let mut payload = json!({
            "model": self.model,
            "messages": anthropic_messages,
        });

        if let Some(system) = system {
            payload["system"] = json!(system);
        }

        payload
    }

    fn extract_text_output(response: AnthropicMessagesResponse) -> String {
        response
            .content
            .into_iter()
            .filter_map(|block| {
                if block.block_type == "text" {
                    block.text
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicInputMessage {
    role: String,
    content: Vec<AnthropicInputBlock>,
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicInputBlock {
    #[serde(rename = "type")]
    block_type: &'static str,
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessagesResponse {
    content: Vec<AnthropicOutputBlock>,
}

#[derive(Debug, Deserialize)]
struct AnthropicOutputBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CountTokensResponse {
    input_tokens: usize,
}

fn split_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<AnthropicInputMessage>) {
    let mut system_parts = Vec::new();
    let mut anthropic_messages = Vec::new();

    for message in messages {
        if message.role == "system" {
            system_parts.push(message.content.clone());
            continue;
        }

        let role = match message.role.as_str() {
            "assistant" => "assistant".to_owned(),
            "user" => "user".to_owned(),
            "tool" => "user".to_owned(),
            _ => "user".to_owned(),
        };

        let text = match message.role.as_str() {
            "tool" => format!("[tool output]\n{}", message.content),
            "assistant" | "user" => message.content.clone(),
            other => format!("[{other}]\n{}", message.content),
        };

        anthropic_messages.push(AnthropicInputMessage {
            role,
            content: vec![AnthropicInputBlock {
                block_type: "text",
                text,
            }],
        });
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    (system, anthropic_messages)
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn generate(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
    ) -> Result<String> {
        let request = self
            .apply_auth(
                self.client
                    .post(&self.messages_endpoint)
                    .json(&self.build_messages_payload(messages, options, false)),
            )
            .await?;

        let response = send_with_retry(request, &self.retry_policy, "Anthropic request").await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Anthropic request failed with status {status}: {body}"
            )));
        }

        let payload: AnthropicMessagesResponse = response.json().await.map_err(|err| {
            Error::Provider(format!("failed to parse Anthropic response payload: {err}"))
        })?;

        Ok(Self::extract_text_output(payload))
    }

    async fn stream_generate(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
    ) -> Result<mpsc::Receiver<String>> {
        let request = self
            .apply_auth(
                self.client
                    .post(&self.messages_endpoint)
                    .json(&self.build_messages_payload(messages, options, true)),
            )
            .await?;

        let response =
            send_with_retry(request, &self.retry_policy, "Anthropic stream request").await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Anthropic stream request failed with status {status}: {body}"
            )));
        }

        Ok(spawn_sse_stream(response))
    }

    async fn count_tokens(&self, messages: &[ChatMessage]) -> Result<usize> {
        let request = self
            .apply_auth(
                self.client
                    .post(&self.count_tokens_endpoint)
                    .json(&self.build_count_tokens_payload(messages)),
            )
            .await?;

        let response = send_with_retry(
            request,
            &self.retry_policy,
            "Anthropic count_tokens request",
        )
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Anthropic count_tokens request failed with status {status}: {body}"
            )));
        }

        let payload: CountTokensResponse = response.json().await.map_err(|err| {
            Error::Provider(format!(
                "failed to parse Anthropic count_tokens response: {err}"
            ))
        })?;

        Ok(payload.input_tokens)
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_functions(&self) -> bool {
        false
    }
}
