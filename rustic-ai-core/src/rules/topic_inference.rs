use crate::error::{Error, Result};
use crate::providers::registry::ProviderRegistry;
use crate::providers::types::{ChatMessage, GenerateOptions};

pub struct TopicInferenceService {
    inference_provider: String,
}

impl TopicInferenceService {
    pub fn new(inference_provider: String) -> Self {
        Self { inference_provider }
    }

    pub async fn infer_topics(
        &self,
        provider_registry: &ProviderRegistry,
        available_topics: &[String],
        conversation: &[ChatMessage],
    ) -> Result<Vec<String>> {
        if available_topics.is_empty() {
            return Ok(Vec::new());
        }

        let Some(provider) = provider_registry.get(&self.inference_provider) else {
            return Err(Error::Provider(format!(
                "topic inference provider '{}' is not registered",
                self.inference_provider
            )));
        };

        let system_prompt = format!(
            "Select at most 3 relevant topics for this conversation. Return only a JSON array of strings. Allowed topics: {}",
            available_topics.join(", ")
        );

        let mut messages = vec![ChatMessage {
            role: "system".to_owned(),
            content: system_prompt,
        }];
        messages.extend(conversation.iter().cloned());

        let raw = provider
            .generate(
                &messages,
                &GenerateOptions {
                    temperature: 0.1,
                    max_tokens: 96,
                },
            )
            .await?;

        let topics: Vec<String> = serde_json::from_str(raw.trim()).map_err(|err| {
            Error::Provider(format!(
                "topic inference returned invalid JSON array: {err}; raw: {}",
                raw.trim()
            ))
        })?;

        Ok(topics
            .into_iter()
            .map(|topic| topic.trim().to_ascii_lowercase())
            .filter(|topic| !topic.is_empty())
            .collect())
    }
}
