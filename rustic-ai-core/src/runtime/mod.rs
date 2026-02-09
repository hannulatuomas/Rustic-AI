pub mod cancellation;

use std::sync::Arc;

use crate::config::schema::Config;
use crate::events::EventBus;
use crate::providers::openai::OpenAiProvider;
use crate::providers::registry::ProviderRegistry;
use crate::providers::types::{ChatMessage, GenerateOptions, ModelProvider};
use crate::tools::registry::ToolRegistry;
use async_trait::async_trait;

pub struct Runtime {
    pub event_bus: EventBus,
    pub providers: ProviderRegistry,
    pub tools: ToolRegistry,
    pub config: Config,
}

impl Runtime {
    pub fn new(config: Config) -> Self {
        let mut providers = ProviderRegistry::default();
        for provider in &config.providers {
            match provider.provider_type {
                crate::config::schema::ProviderType::OpenAi => {
                    let key_env = provider.api_key_env.clone();
                    let model = provider.model.clone();
                    let base_url = provider.base_url.clone();

                    if let (Some(key_env), Some(model), Some(base_url)) = (key_env, model, base_url)
                    {
                        if let Ok(api_key) = std::env::var(&key_env) {
                            let openai = OpenAiProvider::new(
                                provider.name.clone(),
                                model,
                                api_key,
                                Some(base_url),
                            );
                            providers.register(provider.name.clone(), Arc::new(openai));
                        } else {
                            providers.register(
                                provider.name.clone(),
                                Arc::new(UnsupportedProvider {
                                    name: provider.name.clone(),
                                }),
                            );
                        }
                    } else {
                        providers.register(
                            provider.name.clone(),
                            Arc::new(UnsupportedProvider {
                                name: provider.name.clone(),
                            }),
                        );
                    }
                }
                _ => {
                    providers.register(
                        provider.name.clone(),
                        Arc::new(UnsupportedProvider {
                            name: provider.name.clone(),
                        }),
                    );
                }
            }
        }

        Self {
            event_bus: EventBus::default(),
            providers,
            tools: ToolRegistry::default(),
            config,
        }
    }
}

#[derive(Debug)]
struct UnsupportedProvider {
    name: String,
}

#[async_trait]
impl ModelProvider for UnsupportedProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn generate(
        &self,
        _messages: &[ChatMessage],
        _options: &GenerateOptions,
    ) -> crate::error::Result<String> {
        Err(crate::error::Error::Provider(format!(
            "provider '{}' is not implemented yet",
            self.name
        )))
    }
}
