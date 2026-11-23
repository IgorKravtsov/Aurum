//! The exported `AurumClient` object: a thin async wrapper over `aurum-client`.
//!
//! The core stores nothing: the base URL and basic-auth credentials are handed
//! in by the app, which keeps them in the platform secret store (iOS Keychain,
//! Android Keystore/EncryptedSharedPreferences) and never lets the core write
//! them to disk (ADR 0001).

use std::sync::Arc;
use std::time::Duration;

use aurum_client::{BasicAuth, ClientConfig};
use aurum_domain::health::Health;

use crate::error::AurumError;

/// One `name=value` pair of a URL query string. A list (not a map) so order
/// and repeated names survive, like in the underlying client.
#[derive(Debug, Clone, uniffi::Record)]
pub struct QueryParam {
    pub name: String,
    pub value: String,
}

/// `aurum_domain::health::Health` exported as-is (one DTO for server, CLI and apps).
#[uniffi::remote(Record)]
pub struct Health {
    pub status: String,
}

#[derive(uniffi::Object)]
pub struct AurumClient {
    inner: aurum_client::AurumClient,
}

#[uniffi::export(async_runtime = "tokio")]
impl AurumClient {
    /// `base_url` is the API root including `/api`
    /// (`https://aurum.example.com/api`). `username` and `password` go
    /// together (HTTP Basic) or not at all. `timeout_ms` covers the whole
    /// exchange; `None` keeps the default (30 s).
    #[uniffi::constructor]
    pub fn new(
        base_url: String,
        username: Option<String>,
        password: Option<String>,
        timeout_ms: Option<u64>,
    ) -> Result<Arc<Self>, AurumError> {
        let auth = match (username, password) {
            (Some(username), Some(password)) => Some(BasicAuth { username, password }),
            (None, None) => None,
            _ => {
                return Err(AurumError::InvalidArgument {
                    message: "username and password must be given together".to_string(),
                });
            }
        };
        let mut config = ClientConfig { auth, ..ClientConfig::new(base_url) };
        if let Some(timeout_ms) = timeout_ms {
            config.timeout = Duration::from_millis(timeout_ms);
        }
        Ok(Arc::new(Self { inner: aurum_client::AurumClient::with_config(config)? }))
    }

    /// `GET /health` — status only; works without credentials on a stock deployment.
    pub async fn health(&self) -> Result<Health, AurumError> {
        Ok(self.inner.health().await?)
    }

    /// Generic call covering every endpoint. `path` is relative to the API root
    /// (`/accounts`), `body_json` is a JSON document sent byte for byte, and the
    /// result is the decoded answer re-serialized as a JSON string (an empty 2xx
    /// body gives `{"ok":true,"status":N}`; a non-JSON one a JSON string).
    pub async fn request(
        &self,
        method: String,
        path: String,
        query: Vec<QueryParam>,
        body_json: Option<String>,
    ) -> Result<String, AurumError> {
        let query: Vec<(String, String)> = query.into_iter().map(|pair| (pair.name, pair.value)).collect();
        if let Some(body) = &body_json {
            serde_json::from_str::<serde_json::Value>(body)
                .map_err(|error| AurumError::InvalidArgument { message: format!("body is not valid JSON: {error}") })?;
        }
        let value = self.inner.request_raw(&method, &path, &query, body_json.map(String::into_bytes)).await?;
        Ok(value.to_string())
    }
}
