//! SQL for `/api/transactions` (port of the queries in the former Python backend's `app/api/routes/transactions.py`).

use aurum_domain::Decimal;
use aurum_domain::enums::TransactionType;
use aurum_domain::tags::TagRead;
use aurum_domain::transactions::TransactionSort;
use chrono::NaiveDate;
use sqlx::{PgExecutor, Postgres, Transaction};

/// The stored columns of one transaction (what `TransactionFields` + `id` read).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TransactionRow {
    pub id: i32,
    pub account_id: i32,
    pub category_id: Option<i32>,
    pub transfer_account_id: Option<i32>,
    #[sqlx(rename = "type")]
    pub type_: TransactionType,
    pub amount: Decimal,
    pub destination_amount: Option<Decimal>,
    pub description: String,
    pub merchant: Option<String>,
    pub notes: Option<String>,
    pub date: NaiveDate,
}

/// One stored split line (`TransactionSplit`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct SplitRow {
    pub transaction_id: i32,
    pub id: i32,
    pub category_id: Option<i32>,
    pub amount: Decimal,
    pub note: Option<String>,
}

/// One tag attached to a transaction.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TransactionTagRow {
    pub transaction_id: i32,
    pub id: i32,
    pub name: String,
}

/// The filters of `GET /api/transactions`, already bound-checked
/// (`search_pattern` is the ILIKE pattern `%…%`).
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub account_id: Option<i32>,
    pub category_id: Option<i32>,
    pub tag_id: Option<i32>,
    pub type_: Option<TransactionType>,
    pub search_pattern: Option<String>,
}

/// Every new or edited transaction's scalar columns.
#[derive(Debug, Clone, PartialEq)]
pub struct NewTransaction {
    pub account_id: i64,
    pub category_id: Option<i64>,
    pub transfer_account_id: Option<i64>,
    pub type_: TransactionType,
    pub amount: Decimal,
    pub destination_amount: Option<Decimal>,
    pub description: String,
    pub merchant: Option<String>,
    pub notes: Option<String>,
    pub date: NaiveDate,
}

/// `SELECT … FROM transactions WHERE <every optional filter>` + `$tail`.
/// One static statement: an unset filter's parameter is NULL. The category
/// filter also matches split lines (a split transaction has
/// category_id=NULL, its categories live on the splits).
macro_rules! filtered {
    ($head:expr, $tail:expr) => {
        concat!(
            $head,
            " FROM transactions \
             WHERE ($1::int8 IS NULL OR EXTRACT(year FROM transactions.date) = $1) \
             AND ($2::int8 IS NULL OR EXTRACT(month FROM transactions.date) = $2) \
             AND ($3::date IS NULL OR transactions.date >= $3) \
             AND ($4::date IS NULL OR transactions.date <= $4) \
             AND ($5::int4 IS NULL OR transactions.account_id = $5) \
             AND ($6::int4 IS NULL OR transactions.category_id = $6 OR EXISTS (SELECT 1 FROM transaction_splits \
                  WHERE transactions.id = transaction_splits.transaction_id AND transaction_splits.category_id = $6)) \
             AND ($7::int4 IS NULL OR EXISTS (SELECT 1 FROM transaction_tags \
                  WHERE transactions.id = transaction_tags.transaction_id AND transaction_tags.tag_id = $7)) \
             AND ($8::varchar IS NULL OR transactions.type = $8) \
             AND ($9::varchar IS NULL OR transactions.description ILIKE $9 OR transactions.merchant ILIKE $9 \
                  OR transactions.notes ILIKE $9)",
            $tail
        )
    };
}

macro_rules! row_columns {
    () => {
        "SELECT transactions.id, transactions.account_id, transactions.category_id, \
         transactions.transfer_account_id, transactions.type, transactions.amount, \
         transactions.destination_amount, transactions.description, transactions.merchant, \
         transactions.notes, transactions.date"
    };
}

fn bind_filter<'q, O>(
    query: sqlx::query::QueryAs<'q, Postgres, O, sqlx::postgres::PgArguments>,
    f: &'q ListFilter,
) -> sqlx::query::QueryAs<'q, Postgres, O, sqlx::postgres::PgArguments> {
    query
        .bind(f.year)
        .bind(f.month)
        .bind(f.start_date)
        .bind(f.end_date)
        .bind(f.account_id)
        .bind(f.category_id)
        .bind(f.tag_id)
        .bind(f.type_)
        .bind(f.search_pattern.as_deref())
}

/// `select(func.count()).select_from(Transaction)` with the filters.
pub async fn count(db: impl PgExecutor<'_>, filter: &ListFilter) -> sqlx::Result<i64> {
    let (total,): (i64,) = bind_filter(sqlx::query_as(filtered!("SELECT count(*)", "")), filter).fetch_one(db).await?;
    Ok(total)
}

