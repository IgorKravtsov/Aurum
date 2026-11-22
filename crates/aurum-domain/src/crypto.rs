//! API contract types for `/api/crypto` — port of the former Python backend's `app/schemas/crypto.py`,
//! plus the pure parts of the former Python backend's `app/services/crypto_service.py`: the
//! weighted-average-cost position, the derived holding figures, the
//! "biggest investment first" order, the history series and the parsing of
//! CoinGecko's JSON (which the server fetches; no IO happens here).
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.
//!
//! Decimal arithmetic mirrors the Python order of operations exactly (same
//! starting values, same operands), so every derived figure prints with the
//! digits and exponent the Python service produced.

use std::collections::HashMap;

use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::decimal::Decimal;
use crate::enums::{CryptoTransactionType, RiskLevel};
use crate::pyfmt::{PyDateTime, PyFloat, py_float_repr};
use crate::validation::{FromInput, Patch, Val, ValidationErrors};

/// Same 8-hue, colorblind-safe categorical set the seed assigns default
/// categories from — reused here (cycling by creation order) so portfolio tab
/// dots are visually distinct out of the box, with no manual color picker.
pub const PORTFOLIO_PALETTE: [&str; 8] =
    ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#4a3aa7", "#e34948"];

/// Name of the portfolio auto-created the first time a holding is added
/// without a `portfolio_id` (`get_or_create_default_portfolio`).
pub const DEFAULT_PORTFOLIO_NAME: &str = "Main Portfolio";

/// CoinGecko's own Demo-tier data only refreshes every 60s server-side —
/// polling more often than once a day buys nothing and only spends
/// rate-limit budget (`AUTO_REFRESH_INTERVAL`).
pub const AUTO_REFRESH_INTERVAL_HOURS: i64 = 24;

/// `CRYPTO_RANGE_DAYS` — deliberately no "24h" option (the price history is
/// only as dense as the sync cadence).
pub const CRYPTO_RANGE_DAYS: [(&str, i64); 3] = [("7d", 7), ("30d", 30), ("90d", 90)];

/// `_HISTORY_RANGE_PATTERN`: `sorted(set(CRYPTO_RANGE_DAYS) | {"all"})`
/// joined into an anchored alternation (string sort, hence `30d` first).
pub const HISTORY_RANGE_PATTERN: &str = "^(30d|7d|90d|all)$";

/// `/coins/markets` asks for these windows in the same batched call.
pub const MARKET_CHANGE_WINDOWS: &str = "1h,24h,7d,30d,1y";

/// `search_coins` keeps the first 20 coins of CoinGecko's answer.
pub const SEARCH_RESULT_LIMIT: usize = 20;

/// Days of a history range key, `None` for `"all"`.
pub fn range_days(range_key: &str) -> Option<i64> {
    CRYPTO_RANGE_DAYS.iter().find(|(key, _)| *key == range_key).map(|(_, days)| *days)
}

/// `PORTFOLIO_PALETTE[len(existing) % len(PORTFOLIO_PALETTE)]`.
pub fn portfolio_color(existing_portfolios: usize) -> &'static str {
    PORTFOLIO_PALETTE[existing_portfolios % PORTFOLIO_PALETTE.len()]
}

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

fn quantity_like(v: Val<'_>) -> crate::validation::Res<'_, Decimal> {
    v.decimal()?.digits(Some(38), Some(18))?.gt("0")
}

/// `CryptoTransactionCreate` — body of `POST /api/crypto/holdings/{asset_id}/transactions`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoTransactionCreate {
    #[serde(rename = "type")]
    pub type_: CryptoTransactionType,
    pub quantity: Decimal,
    /// Price paid (buy) / received (sell) per unit, in the app's display
    /// currency — what the user actually paid, not a CoinGecko price.
    pub price_per_unit: Decimal,
    pub date: NaiveDate,
    pub note: Option<String>,
}

impl FromInput for CryptoTransactionCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let type_ = o.required("type", |v| v.enum_());
        let quantity = o.required("quantity", quantity_like);
        let price_per_unit = o.required("price_per_unit", quantity_like);
        let date = o.required("date", |v| v.date());
        let note = o.optional("note", |v| v.str()?.max_len(500));
        o.finish()?;
        Ok(Self {
            type_: type_.value(),
            quantity: quantity.value(),
            price_per_unit: price_per_unit.value(),
            date: date.value(),
            note: note.value(),
        })
    }
}

/// `CryptoTransactionUpdate` — partial update (`exclude_unset`). An explicit
/// `null` for a NOT NULL column fails at flush (500), as under Python.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoTransactionUpdate {
    #[serde(rename = "type", skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<CryptoTransactionType>))]
    pub type_: Patch<CryptoTransactionType>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Decimal>))]
    pub quantity: Patch<Decimal>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Decimal>))]
    pub price_per_unit: Patch<Decimal>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<NaiveDate>))]
    pub date: Patch<NaiveDate>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub note: Patch<String>,
}

