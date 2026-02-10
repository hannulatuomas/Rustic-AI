use std::sync::Arc;

use crate::config::schema::{Config, ProviderConfig, ProviderType};
use crate::error::{Error, Result};
use crate::providers::openai::OpenAiProvider;
use crate::providers::registry::ProviderRegistry;
use crate::providers::types::ModelProvider;

pub fn create_provider_registry(config: &Config) -> Result<ProviderRegistry> {
    let mut registry = ProviderRegistry::default();

    for provider in &config.providers {
        let instance = create_provider(provider)?;
        registry.register(provider.name.clone(), instance);
    }

    Ok(registry)
}

pub fn create_provider(provider: &ProviderConfig) -> Result<Arc<dyn ModelProvider>> {
    match provider.provider_type {
        ProviderType::OpenAi => build_openai_provider(provider),
        ProviderType::Anthropic => Err(Error::Provider(
            "provider type 'anthropic' is not implemented yet".to_owned(),
        )),
        ProviderType::Grok => Err(Error::Provider(
            "provider type 'grok' is not implemented yet".to_owned(),
        )),
        ProviderType::Google => Err(Error::Provider(
            "provider type 'google' is not implemented yet".to_owned(),
        )),
        ProviderType::Ollama => Err(Error::Provider(
            "provider type 'ollama' is not implemented yet".to_owned(),
        )),
        ProviderType::Custom => Err(Error::Provider(
            "provider type 'custom' is not implemented yet".to_owned(),
        )),
    }
}

fn build_openai_provider(provider: &ProviderConfig) -> Result<Arc<dyn ModelProvider>> {
    let model = provider.model.clone().ok_or_else(|| {
        Error::Provider(format!(
            "provider '{}' is missing required 'model'",
            provider.name
        ))
    })?;

    let base_url = provider.base_url.clone().ok_or_else(|| {
        Error::Provider(format!(
            "provider '{}' is missing required 'base_url'",
            provider.name
        ))
    })?;

    let api_key_env = provider.api_key_env.clone().ok_or_else(|| {
        Error::Provider(format!(
            "provider '{}' is missing required 'api_key_env'",
            provider.name
        ))
    })?;

    let api_key = std::env::var(&api_key_env).map_err(|_| {
        Error::Provider(format!(
            "provider '{}' requires env var '{}' to be set",
            provider.name, api_key_env
        ))
    })?;

    Ok(Arc::new(OpenAiProvider::new(
        provider.name.clone(),
        model,
        api_key,
        Some(base_url),
    )))
}
