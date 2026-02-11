use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Duration;

use crate::auth::SubscriptionAuthManager;
use crate::error::{Error, Result};
use crate::providers::retry::{send_with_retry, RetryPolicy};
use crate::providers::streaming::spawn_sse_stream;
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Clone)]
pub enum OpenAiAuth {
    ApiKey {
        token: String,
    },
    Subscription {
        manager: Arc<SubscriptionAuthManager>,
        organization: Option<String>,
        project: Option<String>,
        account_id: Option<String>,
    },
}

impl std::fmt::Debug for OpenAiAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey { .. } => f
                .debug_struct("ApiKey")
                .field("token", &"<redacted>")
                .finish(),
            Self::Subscription {
                manager,
                organization,
                project,
                account_id,
                ..
            } => f
                .debug_struct("Subscription")
                .field("provider", &manager.provider_name())
                .field("organization", organization)
                .field("project", project)
                .field("account_id", account_id)
                .finish(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiProviderOptions {
    pub timeout_ms: u64,
    pub extra_headers: Vec<(String, String)>,
    pub retry_policy: RetryPolicy,
}

impl Default for OpenAiProviderOptions {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            extra_headers: Vec::new(),
            retry_policy: RetryPolicy::default(),
        }
    }
}

#[derive(Clone)]
pub struct OpenAiProvider {
    name: String,
    model: String,
    endpoint: String,
    client: reqwest::Client,
    auth: OpenAiAuth,
    subscription_headers: HeaderMap,
    retry_policy: RetryPolicy,
}

impl std::fmt::Debug for OpenAiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiProvider")
            .field("name", &self.name)
            .field("model", &self.model)
            .field("endpoint", &self.endpoint)
            .field("auth", &self.auth)
            .field("client", &"<reqwest::Client>")
            .finish()
    }
}

impl OpenAiProvider {
    pub fn new(
        name: String,
        model: String,
        auth: OpenAiAuth,
        base_url: String,
        options: OpenAiProviderOptions,
    ) -> Result<Self> {
        let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let timeout_ms = if options.timeout_ms == 0 {
            DEFAULT_TIMEOUT_MS
        } else {
            options.timeout_ms
        };

        let (headers, subscription_headers) = Self::build_headers(&auth, &options.extra_headers)?;
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|err| Error::Provider(format!("failed to build OpenAI client: {err}")))?;

        Ok(Self {
            name,
            model,
            endpoint,
            client,
            auth,
            subscription_headers,
            retry_policy: options.retry_policy,
        })
    }

    fn build_headers(
        auth: &OpenAiAuth,
        extra_headers: &[(String, String)],
    ) -> Result<(HeaderMap, HeaderMap)> {
        let mut headers = HeaderMap::new();
        let mut subscription_headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        match auth {
            OpenAiAuth::ApiKey { token } => {
                let auth_value = format!("Bearer {token}");
                let parsed = HeaderValue::from_str(&auth_value).map_err(|err| {
                    Error::Config(format!("invalid OpenAI API key header value: {err}"))
                })?;
                headers.insert(AUTHORIZATION, parsed);
            }
            OpenAiAuth::Subscription {
                organization,
                project,
                account_id,
                ..
            } => {
                if let Some(value) = organization {
                    let parsed = HeaderValue::from_str(value).map_err(|err| {
                        Error::Config(format!("invalid openai organization header value: {err}"))
                    })?;
                    subscription_headers
                        .insert(HeaderName::from_static("openai-organization"), parsed);
                }

                if let Some(value) = project {
                    let parsed = HeaderValue::from_str(value).map_err(|err| {
                        Error::Config(format!("invalid openai project header value: {err}"))
                    })?;
                    subscription_headers.insert(HeaderName::from_static("openai-project"), parsed);
                }

                if let Some(value) = account_id {
                    let parsed = HeaderValue::from_str(value).map_err(|err| {
                        Error::Config(format!("invalid chatgpt account header value: {err}"))
                    })?;
                    subscription_headers
                        .insert(HeaderName::from_static("chatgpt-account-id"), parsed);
                }
            }
        }

        for (name, value) in extra_headers {
            let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
                Error::Config(format!("invalid OpenAI custom header name '{name}': {err}"))
            })?;
            let header_value = HeaderValue::from_str(value).map_err(|err| {
                Error::Config(format!(
                    "invalid OpenAI custom header value for '{name}': {err}"
                ))
            })?;
            headers.insert(header_name, header_value);
        }

        Ok((headers, subscription_headers))
    }

    async fn apply_subscription_auth(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder> {
        let mut next = builder;
        if let OpenAiAuth::Subscription {
            manager,
            organization: _,
            project: _,
            account_id: _,
        } = &self.auth
        {
            let token = manager.ensure_access_token().await?;
            let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|err| {
                Error::Provider(format!("invalid subscription token header value: {err}"))
            })?;
            next = next.header(AUTHORIZATION, value);

            for (name, value) in &self.subscription_headers {
                next = next.header(name, value.clone());
            }
        }
        Ok(next)
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
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: ChatMessage,
}

#[async_trait]
impl ModelProvider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn generate(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
    ) -> Result<String> {
        let request = self
            .apply_subscription_auth(
                self.client
                    .post(&self.endpoint)
                    .json(&self.build_payload(messages, options, false)),
            )
            .await?;

        let response = send_with_retry(request, &self.retry_policy, "OpenAI request").await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "OpenAI request failed with status {status}: {body}"
            )));
        }

        let payload: OpenAiResponse = response
            .json()
            .await
            .map_err(|err| Error::Provider(format!("failed to parse OpenAI response: {err}")))?;

        payload
            .choices
            .first()
            .map(|choice| choice.message.content.clone())
            .ok_or_else(|| Error::Provider("OpenAI returned no choices".to_owned()))
    }

    async fn stream_generate(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
    ) -> Result<mpsc::Receiver<String>> {
        let request = self
            .apply_subscription_auth(
                self.client
                    .post(&self.endpoint)
                    .json(&self.build_payload(messages, options, true)),
            )
            .await?;

        let response =
            send_with_retry(request, &self.retry_policy, "OpenAI streaming request").await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "OpenAI streaming request failed with status {status}: {body}"
            )));
        }

        Ok(spawn_sse_stream(response))
    }

    async fn count_tokens(&self, messages: &[ChatMessage]) -> Result<usize> {
        let estimated = messages
            .iter()
            .map(|message| {
                let role = std::cmp::max(1, message.role.len() / 4);
                let content = std::cmp::max(1, message.content.chars().count() / 4);
                let name = message
                    .name
                    .as_ref()
                    .map(|value| std::cmp::max(1, value.len() / 4))
                    .unwrap_or(0);
                role + content + name + 4
            })
            .sum();
        Ok(estimated)
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_functions(&self) -> bool {
        true
    }
}