impl FromInput for CryptoTransactionUpdate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let type_ = o.patch("type", |v| v.enum_());
        let quantity = o.patch("quantity", quantity_like);
        let price_per_unit = o.patch("price_per_unit", quantity_like);
        let date = o.patch("date", |v| v.date());
        let note = o.patch("note", |v| v.str()?.max_len(500));
        o.finish()?;
        Ok(Self {
            type_: type_.value(),
            quantity: quantity.value(),
            price_per_unit: price_per_unit.value(),
            date: date.value(),
            note: note.value(),
        })
    }
}

/// `CryptoPortfolioCreate` — body of `POST /api/crypto/portfolios`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoPortfolioCreate {
    pub name: String,
}

impl FromInput for CryptoPortfolioCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let name = o.required("name", |v| v.str()?.min_len(1)?.max_len(100));
        o.finish()?;
        Ok(Self { name: name.value() })
    }
}

/// `CryptoPortfolioUpdate` — partial update (`exclude_unset`).
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoPortfolioUpdate {
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub name: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<bool>))]
    pub is_archived: Patch<bool>,
}

impl FromInput for CryptoPortfolioUpdate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let name = o.patch("name", |v| v.str()?.min_len(1)?.max_len(100));
        let is_archived = o.patch("is_archived", |v| v.bool());
        o.finish()?;
        Ok(Self { name: name.value(), is_archived: is_archived.value() })
    }
}

/// `CryptoHoldingCreate` — body of `POST /api/crypto/holdings`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoHoldingCreate {
    /// Which portfolio to file this coin under — omit to use the default
    /// portfolio (the earliest-created one, auto-created if none exists yet).
    pub portfolio_id: Option<i64>,
    pub coingecko_id: String,
    pub symbol: String,
    pub name: String,
    pub thumb_url: Option<String>,
    /// A holding always starts with a buy — there's nothing to "start
    /// tracking" with a sell.
    pub quantity: Decimal,
    pub price_per_unit: Decimal,
    /// `Field(default_factory=date.today)`: `None` when omitted — the server
    /// resolves it to its local "today" (this crate never reads the clock).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<NaiveDate>,
    pub note: Option<String>,
    /// Defaults to HIGH — crypto is this app's textbook HIGH example — but
    /// user-overridable at add time (a large-cap vs. a memecoin).
    pub risk_level: RiskLevel,
    /// Free-form chain name (e.g. "Ethereum", "Tron") — not validated
    /// against a fixed list.
    pub network: Option<String>,
}

impl FromInput for CryptoHoldingCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let portfolio_id = o.optional("portfolio_id", |v| v.int());
        let coingecko_id = o.required("coingecko_id", |v| v.str()?.min_len(1)?.max_len(100));
        let symbol = o.required("symbol", |v| v.str()?.min_len(1)?.max_len(20));
        let name = o.required("name", |v| v.str()?.min_len(1)?.max_len(150));
        let thumb_url = o.optional("thumb_url", |v| v.str()?.max_len(500));
        let quantity = o.required("quantity", quantity_like);
        let price_per_unit = o.required("price_per_unit", quantity_like);
        let date = o.with_default("date", || None, |v| Ok(v.date()?.map(Some)));
        let note = o.optional("note", |v| v.str()?.max_len(500));
        let risk_level = o.with_default("risk_level", || RiskLevel::High, |v| v.enum_());
        let network = o.optional("network", |v| v.str()?.max_len(50));
        o.finish()?;
        Ok(Self {
            portfolio_id: portfolio_id.value(),
            coingecko_id: coingecko_id.value(),
            symbol: symbol.value(),
            name: name.value(),
            thumb_url: thumb_url.value(),
            quantity: quantity.value(),
            price_per_unit: price_per_unit.value(),
            date: date.value(),
            note: note.value(),
            risk_level: risk_level.value(),
            network: network.value(),
        })
    }
}

/// `CryptoHoldingUpdate` — holding-only metadata that isn't part of the
/// buy/sell log, currently just `network` (`exclude_unset`, so `null`
/// clears it and an absent key leaves it alone).
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoHoldingUpdate {
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub network: Patch<String>,
}

impl FromInput for CryptoHoldingUpdate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let network = o.patch("network", |v| v.str()?.max_len(50));
        o.finish()?;
        Ok(Self { network: network.value() })
    }
}

// ---------------------------------------------------------------------------
// Query / path parameters
// ---------------------------------------------------------------------------

/// Query of `GET /api/crypto/portfolios`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortfolioListQuery {
    pub include_archived: bool,
}

impl FromInput for PortfolioListQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let include_archived = o.with_default("include_archived", || false, |v| v.bool());
        o.finish()?;
        Ok(Self { include_archived: include_archived.value() })
    }
}

/// `portfolio_id: int | None = None` — the Crypto tab's portfolio filter on
/// `GET /holdings`, `POST /refresh` and `GET /performance/90d`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortfolioFilterQuery {
    pub portfolio_id: Option<i64>,
}

