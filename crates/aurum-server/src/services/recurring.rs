//! Business logic for `/api/recurring` — port of `backend/app/services/recurring_service.py`
//! (schedule and write-time rules live in `aurum_domain::recurring`).
//!
//! Recurring transaction templates: CRUD, plus "post now" — a one-click
//! action that creates a real Transaction from the template. Deliberately
//! not a background job: the schedule only advances when the user actually
//! clicks Post, so a missed week never silently back-fills a pile of
//! transactions.
//!
//! Response fidelity: Python's session never reloads after commit
//! (`expire_on_commit=False`, and the re-select doesn't overwrite an object
//! already in the identity map), so
//! * `create` answers the request's values (e.g. `"amount": "25"`), with
//!   the relations' names freshly loaded;
//! * `update` answers the stored row overlaid with the patched values, but
//!   the relations' names (`account_name`, `category_*`,
//!   `transfer_account_name`) as they were loaded *before* the patch —
//!   SQLAlchemy doesn't refresh a loaded relationship when its foreign key
//!   column changes.

use std::collections::HashMap;

use aurum_domain::Decimal;
use aurum_domain::enums::TransactionType;
use aurum_domain::recurring::{
    RecurringState, RecurringTransactionCreate, RecurringTransactionRead, RecurringTransactionUpdate,
    category_kind_violation, destination_account_ids, normalise_destination_amount, recurring_read,
};
use aurum_domain::validation::Patch;
use chrono::{Local, NaiveDate};
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::repo::recurring::{RecurringPatch, RecurringRow};
use crate::repo::transactions::NewTransaction;

/// The names a template's relations resolved to when it was loaded
/// (`selectinload` of account, category and transfer_account).
#[derive(Debug, Clone, Default)]
struct Relations {
    account_name: String,
    category: Option<(String, String, Option<String>)>,
    transfer_account_name: Option<String>,
}

/// Loads the relations of several templates at once.
async fn load_relations(pool: &PgPool, rows: &[RecurringRow]) -> ApiResult<Vec<Relations>> {
    let account_ids: Vec<i32> =
        rows.iter().flat_map(|r| std::iter::once(r.account_id).chain(r.transfer_account_id)).collect();
    let category_ids: Vec<i32> = rows.iter().filter_map(|r| r.category_id).collect();
    let accounts: HashMap<i32, String> = if account_ids.is_empty() {
        HashMap::new()
    } else {
        repo::accounts::get_many(pool, &account_ids).await?.into_iter().map(|a| (a.id, a.name)).collect()
    };
    let categories: HashMap<i32, (String, String, Option<String>)> = if category_ids.is_empty() {
        HashMap::new()
    } else {
        repo::categories::get_many(pool, &category_ids)
            .await?
            .into_iter()
            .map(|c| (c.id, (c.name, c.color, c.icon)))
            .collect()
    };
    rows.iter()
        .map(|r| {
            // account_id is NOT NULL with ON DELETE CASCADE: always there.
            let account_name = accounts
                .get(&r.account_id)
                .cloned()
                .ok_or_else(|| ApiError::internal(anyhow::anyhow!("recurring {} has no account", r.id)))?;
            Ok(Relations {
                account_name,
                category: r.category_id.and_then(|id| categories.get(&id).cloned()),
                transfer_account_name: r.transfer_account_id.and_then(|id| accounts.get(&id).cloned()),
            })
        })
        .collect()
}

fn state_of(row: RecurringRow, relations: Relations) -> RecurringState {
    RecurringState {
        id: row.id,
        account_id: row.account_id,
        account_name: relations.account_name,
        category_id: row.category_id,
        category: relations.category,
        transfer_account_id: row.transfer_account_id,
        transfer_account_name: relations.transfer_account_name,
        type_: row.type_,
        amount: row.amount,
        destination_amount: row.destination_amount,
        description: row.description,
        merchant: row.merchant,
        notes: row.notes,
        frequency: row.frequency,
        anchor_date: row.anchor_date,
        last_posted_date: row.last_posted_date,
        is_active: row.is_active,
    }
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// `_to_read`, as of today (local date, like Python's `date.today()`).
fn to_read(state: RecurringState) -> ApiResult<RecurringTransactionRead> {
    let id = state.id;
    recurring_read(state, today())
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("recurring {id}: next due date out of range")))
}

