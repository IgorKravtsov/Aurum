//! Local stand-ins for the two external APIs the server calls — the NBP
//! exchange-rate API (`services/fx.rs`) and CoinGecko (`services/crypto.rs`).
//! Port of the former Python backend's `tests/fake_providers.py`.
//!
//! The app under test is pointed at this fake through
//! `Settings::nbp_base_url` / `Settings::coingecko_base_url`, and tests
//! configure what it answers. Response shapes mirror the real APIs closely
//! enough for any client that parses the real ones:
//!
//! - NBP: GET {base}/{table}/{code}/{start}/{end}/?format=json ->
//!   {"table", "currency", "code", "rates": [{"no", "effectiveDate", "mid"}]}
//!   (also {base}/{table}/{code}/{date}/), 404 "Brak danych" when nothing
//!   matches, 400 for ranges over 93 days — same as api.nbp.pl.
//! - CoinGecko: GET {base}/coins/markets, {base}/coins/{id}/market_chart,
//!   {base}/search — same JSON as api.coingecko.com/api/v3.
//!
//! Unlike the pytest harness (one fake for the whole session, reset before
//! every test) every test gets its own fake server on a random port, so tests
//! run in parallel without seeing each other's configuration.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use aurum_domain::Decimal;
use axum::Router;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde_json::{Value, json};

use super::AbortOnDrop;

const NBP_PREFIX: &str = "/nbp/api/exchangerates/rates";
const COINGECKO_PREFIX: &str = "/coingecko/api/v3";
const NBP_MAX_RANGE_DAYS: i64 = 93;

/// Deterministic "healthy NBP" used when a test configures nothing: fixed
/// PLN mid rates published on every business day, like the real table A.
/// A test that mixes currencies without configuring rates therefore gets
/// stable numbers instead of whatever the live API said that day.
pub const DEFAULT_NBP_RATES: [(&str, &str); 6] =
    [("USD", "4"), ("EUR", "4.25"), ("GBP", "5"), ("CHF", "4.5"), ("UAH", "0.1"), ("RUB", "0.05")];

/// Same fields as the crypto service's market point — what one coin's
/// /coins/markets row carries.
#[derive(Debug, Clone, Default)]
pub struct MarketPoint {
    pub price: Decimal,
    pub change_1h: Option<Decimal>,
    pub change_24h: Option<Decimal>,
    pub change_7d: Option<Decimal>,
    pub change_30d: Option<Decimal>,
    pub change_1y: Option<Decimal>,
}

/// Python's `_json_number`: an integral Decimal goes out as a JSON integer,
/// anything else as a float.
fn json_number(value: Option<&Decimal>) -> Value {
    let Some(value) = value else { return Value::Null };
    let text = value.to_string();
    if let Ok(int) = text.parse::<i64>() {
        return json!(int);
    }
    let float: f64 = text.parse().expect("decimal parses as f64");
    if float.fract() == 0.0 && float.abs() < 9.0e15 { json!(float as i64) } else { json!(float) }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A test that panicked while holding the lock already failed; keep the
    // state readable for the rest of its teardown.
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

enum Body {
    Text(&'static str),
    Json(Value),
}

// --------------------------------------------------------------------------
// NBP
// --------------------------------------------------------------------------

struct NbpState {
    rates: BTreeMap<String, Decimal>,
    status: Option<u16>,
    requests: Vec<String>,
}

#[derive(Clone)]
pub struct FakeNbp(Arc<Mutex<NbpState>>);

impl FakeNbp {
    fn new() -> Self {
        let rates = DEFAULT_NBP_RATES.iter().map(|(code, rate)| (code.to_string(), rate.parse().unwrap())).collect();
        Self(Arc::new(Mutex::new(NbpState { rates, status: None, requests: Vec::new() })))
    }

    /// Replaces the whole rate table: listed codes are published with a
    /// constant PLN mid rate on every business day, any other code 404s
    /// (like a currency NBP doesn't quote).
    pub fn serve_rates(&self, rates: &[(&str, &str)]) {
        lock(&self.0).rates =
            rates.iter().map(|(code, rate)| (code.to_uppercase(), rate.parse().expect("valid rate"))).collect();
    }

    /// Simulates an outage: every request answers `status`.
    pub fn fail(&self, status: u16) {
        lock(&self.0).status = Some(status);
    }

    /// Every path requested so far (relative to the NBP base URL).
    pub fn requests(&self) -> Vec<String> {
        lock(&self.0).requests.clone()
    }

    fn handle(&self, path: &str) -> (u16, Body) {
        let mut state = lock(&self.0);
        state.requests.push(path.to_string());
        if let Some(status) = state.status {
            return (status, Body::Text("Service Unavailable"));
        }
        let Some((table, code, start, end)) = parse_nbp_path(path) else {
            return (400, Body::Text("400 BadRequest - Błędny zakres dat / Invalid date range"));
        };
        if end < start || (end - start).num_days() > NBP_MAX_RANGE_DAYS {
            return (
                400,
                Body::Text("400 BadRequest - Przekroczony limit 93 dni / Limit of 93 days has been exceeded"),
            );
        }
        // Everything configured is quoted in table A; table B has nothing.
        let rate = state.rates.get(&code);
        let (Some(rate), "a") = (rate, table.as_str()) else {
            return (404, Body::Text("404 NotFound - Not Found - Brak danych"));
        };
        let mut rows = Vec::new();
        let mut day = start;
        while day <= end {
            if !matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
                rows.push(json!({
                    "no": format!("{:03}/A/NBP/{}", rows.len() + 1, day.year()),
                    "effectiveDate": day.format("%Y-%m-%d").to_string(),
                    "mid": json_number(Some(rate)),
                }));
            }
            day += Duration::days(1);
        }
        if rows.is_empty() {
            return (404, Body::Text("404 NotFound - Not Found - Brak danych"));
        }
        (200, Body::Json(json!({"table": "A", "currency": code.to_lowercase(), "code": code, "rates": rows})))
    }
}

/// `/{table}/{code}/{start}(/{end})?/?` (case-insensitive), like the
/// Python fake's regex.
fn parse_nbp_path(path: &str) -> Option<(String, String, NaiveDate, NaiveDate)> {
    let rest = path.strip_prefix('/')?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let parts: Vec<&str> = rest.split('/').collect();
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let table = parts[0].to_lowercase();
    if !matches!(table.as_str(), "a" | "b" | "c") {
        return None;
    }
    let code = parts[1];
    if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let date = |text: &str| {
        let shape_ok = text.len() == 10
            && text.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() });
        if shape_ok { NaiveDate::parse_from_str(text, "%Y-%m-%d").ok() } else { None }
    };
    let start = date(parts[2])?;
    let end = match parts.get(3) {
        Some(text) => date(text)?,
        None => start,
    };
    Some((table, code.to_uppercase(), start, end))
}