impl FromInput for PortfolioFilterQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let portfolio_id = o.optional("portfolio_id", |v| v.int());
        o.finish()?;
        Ok(Self { portfolio_id: portfolio_id.value() })
    }
}

/// Query of `GET /api/crypto/history`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoHistoryQuery {
    pub range: String,
    pub portfolio_id: Option<i64>,
}

impl FromInput for CryptoHistoryQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let range = o.with_default("range", || "30d".to_string(), |v| v.str()?.pattern(HISTORY_RANGE_PATTERN));
        let portfolio_id = o.optional("portfolio_id", |v| v.int());
        o.finish()?;
        Ok(Self { range: range.value(), portfolio_id: portfolio_id.value() })
    }
}

/// Query of `GET /api/crypto/search`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoSearchQuery {
    pub q: String,
}

impl FromInput for CryptoSearchQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let q = o.required("q", |v| v.str()?.min_len(1)?.max_len(100));
        o.finish()?;
        Ok(Self { q: q.value() })
    }
}

/// Path of `/api/crypto/portfolios/{portfolio_id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortfolioPath {
    pub portfolio_id: i64,
}

impl FromInput for PortfolioPath {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let portfolio_id = o.required("portfolio_id", |v| v.int());
        o.finish()?;
        Ok(Self { portfolio_id: portfolio_id.value() })
    }
}

/// Path of `/api/crypto/holdings/{asset_id}[/transactions]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldingPath {
    pub asset_id: i64,
}

impl FromInput for HoldingPath {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let asset_id = o.required("asset_id", |v| v.int());
        o.finish()?;
        Ok(Self { asset_id: asset_id.value() })
    }
}

/// Path of `/api/crypto/transactions/{transaction_id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CryptoTransactionPath {
    pub transaction_id: i64,
}

impl FromInput for CryptoTransactionPath {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let transaction_id = o.required("transaction_id", |v| v.int());
        o.finish()?;
        Ok(Self { transaction_id: transaction_id.value() })
    }
}

// ---------------------------------------------------------------------------
// Response DTOs
// ---------------------------------------------------------------------------

/// `CryptoTransactionRead` — also the in-memory transaction the position is
/// replayed from (Python reads the ORM objects, whose values are whatever
/// was last assigned: the DB's scale-18 numbers, or a payload's own digits).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CryptoTransactionRead {
    pub id: i32,
    pub asset_id: i32,
    #[serde(rename = "type")]
    #[cfg_attr(feature = "sqlx", sqlx(rename = "type"))]
    pub type_: CryptoTransactionType,
    pub quantity: Decimal,
    pub price_per_unit: Decimal,
    pub date: NaiveDate,
    pub note: Option<String>,
}

/// `CryptoPortfolioRead`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CryptoPortfolioRead {
    pub id: i32,
    pub name: String,
    pub color: Option<String>,
    pub is_archived: bool,
}

/// `CryptoHoldingRead`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoHoldingRead {
    pub asset_id: i32,
    pub portfolio_id: i32,
    pub coingecko_id: String,
    pub symbol: String,
    pub name: String,
    pub thumb_url: Option<String>,
    /// Lives on the underlying Asset — editable via `PATCH /assets/{asset_id}`.
    pub risk_level: RiskLevel,
    /// Lives on the holding itself — editable via `PATCH /crypto/holdings/{asset_id}`.
    pub network: Option<String>,
    /// Derived from the transaction log ([`compute_position`]) — never stored.
    pub quantity: Decimal,
    pub avg_buy_price: Option<Decimal>,
    /// Cached from the last successful CoinGecko sync — null until the
    /// first one completes.
    pub current_price: Option<Decimal>,
    pub price_change_1h: Option<Decimal>,
    pub price_change_24h: Option<Decimal>,
    pub price_change_7d: Option<Decimal>,
    pub price_change_30d: Option<Decimal>,
    /// Stands in for "all time" on the Best/Worst Performer stat —
    /// CoinGecko's free tier caps historical lookback at 365 days.
    pub price_change_1y: Option<Decimal>,
    /// value = current_price * quantity, cost_basis = avg_buy_price *
    /// quantity, profit_loss = value - cost_basis.
    pub value: Option<Decimal>,
    pub cost_basis: Option<Decimal>,
    pub profit_loss: Option<Decimal>,
    pub profit_loss_percent: Option<PyFloat>,
}

/// `CryptoPerformancePoint` — `price_change_percent` is `None` when the
/// coin's 90-day chart couldn't be fetched (the item is still listed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoPerformancePoint {
    pub asset_id: i32,
    pub price_change_percent: Option<PyFloat>,
}

/// `CryptoPerformanceResponse` — real 90-day % change per held coin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoPerformanceResponse {
    pub items: Vec<CryptoPerformancePoint>,
}

