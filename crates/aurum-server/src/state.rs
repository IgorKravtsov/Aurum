//! Shared application state handed to every handler (`State<AppState>`).

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;

use crate::config::Settings;

#[derive(Clone)]
pub struct AppState {
    /// The Postgres pool (the equivalent of Python's `AsyncSessionLocal`).
    pub pool: PgPool,
    /// Immutable settings resolved at startup (`get_settings()` in Python).
    pub settings: Arc<Settings>,
    /// One shared HTTP client for upstream providers (NBP, CoinGecko).
    /// Python opened a fresh `httpx.AsyncClient` per call; per-call timeouts
    /// (12 s for NBP, 10 s for CoinGecko there) are set on each request with
    /// `RequestBuilder::timeout`, not here.
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(pool: PgPool, settings: Settings) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build()?;
        Ok(Self { pool, settings: Arc::new(settings), http })
    }
}
