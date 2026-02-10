use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::Result;
use crate::providers::openai::{OpenAiAuth, OpenAiProvider, OpenAiProviderOptions};
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};

#[derive(Debug, Clone, Copy)]
pub enum ZAiEndpointProfile {
    General,
    Coding,
}

#[derive(Debug, Clone)]
pub struct ZAiProvider {
    name: String,
    general: OpenAiProvider,
    coding: OpenAiProvider,
    profile: ZAiEndpointProfile,
}

impl ZAiProvider {
    pub fn new(
        name: String,
        model: String,
        api_key: String,
        general_base_url: String,
        coding_base_url: String,
        profile: ZAiEndpointProfile,
        options: OpenAiProviderOptions,
    ) -> Result<Self> {
        let general = OpenAiProvider::new(
            format!("{}:general", name),
            model.clone(),
            OpenAiAuth::ApiKey {
                token: api_key.clone(),
            },
            general_base_url,
            options.clone(),
        )?;

        let coding = OpenAiProvider::new(
            format!("{}:coding", name),
            model,
            OpenAiAuth::ApiKey { token: api_key },
            coding_base_url,
            options,
        )?;

        Ok(Self {
            name,
            general,
            coding,
            profile,
        })
    }

    fn active(&self) -> &OpenAiProvider {
        match self.profile {
            ZAiEndpointProfile::General => &self.general,
            ZAiEndpointProfile::Coding => &self.coding,
        }
    }
}

#[async_trait]
impl ModelProvider for ZAiProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn generate(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
    ) -> Result<String> {
        self.active().generate(messages, options).await
    }

    async fn stream_generate(
        &self,
        messages: &[ChatMessage],
        options: &GenerateOptions,
    ) -> Result<mpsc::Receiver<String>> {
        self.active().stream_generate(messages, options).await
    }

    async fn count_tokens(&self, messages: &[ChatMessage]) -> Result<usize> {
        self.active().count_tokens(messages).await
    }

    fn supports_streaming(&self) -> bool {
        self.active().supports_streaming()
    }

    fn supports_functions(&self) -> bool {
        self.active().supports_functions()
    }
}
