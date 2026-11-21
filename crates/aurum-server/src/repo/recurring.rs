//! SQL for `/api/recurring` (port of the queries in `backend/app/services/recurring_service.py`).

use aurum_domain::Decimal;
use aurum_domain::enums::{RecurringFrequency, TransactionType};
use aurum_domain::recurring::RecurringTransactionCreate;
use chrono::NaiveDate;
use sqlx::{PgExecutor, Postgres, Transaction};

/// The stored columns of one template.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct RecurringRow {
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
    pub frequency: RecurringFrequency,
    pub anchor_date: NaiveDate,
    pub last_posted_date: Option<NaiveDate>,
    pub is_active: bool,
}

macro_rules! select_recurring {
    ($tail:expr) => {
        concat!(
            "SELECT id, account_id, category_id, transfer_account_id, type, amount, destination_amount, \
             description, merchant, notes, frequency, anchor_date, last_posted_date, is_active \
             FROM recurring_transactions",
            $tail
        )
    };
}

/// `select(RecurringTransaction).order_by(RecurringTransaction.id)`.
pub async fn list(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<RecurringRow>> {
    sqlx::query_as(select_recurring!(" ORDER BY id")).fetch_all(db).await
}

pub async fn get(db: impl PgExecutor<'_>, recurring_id: i64) -> sqlx::Result<Option<RecurringRow>> {
    sqlx::query_as(select_recurring!(" WHERE id = $1")).bind(recurring_id).fetch_optional(db).await
}

/// Returns the new id. `is_active` has an ORM-side default only (`True`),
/// so it is written explicitly.
pub async fn insert(
    db: impl PgExecutor<'_>,
    payload: &RecurringTransactionCreate,
    destination_amount: Option<&Decimal>,
) -> sqlx::Result<i32> {
    sqlx::query_scalar(
        "INSERT INTO recurring_transactions (account_id, category_id, transfer_account_id, type, amount, \
         destination_amount, description, merchant, notes, frequency, anchor_date, is_active) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, true) RETURNING id",
    )
    .bind(payload.account_id)
    .bind(payload.category_id)
    .bind(payload.transfer_account_id)
    .bind(payload.type_)
    .bind(&payload.amount)
    .bind(destination_amount)
    .bind(&payload.description)
    .bind(&payload.merchant)
    .bind(&payload.notes)
    .bind(payload.frequency)
    .bind(payload.anchor_date)
    .fetch_one(db)
    .await
}

/// Which columns a PATCH sets (`exclude_unset`); `Some(None)` writes NULL
/// (and a NOT NULL column then fails like Python's commit).
#[derive(Debug, Clone, Default)]
pub struct RecurringPatch {
    pub account_id: Option<Option<i64>>,
    pub category_id: Option<Option<i64>>,
    pub transfer_account_id: Option<Option<i64>>,
    pub type_: Option<Option<TransactionType>>,
    pub amount: Option<Option<Decimal>>,
    /// Always written: the service recomputes it on every PATCH.
    pub destination_amount: Option<Decimal>,
    pub description: Option<Option<String>>,
    pub merchant: Option<Option<String>>,
    pub notes: Option<Option<String>>,
    pub frequency: Option<Option<RecurringFrequency>>,
    pub anchor_date: Option<Option<NaiveDate>>,
    pub is_active: Option<Option<bool>>,
}

/// `setattr(recurring, field, value)` for every set field, as one static
/// statement (see `repo::settings::apply_patch`), bumping `updated_at`.
pub async fn apply_patch(db: impl PgExecutor<'_>, recurring_id: i32, p: &RecurringPatch) -> sqlx::Result<()> {
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
    let (set_frequency, frequency) = parts(&p.frequency);
    let (set_anchor, anchor_date) = parts(&p.anchor_date);
    let (set_active, is_active) = parts(&p.is_active);
    sqlx::query(
        "UPDATE recurring_transactions SET \
           account_id = CASE WHEN $2 THEN $3::int8 ELSE account_id END, \
           category_id = CASE WHEN $4 THEN $5::int8 ELSE category_id END, \
           transfer_account_id = CASE WHEN $6 THEN $7::int8 ELSE transfer_account_id END, \
           type = CASE WHEN $8 THEN $9 ELSE type END, \
           amount = CASE WHEN $10 THEN $11 ELSE amount END, \
           destination_amount = $12, \
           description = CASE WHEN $13 THEN $14 ELSE description END, \
           merchant = CASE WHEN $15 THEN $16 ELSE merchant END, \
           notes = CASE WHEN $17 THEN $18 ELSE notes END, \
           frequency = CASE WHEN $19 THEN $20 ELSE frequency END, \
           anchor_date = CASE WHEN $21 THEN $22 ELSE anchor_date END, \
           is_active = CASE WHEN $23 THEN $24 ELSE is_active END, \
           updated_at = now() \
         WHERE id = $1",
    )
    .bind(recurring_id)
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
    .bind(set_frequency)
    .bind(frequency)
    .bind(set_anchor)
    .bind(anchor_date)
    .bind(set_active)
    .bind(is_active)
    .execute(db)
    .await?;
    Ok(())
}

/// `recurring.last_posted_date = today` (TimestampMixin: bumps `updated_at`).
pub async fn set_last_posted(
    tx: &mut Transaction<'_, Postgres>,
    recurring_id: i32,
    last_posted_date: NaiveDate,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE recurring_transactions SET last_posted_date = $2, updated_at = now() WHERE id = $1")
        .bind(recurring_id)
        .bind(last_posted_date)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn delete(db: impl PgExecutor<'_>, recurring_id: i32) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM recurring_transactions WHERE id = $1").bind(recurring_id).execute(db).await?;
    Ok(())
}
