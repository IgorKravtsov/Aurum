//! Business logic for `/api/crypto` — port of the former Python backend's `app/services/crypto_service.py`.
//!
//! Crypto price sync + position accounting on top of the existing
//! Asset/AssetValuation engine: a holding's value is an ordinary
//! `asset_valuations` row, so Net Worth and reports treat a crypto holding
//! like any other manually tracked asset. Quantity and average buy price are
//! never stored — they're replayed from the transaction log every time
//! (`aurum_domain::crypto::compute_position`).
//!
//! Refreshes are never automatic in the background. Three triggers only:
//! the "Refresh prices" button (`force`), a lazy once-a-day check on
//! `GET /crypto/holdings`, and adding a brand new holding (a one-off seed
//! fetch). A buy/sell never calls CoinGecko — it reuses the cached price.
//!
//! Like the Python ORM objects, a holding is kept in memory while a request
//! changes it, and the response is built from that in-memory state: values
//! just taken from a request body or from CoinGecko keep their own digits
//! (`"0.5"`, `"50000"`), while values read back from Postgres carry the
//! column scale (`"0.500000000000000000"`).

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::Duration;

use aurum_domain::crypto::{
    AUTO_REFRESH_INTERVAL_HOURS, CryptoHistoryResponse, CryptoHolding, CryptoHoldingCreate, CryptoHoldingRead,
    CryptoHoldingUpdate, CryptoPerformancePoint, CryptoPerformanceResponse, CryptoPortfolioCreate, CryptoPortfolioRead,
    CryptoPortfolioUpdate, CryptoSearchResult, CryptoSyncResult, CryptoTransactionCreate, CryptoTransactionRead,
    CryptoTransactionUpdate, DEFAULT_PORTFOLIO_NAME, ERROR_KEY_UNREACHABLE, MARKET_CHANGE_WINDOWS, MarketPoint,
    PORTFOLIO_PALETTE, UpstreamShapeError, compute_position, crypto_history, holding_read, parse_90d_change,
    parse_markets, parse_search, portfolio_color, sort_by_invested,
};
use aurum_domain::enums::{CryptoTransactionType, RiskLevel};
use aurum_domain::validation::Patch;
use aurum_domain::{Decimal, PyDateTime, PyFloat};
use chrono::{DateTime, Local, NaiveDate, SubsecRound, Utc};
use sqlx::{PgConnection, PgPool};

use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::services::settings::get_or_create_app_settings;
use crate::state::AppState;

/// Python's per-call `httpx.AsyncClient(timeout=10.0)`.
const COINGECKO_TIMEOUT: Duration = Duration::from_secs(10);

/// `_get_holding_or_404`'s detail.
const HOLDING_NOT_FOUND: &str = "Crypto holding not found";
const PORTFOLIO_NOT_FOUND: &str = "Crypto portfolio not found";
const TRANSACTION_NOT_FOUND: &str = "Crypto transaction not found";
const OVERSELL: &str = "Cannot sell more than you currently hold";

/// `_require_api_key`'s 400 — the Crypto tab's endpoints refuse to run
/// keyless rather than silently hitting CoinGecko's much stingier tier.
fn require_api_key(state: &AppState) -> ApiResult<String> {
    let key = &state.settings.coingecko_api_key;
    if key.is_empty() {
        return Err(ApiError::bad_request(
            "CoinGecko API key not configured — set AURUM_COINGECKO_API_KEY in .env \
             (free Demo key, no card required: coingecko.com/en/api/pricing).",
        ));
    }
    Ok(key.clone())
}

impl From<UpstreamShapeError> for ApiError {
    fn from(e: UpstreamShapeError) -> Self {
        // A CoinGecko answer Python's service couldn't digest escaped there
        // as an uncaught exception, i.e. a 500.
        ApiError::internal(e)
    }
}

// ---------------------------------------------------------------------------
// CoinGecko transport
// ---------------------------------------------------------------------------

/// What Python caught as `httpx.HTTPError`: a transport failure (connect,
/// timeout, broken body) or a non-2xx status (`raise_for_status`).
#[derive(Debug)]
struct HttpError;

