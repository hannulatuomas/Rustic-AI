use std::sync::Arc;

use crate::auth::SubscriptionAuthManager;
use crate::config::schema::{AuthMode, Config, ProviderConfig, ProviderType};
use crate::error::{Error, Result};
use crate::providers::anthropic::{AnthropicAuth, AnthropicProvider, AnthropicProviderOptions};
use crate::providers::google::{GoogleAuth, GoogleProvider, GoogleProviderOptions};
use crate::providers::grok::{GrokProvider, GrokProviderOptions};
use crate::providers::ollama::{OllamaProvider, OllamaProviderOptions};
use crate::providers::openai::{OpenAiAuth, OpenAiProvider, OpenAiProviderOptions};
use crate::providers::registry::ProviderRegistry;
use crate::providers::retry::RetryPolicy;
use crate::providers::types::ModelProvider;
use crate::providers::z_ai::{ZAiEndpointProfile, ZAiProvider};

pub fn create_provider_registry(
    config: &Config,
    work_dir: &std::path::Path,
) -> Result<ProviderRegistry> {
    let mut registry = ProviderRegistry::default();
    let auth_store_path = crate::auth::resolve_auth_store_path(config, work_dir);

    for provider in &config.providers {
        let instance = create_provider(provider, &auth_store_path)?;
        registry.register(provider.name.clone(), instance);
    }

    Ok(registry)
}

pub fn create_provider(
    provider: &ProviderConfig,
    auth_store_path: &std::path::Path,
) -> Result<Arc<dyn ModelProvider>> {
    match provider.provider_type {
        ProviderType::OpenAi => build_openai_provider(provider, auth_store_path),
        ProviderType::Anthropic => build_anthropic_provider(provider, auth_store_path),
        ProviderType::Grok => build_grok_provider(provider),
        ProviderType::Google => build_google_provider(provider, auth_store_path),
        ProviderType::ZAi => build_zai_provider(provider),
        ProviderType::Ollama => build_ollama_provider(provider),
        ProviderType::Custom => build_custom_provider(provider),
    }
}

fn build_custom_provider(provider: &ProviderConfig) -> Result<Arc<dyn ModelProvider>> {
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

    if !matches!(provider.auth_mode, AuthMode::ApiKey) {
        return Err(Error::Provider(format!(
            "provider '{}' must use auth_mode 'api_key' for custom",
            provider.name
        )));
    }

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

    let options = build_openai_options(provider)?;

    Ok(Arc::new(OpenAiProvider::new(
        provider.name.clone(),
        model,
        OpenAiAuth::ApiKey { token: api_key },
        base_url,
        options,
    )?))
}

