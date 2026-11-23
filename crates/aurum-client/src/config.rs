//! Plain-data client configuration. The client never reads files or the
//! environment: whoever embeds it (CLI config, iOS Keychain, Android
//! Keystore) resolves the values and passes them in (ADR 0001).

use std::time::Duration;

/// Default for [`ClientConfig::timeout`], matches the Node CLI (30 s).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Optional HTTP Basic credentials (the nginx front of a self-hosted instance).
#[derive(Clone)]
pub struct BasicAuth {
    pub username: String,
    pub password: String,
}

// Hand-written so credentials can never leak through `{:?}` in logs.
impl std::fmt::Debug for BasicAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BasicAuth").field("username", &self.username).field("password", &"<redacted>").finish()
    }
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// API root *including* the `/api` prefix, e.g. `https://aurum.example.com/api`.
    /// Request paths (`/accounts`, `/health`, ...) are appended to it.
    pub base_url: String,
    pub auth: Option<BasicAuth>,
    /// Covers the whole exchange: connecting, sending and reading the body.
    pub timeout: Duration,
    /// Extra request headers. A name that already appears here with exactly
    /// the same spelling replaces the earlier value (object semantics, like
    /// the Node CLI); the `Authorization` header derived from `auth` wins
    /// over a user-supplied one of the same spelling.
    pub headers: Vec<(String, String)>,
    /// Honour `HTTP(S)_PROXY`/`NO_PROXY`. Off by default: a self-hosted API
    /// is usually on localhost or a VPN and the Node CLI never used proxies.
    pub system_proxy: bool,
}

impl ClientConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            auth: None,
            timeout: DEFAULT_TIMEOUT,
            headers: Vec::new(),
            system_proxy: false,
        }
    }
}
