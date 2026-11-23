//! SQL for `/api/crypto` (port of the queries in the former Python backend's `app/services/crypto_service.py`).
//!
//! Every query mirrors the SQLAlchemy statement it replaces (same table,
//! same `ORDER BY`), so ties resolve the way they did under Python. Write
//! statements always `SET updated_at = now()` (TimestampMixin `onupdate`).

use aurum_domain::Decimal;
use aurum_domain::crypto::{CryptoHolding, CryptoPortfolioRead, CryptoPortfolioUpdate, CryptoTransactionRead};
use aurum_domain::enums::{AssetClass, CapitalRole, CryptoTransactionType, RiskLevel};
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgExecutor;

// ---------------------------------------------------------------------------
// Portfolios
// ---------------------------------------------------------------------------

/// `select(CryptoPortfolio).order_by(CryptoPortfolio.id)`, optionally
/// without archived portfolios.
pub async fn list_portfolios(
    db: impl PgExecutor<'_>,
    include_archived: bool,
) -> sqlx::Result<Vec<CryptoPortfolioRead>> {
    let sql = if include_archived {
        "SELECT id, name, color, is_archived FROM crypto_portfolios ORDER BY id"
    } else {
        "SELECT id, name, color, is_archived FROM crypto_portfolios WHERE is_archived IS false ORDER BY id"
    };
    sqlx::query_as(sql).fetch_all(db).await
}

pub async fn get_portfolio(db: impl PgExecutor<'_>, portfolio_id: i64) -> sqlx::Result<Option<CryptoPortfolioRead>> {
    sqlx::query_as("SELECT id, name, color, is_archived FROM crypto_portfolios WHERE id = $1")
        .bind(portfolio_id)
        .fetch_optional(db)
        .await
}

/// The earliest-created portfolio (`get_or_create_default_portfolio`).
pub async fn first_portfolio(db: impl PgExecutor<'_>) -> sqlx::Result<Option<CryptoPortfolioRead>> {
    sqlx::query_as("SELECT id, name, color, is_archived FROM crypto_portfolios ORDER BY id LIMIT 1")
        .fetch_optional(db)
        .await
}

/// How many portfolios exist, archived ones included (picks the next color).
pub async fn count_portfolios(db: impl PgExecutor<'_>) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM crypto_portfolios").fetch_one(db).await
}

/// `is_archived` has an ORM-side default only, so it is written explicitly.
pub async fn insert_portfolio(db: impl PgExecutor<'_>, name: &str, color: &str) -> sqlx::Result<CryptoPortfolioRead> {
    sqlx::query_as(
        "INSERT INTO crypto_portfolios (name, color, is_archived) VALUES ($1, $2, false) \
         RETURNING id, name, color, is_archived",
    )
    .bind(name)
    .bind(color)
    .fetch_one(db)
    .await
}

/// Static partial update: only the fields set in the PATCH body change; an
/// explicit `null` writes NULL and fails on the NOT NULL column (500).
pub async fn update_portfolio(
    db: impl PgExecutor<'_>,
    portfolio_id: i64,
    patch: &CryptoPortfolioUpdate,
) -> sqlx::Result<CryptoPortfolioRead> {
    sqlx::query_as(
        "UPDATE crypto_portfolios SET \
           name = CASE WHEN $2 THEN $3 ELSE name END, \
           is_archived = CASE WHEN $4 THEN $5::boolean ELSE is_archived END, \
           updated_at = now() \
         WHERE id = $1 RETURNING id, name, color, is_archived",
    )
    .bind(portfolio_id)
    .bind(patch.name.is_set())
    .bind(patch.name.as_value())
    .bind(patch.is_archived.is_set())
    .bind(patch.is_archived.as_value())
    .fetch_one(db)
    .await
}

pub async fn portfolio_has_holding(db: impl PgExecutor<'_>, portfolio_id: i64) -> sqlx::Result<bool> {
    let found: Option<i32> = sqlx::query_scalar("SELECT asset_id FROM crypto_holdings WHERE portfolio_id = $1 LIMIT 1")
        .bind(portfolio_id)
        .fetch_optional(db)
        .await?;
    Ok(found.is_some())
}

