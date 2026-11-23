//! SQL for `/api/net-worth` (port of the queries in the former Python backend's `app/services/net_worth_service.py`).
//!
//! No `ORDER BY` where Python had none: the row order decides the order
//! Python summed converted amounts in (see `aurum_domain::net_worth`), so
//! these statements keep exactly the shape SQLAlchemy built.

use aurum_domain::net_worth::{
    AssetMetaRow, CapitalRoleRow, CashAccountRow, CashTransactionRow, RiskRow, ValuationRow,
};
use sqlx::PgExecutor;

/// `select(Account.id, Account.type, Account.currency)` — archived included.
pub async fn accounts(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<CashAccountRow>> {
    sqlx::query_as("SELECT accounts.id, accounts.type, accounts.currency FROM accounts").fetch_all(db).await
}

/// Every transaction's cash-relevant columns.
pub async fn transactions(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<CashTransactionRow>> {
    sqlx::query_as(
        "SELECT transactions.date, transactions.type, transactions.amount, transactions.destination_amount, \
         transactions.account_id, transactions.transfer_account_id FROM transactions",
    )
    .fetch_all(db)
    .await
}

/// `select(Asset.id, Asset.asset_class, Asset.currency)`.
pub async fn asset_meta(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<AssetMetaRow>> {
    sqlx::query_as("SELECT assets.id, assets.asset_class, assets.currency FROM assets").fetch_all(db).await
}

/// Every valuation, oldest first.
pub async fn valuations(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<ValuationRow>> {
    sqlx::query_as(
        "SELECT asset_valuations.asset_id, asset_valuations.as_of_date, asset_valuations.value \
         FROM asset_valuations ORDER BY asset_valuations.as_of_date",
    )
    .fetch_all(db)
    .await
}

/// `select(Asset.id, Asset.capital_role, Asset.monthly_cash_flow, Asset.currency)`.
pub async fn capital_roles(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<CapitalRoleRow>> {
    sqlx::query_as("SELECT assets.id, assets.capital_role, assets.monthly_cash_flow, assets.currency FROM assets")
        .fetch_all(db)
        .await
}

/// `select(Asset.id, Asset.name, Asset.risk_level)`.
pub async fn risk_levels(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<RiskRow>> {
    sqlx::query_as("SELECT assets.id, assets.name, assets.risk_level FROM assets").fetch_all(db).await
}
