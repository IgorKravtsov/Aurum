//! Errors surfaced to callers (and later mapped across the FFI boundary).

use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid base URL: {0}")]
    InvalidUrl(String),
    #[error("invalid HTTP method: {0}")]
    InvalidMethod(String),
    #[error("invalid HTTP header name: {0}")]
    InvalidHeaderName(String),
    #[error("invalid value for HTTP header {name}: {value}")]
    InvalidHeaderValue { name: String, value: String },
    /// The request did not finish within [`crate::ClientConfig::timeout`].
    #[error("request timed out")]
    Timeout,
    /// A 2xx answer whose JSON does not fit the requested type.
    #[error("unexpected response shape: {0}")]
    Decode(String),
    #[error("request failed: {0}")]
    Transport(#[from] reqwest::Error),
    /// A non-2xx answer. `payload` is the whole decoded body: parsed JSON when
    /// the body is JSON (FastAPI keeps its message in `{"detail": ...}`), the
    /// raw text when it is not (e.g. a proxy error page), or
    /// `{"ok": true, "status": N}` when the body is empty.
    #[error("server answered {status} {reason}: {payload}")]
    Api { status: u16, reason: String, payload: Value },
}
