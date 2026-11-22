//! SQL for the category rollup (port of the queries in the former Python backend's `app/services/category_rollup.py`).

use aurum_domain::Decimal;
use aurum_domain::enums::TransactionType;
use chrono::NaiveDate;
use sqlx::PgExecutor;

/// One contribution before currency conversion: a transaction's own
/// category, or one of its split lines.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ContributionRow {
    pub transaction_id: i32,
    pub category_id: i32,
    pub amount: Decimal,
    pub date: NaiveDate,
    pub currency: String,
}

macro_rules! plain {
    ($dates:expr) => {
        concat!(
            "SELECT transactions.id AS transaction_id, transactions.category_id, transactions.amount, \
             transactions.date, accounts.currency \
             FROM transactions JOIN accounts ON accounts.id = transactions.account_id \
             WHERE transactions.type = $1 AND transactions.category_id IS NOT NULL",
            $dates
        )
    };
}

macro_rules! split {
    ($dates:expr) => {
        concat!(
            "SELECT transaction_splits.transaction_id, transaction_splits.category_id, transaction_splits.amount, \
             transactions.date, accounts.currency \
             FROM transaction_splits JOIN transactions ON transactions.id = transaction_splits.transaction_id \
             JOIN accounts ON accounts.id = transactions.account_id \
             WHERE transactions.type = $1 AND transaction_splits.category_id IS NOT NULL",
            $dates
        )
    };
}

/// The date-filter variant of a query: one static statement per
/// combination, so each has exactly the shape SQLAlchemy built.
fn pick(
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
    variants: [&'static str; 4],
) -> (&'static str, Option<NaiveDate>, Option<NaiveDate>) {
    match (start, end) {
        (None, None) => (variants[0], None, None),
        (Some(s), None) => (variants[1], Some(s), None),
        (None, Some(e)) => (variants[2], Some(e), None),
        (Some(s), Some(e)) => (variants[3], Some(s), Some(e)),
    }
}

async fn run(
    db: impl PgExecutor<'_>,
    (sql, first, second): (&'static str, Option<NaiveDate>, Option<NaiveDate>),
    type_: TransactionType,
) -> sqlx::Result<Vec<ContributionRow>> {
    let mut query = sqlx::query_as(sql).bind(type_);
    if let Some(d) = first {
        query = query.bind(d);
    }
    if let Some(d) = second {
        query = query.bind(d);
    }
    query.fetch_all(db).await
}

/// Plain transactions with a category (`plain_stmt`).
pub async fn plain_contributions(
    db: impl PgExecutor<'_>,
    type_: TransactionType,
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
) -> sqlx::Result<Vec<ContributionRow>> {
    let variants = [
        plain!(""),
        plain!(" AND transactions.date >= $2"),
        plain!(" AND transactions.date <= $2"),
        plain!(" AND transactions.date >= $2 AND transactions.date <= $3"),
    ];
    run(db, pick(start, end, variants), type_).await
}

/// Split lines with a category (`split_stmt`).
pub async fn split_contributions(
    db: impl PgExecutor<'_>,
    type_: TransactionType,
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
) -> sqlx::Result<Vec<ContributionRow>> {
    let variants = [
        split!(""),
        split!(" AND transactions.date >= $2"),
        split!(" AND transactions.date <= $2"),
        split!(" AND transactions.date >= $2 AND transactions.date <= $3"),
    ];
    run(db, pick(start, end, variants), type_).await
}