/// httpx doesn't follow redirects by default (a 3xx fails
/// `raise_for_status`), reqwest does — so CoinGecko gets its own client.
/// httpx's `timeout=10.0` is per phase (connect, each read), not a total.
fn coingecko_client(state: &AppState) -> reqwest::Client {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(COINGECKO_TIMEOUT)
                .read_timeout(COINGECKO_TIMEOUT)
                .build()
                .ok()
        })
        .clone()
        .unwrap_or_else(|| state.http.clone())
}

/// `GET {base}{path}?{params}` with the Demo-key header; the whole body on
/// a 2xx answer.
async fn coingecko_get(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    path: &str,
    params: &[(&str, &str)],
) -> Result<Vec<u8>, HttpError> {
    let url = format!("{}{path}", base_url.trim_end_matches('/'));
    let url = reqwest::Url::parse_with_params(&url, params).map_err(|_| HttpError)?;
    let response = client.get(url).header("x-cg-demo-api-key", api_key).send().await.map_err(|_| HttpError)?;
    let status = response.status();
    // httpx reads the whole body inside `client.get`, before the status check.
    let body = response.bytes().await.map_err(|_| HttpError)?;
    if !status.is_success() {
        return Err(HttpError);
    }
    Ok(body.to_vec())
}

/// `_fetch_market_data`: one batched `/coins/markets` call for every coin
/// (with 1h/24h/7d/30d/1y change), never one call per coin. The outer
/// error is the 400 for a missing key or a 500 for an undigestible answer;
/// the inner one is the `httpx.HTTPError` callers tolerate.
async fn fetch_market_data(
    state: &AppState,
    coingecko_ids: &[&str],
    vs_currency: &str,
) -> ApiResult<Result<HashMap<String, MarketPoint>, HttpError>> {
    if coingecko_ids.is_empty() {
        return Ok(Ok(HashMap::new()));
    }
    let api_key = require_api_key(state)?;
    let ids = coingecko_ids.join(",");
    let params =
        [("vs_currency", vs_currency), ("ids", ids.as_str()), ("price_change_percentage", MARKET_CHANGE_WINDOWS)];
    let client = coingecko_client(state);
    match coingecko_get(&client, &state.settings.coingecko_base_url, &api_key, "/coins/markets", &params).await {
        Ok(body) => Ok(Ok(parse_markets(&body)?)),
        Err(e) => Ok(Err(e)),
    }
}

/// `search_coins`: backs the "add a coin" picker. An outage is not caught
/// here (Python let the `httpx` error escape → 500).
pub async fn search_coins(state: &AppState, query: &str) -> ApiResult<Vec<CryptoSearchResult>> {
    let api_key = require_api_key(state)?;
    let client = coingecko_client(state);
    let body = coingecko_get(&client, &state.settings.coingecko_base_url, &api_key, "/search", &[("query", query)])
        .await
        .map_err(|_| ApiError::internal(anyhow::anyhow!("CoinGecko /search request failed")))?;
    Ok(parse_search(&body)?)
}

// ---------------------------------------------------------------------------
// Loading holdings (the ORM objects with `_EAGER` relationships)
// ---------------------------------------------------------------------------

/// A holding with its asset's risk level and its transaction log.
#[derive(Debug, Clone)]
struct Loaded {
    holding: CryptoHolding,
    risk_level: RiskLevel,
    transactions: Vec<CryptoTransactionRead>,
}

impl Loaded {
    fn read(&self) -> CryptoHoldingRead {
        holding_read(&self.holding, self.risk_level, &self.transactions)
    }

    fn quantity(&self) -> Decimal {
        compute_position(&self.transactions).quantity
    }
}