fn build_zai_provider(provider: &ProviderConfig) -> Result<Arc<dyn ModelProvider>> {
    let model = provider.model.clone().ok_or_else(|| {
        Error::Provider(format!(
            "provider '{}' is missing required 'model'",
            provider.name
        ))
    })?;

    if !matches!(provider.auth_mode, AuthMode::ApiKey) {
        return Err(Error::Provider(format!(
            "provider '{}' must use auth_mode 'api_key' for z_ai",
            provider.name
        )));
    }

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

    let general_base_url = provider
        .settings
        .as_ref()
        .and_then(|s| s.get("general_base_url"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| provider.base_url.clone())
        .ok_or_else(|| {
            Error::Provider(format!(
                "provider '{}' is missing required 'base_url' or settings.general_base_url",
                provider.name
            ))
        })?;

    let coding_base_url = provider
        .settings
        .as_ref()
        .and_then(|s| s.get("coding_base_url"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            Error::Provider(format!(
                "provider '{}' is missing required settings.coding_base_url for z_ai",
                provider.name
            ))
        })?;

    let profile = provider
        .settings
        .as_ref()
        .and_then(|s| s.get("endpoint_profile"))
        .and_then(|v| v.as_str())
        .unwrap_or("general");

    let profile = match profile {
        "general" => ZAiEndpointProfile::General,
        "coding" => ZAiEndpointProfile::Coding,
        other => {
            return Err(Error::Config(format!(
                "provider '{}' setting endpoint_profile must be 'general' or 'coding', got '{}'",
                provider.name, other
            )));
        }
    };

    let options = build_openai_options(provider)?;

    Ok(Arc::new(ZAiProvider::new(
        provider.name.clone(),
        model,
        api_key,
        general_base_url,
        coding_base_url,
        profile,
        options,
    )?))
}

fn build_ollama_provider(provider: &ProviderConfig) -> Result<Arc<dyn ModelProvider>> {
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

    if !matches!(provider.auth_mode, AuthMode::ApiKey) {
        return Err(Error::Provider(format!(
            "provider '{}' must use auth_mode 'api_key' for ollama",
            provider.name
        )));
    }

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

    let options = build_ollama_options(provider)?;

    Ok(Arc::new(OllamaProvider::new(
        provider.name.clone(),
        model,
        api_key,
        base_url,
        options,
    )?))
}

fn build_grok_provider(provider: &ProviderConfig) -> Result<Arc<dyn ModelProvider>> {
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

    if !matches!(provider.auth_mode, AuthMode::ApiKey) {
        return Err(Error::Provider(format!(
            "provider '{}' must use auth_mode 'api_key' for grok",
            provider.name
        )));
    }

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

    let options = build_grok_options(provider)?;

    Ok(Arc::new(GrokProvider::new(
        provider.name.clone(),
        model,
        api_key,
        base_url,
        options,
    )?))
}

fn build_google_provider(
    provider: &ProviderConfig,
    auth_store_path: &std::path::Path,
) -> Result<Arc<dyn ModelProvider>> {
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

    let auth = match provider.auth_mode {
        AuthMode::ApiKey => {
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
            GoogleAuth::ApiKey { token: api_key }
        }
        AuthMode::Subscription => {
            let manager = Arc::new(SubscriptionAuthManager::from_provider_config(
                provider,
                auth_store_path.to_path_buf(),
            )?);
            GoogleAuth::Subscription { manager }
        }
        AuthMode::None => {
            return Err(Error::Provider(format!(
                "provider '{}' does not support auth_mode 'none' for google",
                provider.name
            )));
        }
    };

    let options = build_google_options(provider)?;

    Ok(Arc::new(GoogleProvider::new(
        provider.name.clone(),
        model,
        auth,
        base_url,
        options,
    )?))
}

fn build_anthropic_provider(
    provider: &ProviderConfig,
    auth_store_path: &std::path::Path,
) -> Result<Arc<dyn ModelProvider>> {
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

    let auth = match provider.auth_mode {
        AuthMode::ApiKey => {
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
            AnthropicAuth::ApiKey { token: api_key }
        }
        AuthMode::Subscription => {
            let manager = Arc::new(SubscriptionAuthManager::from_provider_config(
                provider,
                auth_store_path.to_path_buf(),
            )?);
            AnthropicAuth::Subscription { manager }
        }
        AuthMode::None => {
            return Err(Error::Provider(format!(
                "provider '{}' does not support auth_mode 'none' for anthropic",
                provider.name
            )));
        }
    };

    let options = build_anthropic_options(provider)?;

    Ok(Arc::new(AnthropicProvider::new(
        provider.name.clone(),
        model,
        auth,
        base_url,
        options,
    )?))
}

fn build_openai_provider(
    provider: &ProviderConfig,
    auth_store_path: &std::path::Path,
) -> Result<Arc<dyn ModelProvider>> {
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

    let auth = match provider.auth_mode {
        AuthMode::ApiKey => {
            let credential_env = provider.api_key_env.clone().ok_or_else(|| {
                Error::Provider(format!(
                    "provider '{}' is missing required 'api_key_env'",
                    provider.name
                ))
            })?;
            let credential = std::env::var(&credential_env).map_err(|_| {
                Error::Provider(format!(
                    "provider '{}' requires env var '{}' to be set",
                    provider.name, credential_env
                ))
            })?;
            OpenAiAuth::ApiKey { token: credential }
        }
        AuthMode::Subscription => {
            let manager = Arc::new(SubscriptionAuthManager::from_provider_config(
                provider,
                auth_store_path.to_path_buf(),
            )?);

            let organization = provider
                .settings
                .as_ref()
                .and_then(|settings| settings.get("organization"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            let project = provider
                .settings
                .as_ref()
                .and_then(|settings| settings.get("project"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            let account_id = provider
                .settings
                .as_ref()
                .and_then(|settings| settings.get("account_id"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);

            OpenAiAuth::Subscription {
                manager,
                organization,
                project,
                account_id,
            }
        }
        AuthMode::None => {
            return Err(Error::Provider(format!(
                "provider '{}' does not support auth_mode 'none' for open_ai",
                provider.name
            )));
        }
    };

    let options = build_openai_options(provider)?;

    Ok(Arc::new(OpenAiProvider::new(
        provider.name.clone(),
        model,
        auth,
        base_url,
        options,
    )?))
}

fn build_openai_options(provider: &ProviderConfig) -> Result<OpenAiProviderOptions> {
    let timeout_ms = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("request_timeout_ms"))
        .and_then(|value| value.as_u64())
        .unwrap_or(30_000);

    let mut extra_headers = Vec::new();
    if let Some(raw_headers) = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("headers"))
    {
        let headers_object = raw_headers.as_object().ok_or_else(|| {
            Error::Config(format!(
                "provider '{}' setting 'headers' must be an object of string values",
                provider.name
            ))
        })?;

        for (name, value) in headers_object {
            let header_value = value.as_str().ok_or_else(|| {
                Error::Config(format!(
                    "provider '{}' setting headers['{name}'] must be a string",
                    provider.name
                ))
            })?;
            extra_headers.push((name.clone(), header_value.to_owned()));
        }
    }

    Ok(OpenAiProviderOptions {
        timeout_ms,
        extra_headers,
        retry_policy: build_retry_policy(provider),
    })
}

fn build_anthropic_options(provider: &ProviderConfig) -> Result<AnthropicProviderOptions> {
    let timeout_ms = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("request_timeout_ms"))
        .and_then(|value| value.as_u64())
        .unwrap_or(30_000);

    let api_version = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("api_version"))
        .and_then(|value| value.as_str())
        .unwrap_or("2023-06-01")
        .to_owned();

    let betas = if let Some(raw) = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("betas"))
    {
        let array = raw.as_array().ok_or_else(|| {
            Error::Config(format!(
                "provider '{}' setting 'betas' must be an array of strings",
                provider.name
            ))
        })?;

        let mut parsed = Vec::with_capacity(array.len());
        for value in array {
            parsed.push(
                value
                    .as_str()
                    .ok_or_else(|| {
                        Error::Config(format!(
                            "provider '{}' setting 'betas' must only contain strings",
                            provider.name
                        ))
                    })?
                    .to_owned(),
            );
        }
        parsed
    } else {
        Vec::new()
    };

    let mut extra_headers = Vec::new();
    if let Some(raw_headers) = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("headers"))
    {
        let headers_object = raw_headers.as_object().ok_or_else(|| {
            Error::Config(format!(
                "provider '{}' setting 'headers' must be an object of string values",
                provider.name
            ))
        })?;

        for (name, value) in headers_object {
            let header_value = value.as_str().ok_or_else(|| {
                Error::Config(format!(
                    "provider '{}' setting headers['{name}'] must be a string",
                    provider.name
                ))
            })?;
            extra_headers.push((name.clone(), header_value.to_owned()));
        }
    }

    Ok(AnthropicProviderOptions {
        timeout_ms,
        api_version,
        betas,
        extra_headers,
        retry_policy: build_retry_policy(provider),
    })
}

