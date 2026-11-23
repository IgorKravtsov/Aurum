//! SQL for `/api/insights` (the idle-cash check; the other signals reuse the
//! dashboard, net worth and budget services).

use std::collections::HashMap;

use aurum_domain::enums::AccountType;
use chrono::NaiveDate;
use sqlx::PgExecutor;

/// `select(Account.id, Account.currency, Account.type).where(Account.is_archived.is_(False))`
/// — the caller keeps the `IDLE_CASH_ACCOUNT_TYPES` (a domain rule).
pub async fn active_accounts(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<(i32, String, AccountType)>> {
    sqlx::query_as("SELECT id, currency, type FROM accounts WHERE is_archived IS false").fetch_all(db).await
}

/// The most recent transaction date that touched each account, on either
/// side of a transfer (`touch(account_id)` / `touch(transfer_account_id)`).
pub async fn last_activity(db: impl PgExecutor<'_>) -> sqlx::Result<HashMap<i32, NaiveDate>> {
    let rows: Vec<(i32, NaiveDate)> = sqlx::query_as(
        "SELECT account, MAX(date) FROM ( \
           SELECT account_id AS account, date FROM transactions \
           UNION ALL \
           SELECT transfer_account_id, date FROM transactions WHERE transfer_account_id IS NOT NULL \
         ) AS touched GROUP BY account",
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().collect())
}