/// `CryptoSyncResult`. `synced` is false both when the lazy daily check
/// didn't call CoinGecko (`error_key` null) and when CoinGecko couldn't be
/// reached (`error_key` = `"unreachable"`, a key the frontend translates).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoSyncResult {
    pub synced: bool,
    pub last_synced_at: Option<PyDateTime>,
    pub error_key: Option<String>,
    pub holdings: Vec<CryptoHoldingRead>,
}

/// `error_key` of a refresh CoinGecko couldn't serve.
pub const ERROR_KEY_UNREACHABLE: &str = "unreachable";

/// `CryptoSearchResult` — one row of the "add a coin" picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoSearchResult {
    pub coingecko_id: String,
    pub symbol: String,
    pub name: String,
    pub thumb_url: Option<String>,
}

/// `CryptoHistoryPoint`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoHistoryPoint {
    pub date: NaiveDate,
    pub value: Decimal,
}

/// `CryptoHistoryResponse` — total crypto value over time, forward-filled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CryptoHistoryResponse {
    pub range: String,
    pub current: Decimal,
    pub change_amount: Decimal,
    pub change_percent: Option<PyFloat>,
    pub series: Vec<CryptoHistoryPoint>,
}

impl CryptoHistoryResponse {
    /// The "no crypto / no valuations yet" answer.
    pub fn empty(range_key: &str) -> Self {
        Self {
            range: range_key.to_string(),
            current: Decimal::zero(),
            change_amount: Decimal::zero(),
            change_percent: None,
            series: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Holding state and position maths
// ---------------------------------------------------------------------------

/// A `crypto_holdings` row plus its asset's `risk_level` — the in-memory
/// holding the service mutates (prices, network) and reads back.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CryptoHolding {
    pub asset_id: i32,
    pub portfolio_id: i32,
    pub coingecko_id: String,
    pub symbol: String,
    pub name: String,
    pub thumb_url: Option<String>,
    pub network: Option<String>,
    pub last_price: Option<Decimal>,
    pub price_change_1h: Option<Decimal>,
    pub price_change_24h: Option<Decimal>,
    pub price_change_7d: Option<Decimal>,
    pub price_change_30d: Option<Decimal>,
    pub price_change_1y: Option<Decimal>,
}

/// One coin's row of `/coins/markets` (`_MarketPoint`): values exactly as
/// `Decimal(str(json_value))` gives them.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketPoint {
    pub price: Decimal,
    pub change_1h: Option<Decimal>,
    pub change_24h: Option<Decimal>,
    pub change_7d: Option<Decimal>,
    pub change_30d: Option<Decimal>,
    pub change_1y: Option<Decimal>,
}

impl CryptoHolding {
    /// The in-place price update of a successful sync.
    pub fn apply_market(&mut self, point: &MarketPoint) {
        self.last_price = Some(point.price.clone());
        self.price_change_1h = point.change_1h.clone();
        self.price_change_24h = point.change_24h.clone();
        self.price_change_7d = point.change_7d.clone();
        self.price_change_30d = point.change_30d.clone();
        self.price_change_1y = point.change_1y.clone();
    }
}

/// Quantity held and average buy price (`None` once nothing is held).
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub quantity: Decimal,
    pub avg_buy_price: Option<Decimal>,
}

/// `_compute_position` — weighted-average-cost method, replayed over the
/// log in `(date, id)` order: a BUY blends into the running average cost; a
/// SELL reduces quantity but leaves the average cost of what's still held
/// unchanged. Once quantity hits zero the average is gone.
pub fn compute_position<'a>(transactions: impl IntoIterator<Item = &'a CryptoTransactionRead>) -> Position {
    let mut ordered: Vec<&CryptoTransactionRead> = transactions.into_iter().collect();
    ordered.sort_by_key(|t| (t.date, t.id));
    let mut quantity = Decimal::zero();
    let mut avg_price: Option<Decimal> = None;
    for tx in ordered {
        match tx.type_ {
            CryptoTransactionType::Buy => {
                // `(avg_price or Decimal("0"))` — a zero average is falsy too.
                let base = match &avg_price {
                    Some(avg) if !avg.is_zero() => avg.clone(),
                    _ => Decimal::zero(),
                };
                let existing_cost = base.mul(&quantity);
                quantity = quantity.add(&tx.quantity);
                avg_price = if quantity.is_zero() {
                    None
                } else {
                    existing_cost.add(&tx.price_per_unit.mul(&tx.quantity)).div(&quantity)
                };
            }
            CryptoTransactionType::Sell => {
                quantity = quantity.sub(&tx.quantity);
                if quantity <= Decimal::zero() {
                    quantity = Decimal::zero();
                    avg_price = None;
                }
            }
        }
    }
    Position { quantity, avg_buy_price: avg_price }
}