fn build_google_options(provider: &ProviderConfig) -> Result<GoogleProviderOptions> {
    let timeout_ms = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("request_timeout_ms"))
        .and_then(|value| value.as_u64())
        .unwrap_or(30_000);

    let mut extra_headers = Vec::new();
    if let Some(raw_headers) = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("headers"))
    {
        let headers_object = raw_headers.as_object().ok_or_else(|| {
            Error::Config(format!(
                "provider '{}' setting 'headers' must be an object of string values",
                provider.name
            ))
        })?;

        for (name, value) in headers_object {
            let header_value = value.as_str().ok_or_else(|| {
                Error::Config(format!(
                    "provider '{}' setting headers['{name}'] must be a string",
                    provider.name
                ))
            })?;
            extra_headers.push((name.clone(), header_value.to_owned()));
        }
    }

    Ok(GoogleProviderOptions {
        timeout_ms,
        extra_headers,
        retry_policy: build_retry_policy(provider),
    })
}

fn build_grok_options(provider: &ProviderConfig) -> Result<GrokProviderOptions> {
    let timeout_ms = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("request_timeout_ms"))
        .and_then(|value| value.as_u64())
        .unwrap_or(30_000);

    let mut extra_headers = Vec::new();
    if let Some(raw_headers) = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("headers"))
    {
        let headers_object = raw_headers.as_object().ok_or_else(|| {
            Error::Config(format!(
                "provider '{}' setting 'headers' must be an object of string values",
                provider.name
            ))
        })?;

        for (name, value) in headers_object {
            let header_value = value.as_str().ok_or_else(|| {
                Error::Config(format!(
                    "provider '{}' setting headers['{name}'] must be a string",
                    provider.name
                ))
            })?;
            extra_headers.push((name.clone(), header_value.to_owned()));
        }
    }

    Ok(GrokProviderOptions {
        timeout_ms,
        extra_headers,
        retry_policy: build_retry_policy(provider),
    })
}

