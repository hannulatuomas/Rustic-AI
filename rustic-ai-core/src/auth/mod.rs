use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use chrono::Utc;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::config::schema::{Config, ProviderConfig, ProviderType};
use crate::error::{Error, Result};
use crate::storage::paths::StoragePaths;

const AUTH_STORE_FILE_NAME: &str = "auth.json";
const DEFAULT_TOKEN_REFRESH_SKEW_SECS: i64 = 60;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionAuthMethod {
    Browser,
    Headless,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCredential {
    pub provider_name: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_at_epoch_secs: Option<i64>,
    pub scopes: Vec<String>,
    pub metadata: BTreeMap<String, String>,
    pub created_at_epoch_secs: i64,
    pub updated_at_epoch_secs: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialSummary {
    pub provider_name: String,
    pub token_type: String,
    pub expires_at_epoch_secs: Option<i64>,
    pub scopes: Vec<String>,
    pub updated_at_epoch_secs: i64,
}

#[derive(Debug, Clone)]
pub struct SubscriptionOAuthConfig {
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub device_authorization_url: Option<String>,
    pub scopes: Vec<String>,
    pub redirect_host: String,
    pub redirect_port: u16,
    pub extra_authorize_params: BTreeMap<String, String>,
    pub extra_token_params: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct BrowserAuthorizationRequest {
    pub authorization_url: String,
    pub expected_state: String,
    pub code_verifier: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone)]
pub struct DeviceAuthorizationStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in_secs: u64,
    pub interval_secs: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthStoreFile {
    version: u32,
    credentials: BTreeMap<String, StoredCredential>,
}

impl Default for AuthStoreFile {
    fn default() -> Self {
        Self {
            version: 1,
            credentials: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    async fn load_file(&self) -> Result<AuthStoreFile> {
        if !self.path.exists() {
            return Ok(AuthStoreFile::default());
        }

        let raw = tokio::fs::read_to_string(&self.path).await.map_err(|err| {
            Error::Config(format!(
                "failed to read auth store '{}': {err}",
                self.path.display()
            ))
        })?;

        if raw.trim().is_empty() {
            return Ok(AuthStoreFile::default());
        }

        serde_json::from_str(&raw).map_err(|err| {
            Error::Config(format!(
                "failed to parse auth store '{}': {err}",
                self.path.display()
            ))
        })
    }

    async fn save_file(&self, file: &AuthStoreFile) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                Error::Config(format!(
                    "failed to create auth store parent directory '{}': {err}",
                    parent.display()
                ))
            })?;
        }

        let serialized = serde_json::to_string_pretty(file)?;
        tokio::fs::write(&self.path, serialized)
            .await
            .map_err(|err| {
                Error::Config(format!(
                    "failed to write auth store '{}': {err}",
                    self.path.display()
                ))
            })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600)).map_err(
                |err| {
                    Error::Config(format!(
                        "failed to secure auth store permissions '{}': {err}",
                        self.path.display()
                    ))
                },
            )?;
        }

        Ok(())
    }

    pub async fn get(&self, provider_name: &str) -> Result<Option<StoredCredential>> {
        let file = self.load_file().await?;
        Ok(file.credentials.get(provider_name).cloned())
    }

    pub async fn upsert(&self, credential: StoredCredential) -> Result<()> {
        let mut file = self.load_file().await?;
        file.credentials
            .insert(credential.provider_name.clone(), credential);
        self.save_file(&file).await
    }

    pub async fn remove(&self, provider_name: &str) -> Result<bool> {
        let mut file = self.load_file().await?;
        let removed = file.credentials.remove(provider_name).is_some();
        self.save_file(&file).await?;
        Ok(removed)
    }

    pub async fn list_summaries(&self) -> Result<Vec<CredentialSummary>> {
        let file = self.load_file().await?;
        let mut items = file
            .credentials
            .values()
            .map(|entry| CredentialSummary {
                provider_name: entry.provider_name.clone(),
                token_type: entry.token_type.clone(),
                expires_at_epoch_secs: entry.expires_at_epoch_secs,
                scopes: entry.scopes.clone(),
                updated_at_epoch_secs: entry.updated_at_epoch_secs,
            })
            .collect::<Vec<_>>();
        items.sort_by(|a, b| a.provider_name.cmp(&b.provider_name));
        Ok(items)
    }
}