// --------------------------------------------------------------------------
// CoinGecko
// --------------------------------------------------------------------------

#[derive(Default)]
struct CoinGeckoState {
    markets: HashMap<String, MarketPoint>,
    changes_90d: HashMap<String, f64>,
    status: Option<u16>,
    forbidden_reason: Option<String>,
    violations: Vec<String>,
    market_calls: Vec<Vec<String>>,
    chart_calls: Vec<String>,
}

#[derive(Clone)]
pub struct FakeCoinGecko(Arc<Mutex<CoinGeckoState>>);

impl FakeCoinGecko {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(CoinGeckoState::default())))
    }

    /// What /coins/markets returns from now on: one row per requested id
    /// that's in `prices`; ids not listed are simply absent from the
    /// response (CoinGecko's own behavior for an unknown id).
    pub fn serve_markets(&self, prices: &[(&str, MarketPoint)]) {
        let mut state = lock(&self.0);
        state.markets = prices.iter().map(|(id, point)| (id.to_string(), point.clone())).collect();
        state.status = None;
        state.forbidden_reason = None;
    }

    /// /coins/{id}/market_chart?days=90 returns a two-point price series
    /// whose first->last change is exactly `changes[id]` percent; any other
    /// id 404s.
    pub fn serve_90d_change(&self, changes: &[(&str, f64)]) {
        let mut state = lock(&self.0);
        state.changes_90d = changes.iter().map(|(id, change)| (id.to_string(), *change)).collect();
        state.status = None;
        state.forbidden_reason = None;
    }

    /// Simulates an outage/rate limit: every request answers `status`.
    pub fn fail(&self, status: u16) {
        let mut state = lock(&self.0);
        state.status = Some(status);
        state.forbidden_reason = None;
    }

    /// Any CoinGecko request from now on is a test failure: it's recorded
    /// in `violations` (checked when the test's `TestApp` is dropped) and
    /// answered with HTTP 500.
    pub fn forbid(&self, reason: &str) {
        lock(&self.0).forbidden_reason = Some(reason.to_string());
    }

    /// Starts a fresh log of /coins/markets calls (each entry is that
    /// call's list of ids); read it back with [`Self::market_calls`].
    pub fn record_market_calls(&self) {
        lock(&self.0).market_calls.clear();
    }

    pub fn market_calls(&self) -> Vec<Vec<String>> {
        lock(&self.0).market_calls.clone()
    }

    /// Same as record_market_calls, for /coins/{id}/market_chart (one
    /// coin id per call).
    pub fn record_chart_calls(&self) {
        lock(&self.0).chart_calls.clear();
    }

    pub fn chart_calls(&self) -> Vec<String> {
        lock(&self.0).chart_calls.clone()
    }

    pub fn violations(&self) -> Vec<String> {
        lock(&self.0).violations.clone()
    }

    fn handle(&self, path: &str, query: &str) -> (u16, Body) {
        let mut state = lock(&self.0);
        if let Some(reason) = &state.forbidden_reason {
            let violation = format!("{reason} (got GET {path})");
            state.violations.push(violation);
            return (500, Body::Json(json!({"error": "forbidden by test"})));
        }
        let outage = |status: u16| (status, Body::Json(json!({"error": "simulated outage"})));
        if path == "/coins/markets" {
            let pairs: Vec<(String, String)> = serde_urlencoded::from_str(query).unwrap_or_default();
            let ids: Vec<String> = pairs
                .iter()
                .filter(|(key, _)| key == "ids")
                .flat_map(|(_, value)| value.split(','))
                .filter(|id| !id.is_empty())
                .map(String::from)
                .collect();
            state.market_calls.push(ids.clone());
            if let Some(status) = state.status {
                return outage(status);
            }
            let rows: Vec<Value> = ids
                .iter()
                .filter_map(|id| {
                    let point = state.markets.get(id)?;
                    Some(json!({
                        "id": id,
                        "symbol": id.chars().take(3).collect::<String>(),
                        "name": title_case(id),
                        "current_price": json_number(Some(&point.price)),
                        "price_change_percentage_1h_in_currency": json_number(point.change_1h.as_ref()),
                        "price_change_percentage_24h_in_currency": json_number(point.change_24h.as_ref()),
                        "price_change_percentage_7d_in_currency": json_number(point.change_7d.as_ref()),
                        "price_change_percentage_30d_in_currency": json_number(point.change_30d.as_ref()),
                        "price_change_percentage_1y_in_currency": json_number(point.change_1y.as_ref()),
                    }))
                })
                .collect();
            return (200, Body::Json(Value::Array(rows)));
        }
        if let Some(id) = path.strip_prefix("/coins/").and_then(|rest| rest.strip_suffix("/market_chart"))
            && !id.is_empty()
            && !id.contains('/')
        {
            state.chart_calls.push(id.to_string());
            if let Some(status) = state.status {
                return outage(status);
            }
            let Some(change) = state.changes_90d.get(id) else {
                return (404, Body::Json(json!({"error": "coin not found"})));
            };
            let first = 100.0_f64;
            let last = first * (1.0 + change / 100.0);
            let day_ms: i64 = 86_400_000;
            return (
                200,
                Body::Json(json!({
                    "prices": [[0, first], [90 * day_ms, last]],
                    "market_caps": [],
                    "total_volumes": [],
                })),
            );
        }
        if path == "/search" {
            if let Some(status) = state.status {
                return outage(status);
            }
            return (200, Body::Json(json!({"coins": [], "exchanges": [], "categories": [], "nfts": []})));
        }
        (404, Body::Json(json!({"error": "Not Found"})))
    }
}