async fn get_or_404(pool: &PgPool, recurring_id: i64) -> ApiResult<RecurringRow> {
    repo::recurring::get(pool, recurring_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Recurring transaction not found"))
}

async fn relations_of(pool: &PgPool, row: &RecurringRow) -> ApiResult<Relations> {
    Ok(load_relations(pool, std::slice::from_ref(row)).await?.pop().unwrap_or_default())
}

/// `_ensure_category_matches_type`: the category must exist and, for
/// income or expense, have the matching kind.
async fn ensure_category_matches_type(
    pool: &PgPool,
    category_id: Option<i64>,
    type_: Option<TransactionType>,
) -> ApiResult<()> {
    let Some(category_id) = category_id else {
        return Ok(());
    };
    let Some(category) = repo::categories::get(pool, category_id).await? else {
        return Err(ApiError::bad_request("Category not found"));
    };
    match category_kind_violation(&category, type_) {
        Some(violation) => Err(ApiError::bad_request(violation)),
        None => Ok(()),
    }
}

/// `_normalise_destination_amount`, looking the accounts' currencies up
/// only when the rule needs them (as Python does).
async fn normalise_destination(
    pool: &PgPool,
    type_: Option<TransactionType>,
    account_id: Option<i64>,
    transfer_account_id: Option<i64>,
    amount: Option<&Decimal>,
    destination_amount: Option<&Decimal>,
) -> ApiResult<Option<Decimal>> {
    let currencies: HashMap<i64, String> = match destination_account_ids(type_, account_id, transfer_account_id) {
        Some((account_id, transfer_account_id)) => {
            let ids: Vec<i64> = account_id.into_iter().chain([transfer_account_id]).collect();
            repo::accounts::currencies(pool, &ids).await?.into_iter().map(|(id, c)| (i64::from(id), c)).collect()
        }
        None => HashMap::new(),
    };
    normalise_destination_amount(type_, account_id, transfer_account_id, amount, destination_amount, &currencies)
        .map_err(ApiError::bad_request)
}

pub async fn list(pool: &PgPool) -> ApiResult<Vec<RecurringTransactionRead>> {
    let rows = repo::recurring::list(pool).await?;
    let relations = load_relations(pool, &rows).await?;
    rows.into_iter().zip(relations).map(|(row, rel)| to_read(state_of(row, rel))).collect()
}

pub async fn create(pool: &PgPool, payload: RecurringTransactionCreate) -> ApiResult<RecurringTransactionRead> {
    ensure_category_matches_type(pool, payload.category_id, Some(payload.type_)).await?;
    let destination_amount = normalise_destination(
        pool,
        Some(payload.type_),
        Some(payload.account_id),
        payload.transfer_account_id,
        Some(&payload.amount),
        payload.destination_amount.as_ref(),
    )
    .await?;
    let id = repo::recurring::insert(pool, &payload, destination_amount.as_ref()).await?;
    // Stored ids fit int4 (the insert would have failed otherwise).
    let row = RecurringRow {
        id,
        account_id: payload.account_id as i32,
        category_id: payload.category_id.map(|v| v as i32),
        transfer_account_id: payload.transfer_account_id.map(|v| v as i32),
        type_: payload.type_,
        amount: payload.amount,
        destination_amount,
        description: payload.description,
        merchant: payload.merchant,
        notes: payload.notes,
        frequency: payload.frequency,
        anchor_date: payload.anchor_date,
        last_posted_date: None,
        is_active: true,
    };
    let relations = relations_of(pool, &row).await?;
    to_read(state_of(row, relations))
}

/// `updates.get(field, current)` for a PATCH field: unset keeps the stored
/// value, `null` is `None`.
fn effective<T: Clone>(patch: &Patch<T>, current: Option<T>) -> Option<T> {
    match patch {
        Patch::Unset => current,
        Patch::Null => None,
        Patch::Value(v) => Some(v.clone()),
    }
}

fn patch_field<T: Clone>(patch: &Patch<T>) -> Option<Option<T>> {
    patch.clone().into_option()
}

/// The patched value of a NOT NULL column. A `null` there already made the
/// UPDATE fail, so the fallback only keeps this total.
fn overlay<T: Clone>(patch: &Patch<T>, current: T) -> T {
    patch.as_value().cloned().unwrap_or(current)
}

/// An id PATCH field as the int4 column stores it (ids that reached the
/// response were accepted by the UPDATE, so they fit).
fn int4(patch: &Patch<i64>) -> Patch<i32> {
    match patch {
        Patch::Unset => Patch::Unset,
        Patch::Null => Patch::Null,
        Patch::Value(v) => Patch::Value(*v as i32),
    }
}

fn overlay_nullable<T: Clone>(patch: &Patch<T>, current: Option<T>) -> Option<T> {
    effective(patch, current)
}

pub async fn update(
    pool: &PgPool,
    recurring_id: i64,
    payload: RecurringTransactionUpdate,
) -> ApiResult<RecurringTransactionRead> {
    let row = get_or_404(pool, recurring_id).await?;
    let relations = relations_of(pool, &row).await?;

    let effective_type = effective(&payload.type_, Some(row.type_));
    let effective_category_id = effective(&payload.category_id, row.category_id.map(i64::from));
    ensure_category_matches_type(pool, effective_category_id, effective_type).await?;
    let amount = effective(&payload.amount, Some(row.amount.clone()));
    let destination_amount = normalise_destination(
        pool,
        effective_type,
        effective(&payload.account_id, Some(i64::from(row.account_id))),
        effective(&payload.transfer_account_id, row.transfer_account_id.map(i64::from)),
        amount.as_ref(),
        effective(&payload.destination_amount, row.destination_amount.clone()).as_ref(),
    )
    .await?;

    let patch = RecurringPatch {
        account_id: patch_field(&payload.account_id),
        category_id: patch_field(&payload.category_id),
        transfer_account_id: patch_field(&payload.transfer_account_id),
        type_: patch_field(&payload.type_),
        amount: patch_field(&payload.amount),
        destination_amount: destination_amount.clone(),
        description: patch_field(&payload.description),
        merchant: patch_field(&payload.merchant),
        notes: patch_field(&payload.notes),
        frequency: patch_field(&payload.frequency),
        anchor_date: patch_field(&payload.anchor_date),
        is_active: patch_field(&payload.is_active),
    };
    repo::recurring::apply_patch(pool, row.id, &patch).await?;

    let updated = RecurringRow {
        id: row.id,
        account_id: overlay(&int4(&payload.account_id), row.account_id),
        category_id: overlay_nullable(&int4(&payload.category_id), row.category_id),
        transfer_account_id: overlay_nullable(&int4(&payload.transfer_account_id), row.transfer_account_id),
        type_: overlay(&payload.type_, row.type_),
        amount: overlay(&payload.amount, row.amount),
        destination_amount,
        description: overlay(&payload.description, row.description),
        merchant: overlay_nullable(&payload.merchant, row.merchant),
        notes: overlay_nullable(&payload.notes, row.notes),
        frequency: overlay(&payload.frequency, row.frequency),
        anchor_date: overlay(&payload.anchor_date, row.anchor_date),
        last_posted_date: row.last_posted_date,
        is_active: overlay(&payload.is_active, row.is_active),
    };
    to_read(state_of(updated, relations))
}

pub async fn delete(pool: &PgPool, recurring_id: i64) -> ApiResult<()> {
    let row = get_or_404(pool, recurring_id).await?;
    repo::recurring::delete(pool, row.id).await?;
    Ok(())
}

/// Creates a real Transaction from the template, dated today, and moves
/// last_posted_date forward — the only thing that advances the schedule.
pub async fn post(pool: &PgPool, recurring_id: i64) -> ApiResult<RecurringTransactionRead> {
    let mut row = get_or_404(pool, recurring_id).await?;
    let relations = relations_of(pool, &row).await?;
    let today = today();
    let destination_amount = normalise_destination(
        pool,
        Some(row.type_),
        Some(i64::from(row.account_id)),
        row.transfer_account_id.map(i64::from),
        Some(&row.amount),
        row.destination_amount.as_ref(),
    )
    .await?;

    let mut tx = pool.begin().await?;
    let transaction = NewTransaction {
        account_id: i64::from(row.account_id),
        category_id: row.category_id.map(i64::from),
        transfer_account_id: row.transfer_account_id.map(i64::from),
        type_: row.type_,
        amount: row.amount.clone(),
        destination_amount,
        description: row.description.clone(),
        merchant: row.merchant.clone(),
        notes: row.notes.clone(),
        date: today,
    };
    repo::transactions::insert(&mut tx, &transaction).await?;
    repo::recurring::set_last_posted(&mut tx, row.id, today).await?;
    tx.commit().await?;

    row.last_posted_date = Some(today);
    to_read(state_of(row, relations))
}