/// Attaches risk levels and transactions (`selectinload`) to holding rows.
async fn hydrate(conn: &mut PgConnection, holdings: Vec<CryptoHolding>) -> ApiResult<Vec<Loaded>> {
    if holdings.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i32> = holdings.iter().map(|h| h.asset_id).collect();
    let risk: HashMap<i32, RiskLevel> = repo::crypto::risk_levels(&mut *conn, &ids).await?.into_iter().collect();
    let mut by_asset: HashMap<i32, Vec<CryptoTransactionRead>> = HashMap::new();
    for tx in repo::crypto::transactions_of(&mut *conn, &ids).await? {
        by_asset.entry(tx.asset_id).or_default().push(tx);
    }
    holdings
        .into_iter()
        .map(|holding| {
            let risk_level = *risk
                .get(&holding.asset_id)
                .ok_or_else(|| ApiError::internal(anyhow::anyhow!("asset {} missing", holding.asset_id)))?;
            let transactions = by_asset.remove(&holding.asset_id).unwrap_or_default();
            Ok(Loaded { holding, risk_level, transactions })
        })
        .collect()
}

/// `list_holdings`: ordered by name, optionally one portfolio's.
async fn list_holdings(conn: &mut PgConnection, portfolio_id: Option<i64>) -> ApiResult<Vec<Loaded>> {
    let rows = repo::crypto::list_holdings(&mut *conn, portfolio_id).await?;
    hydrate(conn, rows).await
}

/// `_get_holding_or_404`.
async fn get_holding_or_404(conn: &mut PgConnection, asset_id: i64) -> ApiResult<Loaded> {
    let row =
        repo::crypto::get_holding(&mut *conn, asset_id).await?.ok_or_else(|| ApiError::not_found(HOLDING_NOT_FOUND))?;
    let mut loaded = hydrate(conn, vec![row]).await?;
    loaded.pop().ok_or_else(|| ApiError::internal(anyhow::anyhow!("holding {asset_id} vanished")))
}

/// `datetime.now(timezone.utc)` — microsecond precision, like Python's.
fn utc_now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// `date.today()` (the server's local date).
fn today() -> NaiveDate {
    Local::now().date_naive()
}

// ---------------------------------------------------------------------------
// Portfolios
// ---------------------------------------------------------------------------

pub async fn list_portfolios(pool: &PgPool, include_archived: bool) -> ApiResult<Vec<CryptoPortfolioRead>> {
    Ok(repo::crypto::list_portfolios(pool, include_archived).await?)
}

pub async fn create_portfolio(pool: &PgPool, payload: CryptoPortfolioCreate) -> ApiResult<CryptoPortfolioRead> {
    let existing = repo::crypto::count_portfolios(pool).await?;
    let color = portfolio_color(usize::try_from(existing).unwrap_or_default());
    Ok(repo::crypto::insert_portfolio(pool, &payload.name, color).await?)
}

pub async fn update_portfolio(
    pool: &PgPool,
    portfolio_id: i64,
    payload: CryptoPortfolioUpdate,
) -> ApiResult<CryptoPortfolioRead> {
    if repo::crypto::get_portfolio(pool, portfolio_id).await?.is_none() {
        return Err(ApiError::not_found(PORTFOLIO_NOT_FOUND));
    }
    Ok(repo::crypto::update_portfolio(pool, portfolio_id, &payload).await?)
}

/// A non-empty portfolio can't be deleted (the FK is RESTRICT): its coins
/// have to be moved or deleted first.
pub async fn delete_portfolio(pool: &PgPool, portfolio_id: i64) -> ApiResult<()> {
    if repo::crypto::get_portfolio(pool, portfolio_id).await?.is_none() {
        return Err(ApiError::not_found(PORTFOLIO_NOT_FOUND));
    }
    if repo::crypto::portfolio_has_holding(pool, portfolio_id).await? {
        return Err(ApiError::bad_request("Move or delete its coins before deleting a portfolio"));
    }
    repo::crypto::delete_portfolio(pool, portfolio_id).await?;
    Ok(())
}