/// Python's `str.title()` for coin ids ("usd-coin" -> "Usd-Coin").
fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_is_letter = false;
    for c in text.chars() {
        if previous_is_letter {
            out.extend(c.to_lowercase());
        } else {
            out.extend(c.to_uppercase());
        }
        previous_is_letter = c.is_alphabetic();
    }
    out
}

// --------------------------------------------------------------------------
// The HTTP server owning both fakes
// --------------------------------------------------------------------------

/// One fake HTTP server (NBP + CoinGecko) on a random local port.
pub struct FakeProviders {
    pub nbp: FakeNbp,
    pub coingecko: FakeCoinGecko,
    root_url: String,
    _server: AbortOnDrop,
}

impl FakeProviders {
    pub async fn start() -> Self {
        let nbp = FakeNbp::new();
        let coingecko = FakeCoinGecko::new();
        let (nbp_handle, coingecko_handle) = (nbp.clone(), coingecko.clone());
        let app = Router::new().fallback(move |uri: Uri| {
            let (nbp, coingecko) = (nbp_handle.clone(), coingecko_handle.clone());
            async move {
                let path = uri.path();
                let (status, body) = if let Some(rest) = path.strip_prefix(NBP_PREFIX) {
                    nbp.handle(rest)
                } else if let Some(rest) = path.strip_prefix(COINGECKO_PREFIX) {
                    coingecko.handle(rest, uri.query().unwrap_or(""))
                } else {
                    (404, Body::Text("unknown fake provider"))
                };
                respond(status, body)
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind fake providers");
        let addr = listener.local_addr().expect("fake providers address");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { nbp, coingecko, root_url: format!("http://{addr}"), _server: AbortOnDrop(server) }
    }

    pub fn nbp_base_url(&self) -> String {
        format!("{}{NBP_PREFIX}", self.root_url)
    }

    pub fn coingecko_base_url(&self) -> String {
        format!("{}{COINGECKO_PREFIX}", self.root_url)
    }
}

fn respond(status: u16, body: Body) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    match body {
        Body::Text(text) => (status, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response(),
        Body::Json(value) => {
            (status, [(header::CONTENT_TYPE, "application/json; charset=utf-8")], value.to_string()).into_response()
        }
    }
}
