use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::error::{Error, Result};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub fn normalize_timeout_ms(timeout_ms: u64) -> u64 {
    if timeout_ms == 0 {
        DEFAULT_TIMEOUT_MS
    } else {
        timeout_ms
    }
}

pub fn append_extra_headers(
    headers: &mut HeaderMap,
    extra_headers: &[(String, String)],
    provider_label: &str,
) -> Result<()> {
    for (name, value) in extra_headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
            Error::Config(format!(
                "invalid {provider_label} custom header name '{name}': {err}"
            ))
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|err| {
            Error::Config(format!(
                "invalid {provider_label} custom header value for '{name}': {err}"
            ))
        })?;
        headers.insert(header_name, header_value);
    }

    Ok(())
}

pub fn build_client(
    headers: HeaderMap,
    timeout_ms: u64,
    provider_label: &str,
) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_millis(normalize_timeout_ms(
            timeout_ms,
        )))
        .build()
        .map_err(|err| Error::Provider(format!("failed to build {provider_label} client: {err}")))
}
