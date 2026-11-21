//! Application configuration, sourced from environment variables (.env) —
//! port of `backend/app/core/config.py`.
//!
//! Resolution matches pydantic-settings (`env_prefix="AURUM_"`,
//! `env_file=".env"`, `extra="ignore"`): a real environment variable beats
//! the `.env` file in the working directory, which beats the default; names
//! are matched case-insensitively; unknown `AURUM_*` keys are ignored.

use std::collections::HashMap;
use std::path::Path;

use sqlx::postgres::PgConnectOptions;

/// Python's hard-coded NBP endpoint (`services/fx_service.py::NBP_BASE_URL`).
pub const DEFAULT_NBP_BASE_URL: &str = "https://api.nbp.pl/api/exchangerates/rates";
/// Python's hard-coded CoinGecko endpoint (`services/crypto_service.py::COINGECKO_BASE_URL`).
pub const DEFAULT_COINGECKO_BASE_URL: &str = "https://api.coingecko.com/api/v3";

const PREFIX: &str = "aurum_";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid value for AURUM_{key}: {value:?} ({reason})")]
    Invalid { key: String, value: String, reason: &'static str },
    #[error("cannot read .env: {0}")]
    DotEnv(#[from] dotenvy::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    // Postgres connection
    pub postgres_user: String,
    pub postgres_password: String,
    pub postgres_db: String,
    pub postgres_host: String,
    pub postgres_port: u16,

    /// Base currency used by every cross-account financial aggregate.
    pub default_currency: String,

    /// Comma-separated list of browser origins allowed to call the API. Empty
    /// by default, which allows none: the shipped compose serves the UI and the
    /// API from one nginx, and the Vite dev server proxies /api, so neither is
    /// a cross-origin caller. Set it only for a genuinely separate frontend.
    pub cors_origins: String,

    /// CoinGecko Demo API key (free, no card required) for the crypto
    /// service's price lookups. Empty by default; the Crypto tab's endpoints
    /// 400 with a clear message until this is set, rather than silently
    /// hitting CoinGecko's much stingier keyless tier.
    pub coingecko_api_key: String,

    /// Swagger/ReDoc/openapi.json at /api/docs. On by default because the API
    /// is a documented feature of this app (see DOCS.md), and because when
    /// basic auth is configured these sit behind it like everything else under
    /// /api/. Turn it off on an instance that's reachable from the internet
    /// *without* basic auth: an open /api/openapi.json is a complete,
    /// machine-readable map of every endpoint and payload shape.
    pub enable_docs: bool,

    // New in the Rust server (Python got these from the uvicorn command line
    // or hard-coded them): listen address and upstream provider endpoints,
    // the latter overridable so tests can point them at a stub.
    pub host: String,
    pub port: u16,
    pub nbp_base_url: String,
    pub coingecko_base_url: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            postgres_user: "aurum".into(),
            postgres_password: "aurum".into(),
            postgres_db: "aurum".into(),
            postgres_host: "db".into(),
            postgres_port: 5432,
            default_currency: "USD".into(),
            cors_origins: String::new(),
            coingecko_api_key: String::new(),
            enable_docs: true,
            host: "0.0.0.0".into(),
            port: 8000,
            nbp_base_url: DEFAULT_NBP_BASE_URL.into(),
            coingecko_base_url: DEFAULT_COINGECKO_BASE_URL.into(),
        }
    }
}