#[derive(Debug, Clone)]
pub struct SubscriptionAuthManager {
    provider_name: String,
    store: CredentialStore,
    oauth: SubscriptionOAuthConfig,
    http: reqwest::Client,
}

impl SubscriptionAuthManager {
    pub fn from_provider_config(
        provider: &ProviderConfig,
        auth_store_path: PathBuf,
    ) -> Result<Self> {
        let oauth = parse_subscription_oauth_config(provider)?;
        Ok(Self {
            provider_name: provider.name.clone(),
            store: CredentialStore::new(auth_store_path),
            oauth,
            http: reqwest::Client::new(),
        })
    }

    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    pub fn oauth_config(&self) -> &SubscriptionOAuthConfig {
        &self.oauth
    }

    pub fn store(&self) -> &CredentialStore {
        &self.store
    }

    pub async fn ensure_access_token(&self) -> Result<String> {
        let Some(credential) = self.store.get(&self.provider_name).await? else {
            return Err(Error::Provider(format!(
                "subscription credential for provider '{}' is missing; run auth connect",
                self.provider_name
            )));
        };

        if is_token_valid(&credential) {
            return Ok(credential.access_token);
        }

        let Some(refresh_token) = credential.refresh_token else {
            return Err(Error::Provider(format!(
                "subscription token for provider '{}' is expired and no refresh token is available; re-authenticate",
                self.provider_name
            )));
        };

        let refreshed = self
            .refresh_token(refresh_token, credential.scopes.clone())
            .await?;
        self.store.upsert(refreshed.clone()).await?;
        Ok(refreshed.access_token)
    }