/// One page; `id` as a tiebreaker keeps pagination stable when many rows
/// share a date/amount.
pub async fn page(
    db: impl PgExecutor<'_>,
    filter: &ListFilter,
    sort: TransactionSort,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<TransactionRow>> {
    let sql = match sort {
        TransactionSort::AmountDesc => {
            filtered!(row_columns!(), " ORDER BY transactions.amount DESC, transactions.id DESC LIMIT $10 OFFSET $11")
        }
        TransactionSort::AmountAsc => {
            filtered!(row_columns!(), " ORDER BY transactions.amount ASC, transactions.id DESC LIMIT $10 OFFSET $11")
        }
        TransactionSort::DateDesc => {
            filtered!(row_columns!(), " ORDER BY transactions.date DESC, transactions.id DESC LIMIT $10 OFFSET $11")
        }
    };
    bind_filter(sqlx::query_as(sql), filter).bind(limit).bind(offset).fetch_all(db).await
}

pub async fn get(db: impl PgExecutor<'_>, transaction_id: i64) -> sqlx::Result<Option<TransactionRow>> {
    sqlx::query_as(concat!(row_columns!(), " FROM transactions WHERE transactions.id = $1"))
        .bind(transaction_id)
        .fetch_optional(db)
        .await
}

pub async fn exists(db: impl PgExecutor<'_>, transaction_id: i64) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM transactions WHERE id = $1)")
        .bind(transaction_id)
        .fetch_one(db)
        .await
}

/// Split lines of the given transactions, `order_by=TransactionSplit.id`.
pub async fn splits_of(db: impl PgExecutor<'_>, transaction_ids: &[i32]) -> sqlx::Result<Vec<SplitRow>> {
    sqlx::query_as(
        "SELECT transaction_splits.transaction_id, transaction_splits.id, transaction_splits.category_id, \
         transaction_splits.amount, transaction_splits.note FROM transaction_splits \
         WHERE transaction_splits.transaction_id = ANY($1) ORDER BY transaction_splits.id",
    )
    .bind(transaction_ids)
    .fetch_all(db)
    .await
}

/// Tags of the given transactions — SQLAlchemy's `selectinload` query for
/// the many-to-many, kept verbatim (no ORDER BY, like Python) so Postgres
/// returns the rows in the same order.
pub async fn tags_of(db: impl PgExecutor<'_>, transaction_ids: &[i32]) -> sqlx::Result<Vec<TransactionTagRow>> {
    sqlx::query_as(
        "SELECT transactions_1.id AS transaction_id, tags.id, tags.name \
         FROM transactions AS transactions_1 \
         JOIN transaction_tags AS transaction_tags_1 ON transactions_1.id = transaction_tags_1.transaction_id \
         JOIN tags ON tags.id = transaction_tags_1.tag_id \
         WHERE transactions_1.id = ANY($1)",
    )
    .bind(transaction_ids)
    .fetch_all(db)
    .await
}

/// `select(Tag).where(Tag.id.in_(tag_ids))` (no ORDER BY, like Python).
pub async fn find_tags(db: impl PgExecutor<'_>, tag_ids: &[i64]) -> sqlx::Result<Vec<TagRead>> {
    sqlx::query_as("SELECT tags.id, tags.name FROM tags WHERE tags.id = ANY($1)").bind(tag_ids).fetch_all(db).await
}

/// `min(date), max(date)` over every transaction.
pub async fn date_bounds(db: impl PgExecutor<'_>) -> sqlx::Result<(Option<NaiveDate>, Option<NaiveDate>)> {
    sqlx::query_as("SELECT min(transactions.date), max(transactions.date) FROM transactions").fetch_one(db).await
}

pub async fn insert(tx: &mut Transaction<'_, Postgres>, t: &NewTransaction) -> sqlx::Result<i32> {
    sqlx::query_scalar(
        "INSERT INTO transactions (account_id, category_id, transfer_account_id, type, amount, destination_amount, \
         description, merchant, notes, date) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(t.account_id)
    .bind(t.category_id)
    .bind(t.transfer_account_id)
    .bind(t.type_)
    .bind(&t.amount)
    .bind(&t.destination_amount)
    .bind(&t.description)
    .bind(&t.merchant)
    .bind(&t.notes)
    .bind(t.date)
    .fetch_one(&mut **tx)
    .await
}

/// Inserts one split line, returning its id (lines are inserted in list
/// order, so ids follow it like under SQLAlchemy).
pub async fn insert_split(
    tx: &mut Transaction<'_, Postgres>,
    transaction_id: i32,
    category_id: i64,
    amount: &Decimal,
    note: Option<&str>,
) -> sqlx::Result<i32> {
    sqlx::query_scalar(
        "INSERT INTO transaction_splits (transaction_id, category_id, amount, note) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(transaction_id)
    .bind(category_id)
    .bind(amount)
    .bind(note)
    .fetch_one(&mut **tx)
    .await
}

pub async fn delete_splits(tx: &mut Transaction<'_, Postgres>, transaction_id: i32) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM transaction_splits WHERE transaction_id = $1")
        .bind(transaction_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Makes the transaction's tag set exactly `tag_ids`: removes the others,
/// adds the missing ones in list order (SQLAlchemy's collection diff).
pub async fn set_tags(tx: &mut Transaction<'_, Postgres>, transaction_id: i32, tag_ids: &[i32]) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM transaction_tags WHERE transaction_id = $1 AND NOT (tag_id = ANY($2))")
        .bind(transaction_id)
        .bind(tag_ids)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO transaction_tags (transaction_id, tag_id) \
         SELECT $1, new.tag_id FROM unnest($2::int4[]) WITH ORDINALITY AS new(tag_id, n) \
         WHERE NOT EXISTS (SELECT 1 FROM transaction_tags WHERE transaction_id = $1 AND tag_id = new.tag_id) \
         ORDER BY new.n",
    )
    .bind(transaction_id)
    .bind(tag_ids)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Which scalar columns a PATCH sets (`exclude_unset`); `None` inside a set
/// field writes NULL (and a NOT NULL column then fails like Python's commit).
#[derive(Debug, Clone, Default)]
pub struct TransactionPatch {
    pub account_id: Option<Option<i64>>,
    pub category_id: Option<Option<i64>>,
    pub transfer_account_id: Option<Option<i64>>,
    pub type_: Option<Option<TransactionType>>,
    pub amount: Option<Option<Decimal>>,
    /// Always written: the route recomputes it on every PATCH.
    pub destination_amount: Option<Decimal>,
    pub description: Option<Option<String>>,
    pub merchant: Option<Option<String>>,
    pub notes: Option<Option<String>>,
    pub date: Option<Option<NaiveDate>>,
}

/// `setattr(transaction, field, value)` for every set field, as one static
/// statement (see `repo::settings::apply_patch`), bumping `updated_at`.
pub async fn apply_patch(
    tx: &mut Transaction<'_, Postgres>,
    transaction_id: i32,
    p: &TransactionPatch,
) -> sqlx::Result<()> {
    fn parts<T: Clone>(field: &Option<Option<T>>) -> (bool, Option<T>) {
        (field.is_some(), field.clone().flatten())
    }
    let (set_account, account_id) = parts(&p.account_id);
    let (set_category, category_id) = parts(&p.category_id);
    let (set_transfer, transfer_account_id) = parts(&p.transfer_account_id);
    let (set_type, type_) = parts(&p.type_);
    let (set_amount, amount) = parts(&p.amount);
    let (set_description, description) = parts(&p.description);
    let (set_merchant, merchant) = parts(&p.merchant);
    let (set_notes, notes) = parts(&p.notes);
    let (set_date, date) = parts(&p.date);
    sqlx::query(
        "UPDATE transactions SET \
           account_id = CASE WHEN $2 THEN $3::int8 ELSE account_id END, \
           category_id = CASE WHEN $4 THEN $5::int8 ELSE category_id END, \
           transfer_account_id = CASE WHEN $6 THEN $7::int8 ELSE transfer_account_id END, \
           type = CASE WHEN $8 THEN $9 ELSE type END, \
           amount = CASE WHEN $10 THEN $11 ELSE amount END, \
           destination_amount = $12, \
           description = CASE WHEN $13 THEN $14 ELSE description END, \
           merchant = CASE WHEN $15 THEN $16 ELSE merchant END, \
           notes = CASE WHEN $17 THEN $18 ELSE notes END, \
           date = CASE WHEN $19 THEN $20 ELSE date END, \
           updated_at = now() \
         WHERE id = $1",
    )
    .bind(transaction_id)
    .bind(set_account)
    .bind(account_id)
    .bind(set_category)
    .bind(category_id)
    .bind(set_transfer)
    .bind(transfer_account_id)
    .bind(set_type)
    .bind(type_)
    .bind(set_amount)
    .bind(amount)
    .bind(&p.destination_amount)
    .bind(set_description)
    .bind(description)
    .bind(set_merchant)
    .bind(merchant)
    .bind(set_notes)
    .bind(notes)
    .bind(set_date)
    .bind(date)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Its split lines and tag links go with it (ON DELETE CASCADE).
pub async fn delete(db: impl PgExecutor<'_>, transaction_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM transactions WHERE id = $1").bind(transaction_id).execute(db).await?;
    Ok(())
}