impl Settings {
    /// Process environment + `./.env` (if present), like pydantic-settings.
    pub fn from_env() -> Result<Self, ConfigError> {
        let dotenv = if Path::new(".env").is_file() {
            dotenvy::from_path_iter(".env")?.collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        Self::from_sources(dotenv, std::env::vars())
    }

    /// Later sources win: `.env` entries first, then the real environment.
    pub fn from_sources(
        dotenv: impl IntoIterator<Item = (String, String)>,
        env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, ConfigError> {
        let mut values: HashMap<String, String> = HashMap::new();
        for (key, value) in dotenv.into_iter().chain(env) {
            let key = key.to_ascii_lowercase();
            if let Some(name) = key.strip_prefix(PREFIX) {
                values.insert(name.to_string(), value);
            }
        }
        let mut s = Self::default();
        let take = |name: &str, target: &mut String| {
            if let Some(v) = values.get(name) {
                *target = v.clone();
            }
        };
        take("postgres_user", &mut s.postgres_user);
        take("postgres_password", &mut s.postgres_password);
        take("postgres_db", &mut s.postgres_db);
        take("postgres_host", &mut s.postgres_host);
        take("default_currency", &mut s.default_currency);
        take("cors_origins", &mut s.cors_origins);
        take("coingecko_api_key", &mut s.coingecko_api_key);
        take("host", &mut s.host);
        take("nbp_base_url", &mut s.nbp_base_url);
        take("coingecko_base_url", &mut s.coingecko_base_url);
        if let Some(v) = values.get("postgres_port") {
            s.postgres_port = parse_port("POSTGRES_PORT", v)?;
        }
        if let Some(v) = values.get("port") {
            s.port = parse_port("PORT", v)?;
        }
        if let Some(v) = values.get("enable_docs") {
            s.enable_docs = parse_bool("ENABLE_DOCS", v)?;
        }
        Ok(s)
    }

    /// Exactly `Settings.cors_origins_list` from config.py.
    pub fn cors_origins_list(&self) -> Vec<String> {
        if self.cors_origins == "*" {
            return vec!["*".into()];
        }
        self.cors_origins.split(',').map(str::trim).filter(|o| !o.is_empty()).map(String::from).collect()
    }

    /// Connection options for the configured database.
    pub fn connect_options(&self) -> PgConnectOptions {
        self.connect_options_for(&self.postgres_db)
    }

    /// Same server/credentials, another database (used by tooling/tests).
    pub fn connect_options_for(&self, database: &str) -> PgConnectOptions {
        PgConnectOptions::new()
            .host(&self.postgres_host)
            .port(self.postgres_port)
            .username(&self.postgres_user)
            .password(&self.postgres_password)
            .database(database)
            .application_name("aurum-server")
    }
}

fn parse_port(key: &str, value: &str) -> Result<u16, ConfigError> {
    value.trim().parse().map_err(|_| ConfigError::Invalid {
        key: key.into(),
        value: value.into(),
        reason: "expected a port number",
    })
}

/// pydantic's lax bool parsing.
fn parse_bool(key: &str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "t" | "true" | "y" | "yes" => Ok(true),
        "0" | "off" | "f" | "false" | "n" | "no" => Ok(false),
        _ => Err(ConfigError::Invalid { key: key.into(), value: value.into(), reason: "expected a boolean" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn env_beats_dotenv_and_keys_are_case_insensitive() {
        let s = Settings::from_sources(
            kv(&[("AURUM_POSTGRES_HOST", "fromfile"), ("AURUM_PORT", "9000")]),
            kv(&[("aurum_postgres_host", "fromenv"), ("AURUM_ENABLE_DOCS", "off"), ("OTHER", "x")]),
        )
        .unwrap();
        assert_eq!(s.postgres_host, "fromenv");
        assert_eq!(s.port, 9000);
        assert!(!s.enable_docs);
        assert_eq!(s.default_currency, "USD");
        assert_eq!(s.coingecko_base_url, DEFAULT_COINGECKO_BASE_URL);
    }

    #[test]
    fn cors_list_matches_python() {
        let mut s = Settings::default();
        assert!(s.cors_origins_list().is_empty());
        s.cors_origins = "*".into();
        assert_eq!(s.cors_origins_list(), vec!["*"]);
        s.cors_origins = " https://a.example , ,https://b.example,".into();
        assert_eq!(s.cors_origins_list(), vec!["https://a.example", "https://b.example"]);
        // Only the exact string "*" is the wildcard shortcut.
        s.cors_origins = "*, https://a.example".into();
        assert_eq!(s.cors_origins_list(), vec!["*", "https://a.example"]);
    }

    #[test]
    fn invalid_values_fail_loudly() {
        assert!(Settings::from_sources(kv(&[]), kv(&[("AURUM_POSTGRES_PORT", "abc")])).is_err());
        assert!(Settings::from_sources(kv(&[]), kv(&[("AURUM_ENABLE_DOCS", "maybe")])).is_err());
    }
}