fn build_ollama_options(provider: &ProviderConfig) -> Result<OllamaProviderOptions> {
    let timeout_ms = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("request_timeout_ms"))
        .and_then(|value| value.as_u64())
        .unwrap_or(30_000);

    let mut extra_headers = Vec::new();
    if let Some(raw_headers) = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("headers"))
    {
        let headers_object = raw_headers.as_object().ok_or_else(|| {
            Error::Config(format!(
                "provider '{}' setting 'headers' must be an object of string values",
                provider.name
            ))
        })?;

        for (name, value) in headers_object {
            let header_value = value.as_str().ok_or_else(|| {
                Error::Config(format!(
                    "provider '{}' setting headers['{name}'] must be a string",
                    provider.name
                ))
            })?;
            extra_headers.push((name.clone(), header_value.to_owned()));
        }
    }

    Ok(OllamaProviderOptions {
        timeout_ms,
        extra_headers,
        retry_policy: build_retry_policy(provider),
    })
}

fn build_retry_policy(provider: &ProviderConfig) -> RetryPolicy {
    let max_retries = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("request_max_retries"))
        .and_then(|value| value.as_u64())
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(2);

    let base_delay_ms = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("retry_base_delay_ms"))
        .and_then(|value| value.as_u64())
        .unwrap_or(250);

    let max_delay_ms = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("retry_max_delay_ms"))
        .and_then(|value| value.as_u64())
        .unwrap_or(3_000);

    let jitter_ms = provider
        .settings
        .as_ref()
        .and_then(|settings| settings.get("retry_jitter_ms"))
        .and_then(|value| value.as_u64())
        .unwrap_or(100);

    RetryPolicy {
        max_retries,
        base_delay_ms,
        max_delay_ms,
        jitter_ms,
    }
}
