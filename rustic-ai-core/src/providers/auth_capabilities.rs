use crate::config::schema::{AuthMode, ProviderType};

pub fn supported_auth_modes(provider_type: &ProviderType) -> Vec<AuthMode> {
    match provider_type {
        ProviderType::OpenAi | ProviderType::Anthropic | ProviderType::Google => {
            vec![AuthMode::ApiKey, AuthMode::Subscription]
        }
        ProviderType::ZAi => vec![AuthMode::ApiKey],
        ProviderType::Grok => vec![AuthMode::ApiKey],
        ProviderType::Ollama => vec![AuthMode::ApiKey],
        ProviderType::Custom => vec![AuthMode::ApiKey],
    }
}

pub fn supports_auth_mode(provider_type: &ProviderType, mode: &AuthMode) -> bool {
    supported_auth_modes(provider_type)
        .iter()
        .any(|item| item == mode)
}

pub fn auth_mode_name(mode: &AuthMode) -> &'static str {
    match mode {
        AuthMode::ApiKey => "api_key",
        AuthMode::Subscription => "subscription",
        AuthMode::None => "none",
    }
}

pub fn supported_auth_mode_names(provider_type: &ProviderType) -> Vec<&'static str> {
    supported_auth_modes(provider_type)
        .iter()
        .map(auth_mode_name)
        .collect()
}
