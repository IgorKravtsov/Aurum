//! SQL for `/api/cash-flow` (port of the queries in `backend/app/services/cash_flow_service.py`).

use aurum_domain::Decimal;
use aurum_domain::enums::TransactionType;
use chrono::NaiveDate;
use sqlx::PgExecutor;

/// One income/expense transaction with its account's currency.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct CashFlowRow {
    pub date: NaiveDate,
    #[sqlx(rename = "type")]
    pub type_: TransactionType,
    pub amount: Decimal,
    pub currency: String,
}

/// First and last date of an income/expense transaction within the
/// optional bounds (`(None, None)` when there is none).
pub async fn bounds(
    db: impl PgExecutor<'_>,
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
) -> sqlx::Result<(Option<NaiveDate>, Option<NaiveDate>)> {
    sqlx::query_as(
        "SELECT min(transactions.date), max(transactions.date) FROM transactions \
         WHERE transactions.type != $1 \
           AND ($2::date IS NULL OR transactions.date >= $2) \
           AND ($3::date IS NULL OR transactions.date <= $3)",
    )
    .bind(TransactionType::Transfer)
    .bind(start)
    .bind(end)
    .fetch_one(db)
    .await
}

/// Every income/expense transaction dated within `[start, end]`, in
/// Postgres' order — the order Python summed the converted amounts in.
pub async fn rows(db: impl PgExecutor<'_>, start: NaiveDate, end: NaiveDate) -> sqlx::Result<Vec<CashFlowRow>> {
    sqlx::query_as(
        "SELECT transactions.date, transactions.type, transactions.amount, accounts.currency \
         FROM transactions JOIN accounts ON accounts.id = transactions.account_id \
         WHERE transactions.type != $1 AND transactions.date >= $2 AND transactions.date <= $3",
    )
    .bind(TransactionType::Transfer)
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await
}
