//! The HTTP client itself: one generic [`AurumClient::request`] plus typed
//! shortcuts. Endpoint-specific wrappers are added per route group on top of it.

use aurum_domain::health::Health;
use base64::Engine;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::config::{BasicAuth, ClientConfig};
use crate::error::ClientError;

#[derive(Debug, Clone)]
pub struct AurumClient {
    base_url: String,
    auth: Option<BasicAuth>,
    headers: Vec<(String, String)>,
    http: reqwest::Client,
}

/// Sets `name`, replacing a previous entry spelled identically (JS-object
/// semantics: `Accept` and `accept` stay two entries and are both sent).
fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    match headers.iter_mut().find(|(existing, _)| existing == name) {
        Some(entry) => entry.1 = value,
        None => headers.push((name.to_string(), value)),
    }
}

/// Decodes a response body like `Response.text()`: lossy UTF-8, BOM dropped.
fn decode_text(body: &[u8]) -> String {
    let body = body.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(body);
    String::from_utf8_lossy(body).into_owned()
}

impl AurumClient {
    /// Shortcut for a client with default timeout/headers.
    /// `base_url` is the API root including `/api`, e.g. `https://aurum.example.com/api`.
    pub fn new(base_url: impl Into<String>, auth: Option<BasicAuth>) -> Result<Self, ClientError> {
        Self::with_config(ClientConfig { auth, ..ClientConfig::new(base_url) })
    }

    pub fn with_config(config: ClientConfig) -> Result<Self, ClientError> {
        let base_url = config.base_url.trim_end_matches('/').to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(ClientError::InvalidUrl(base_url));
        }
        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("aurum-client/", env!("CARGO_PKG_VERSION")))
            .timeout(config.timeout);
        if !config.system_proxy {
            builder = builder.no_proxy();
        }
        Ok(Self { base_url, auth: config.auth, headers: config.headers, http: builder.build()? })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn build_url(&self, path: &str, query: &[(String, String)]) -> Result<Url, ClientError> {
        let separator = if path.starts_with('/') { "" } else { "/" };
        let mut url = Url::parse(&format!("{}{separator}{path}", self.base_url))
            .map_err(|error| ClientError::InvalidUrl(format!("{path}: {error}")))?;
        // Only touch the query when there is something to add: serializing an
        // empty list would rewrite a query already present in `path`.
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in query {
                pairs.append_pair(name, value);
            }
        }
        Ok(url)
    }

    fn build_headers(&self, has_body: bool) -> Result<HeaderMap, ClientError> {
        let mut headers: Vec<(String, String)> = Vec::new();
        set_header(&mut headers, "Accept", "application/json".to_string());
        if has_body {
            set_header(&mut headers, "Content-Type", "application/json".to_string());
        }
        for (name, value) in &self.headers {
            set_header(&mut headers, name, value.clone());
        }
        if let Some(auth) = &self.auth {
            let credentials =
                base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", auth.username, auth.password));
            set_header(&mut headers, "Authorization", format!("Basic {credentials}"));
        }

        // Like `fetch`, entries whose names differ only by case are merged
        // into one header whose values are joined with ", ".
        let mut merged: Vec<(HeaderName, String)> = Vec::new();
        for (name, value) in headers {
            let header_name =
                HeaderName::from_bytes(name.as_bytes()).map_err(|_| ClientError::InvalidHeaderName(name.clone()))?;
            HeaderValue::from_str(&value)
                .map_err(|_| ClientError::InvalidHeaderValue { name, value: value.clone() })?;
            match merged.iter_mut().find(|(existing, _)| *existing == header_name) {
                Some((_, joined)) => {
                    joined.push_str(", ");
                    joined.push_str(&value);
                }
                None => merged.push((header_name, value)),
            }
        }

        let mut map = HeaderMap::new();
        for (name, value) in merged {
            // Validated above; the joined form only adds ", " separators.
            let mut header_value = HeaderValue::from_str(&value)
                .map_err(|_| ClientError::InvalidHeaderValue { name: name.to_string(), value: value.clone() })?;
            header_value.set_sensitive(name == reqwest::header::AUTHORIZATION);
            map.insert(name, header_value);
        }
        Ok(map)
    }

    /// Performs one request and returns the decoded payload of a 2xx answer.
    ///
    /// - `path` is relative to the API root; a missing leading `/` is added.
    /// - `query` pairs are form-urlencoded and appended in order.
    /// - an empty 2xx body yields `{"ok": true, "status": N}`;
    ///   a non-JSON body yields `Value::String(text)`.
    ///
    /// Non-2xx answers become [`ClientError::Api`].
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        body: Option<&Value>,
    ) -> Result<Value, ClientError> {
        let body = body.map(|value| serde_json::to_vec(value).expect("a JSON value always serializes"));
        self.request_raw(method, path, query, body).await
    }

    /// Like [`Self::request`], but the caller supplies the already serialized
    /// JSON body (when its exact bytes matter).
    pub async fn request_raw(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        body: Option<Vec<u8>>,
    ) -> Result<Value, ClientError> {
        let method =
            Method::from_bytes(method.as_bytes()).map_err(|_| ClientError::InvalidMethod(method.to_string()))?;
        let url = self.build_url(path, query)?;
        let mut request = self.http.request(method, url).headers(self.build_headers(body.is_some())?);
        if let Some(body) = body {
            request = request.body(body);
        }

        let map_error = |error: reqwest::Error| {
            if error.is_timeout() { ClientError::Timeout } else { ClientError::Transport(error) }
        };
        let response = request.send().await.map_err(map_error)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(map_error)?;

        let text = decode_text(&bytes);
        let payload = if text.is_empty() {
            json!({ "ok": true, "status": status.as_u16() })
        } else {
            // Preserve non-JSON proxy errors: they are more useful than a parsing failure.
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        if !status.is_success() {
            let reason = status.canonical_reason().unwrap_or("").to_string();
            return Err(ClientError::Api { status: status.as_u16(), reason, payload });
        }
        Ok(payload)
    }

    /// [`Self::request`] decoded into a typed DTO (types live in `aurum-domain`).
    pub async fn request_as<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        body: Option<&Value>,
    ) -> Result<T, ClientError> {
        let value = self.request(method, path, query, body).await?;
        serde_json::from_value(value).map_err(|error| ClientError::Decode(error.to_string()))
    }

    pub async fn get(&self, path: &str, query: &[(String, String)]) -> Result<Value, ClientError> {
        self.request("GET", path, query, None).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value, ClientError> {
        self.request("POST", path, &[], Some(body)).await
    }

    pub async fn patch(&self, path: &str, body: &Value) -> Result<Value, ClientError> {
        self.request("PATCH", path, &[], Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> Result<Value, ClientError> {
        self.request("DELETE", path, &[], None).await
    }

    /// `GET /health` (unauthenticated on a stock deployment).
    pub async fn health(&self) -> Result<Health, ClientError> {
        self.request_as("GET", "/health", &[], None).await
    }
}