pub async fn delete_portfolio(db: impl PgExecutor<'_>, portfolio_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM crypto_portfolios WHERE id = $1").bind(portfolio_id).execute(db).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Holdings
// ---------------------------------------------------------------------------

/// Columns of [`CryptoHolding`] (a macro so the SQL stays `&'static str`).
macro_rules! holding_columns {
    () => {
        "asset_id, portfolio_id, coingecko_id, symbol, name, thumb_url, network, last_price, \
         price_change_1h, price_change_24h, price_change_7d, price_change_30d, price_change_1y"
    };
}

/// `select(CryptoHolding).order_by(CryptoHolding.name)`, optionally scoped
/// to one portfolio. The asset and the transactions are loaded separately
/// (`selectinload`), so the ordering query is the same single-table one.
pub async fn list_holdings(db: impl PgExecutor<'_>, portfolio_id: Option<i64>) -> sqlx::Result<Vec<CryptoHolding>> {
    match portfolio_id {
        None => {
            sqlx::query_as(concat!("SELECT ", holding_columns!(), " FROM crypto_holdings ORDER BY name"))
                .fetch_all(db)
                .await
        }
        Some(id) => {
            sqlx::query_as(concat!(
                "SELECT ",
                holding_columns!(),
                " FROM crypto_holdings WHERE portfolio_id = $1 ORDER BY name"
            ))
            .bind(id)
            .fetch_all(db)
            .await
        }
    }
}

pub async fn get_holding(db: impl PgExecutor<'_>, asset_id: i64) -> sqlx::Result<Option<CryptoHolding>> {
    sqlx::query_as(concat!("SELECT ", holding_columns!(), " FROM crypto_holdings WHERE asset_id = $1"))
        .bind(asset_id)
        .fetch_optional(db)
        .await
}

/// `risk_level` of the underlying assets (`selectinload(CryptoHolding.asset)`).
pub async fn risk_levels(db: impl PgExecutor<'_>, asset_ids: &[i32]) -> sqlx::Result<Vec<(i32, RiskLevel)>> {
    sqlx::query_as("SELECT id, risk_level FROM assets WHERE id = ANY($1)").bind(asset_ids).fetch_all(db).await
}

/// The holding's own row; `portfolio_id` is the resolved portfolio.
pub async fn insert_holding(db: impl PgExecutor<'_>, holding: &CryptoHolding) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO crypto_holdings (asset_id, portfolio_id, coingecko_id, symbol, name, thumb_url, network) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(holding.asset_id)
    .bind(holding.portfolio_id)
    .bind(&holding.coingecko_id)
    .bind(&holding.symbol)
    .bind(&holding.name)
    .bind(&holding.thumb_url)
    .bind(&holding.network)
    .execute(db)
    .await?;
    Ok(())
}

/// The cached market data of a sync.
pub async fn update_holding_prices(db: impl PgExecutor<'_>, holding: &CryptoHolding) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE crypto_holdings SET last_price = $2, price_change_1h = $3, price_change_24h = $4, \
           price_change_7d = $5, price_change_30d = $6, price_change_1y = $7, updated_at = now() \
         WHERE asset_id = $1",
    )
    .bind(holding.asset_id)
    .bind(&holding.last_price)
    .bind(&holding.price_change_1h)
    .bind(&holding.price_change_24h)
    .bind(&holding.price_change_7d)
    .bind(&holding.price_change_30d)
    .bind(&holding.price_change_1y)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn update_holding_network(db: impl PgExecutor<'_>, asset_id: i32, network: Option<&str>) -> sqlx::Result<()> {
    sqlx::query("UPDATE crypto_holdings SET network = $2, updated_at = now() WHERE asset_id = $1")
        .bind(asset_id)
        .bind(network)
        .execute(db)
        .await?;
    Ok(())
}

/// The Asset a new holding sits on: `asset_class=CRYPTO`, the app's
/// currency, NEUTRAL capital role and the chosen risk level (ORM defaults
/// written explicitly).
pub async fn insert_asset(
    db: impl PgExecutor<'_>,
    name: &str,
    currency: &str,
    risk_level: RiskLevel,
) -> sqlx::Result<i32> {
    sqlx::query_scalar(
        "INSERT INTO assets (name, asset_class, currency, capital_role, risk_level) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(name)
    .bind(AssetClass::Crypto)
    .bind(currency)
    .bind(CapitalRole::Neutral)
    .bind(risk_level)
    .fetch_one(db)
    .await
}

// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

/// Columns of [`CryptoTransactionRead`].
macro_rules! transaction_columns {
    () => {
        "id, asset_id, type, quantity, price_per_unit, date, note"
    };
}

/// Every transaction of the given holdings, in the relationship's
/// `order_by=CryptoTransaction.date`.
pub async fn transactions_of(db: impl PgExecutor<'_>, asset_ids: &[i32]) -> sqlx::Result<Vec<CryptoTransactionRead>> {
    sqlx::query_as(concat!(
        "SELECT ",
        transaction_columns!(),
        " FROM crypto_transactions WHERE asset_id = ANY($1) ORDER BY date"
    ))
    .bind(asset_ids)
    .fetch_all(db)
    .await
}

/// `GET /holdings/{asset_id}/transactions`: newest first.
pub async fn list_transactions(db: impl PgExecutor<'_>, asset_id: i64) -> sqlx::Result<Vec<CryptoTransactionRead>> {
    sqlx::query_as(concat!(
        "SELECT ",
        transaction_columns!(),
        " FROM crypto_transactions WHERE asset_id = $1 ORDER BY date DESC, id DESC"
    ))
    .bind(asset_id)
    .fetch_all(db)
    .await
}

pub async fn get_transaction(
    db: impl PgExecutor<'_>,
    transaction_id: i64,
) -> sqlx::Result<Option<CryptoTransactionRead>> {
    sqlx::query_as(concat!("SELECT ", transaction_columns!(), " FROM crypto_transactions WHERE id = $1"))
        .bind(transaction_id)
        .fetch_optional(db)
        .await
}

/// A new buy/sell row; returns its id.
pub async fn insert_transaction(
    db: impl PgExecutor<'_>,
    asset_id: i32,
    type_: CryptoTransactionType,
    quantity: &Decimal,
    price_per_unit: &Decimal,
    date: NaiveDate,
    note: Option<&str>,
) -> sqlx::Result<i32> {
    sqlx::query_scalar(
        "INSERT INTO crypto_transactions (asset_id, type, quantity, price_per_unit, date, note) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(asset_id)
    .bind(type_)
    .bind(quantity)
    .bind(price_per_unit)
    .bind(date)
    .bind(note)
    .fetch_one(db)
    .await
}

/// Writes an edited transaction back (every editable column).
pub async fn update_transaction(db: impl PgExecutor<'_>, tx: &CryptoTransactionRead) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE crypto_transactions SET type = $2, quantity = $3, price_per_unit = $4, date = $5, note = $6, \
           updated_at = now() WHERE id = $1",
    )
    .bind(tx.id)
    .bind(tx.type_)
    .bind(&tx.quantity)
    .bind(&tx.price_per_unit)
    .bind(tx.date)
    .bind(&tx.note)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_transaction(db: impl PgExecutor<'_>, transaction_id: i32) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM crypto_transactions WHERE id = $1").bind(transaction_id).execute(db).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Valuations, sync state, history
// ---------------------------------------------------------------------------

/// `_upsert_valuation`: re-syncing the same day updates that day's value.
pub async fn upsert_valuation(
    db: impl PgExecutor<'_>,
    asset_id: i32,
    value: &Decimal,
    as_of_date: NaiveDate,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO asset_valuations (asset_id, value, as_of_date) VALUES ($1, $2, $3) \
         ON CONFLICT (asset_id, as_of_date) DO UPDATE SET value = $2",
    )
    .bind(asset_id)
    .bind(value)
    .bind(as_of_date)
    .execute(db)
    .await?;
    Ok(())
}

/// `get_or_create_sync_state`: the singleton row (id=1), created with no
/// timestamp when missing; returns `last_synced_at`.
pub async fn get_or_create_sync_state(db: impl PgExecutor<'_> + Copy) -> sqlx::Result<Option<DateTime<Utc>>> {
    let existing: Option<Option<DateTime<Utc>>> =
        sqlx::query_scalar("SELECT last_synced_at FROM crypto_sync_state WHERE id = 1").fetch_optional(db).await?;
    if let Some(last) = existing {
        return Ok(last);
    }
    sqlx::query("INSERT INTO crypto_sync_state (id, last_synced_at) VALUES (1, NULL) ON CONFLICT (id) DO NOTHING")
        .execute(db)
        .await?;
    sqlx::query_scalar("SELECT last_synced_at FROM crypto_sync_state WHERE id = 1").fetch_one(db).await
}

/// Records a successful sync (creating the singleton row if needed).
pub async fn set_last_synced(db: impl PgExecutor<'_>, at: DateTime<Utc>) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO crypto_sync_state (id, last_synced_at) VALUES (1, $1) \
         ON CONFLICT (id) DO UPDATE SET last_synced_at = $1",
    )
    .bind(at)
    .execute(db)
    .await?;
    Ok(())
}

/// Ids of crypto-class assets that are holdings, optionally of one portfolio.
pub async fn crypto_asset_ids(db: impl PgExecutor<'_>, portfolio_id: Option<i64>) -> sqlx::Result<Vec<i32>> {
    sqlx::query_scalar(
        "SELECT assets.id FROM assets JOIN crypto_holdings ON crypto_holdings.asset_id = assets.id \
         WHERE assets.asset_class = $1 AND ($2::int8 IS NULL OR crypto_holdings.portfolio_id = $2)",
    )
    .bind(AssetClass::Crypto)
    .bind(portfolio_id)
    .fetch_all(db)
    .await
}

/// `(asset_id, as_of_date, value)` of the given assets, by date.
pub async fn valuations(db: impl PgExecutor<'_>, asset_ids: &[i32]) -> sqlx::Result<Vec<(i32, NaiveDate, Decimal)>> {
    sqlx::query_as(
        "SELECT asset_id, as_of_date, value FROM asset_valuations WHERE asset_id = ANY($1) ORDER BY as_of_date",
    )
    .bind(asset_ids)
    .fetch_all(db)
    .await
}
