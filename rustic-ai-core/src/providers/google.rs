use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Duration;

use crate::auth::SubscriptionAuthManager;
use crate::error::{Error, Result};
use crate::providers::retry::{send_with_retry, RetryPolicy};
use crate::providers::streaming::spawn_sse_stream_with_data_parser;
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Clone)]
pub enum GoogleAuth {
    ApiKey {
        token: String,
    },
    Subscription {
        manager: Arc<SubscriptionAuthManager>,
    },
}

impl std::fmt::Debug for GoogleAuth {
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

#[derive(Debug, Clone)]
pub struct GoogleProviderOptions {
    pub timeout_ms: u64,
    pub extra_headers: Vec<(String, String)>,
    pub retry_policy: RetryPolicy,
}

impl Default for GoogleProviderOptions {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            extra_headers: Vec::new(),
            retry_policy: RetryPolicy::default(),
        }
    }
}

#[derive(Clone)]
pub struct GoogleProvider {
    name: String,
    model: String,
    generate_endpoint: String,
    stream_endpoint: String,
    count_tokens_endpoint: String,
    client: reqwest::Client,
    auth: GoogleAuth,
    retry_policy: RetryPolicy,
}

impl std::fmt::Debug for GoogleProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleProvider")
            .field("name", &self.name)
            .field("model", &self.model)
            .field("generate_endpoint", &self.generate_endpoint)
            .field("stream_endpoint", &self.stream_endpoint)
            .field("count_tokens_endpoint", &self.count_tokens_endpoint)
            .field("auth", &self.auth)
            .field("client", &"<reqwest::Client>")
            .finish()
    }
}

impl GoogleProvider {
    pub fn new(
        name: String,
        model: String,
        auth: GoogleAuth,
        base_url: String,
        options: GoogleProviderOptions,
    ) -> Result<Self> {
        let timeout_ms = if options.timeout_ms == 0 {
            DEFAULT_TIMEOUT_MS
        } else {
            options.timeout_ms
        };

        let headers = Self::build_headers(&options)?;
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|err| Error::Provider(format!("failed to build Google client: {err}")))?;

        let base = base_url.trim_end_matches('/');
        let model_path = format!("{base}/models/{model}");

        Ok(Self {
            name,
            model,
            generate_endpoint: format!("{model_path}:generateContent"),
            stream_endpoint: format!("{model_path}:streamGenerateContent"),
            count_tokens_endpoint: format!("{model_path}:countTokens"),
            client,
            auth,
            retry_policy: options.retry_policy,
        })
    }

    fn build_headers(options: &GoogleProviderOptions) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        for (name, value) in &options.extra_headers {
            let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
                Error::Config(format!("invalid Google custom header name '{name}': {err}"))
            })?;
            let header_value = HeaderValue::from_str(value).map_err(|err| {
                Error::Config(format!(
                    "invalid Google custom header value for '{name}': {err}"
                ))
            })?;
            headers.insert(header_name, header_value);
        }

        Ok(headers)
    }

    async fn apply_auth(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder> {
        match &self.auth {
            GoogleAuth::ApiKey { token } => Ok(builder.query(&[("key", token)])),
            GoogleAuth::Subscription { manager } => {
                let access_token = manager.ensure_access_token().await?;
                Ok(builder.header(
                    HeaderName::from_static("authorization"),
                    format!("Bearer {access_token}"),
                ))
            }
        }
    }

    fn build_payload(&self, messages: &[ChatMessage], options: &GenerateOptions) -> Value {
        let (system_instruction, contents) = convert_messages(messages);

        let mut generation = json!({
            "temperature": options.temperature,
            "maxOutputTokens": options.max_tokens,
        });

        if let Some(top_p) = options.top_p {
            generation["topP"] = json!(top_p);
        }
        if let Some(top_k) = options.top_k {
            generation["topK"] = json!(top_k);
        }
        if let Some(stop) = &options.stop_sequences {
            generation["stopSequences"] = json!(stop);
        }
        if let Some(penalty) = options.presence_penalty {
            generation["presencePenalty"] = json!(penalty);
        }
        if let Some(penalty) = options.frequency_penalty {
            generation["frequencyPenalty"] = json!(penalty);
        }

        let mut payload = json!({
            "contents": contents,
            "generationConfig": generation,
        });

        if let Some(system_text) = system_instruction {
            payload["system_instruction"] = json!({
                "parts": [
                    {
                        "text": system_text
                    }
                ]
            });
        }

        payload
    }

    fn extract_text_output(response: GoogleGenerateResponse) -> String {
        response
            .candidates
            .into_iter()
            .flat_map(|candidate| candidate.content.parts.into_iter())
            .filter_map(|part| part.text)
            .collect::<Vec<_>>()
            .join("")
    }

    fn extract_stream_text(json_payload: &str) -> Option<String> {
        let parsed: Value = serde_json::from_str(json_payload).ok()?;

        if let Some(text) = parsed
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(Value::as_str)
        {
            return Some(text.to_owned());
        }

        if let Some(parts) = parsed
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
        {
            let merged = parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            if !merged.is_empty() {
                return Some(merged);
            }
        }

        None
    }
}

