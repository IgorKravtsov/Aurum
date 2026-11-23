//! SQL for `/api/dashboard` (port of the queries in the former Python backend's `app/services/dashboard_service.py`).

use aurum_domain::Decimal;
use aurum_domain::enums::TransactionType;
use chrono::NaiveDate;
use sqlx::PgExecutor;

/// One transaction of the month with its account's currency.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct MonthRow {
    #[sqlx(rename = "type")]
    pub type_: TransactionType,
    pub amount: Decimal,
    pub date: NaiveDate,
    pub currency: String,
}

/// Every transaction (any type) dated within `[start, end]`, in Postgres'
/// order — the order Python summed the converted amounts in.
pub async fn month_rows(db: impl PgExecutor<'_>, start: NaiveDate, end: NaiveDate) -> sqlx::Result<Vec<MonthRow>> {
    sqlx::query_as(
        "SELECT transactions.type, transactions.amount, transactions.date, accounts.currency \
         FROM transactions JOIN accounts ON accounts.id = transactions.account_id \
         WHERE transactions.date >= $1 AND transactions.date <= $2",
    )
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await
}
