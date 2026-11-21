//! `aurum-client` — async client for the Aurum REST API.
//!
//! Skeleton: only [`AurumClient::health`] exists so far. Endpoints are added
//! per route group, reusing the request/response types from `aurum-domain`
//! so the client can never drift from the server's contract. Credentials are
//! passed in by the caller (CLI config, iOS Keychain, Android Keystore);
//! this crate never persists anything (ADR 0001).

use aurum_domain::health::Health;
use serde::de::DeserializeOwned;

/// Errors surfaced to callers (and later mapped across the FFI boundary).
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid base URL: {0}")]
    InvalidUrl(String),
    #[error("request failed: {0}")]
    Transport(#[from] reqwest::Error),
    /// A non-2xx answer; `detail` is FastAPI's `{"detail": ...}` payload when present.
    #[error("server answered {status}: {detail}")]
    Api { status: u16, detail: serde_json::Value },
}

/// Optional HTTP Basic credentials (the nginx front of a self-hosted instance).
#[derive(Debug, Clone)]
pub struct BasicAuth {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone)]
pub struct AurumClient {
    base_url: String,
    auth: Option<BasicAuth>,
    http: reqwest::Client,
}

impl AurumClient {
    /// `base_url` is the instance root, e.g. `https://aurum.example.com`
    /// (the `/api/...` prefix is added per request).
    pub fn new(base_url: impl Into<String>, auth: Option<BasicAuth>) -> Result<Self, ClientError> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(ClientError::InvalidUrl(base_url));
        }
        let http =
            reqwest::Client::builder().user_agent(concat!("aurum-client/", env!("CARGO_PKG_VERSION"))).build()?;
        Ok(Self { base_url, auth, http })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let mut request = self.http.get(self.url(path));
        if let Some(auth) = &self.auth {
            request = request.basic_auth(&auth.username, Some(&auth.password));
        }
        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
            let detail = body.get("detail").cloned().unwrap_or(body);
            return Err(ClientError::Api { status: status.as_u16(), detail });
        }
        Ok(response.json().await?)
    }

    /// `GET /api/health` (unauthenticated on a stock deployment).
    pub async fn health(&self) -> Result<Health, ClientError> {
        self.get_json("/api/health").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_base_url() {
        assert!(matches!(AurumClient::new("localhost:8000", None), Err(ClientError::InvalidUrl(_))));
        assert!(AurumClient::new("http://localhost:8000/", None).is_ok());
    }
}