#[derive(Debug, Clone, Serialize)]
struct GoogleInputContent {
    role: String,
    parts: Vec<GoogleInputPart>,
}

#[derive(Debug, Clone, Serialize)]
struct GoogleInputPart {
    text: String,
}

#[derive(Debug, Deserialize)]
struct GoogleGenerateResponse {
    #[serde(default)]
    candidates: Vec<GoogleCandidate>,
}

#[derive(Debug, Deserialize)]
struct GoogleCandidate {
    content: GoogleOutputContent,
}

#[derive(Debug, Deserialize)]
struct GoogleOutputContent {
    #[serde(default)]
    parts: Vec<GoogleOutputPart>,
}

#[derive(Debug, Deserialize)]
struct GoogleOutputPart {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleCountTokensResponse {
    #[serde(rename = "totalTokens")]
    total_tokens_camel: Option<usize>,
    #[serde(rename = "total_tokens")]
    total_tokens_snake: Option<usize>,
}

fn convert_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<GoogleInputContent>) {
    let mut system_parts = Vec::new();
    let mut contents = Vec::new();

    for message in messages {
        if message.role == "system" {
            system_parts.push(message.content.clone());
            continue;
        }

        let role = match message.role.as_str() {
            "assistant" => "model".to_owned(),
            _ => "user".to_owned(),
        };

        let text = match message.role.as_str() {
            "tool" => format!("[tool output]\n{}", message.content),
            _ => message.content.clone(),
        };

        contents.push(GoogleInputContent {
            role,
            parts: vec![GoogleInputPart { text }],
        });
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    (system, contents)
}

#[async_trait]
impl ModelProvider for GoogleProvider {
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
                    .post(&self.generate_endpoint)
                    .json(&self.build_payload(messages, options)),
            )
            .await?;

        let response = send_with_retry(request, &self.retry_policy, "Google request").await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Google request failed with status {status}: {body}"
            )));
        }

        let payload: GoogleGenerateResponse = response
            .json()
            .await
            .map_err(|err| Error::Provider(format!("failed to parse Google response: {err}")))?;

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
                    .post(&self.stream_endpoint)
                    .query(&[("alt", "sse")])
                    .json(&self.build_payload(messages, options)),
            )
            .await?;

        let response =
            send_with_retry(request, &self.retry_policy, "Google stream request").await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Google stream request failed with status {status}: {body}"
            )));
        }

        Ok(spawn_sse_stream_with_data_parser(
            response,
            Self::extract_stream_text,
        ))
    }

    async fn count_tokens(&self, messages: &[ChatMessage]) -> Result<usize> {
        let (system_instruction, contents) = convert_messages(messages);
        let mut payload = json!({
            "contents": contents,
        });

        if let Some(system_text) = system_instruction {
            payload["system_instruction"] = json!({
                "parts": [
                    {
                        "text": system_text
                    }
                ]
            });
        }

        let request = self
            .apply_auth(self.client.post(&self.count_tokens_endpoint).json(&payload))
            .await?;

        let response =
            send_with_retry(request, &self.retry_policy, "Google count_tokens request").await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read body>"));
            return Err(Error::Provider(format!(
                "Google count_tokens request failed with status {status}: {body}"
            )));
        }

        let payload: GoogleCountTokensResponse = response.json().await.map_err(|err| {
            Error::Provider(format!(
                "failed to parse Google count_tokens response: {err}"
            ))
        })?;

        payload
            .total_tokens_camel
            .or(payload.total_tokens_snake)
            .ok_or_else(|| {
                Error::Provider("Google count_tokens response missing total tokens".to_owned())
            })
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_functions(&self) -> bool {
        false
    }
}
