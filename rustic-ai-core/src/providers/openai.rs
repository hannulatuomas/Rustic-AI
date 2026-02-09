use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};

#[derive(Debug, Clone)]
pub struct OpenAiProvider {
    name: String,
    model: String,
    endpoint: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(name: String, model: String, api_key: String, base_url: Option<String>) -> Self {
        let endpoint = base_url
            .map(|url| format!("{}/chat/completions", url.trim_end_matches('/')))
            .unwrap_or_default();

        let client = reqwest::Client::builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {api_key}")
                        .parse()
                        .expect("authorization header should be valid"),
                );
                headers.insert(
                    reqwest::header::CONTENT_TYPE,
                    "application/json"
                        .parse()
                        .expect("content type header should be valid"),
                );
                headers
            })
            .build()
            .expect("client creation should not fail");

        Self {
            name,
            model,
            endpoint,
            client,
        }
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
        let response = self
            .client
            .post(&self.endpoint)
            .json(&serde_json::json!({
                "model": self.model,
                "messages": messages,
                "temperature": options.temperature,
                "max_tokens": options.max_tokens,
            }))
            .send()
            .await
            .map_err(|err| Error::Provider(format!("OpenAI request failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_else(|_| String::new());
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
}