    pub fn build_browser_authorization_request(&self) -> Result<BrowserAuthorizationRequest> {
        let expected_state = uuid::Uuid::new_v4().to_string();
        let code_verifier = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());

        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());

        let redirect_uri = format!(
            "http://{}:{}/callback",
            self.oauth.redirect_host, self.oauth.redirect_port
        );

        let mut url = Url::parse(&self.oauth.authorize_url).map_err(|err| {
            Error::Config(format!(
                "invalid authorize_url '{}' for provider '{}': {err}",
                self.oauth.authorize_url, self.provider_name
            ))
        })?;

        {
            let mut query = url.query_pairs_mut();
            query.append_pair("response_type", "code");
            query.append_pair("client_id", &self.oauth.client_id);
            query.append_pair("redirect_uri", &redirect_uri);
            query.append_pair("scope", &self.oauth.scopes.join(" "));
            query.append_pair("state", &expected_state);
            query.append_pair("code_challenge", &challenge);
            query.append_pair("code_challenge_method", "S256");
            for (key, value) in &self.oauth.extra_authorize_params {
                query.append_pair(key, value);
            }
        }

        Ok(BrowserAuthorizationRequest {
            authorization_url: url.to_string(),
            expected_state,
            code_verifier,
            redirect_uri,
        })
    }

    pub async fn await_browser_callback_code(
        &self,
        expected_state: &str,
        timeout: Duration,
    ) -> Result<String> {
        let bind_addr = format!("{}:{}", self.oauth.redirect_host, self.oauth.redirect_port);
        let listener = TcpListener::bind(&bind_addr).await.map_err(|err| {
            Error::Config(format!(
                "failed to bind OAuth callback listener on '{}': {err}",
                bind_addr
            ))
        })?;

        let future = async {
            let (mut stream, _peer) = listener.accept().await.map_err(|err| {
                Error::Config(format!("failed to accept OAuth callback connection: {err}"))
            })?;

            let mut buffer = [0u8; 4096];
            let read_len = stream.read(&mut buffer).await.map_err(|err| {
                Error::Config(format!("failed to read OAuth callback request: {err}"))
            })?;

            let request = String::from_utf8_lossy(&buffer[..read_len]);
            let request_line = request.lines().next().unwrap_or_default();
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or_default();
            let target = parts.next().unwrap_or_default();

            if method != "GET" {
                let _ = stream
                    .write_all(http_response(405, "Method Not Allowed").as_bytes())
                    .await;
                return Err(Error::Config("OAuth callback only supports GET".to_owned()));
            }

            let full_url = format!("http://localhost{target}");
            let parsed = Url::parse(&full_url).map_err(|err| {
                Error::Config(format!("failed to parse OAuth callback URL: {err}"))
            })?;
            let params = parsed
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>();

            let code = params.get("code").cloned();
            let state = params.get("state").cloned();
            let error = params.get("error").cloned();

            if let Some(err) = error {
                let _ = stream
                    .write_all(http_response(400, "Authorization failed").as_bytes())
                    .await;
                return Err(Error::Provider(format!(
                    "OAuth authorization returned error: {err}"
                )));
            }

            if state.as_deref() != Some(expected_state) {
                let _ = stream
                    .write_all(http_response(400, "State mismatch").as_bytes())
                    .await;
                return Err(Error::Provider(
                    "OAuth callback state mismatch; possible CSRF detected".to_owned(),
                ));
            }

            let Some(code) = code else {
                let _ = stream
                    .write_all(http_response(400, "Missing code").as_bytes())
                    .await;
                return Err(Error::Provider(
                    "OAuth callback did not include authorization code".to_owned(),
                ));
            };

            let _ = stream
                .write_all(
                    http_response(200, "Authentication complete, you can close this tab.")
                        .as_bytes(),
                )
                .await;

            Ok(code)
        };

        tokio::time::timeout(timeout, future).await.map_err(|_| {
            Error::Provider("timed out waiting for OAuth browser callback".to_owned())
        })?
    }

    pub async fn exchange_authorization_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<StoredCredential> {
        let mut form = vec![
            ("grant_type", "authorization_code".to_owned()),
            ("client_id", self.oauth.client_id.clone()),
            ("code", code.to_owned()),
            ("code_verifier", code_verifier.to_owned()),
            ("redirect_uri", redirect_uri.to_owned()),
        ];
        for (key, value) in &self.oauth.extra_token_params {
            form.push((key.as_str(), value.clone()));
        }

        let response = self
            .http
            .post(&self.oauth.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|err| Error::Provider(format!("OAuth token exchange failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "OAuth token exchange failed with status {status}: {body}"
            )));
        }

        let payload: TokenResponse = response.json().await.map_err(|err| {
            Error::Provider(format!("failed to parse OAuth token response: {err}"))
        })?;

        Ok(self.to_stored_credential(payload))
    }

    pub async fn start_device_authorization(&self) -> Result<DeviceAuthorizationStart> {
        let Some(device_url) = &self.oauth.device_authorization_url else {
            return Err(Error::Config(format!(
                "provider '{}' subscription auth does not define device_authorization_url",
                self.provider_name
            )));
        };

        let response = self
            .http
            .post(device_url)
            .form(&[
                ("client_id", self.oauth.client_id.clone()),
                ("scope", self.oauth.scopes.join(" ")),
            ])
            .send()
            .await
            .map_err(|err| {
                Error::Provider(format!("device authorization request failed: {err}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "device authorization request failed with status {status}: {body}"
            )));
        }

        let payload: DeviceAuthorizationResponse = response.json().await.map_err(|err| {
            Error::Provider(format!(
                "failed to parse device authorization response: {err}"
            ))
        })?;

        Ok(DeviceAuthorizationStart {
            device_code: payload.device_code,
            user_code: payload.user_code,
            verification_uri: payload.verification_uri,
            verification_uri_complete: payload.verification_uri_complete,
            expires_in_secs: payload.expires_in,
            interval_secs: payload.interval.unwrap_or(5),
        })
    }

    pub async fn poll_device_authorization(
        &self,
        start: &DeviceAuthorizationStart,
    ) -> Result<StoredCredential> {
        let deadline = Utc::now().timestamp() + start.expires_in_secs as i64;
        let mut interval_secs = start.interval_secs.max(1);

        loop {
            if Utc::now().timestamp() >= deadline {
                return Err(Error::Provider(
                    "device authorization expired before completion".to_owned(),
                ));
            }

            let response = self
                .http
                .post(&self.oauth.token_url)
                .form(&[
                    (
                        "grant_type",
                        "urn:ietf:params:oauth:grant-type:device_code".to_owned(),
                    ),
                    ("client_id", self.oauth.client_id.clone()),
                    ("device_code", start.device_code.clone()),
                ])
                .send()
                .await
                .map_err(|err| {
                    Error::Provider(format!("device authorization polling failed: {err}"))
                })?;

            if response.status().is_success() {
                let payload: TokenResponse = response.json().await.map_err(|err| {
                    Error::Provider(format!("failed to parse device token response: {err}"))
                })?;
                return Ok(self.to_stored_credential(payload));
            }

            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let error_code = serde_json::from_str::<Value>(&body).ok().and_then(|value| {
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });

            match error_code.as_deref() {
                Some("authorization_pending") => {
                    tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                    continue;
                }
                Some("slow_down") => {
                    interval_secs += 5;
                    tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                    continue;
                }
                Some("access_denied") => {
                    return Err(Error::Provider(
                        "device authorization denied by user".to_owned(),
                    ));
                }
                Some("expired_token") => {
                    return Err(Error::Provider(
                        "device authorization token expired".to_owned(),
                    ));
                }
                _ => {
                    return Err(Error::Provider(format!(
                        "device authorization polling failed with status {status}: {body}"
                    )));
                }
            }
        }
    }

    pub async fn save_credential(&self, credential: StoredCredential) -> Result<()> {
        self.store.upsert(credential).await
    }

    async fn refresh_token(
        &self,
        refresh_token: String,
        fallback_scopes: Vec<String>,
    ) -> Result<StoredCredential> {
        let mut form = vec![
            ("grant_type", "refresh_token".to_owned()),
            ("client_id", self.oauth.client_id.clone()),
            ("refresh_token", refresh_token),
        ];
        for (key, value) in &self.oauth.extra_token_params {
            form.push((key.as_str(), value.clone()));
        }

        let response = self
            .http
            .post(&self.oauth.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|err| Error::Provider(format!("subscription token refresh failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "subscription token refresh failed with status {status}: {body}"
            )));
        }

        let payload: TokenResponse = response.json().await.map_err(|err| {
            Error::Provider(format!(
                "failed to parse subscription refresh response: {err}"
            ))
        })?;

        let mut credential = self.to_stored_credential(payload);
        if credential.scopes.is_empty() {
            credential.scopes = fallback_scopes;
        }
        Ok(credential)
    }

    fn to_stored_credential(&self, payload: TokenResponse) -> StoredCredential {
        let now = Utc::now().timestamp();
        let expires_at_epoch_secs = payload.expires_in.map(|value| now + value as i64);
        let scopes = payload
            .scope
            .as_deref()
            .map(|raw| {
                raw.split_whitespace()
                    .filter(|item| !item.trim().is_empty())
                    .map(|item| item.to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        StoredCredential {
            provider_name: self.provider_name.clone(),
            access_token: payload.access_token,
            refresh_token: payload.refresh_token,
            token_type: payload.token_type.unwrap_or_else(|| "Bearer".to_owned()),
            expires_at_epoch_secs,
            scopes,
            metadata: BTreeMap::new(),
            created_at_epoch_secs: now,
            updated_at_epoch_secs: now,
        }
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: Option<String>,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthorizationResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

pub fn resolve_auth_store_path(config: &Config, work_dir: &Path) -> PathBuf {
    let paths = StoragePaths::resolve(work_dir, config);
    paths.global_data_dir.join(AUTH_STORE_FILE_NAME)
}

fn parse_subscription_oauth_config(provider: &ProviderConfig) -> Result<SubscriptionOAuthConfig> {
    let maybe = provider
        .settings
        .as_ref()
        .and_then(|value| value.get("subscription_auth"));

    let mut defaults = default_subscription_oauth_config_for_provider(&provider.provider_type);

    if let Some(raw) = maybe {
        let object = raw.as_object().ok_or_else(|| {
            Error::Config(format!(
                "provider '{}' settings.subscription_auth must be an object",
                provider.name
            ))
        })?;

        if let Some(value) = object.get("client_id").and_then(Value::as_str) {
            defaults.client_id = value.to_owned();
        }
        if let Some(value) = object.get("authorize_url").and_then(Value::as_str) {
            defaults.authorize_url = value.to_owned();
        }
        if let Some(value) = object.get("token_url").and_then(Value::as_str) {
            defaults.token_url = value.to_owned();
        }
        if let Some(value) = object.get("device_authorization_url") {
            defaults.device_authorization_url = value.as_str().map(ToOwned::to_owned);
        }
        if let Some(value) = object.get("scopes") {
            defaults.scopes = parse_string_vec(value, "scopes", &provider.name)?;
        }
        if let Some(value) = object.get("redirect_host").and_then(Value::as_str) {
            defaults.redirect_host = value.to_owned();
        }
        if let Some(value) = object.get("redirect_port").and_then(Value::as_u64) {
            defaults.redirect_port = value as u16;
        }
        if let Some(value) = object.get("extra_authorize_params") {
            defaults.extra_authorize_params =
                parse_string_map(value, "extra_authorize_params", &provider.name)?;
        }
        if let Some(value) = object.get("extra_token_params") {
            defaults.extra_token_params =
                parse_string_map(value, "extra_token_params", &provider.name)?;
        }
    }

    if defaults.client_id.trim().is_empty() {
        return Err(Error::Config(format!(
            "provider '{}' requires settings.subscription_auth.client_id",
            provider.name
        )));
    }

    if defaults.authorize_url.trim().is_empty() {
        return Err(Error::Config(format!(
            "provider '{}' requires settings.subscription_auth.authorize_url",
            provider.name
        )));
    }

    if defaults.token_url.trim().is_empty() {
        return Err(Error::Config(format!(
            "provider '{}' requires settings.subscription_auth.token_url",
            provider.name
        )));
    }

    if defaults.scopes.is_empty() {
        return Err(Error::Config(format!(
            "provider '{}' requires non-empty settings.subscription_auth.scopes",
            provider.name
        )));
    }

    Ok(defaults)
}

fn default_subscription_oauth_config_for_provider(
    provider_type: &ProviderType,
) -> SubscriptionOAuthConfig {
    match provider_type {
        ProviderType::OpenAi => SubscriptionOAuthConfig {
            client_id: String::new(),
            authorize_url: "https://auth.openai.com/oauth/authorize".to_owned(),
            token_url: "https://auth.openai.com/oauth/token".to_owned(),
            device_authorization_url: Some("https://auth.openai.com/oauth/device/code".to_owned()),
            scopes: vec![
                "openid".to_owned(),
                "profile".to_owned(),
                "offline_access".to_owned(),
            ],
            redirect_host: "127.0.0.1".to_owned(),
            redirect_port: 8787,
            extra_authorize_params: BTreeMap::new(),
            extra_token_params: BTreeMap::new(),
        },
        ProviderType::Anthropic
        | ProviderType::Google
        | ProviderType::ZAi
        | ProviderType::Grok
        | ProviderType::Ollama
        | ProviderType::Custom => SubscriptionOAuthConfig {
            client_id: String::new(),
            authorize_url: String::new(),
            token_url: String::new(),
            device_authorization_url: None,
            scopes: Vec::new(),
            redirect_host: "127.0.0.1".to_owned(),
            redirect_port: 8787,
            extra_authorize_params: BTreeMap::new(),
            extra_token_params: BTreeMap::new(),
        },
    }
}

fn parse_string_vec(value: &Value, field: &str, provider_name: &str) -> Result<Vec<String>> {
    let array = value.as_array().ok_or_else(|| {
        Error::Config(format!(
            "provider '{}' settings.subscription_auth.{field} must be an array of strings",
            provider_name
        ))
    })?;
    let mut result = Vec::with_capacity(array.len());
    for item in array {
        let text = item.as_str().ok_or_else(|| {
            Error::Config(format!(
                "provider '{}' settings.subscription_auth.{field} must only contain strings",
                provider_name
            ))
        })?;
        result.push(text.to_owned());
    }
    Ok(result)
}

fn parse_string_map(
    value: &Value,
    field: &str,
    provider_name: &str,
) -> Result<BTreeMap<String, String>> {
    let object = value.as_object().ok_or_else(|| {
        Error::Config(format!(
            "provider '{}' settings.subscription_auth.{field} must be an object of string values",
            provider_name
        ))
    })?;
    let mut result = BTreeMap::new();
    for (key, value) in object {
        let text = value.as_str().ok_or_else(|| {
            Error::Config(format!(
                "provider '{}' settings.subscription_auth.{field}.{key} must be a string",
                provider_name
            ))
        })?;
        result.insert(key.clone(), text.to_owned());
    }
    Ok(result)
}

fn is_token_valid(credential: &StoredCredential) -> bool {
    if credential.access_token.trim().is_empty() {
        return false;
    }

    match credential.expires_at_epoch_secs {
        None => true,
        Some(expires_at) => {
            let now = Utc::now().timestamp();
            expires_at > now + DEFAULT_TOKEN_REFRESH_SKEW_SECS
        }
    }
}

fn http_response(status_code: u16, message: &str) -> String {
    let status = match status_code {
        200 => "200 OK",
        400 => "400 Bad Request",
        405 => "405 Method Not Allowed",
        _ => "500 Internal Server Error",
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        message.len(),
        message
    )
}