/// `get_or_create_default_portfolio`: the earliest-created portfolio,
/// auto-created the first time it's needed (inside the caller's
/// transaction, like Python's flush).
async fn default_portfolio_id(conn: &mut PgConnection) -> ApiResult<i32> {
    if let Some(existing) = repo::crypto::first_portfolio(&mut *conn).await? {
        return Ok(existing.id);
    }
    Ok(repo::crypto::insert_portfolio(&mut *conn, DEFAULT_PORTFOLIO_NAME, PORTFOLIO_PALETTE[0]).await?.id)
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

/// `refresh_prices`: a sync always covers every holding regardless of
/// `portfolio_id`, which only narrows the returned `holdings`.
pub async fn refresh_prices(state: &AppState, force: bool, portfolio_id: Option<i64>) -> ApiResult<CryptoSyncResult> {
    let pool = &state.pool;
    let last_synced_at = repo::crypto::get_or_create_sync_state(pool).await?;
    let now = utc_now();
    let mut conn = pool.acquire().await?;

    if let Some(last) = last_synced_at
        && !force
        && now.signed_duration_since(last) < chrono::Duration::hours(AUTO_REFRESH_INTERVAL_HOURS)
    {
        let holdings = list_holdings(&mut conn, portfolio_id).await?;
        return Ok(CryptoSyncResult {
            synced: false,
            last_synced_at: Some(PyDateTime(last)),
            error_key: None,
            holdings: sort_by_invested(holdings.iter().map(Loaded::read).collect()),
        });
    }

    let mut holdings = list_holdings(&mut conn, None).await?;
    drop(conn);
    let mut unreachable = false;
    let mut tx = pool.begin().await?;
    if !holdings.is_empty() {
        let settings = get_or_create_app_settings(pool).await?;
        let ids: Vec<&str> = holdings.iter().map(|h| h.holding.coingecko_id.as_str()).collect();
        match fetch_market_data(state, &ids, &settings.currency.to_lowercase()).await? {
            // CoinGecko down/rate-limited — don't touch last_synced_at (the
            // next open of the tab retries) and don't touch any existing
            // price/valuation: the tab still shows the last known values.
            Err(HttpError) => unreachable = true,
            Ok(market_data) => {
                let today = today();
                for loaded in &mut holdings {
                    // A coin missing from the response keeps its last known
                    // values instead of being zeroed out.
                    let Some(point) = market_data.get(&loaded.holding.coingecko_id) else {
                        continue;
                    };
                    loaded.holding.apply_market(point);
                    let quantity = loaded.quantity();
                    if quantity > Decimal::zero() {
                        let value = quantity.mul(&point.price);
                        repo::crypto::upsert_valuation(&mut *tx, loaded.holding.asset_id, &value, today).await?;
                    }
                }
                // Flushed at commit under Python (autoflush is off).
                for loaded in &holdings {
                    if market_data.contains_key(&loaded.holding.coingecko_id) {
                        repo::crypto::update_holding_prices(&mut *tx, &loaded.holding).await?;
                    }
                }
            }
        }
    }
    let last_synced_at = if unreachable {
        last_synced_at
    } else {
        repo::crypto::set_last_synced(&mut *tx, now).await?;
        Some(now)
    };
    tx.commit().await?;

    // The holdings were updated in memory above — they already reflect
    // what was just committed.
    let visible = holdings.iter().filter(|h| portfolio_id.is_none_or(|p| i64::from(h.holding.portfolio_id) == p));
    Ok(CryptoSyncResult {
        synced: !unreachable,
        last_synced_at: last_synced_at.map(PyDateTime),
        error_key: unreachable.then(|| ERROR_KEY_UNREACHABLE.to_string()),
        holdings: sort_by_invested(visible.map(Loaded::read).collect()),
    })
}

// ---------------------------------------------------------------------------
// Holdings and transactions
// ---------------------------------------------------------------------------

pub async fn create_holding(state: &AppState, payload: CryptoHoldingCreate) -> ApiResult<CryptoHoldingRead> {
    let pool = &state.pool;
    let settings = get_or_create_app_settings(pool).await?;
    let mut tx = pool.begin().await?;

    let portfolio_id = match payload.portfolio_id {
        Some(id) => {
            repo::crypto::get_portfolio(&mut *tx, id)
                .await?
                .ok_or_else(|| ApiError::bad_request(PORTFOLIO_NOT_FOUND))?
                .id
        }
        None => default_portfolio_id(&mut tx).await?,
    };

    // User-chosen risk level (default HIGH, not the Asset model's MEDIUM),
    // editable later via PATCH /assets/{asset_id} like any other Asset.
    let asset_id = repo::crypto::insert_asset(&mut *tx, &payload.name, &settings.currency, payload.risk_level).await?;
    let mut holding = CryptoHolding {
        asset_id,
        portfolio_id,
        coingecko_id: payload.coingecko_id.clone(),
        symbol: payload.symbol.to_uppercase(),
        name: payload.name.clone(),
        thumb_url: payload.thumb_url.clone(),
        network: payload.network.clone(),
        last_price: None,
        price_change_1h: None,
        price_change_24h: None,
        price_change_7d: None,
        price_change_30d: None,
        price_change_1y: None,
    };
    repo::crypto::insert_holding(&mut *tx, &holding).await?;
    let date = payload.date.unwrap_or_else(today);
    let transaction_id = repo::crypto::insert_transaction(
        &mut *tx,
        asset_id,
        CryptoTransactionType::Buy,
        &payload.quantity,
        &payload.price_per_unit,
        date,
        payload.note.as_deref(),
    )
    .await?;
    let transactions = vec![CryptoTransactionRead {
        id: transaction_id,
        asset_id,
        type_: CryptoTransactionType::Buy,
        quantity: payload.quantity.clone(),
        price_per_unit: payload.price_per_unit.clone(),
        date,
        note: payload.note.clone(),
    }];

    // Seed today's live price immediately — justified by the explicit user
    // action. A missing key still fails the whole create (400, rolled back);
    // an outage doesn't: the next daily/manual sync will price it.
    let currency = settings.currency.to_lowercase();
    if let Ok(market_data) = fetch_market_data(state, &[payload.coingecko_id.as_str()], &currency).await? {
        if let Some(point) = market_data.get(&payload.coingecko_id) {
            holding.apply_market(point);
            let value = payload.quantity.mul(&point.price);
            repo::crypto::upsert_valuation(&mut *tx, asset_id, &value, today()).await?;
            repo::crypto::update_holding_prices(&mut *tx, &holding).await?;
        }
        // This counts as a real sync — bump the shared timestamp so the next
        // GET /crypto/holdings doesn't immediately re-fetch everything.
        repo::crypto::set_last_synced(&mut *tx, utc_now()).await?;
    }
    tx.commit().await?;

    Ok(Loaded { holding, risk_level: payload.risk_level, transactions }.read())
}

/// Holding-only metadata (currently just `network`); quantity/price/date go
/// through the transaction routes, risk level through `PATCH /assets/{id}`.
pub async fn update_holding(
    pool: &PgPool,
    asset_id: i64,
    payload: CryptoHoldingUpdate,
) -> ApiResult<CryptoHoldingRead> {
    let mut conn = pool.acquire().await?;
    let mut loaded = get_holding_or_404(&mut conn, asset_id).await?;
    if let Some(network) = payload.network.into_option() {
        repo::crypto::update_holding_network(&mut *conn, loaded.holding.asset_id, network.as_deref()).await?;
        loaded.holding.network = network;
    }
    Ok(loaded.read())
}

/// Re-values the holding at the cached price after its log changed (never
/// calls CoinGecko).
async fn revalue(conn: &mut PgConnection, loaded: &Loaded) -> ApiResult<()> {
    if let Some(price) = &loaded.holding.last_price {
        let value = loaded.quantity().mul(price);
        repo::crypto::upsert_valuation(conn, loaded.holding.asset_id, &value, today()).await?;
    }
    Ok(())
}

/// A buy or sell against an existing holding.
pub async fn add_transaction(
    pool: &PgPool,
    asset_id: i64,
    payload: CryptoTransactionCreate,
) -> ApiResult<CryptoHoldingRead> {
    let mut tx = pool.begin().await?;
    let mut loaded = get_holding_or_404(&mut tx, asset_id).await?;
    if payload.type_ == CryptoTransactionType::Sell && payload.quantity > loaded.quantity() {
        return Err(ApiError::bad_request(OVERSELL));
    }
    let id = repo::crypto::insert_transaction(
        &mut *tx,
        loaded.holding.asset_id,
        payload.type_,
        &payload.quantity,
        &payload.price_per_unit,
        payload.date,
        payload.note.as_deref(),
    )
    .await?;
    loaded.transactions.push(CryptoTransactionRead {
        id,
        asset_id: loaded.holding.asset_id,
        type_: payload.type_,
        quantity: payload.quantity,
        price_per_unit: payload.price_per_unit,
        date: payload.date,
        note: payload.note,
    });
    revalue(&mut tx, &loaded).await?;
    tx.commit().await?;
    Ok(loaded.read())
}

/// The "edit a transaction" flow: keeps the id, re-checks the sell guard
/// against every *other* entry, then re-values at the cached price.
pub async fn update_transaction(
    pool: &PgPool,
    transaction_id: i64,
    payload: CryptoTransactionUpdate,
) -> ApiResult<CryptoHoldingRead> {
    let mut tx = pool.begin().await?;
    let current = repo::crypto::get_transaction(&mut *tx, transaction_id)
        .await?
        .ok_or_else(|| ApiError::not_found(TRANSACTION_NOT_FOUND))?;
    let mut loaded = get_holding_or_404(&mut tx, i64::from(current.asset_id)).await?;

    // `updates.get(field, current)` — an explicit null stays null.
    let effective_type = match &payload.type_ {
        Patch::Unset => Some(current.type_),
        Patch::Null => None,
        Patch::Value(t) => Some(*t),
    };
    if effective_type == Some(CryptoTransactionType::Sell) {
        let effective_quantity = match &payload.quantity {
            Patch::Unset => current.quantity.clone(),
            Patch::Value(q) => q.clone(),
            // `None > Decimal` raises TypeError in Python → 500.
            Patch::Null => return Err(ApiError::internal(anyhow::anyhow!("null quantity on a sell"))),
        };
        let others: Vec<&CryptoTransactionRead> = loaded.transactions.iter().filter(|t| t.id != current.id).collect();
        if effective_quantity > compute_position(others).quantity {
            return Err(ApiError::bad_request(OVERSELL));
        }
    }

    // A null for a NOT NULL column fails the flush (IntegrityError → 500).
    fn required<T: Clone>(patch: &Patch<T>, current: &T) -> ApiResult<T> {
        match patch {
            Patch::Unset => Ok(current.clone()),
            Patch::Value(v) => Ok(v.clone()),
            Patch::Null => Err(ApiError::internal(anyhow::anyhow!("NOT NULL column set to null"))),
        }
    }
    let updated = CryptoTransactionRead {
        id: current.id,
        asset_id: current.asset_id,
        type_: required(&payload.type_, &current.type_)?,
        quantity: required(&payload.quantity, &current.quantity)?,
        price_per_unit: required(&payload.price_per_unit, &current.price_per_unit)?,
        date: required(&payload.date, &current.date)?,
        note: match payload.note.into_option() {
            None => current.note.clone(),
            Some(note) => note,
        },
    };
    repo::crypto::update_transaction(&mut *tx, &updated).await?;
    match loaded.transactions.iter().position(|t| t.id == updated.id) {
        Some(index) => loaded.transactions[index] = updated,
        None => loaded.transactions.push(updated),
    }
    revalue(&mut tx, &loaded).await?;
    tx.commit().await?;
    Ok(loaded.read())
}

/// `GET /holdings/{asset_id}/transactions` — 404s if the holding doesn't exist.
pub async fn list_transactions(pool: &PgPool, asset_id: i64) -> ApiResult<Vec<CryptoTransactionRead>> {
    if repo::crypto::get_holding(pool, asset_id).await?.is_none() {
        return Err(ApiError::not_found(HOLDING_NOT_FOUND));
    }
    Ok(repo::crypto::list_transactions(pool, asset_id).await?)
}

pub async fn delete_transaction(pool: &PgPool, transaction_id: i64) -> ApiResult<()> {
    let mut tx = pool.begin().await?;
    let current = repo::crypto::get_transaction(&mut *tx, transaction_id)
        .await?
        .ok_or_else(|| ApiError::not_found(TRANSACTION_NOT_FOUND))?;
    repo::crypto::delete_transaction(&mut *tx, current.id).await?;
    let loaded = get_holding_or_404(&mut tx, i64::from(current.asset_id)).await?;
    revalue(&mut tx, &loaded).await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// History and 90-day performance
// ---------------------------------------------------------------------------

/// `get_crypto_history`: total crypto value over time, scoped to
/// crypto-class assets (and to one portfolio when asked).
pub async fn history(pool: &PgPool, range_key: &str, portfolio_id: Option<i64>) -> ApiResult<CryptoHistoryResponse> {
    let today = today();
    let asset_ids = repo::crypto::crypto_asset_ids(pool, portfolio_id).await?;
    if asset_ids.is_empty() {
        return Ok(CryptoHistoryResponse::empty(range_key));
    }
    let valuations = repo::crypto::valuations(pool, &asset_ids).await?;
    Ok(crypto_history(range_key, &valuations, today))
}

/// Python's `asyncio.Semaphore(5)` around the per-coin chart calls.
const CHART_CONCURRENCY: usize = 5;

/// `get_90d_performance`: one `market_chart` call per currently-held coin
/// (CoinGecko has no 90-day window on `/coins/markets`), on demand only.
/// One coin's failed request gives `null` for that coin, not an error.
pub async fn performance_90d(state: &AppState, portfolio_id: Option<i64>) -> ApiResult<CryptoPerformanceResponse> {
    let pool = &state.pool;
    let mut conn = pool.acquire().await?;
    let holdings = list_holdings(&mut conn, portfolio_id).await?;
    drop(conn);
    let held: Vec<&Loaded> = holdings.iter().filter(|h| h.quantity() > Decimal::zero()).collect();
    if held.is_empty() {
        return Ok(CryptoPerformanceResponse { items: Vec::new() });
    }
    let settings = get_or_create_app_settings(pool).await?;
    let api_key = require_api_key(state)?;
    let currency = settings.currency.to_lowercase();
    let client = coingecko_client(state);

    type Job = tokio::task::JoinHandle<Result<Option<f64>, UpstreamShapeError>>;
    let spawn = |coingecko_id: String| -> Job {
        let (client, base, key, currency) =
            (client.clone(), state.settings.coingecko_base_url.clone(), api_key.clone(), currency.clone());
        tokio::spawn(async move {
            let path = format!("/coins/{coingecko_id}/market_chart");
            let params = [("vs_currency", currency.as_str()), ("days", "90")];
            match coingecko_get(&client, &base, &key, &path, &params).await {
                Ok(body) => parse_90d_change(&body),
                Err(HttpError) => Ok(None),
            }
        })
    };

    // A sliding window of at most CHART_CONCURRENCY requests in flight,
    // collected in holding order (like `asyncio.gather`).
    let mut pending: VecDeque<Job> = VecDeque::new();
    let mut changes = Vec::with_capacity(held.len());
    for loaded in &held {
        if pending.len() == CHART_CONCURRENCY
            && let Some(job) = pending.pop_front()
        {
            changes.push(job.await.map_err(ApiError::internal)??);
        }
        pending.push_back(spawn(loaded.holding.coingecko_id.clone()));
    }
    while let Some(job) = pending.pop_front() {
        changes.push(job.await.map_err(ApiError::internal)??);
    }

    let items = held
        .iter()
        .zip(changes)
        .map(|(loaded, change)| CryptoPerformancePoint {
            asset_id: loaded.holding.asset_id,
            price_change_percent: change.map(PyFloat),
        })
        .collect();
    Ok(CryptoPerformanceResponse { items })
}