/// `_to_read`: the holding's response with every derived figure.
pub fn holding_read(
    holding: &CryptoHolding,
    risk_level: RiskLevel,
    transactions: &[CryptoTransactionRead],
) -> CryptoHoldingRead {
    let Position { quantity, avg_buy_price } = compute_position(transactions);
    let current_price = holding.last_price.clone();
    let value = current_price.as_ref().map(|p| p.mul(&quantity));
    let cost_basis = avg_buy_price.as_ref().map(|a| a.mul(&quantity));
    let profit_loss = match (&value, &cost_basis) {
        (Some(v), Some(c)) => Some(v.sub(c)),
        _ => None,
    };
    let profit_loss_percent = match (&profit_loss, &cost_basis) {
        (Some(pl), Some(c)) if !c.is_zero() => pl.div(c).map(|ratio| PyFloat(ratio.mul(&Decimal::from(100)).to_f64())),
        _ => None,
    };
    CryptoHoldingRead {
        asset_id: holding.asset_id,
        portfolio_id: holding.portfolio_id,
        coingecko_id: holding.coingecko_id.clone(),
        symbol: holding.symbol.clone(),
        name: holding.name.clone(),
        thumb_url: holding.thumb_url.clone(),
        risk_level,
        network: holding.network.clone(),
        quantity,
        avg_buy_price,
        current_price,
        price_change_1h: holding.price_change_1h.clone(),
        price_change_24h: holding.price_change_24h.clone(),
        price_change_7d: holding.price_change_7d.clone(),
        price_change_30d: holding.price_change_30d.clone(),
        price_change_1y: holding.price_change_1y.clone(),
        value,
        cost_basis,
        profit_loss,
        profit_loss_percent,
    }
}

/// `_sort_by_invested`: biggest cost basis first; a fully-exited position
/// (no cost basis) sorts last. Stable, like Python's `sorted(reverse=True)`.
pub fn sort_by_invested(mut reads: Vec<CryptoHoldingRead>) -> Vec<CryptoHoldingRead> {
    let minus_one = Decimal::from(-1);
    let key = |r: &CryptoHoldingRead| r.cost_basis.clone().unwrap_or_else(|| minus_one.clone());
    reads.sort_by_key(|r| std::cmp::Reverse(key(r)));
    reads
}

/// `get_crypto_history`'s series: cumulative point events (each asset's
/// latest valuation, summed per day), forward-filled from the range start
/// to `today`. `valuations` are `(asset_id, as_of_date, value)` in
/// `as_of_date` order; empty gives the empty response.
pub fn crypto_history(
    range_key: &str,
    valuations: &[(i32, NaiveDate, Decimal)],
    today: NaiveDate,
) -> CryptoHistoryResponse {
    if valuations.is_empty() {
        return CryptoHistoryResponse::empty(range_key);
    }
    // dict insertion order, like `current_by_asset` (sum order).
    let mut current_by_asset: Vec<(i32, Decimal)> = Vec::new();
    let mut events: Vec<(NaiveDate, Decimal)> = Vec::new();
    let mut i = 0;
    while i < valuations.len() {
        let day = valuations[i].1;
        while i < valuations.len() && valuations[i].1 == day {
            let (asset_id, _, value) = &valuations[i];
            match current_by_asset.iter_mut().find(|(id, _)| id == asset_id) {
                Some(slot) => slot.1 = value.clone(),
                None => current_by_asset.push((*asset_id, value.clone())),
            }
            i += 1;
        }
        let total = current_by_asset.iter().fold(Decimal::zero(), |acc, (_, v)| acc.add(v));
        events.push((day, total));
    }

    let start = match range_days(range_key) {
        Some(days) => today - Duration::days(days - 1),
        None => events[0].0,
    };
    let mut points = Vec::new();
    let mut idx = 0;
    let mut running = Decimal::zero();
    let mut day = start;
    while day <= today {
        while idx < events.len() && events[idx].0 <= day {
            running = events[idx].1.clone();
            idx += 1;
        }
        points.push(CryptoHistoryPoint { date: day, value: running.clone() });
        day += Duration::days(1);
    }

    let current = points.last().map(|p| p.value.clone()).unwrap_or_else(Decimal::zero);
    let start_value = points.first().map(|p| p.value.clone()).unwrap_or_else(Decimal::zero);
    let change_amount = current.sub(&start_value);
    let change_percent = if start_value.is_zero() {
        None
    } else {
        change_amount.div(&start_value).map(|r| PyFloat(r.mul(&Decimal::from(100)).to_f64()))
    };
    CryptoHistoryResponse { range: range_key.to_string(), current, change_amount, change_percent, series: points }
}

/// `_fetch_90d_change`'s arithmetic: `(last - first) / first * 100`, `None`
/// for a zero first price (`if not first_price`).
pub fn percent_change(first: f64, last: f64) -> Option<f64> {
    if first == 0.0 { None } else { Some((last - first) / first * 100.0) }
}

// ---------------------------------------------------------------------------
// CoinGecko JSON — parsed the way the Python service reads `response.json()`
// ---------------------------------------------------------------------------

/// A CoinGecko answer the Python service could not digest: there it raised
/// a non-`httpx` exception (KeyError, TypeError, InvalidOperation, ...) that
/// escaped as a 500. Carries a short description for the log.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unexpected CoinGecko response: {0}")]
pub struct UpstreamShapeError(pub String);

fn shape(what: &str) -> UpstreamShapeError {
    UpstreamShapeError(what.to_string())
}

/// The shapes of a JSON value that matter for Python's duck typing.
enum Json<'a> {
    Null,
    Bool(bool),
    /// Raw number text (kept so ints/floats convert exactly like Python's).
    Number(&'a str),
    String(String),
    Array(Vec<&'a RawValue>),
    Object(HashMap<String, &'a RawValue>),
}

fn classify(raw: &RawValue) -> Result<Json<'_>, UpstreamShapeError> {
    let text = raw.get().trim();
    let bad = |_| shape("malformed JSON");
    match text.as_bytes().first() {
        Some(b'n') => Ok(Json::Null),
        Some(b't') => Ok(Json::Bool(true)),
        Some(b'f') => Ok(Json::Bool(false)),
        Some(b'"') => serde_json::from_str::<String>(text).map(Json::String).map_err(bad),
        Some(b'[') => serde_json::from_str::<Vec<&RawValue>>(text).map(Json::Array).map_err(bad),
        Some(b'{') => serde_json::from_str::<HashMap<String, &RawValue>>(text).map(Json::Object).map_err(bad),
        Some(_) => Ok(Json::Number(text)),
        None => Err(shape("empty JSON value")),
    }
}

fn parse_root(body: &[u8]) -> Result<&RawValue, UpstreamShapeError> {
    serde_json::from_slice::<&RawValue>(body).map_err(|_| shape("body is not JSON"))
}

/// Python `float(text)` for a JSON number literal: ints and floats alike.
fn number_f64(text: &str) -> Result<f64, UpstreamShapeError> {
    text.parse::<f64>().map_err(|_| shape("bad number"))
}

fn is_float_literal(text: &str) -> bool {
    text.contains(['.', 'e', 'E'])
}

/// `Decimal(str(value))` for a JSON value (`None` for null), as
/// `_decimal_or_none` does: ints print their digits, floats their `repr`.
fn py_decimal(raw: &RawValue) -> Result<Option<Decimal>, UpstreamShapeError> {
    let text = match classify(raw)? {
        Json::Null => return Ok(None),
        Json::Number(n) if is_float_literal(n) => {
            let f = number_f64(n)?;
            if !f.is_finite() {
                return Err(shape("non-finite number"));
            }
            py_float_repr(f)
        }
        Json::Number(n) => n.to_string(),
        Json::String(s) => s,
        _ => return Err(shape("value is not a number")),
    };
    text.parse::<Decimal>().map(Some).map_err(|_| shape("value is not a decimal"))
}

/// `_fetch_market_data`'s loop over a `/coins/markets` answer: coin id →
/// point, skipping coins without a `current_price`.
pub fn parse_markets(body: &[u8]) -> Result<HashMap<String, MarketPoint>, UpstreamShapeError> {
    let coins = match classify(parse_root(body)?)? {
        Json::Array(items) => items,
        // Iterating an empty dict/str yields nothing; anything else fails.
        Json::Object(map) if map.is_empty() => Vec::new(),
        Json::String(s) if s.is_empty() => Vec::new(),
        _ => return Err(shape("markets answer is not a list")),
    };
    let mut points = HashMap::new();
    for coin in coins {
        let Json::Object(coin) = classify(coin)? else {
            return Err(shape("market row is not an object"));
        };
        let field = |key: &str| -> Result<Option<Decimal>, UpstreamShapeError> {
            coin.get(key).map(|raw| py_decimal(raw)).transpose().map(Option::flatten)
        };
        let Some(price) = field("current_price")? else {
            continue;
        };
        let point = MarketPoint {
            price,
            change_1h: field("price_change_percentage_1h_in_currency")?,
            change_24h: field("price_change_percentage_24h_in_currency")?,
            change_7d: field("price_change_percentage_7d_in_currency")?,
            change_30d: field("price_change_percentage_30d_in_currency")?,
            change_1y: field("price_change_percentage_1y_in_currency")?,
        };
        let id = coin.get("id").ok_or_else(|| shape("market row without id"))?;
        // A non-string id is a valid dict key in Python but never matches a
        // holding's coingecko_id, so it's as good as absent.
        if let Json::String(id) = classify(id)? {
            points.insert(id, point);
        }
    }
    Ok(points)
}

/// Python `str(value)` for the scalar JSON values `search_coins` feeds to
/// `str(coin["symbol"])`.
fn py_str(raw: &RawValue) -> Result<String, UpstreamShapeError> {
    Ok(match classify(raw)? {
        Json::Null => "None".to_string(),
        Json::Bool(true) => "True".to_string(),
        Json::Bool(false) => "False".to_string(),
        Json::Number(n) if is_float_literal(n) => py_float_repr(number_f64(n)?),
        Json::Number(n) => n.to_string(),
        Json::String(s) => s,
        Json::Array(_) | Json::Object(_) => return Err(shape("symbol is not a scalar")),
    })
}

/// `search_coins`' mapping of a `/search` answer: the first
/// [`SEARCH_RESULT_LIMIT`] coins, symbol upper-cased.
pub fn parse_search(body: &[u8]) -> Result<Vec<CryptoSearchResult>, UpstreamShapeError> {
    let Json::Object(root) = classify(parse_root(body)?)? else {
        return Err(shape("search answer is not an object"));
    };
    let coins = match root.get("coins").map(|raw| classify(raw)).transpose()? {
        None => Vec::new(),
        Some(Json::Array(items)) => items,
        Some(Json::String(s)) if s.is_empty() => Vec::new(),
        Some(_) => return Err(shape("coins is not a list")),
    };
    let mut results = Vec::new();
    for coin in coins.into_iter().take(SEARCH_RESULT_LIMIT) {
        let Json::Object(coin) = classify(coin)? else {
            return Err(shape("coin is not an object"));
        };
        let get = |key: &str| coin.get(key).copied().ok_or_else(|| shape("coin field missing"));
        let as_str = |raw: &RawValue| match classify(raw)? {
            Json::String(s) => Ok(s),
            _ => Err(shape("coin field is not a string")),
        };
        let coingecko_id = as_str(get("id")?)?;
        let symbol = py_str(get("symbol")?)?.to_uppercase();
        let name = as_str(get("name")?)?;
        let thumb_url = match coin.get("thumb").map(|raw| classify(raw)).transpose()? {
            None | Some(Json::Null) => None,
            Some(Json::String(s)) => Some(s),
            Some(_) => return Err(shape("thumb is not a string")),
        };
        results.push(CryptoSearchResult { coingecko_id, symbol, name, thumb_url });
    }
    Ok(results)
}

/// A price of a `market_chart` point, as Python arithmetic sees it.
fn chart_price(point: &RawValue) -> Result<f64, UpstreamShapeError> {
    let Json::Array(pair) = classify(point)? else {
        return Err(shape("chart point is not a list"));
    };
    let price = pair.get(1).ok_or_else(|| shape("chart point without a price"))?;
    match classify(price)? {
        Json::Number(n) => number_f64(n),
        Json::Bool(b) => Ok(if b { 1.0 } else { 0.0 }),
        _ => Err(shape("chart price is not a number")),
    }
}

/// `_fetch_90d_change` after a successful request: first vs last price of
/// the `prices` series; `None` with fewer than two points or a zero start.
pub fn parse_90d_change(body: &[u8]) -> Result<Option<f64>, UpstreamShapeError> {
    let Json::Object(root) = classify(parse_root(body)?)? else {
        return Err(shape("chart answer is not an object"));
    };
    let prices = match root.get("prices").map(|raw| classify(raw)).transpose()? {
        None => return Ok(None),
        Some(Json::Array(items)) => items,
        // `len()` works on these, but indexing them like a list then fails.
        Some(Json::Object(map)) if map.len() < 2 => return Ok(None),
        Some(Json::String(s)) if s.chars().count() < 2 => return Ok(None),
        Some(_) => return Err(shape("prices is not a list")),
    };
    if prices.len() < 2 {
        return Ok(None);
    }
    let first = chart_price(prices[0])?;
    let last = chart_price(prices[prices.len() - 1])?;
    Ok(percent_change(first, last))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn tx(id: i32, type_: CryptoTransactionType, q: &str, p: &str, date: &str) -> CryptoTransactionRead {
        CryptoTransactionRead {
            id,
            asset_id: 1,
            type_,
            quantity: d(q),
            price_per_unit: d(p),
            date: date.parse().unwrap(),
            note: None,
        }
    }

    #[test]
    fn buy_blends_and_sell_keeps_average() {
        use CryptoTransactionType::{Buy, Sell};
        let log = [tx(1, Buy, "1", "100", "2026-01-01"), tx(2, Buy, "1", "200", "2026-01-02")];
        let p = compute_position(&log);
        assert_eq!(p.quantity.to_string(), "2");
        assert_eq!(p.avg_buy_price.unwrap().to_string(), "150");

        let log = [
            tx(3, Sell, "1.5", "300", "2026-01-03"),
            tx(1, Buy, "1", "100", "2026-01-01"),
            tx(2, Buy, "1", "200", "2026-01-02"),
        ];
        let p = compute_position(&log);
        assert_eq!(p.quantity.to_string(), "0.5");
        assert_eq!(p.avg_buy_price.unwrap().to_string(), "150");

        let log = [tx(1, Buy, "1.000000000000000000", "100", "2026-01-01"), tx(2, Sell, "1", "1", "2026-01-02")];
        let p = compute_position(&log);
        assert_eq!(p.quantity.to_string(), "0");
        assert_eq!(p.avg_buy_price, None);
    }

    #[test]
    fn holding_figures_follow_python_decimal_rules() {
        let holding = CryptoHolding {
            asset_id: 7,
            portfolio_id: 1,
            coingecko_id: "bitcoin".into(),
            symbol: "BTC".into(),
            name: "Bitcoin".into(),
            thumb_url: None,
            network: None,
            last_price: Some(d("50000")),
            price_change_1h: None,
            price_change_24h: None,
            price_change_7d: None,
            price_change_30d: None,
            price_change_1y: None,
        };
        let log = [tx(1, CryptoTransactionType::Buy, "0.5", "40000", "2026-01-01")];
        let read = holding_read(&holding, RiskLevel::High, &log);
        assert_eq!(read.value.unwrap().to_string(), "25000.0");
        assert_eq!(read.cost_basis.unwrap().to_string(), "20000.0");
        assert_eq!(read.profit_loss.unwrap().to_string(), "5000.0");
        assert_eq!(read.profit_loss_percent, Some(PyFloat(25.0)));
    }

    #[test]
    fn invested_order_is_stable_and_puts_exited_last() {
        let base = CryptoHoldingRead {
            asset_id: 0,
            portfolio_id: 1,
            coingecko_id: String::new(),
            symbol: String::new(),
            name: String::new(),
            thumb_url: None,
            risk_level: RiskLevel::High,
            network: None,
            quantity: Decimal::zero(),
            avg_buy_price: None,
            current_price: None,
            price_change_1h: None,
            price_change_24h: None,
            price_change_7d: None,
            price_change_30d: None,
            price_change_1y: None,
            value: None,
            cost_basis: None,
            profit_loss: None,
            profit_loss_percent: None,
        };
        let with =
            |id: i32, cost: Option<&str>| CryptoHoldingRead { asset_id: id, cost_basis: cost.map(d), ..base.clone() };
        let sorted =
            sort_by_invested(vec![with(1, None), with(2, Some("10")), with(3, Some("10.0")), with(4, Some("50"))]);
        let ids: Vec<i32> = sorted.iter().map(|r| r.asset_id).collect();
        assert_eq!(ids, [4, 2, 3, 1]);
    }

    #[test]
    fn history_forward_fills_and_sums_per_day() {
        let today: NaiveDate = "2026-03-10".parse().unwrap();
        let vals = [(1, "2026-03-08".parse().unwrap(), d("100.00")), (2, "2026-03-09".parse().unwrap(), d("50.00"))];
        let h = crypto_history("7d", &vals, today);
        assert_eq!(h.series.len(), 7);
        assert_eq!(h.series[0].value.to_string(), "0");
        assert_eq!(h.current.to_string(), "150.00");
        assert_eq!(h.change_amount.to_string(), "150.00");
        assert_eq!(h.change_percent, None);
        let all = crypto_history("all", &vals, today);
        assert_eq!(all.series.len(), 3);
        assert_eq!(all.change_percent, Some(PyFloat(50.0)));
    }

    #[test]
    fn coingecko_numbers_convert_like_decimal_str() {
        let body = br#"[{"id":"bitcoin","current_price":50000,"price_change_percentage_1h_in_currency":1e-05,
            "price_change_percentage_24h_in_currency":2.50,"price_change_percentage_7d_in_currency":null},
            {"id":"nothing","current_price":null}]"#;
        let points = parse_markets(body).unwrap();
        assert_eq!(points.len(), 1);
        let p = &points["bitcoin"];
        assert_eq!(p.price.to_string(), "50000");
        assert_eq!(p.change_1h.as_ref().unwrap().to_string(), "0.00001");
        assert_eq!(p.change_24h.as_ref().unwrap().to_string(), "2.5");
        assert_eq!(p.change_7d, None);
        assert!(parse_markets(br#"[{"current_price":1}]"#).is_err());
        assert!(parse_markets(b"not json").is_err());
    }

    #[test]
    fn search_and_chart_parsing() {
        let body = br#"{"coins":[{"id":"bitcoin","symbol":"btc","name":"Bitcoin","thumb":"t.png"},
            {"id":"x","symbol":5,"name":"X"}]}"#;
        let found = parse_search(body).unwrap();
        assert_eq!(found[0].symbol, "BTC");
        assert_eq!(found[1].symbol, "5");
        assert_eq!(found[1].thumb_url, None);
        assert!(parse_search(b"[]").is_err());

        assert_eq!(parse_90d_change(br#"{"prices":[[0,100.0],[1,125.0]]}"#).unwrap(), Some(25.0));
        assert_eq!(parse_90d_change(br#"{"prices":[[0,100.0]]}"#).unwrap(), None);
        assert_eq!(parse_90d_change(br#"{"prices":[[0,0],[1,5]]}"#).unwrap(), None);
        assert!(parse_90d_change(br#"{"prices":[[0,null],[1,5]]}"#).is_err());
    }
}
