//! Business logic for `/api/transactions` (inline in
//! `backend/app/api/routes/transactions.py` under Python).
//!
//! Response fidelity note: Python's session uses `expire_on_commit=False`
//! and never refreshes after create/update, so the `TransactionRead` it
//! returns carries the *request's* values (e.g. `"amount": "10.5"`, not the
//! stored `"10.50"`) for every field the request wrote, and stored values
//! for the rest. [`create`] and [`update`] rebuild exactly that.

use std::collections::HashMap;

use aurum_domain::Decimal;
use aurum_domain::accounts::AccountRead;
use aurum_domain::categories::CategoryRead;
use aurum_domain::enums::TransactionType;
use aurum_domain::tags::TagRead;
use aurum_domain::text::{capitalize_first_letter, py_strip};
use aurum_domain::transactions::{
    TransactionBulkCreate, TransactionBulkCreateResult, TransactionCreate, TransactionListQuery, TransactionPage,
    TransactionRead, TransactionSplitInput, TransactionSplitRead, TransactionUpdate, category_kind_violation,
    destination_needs_currencies, normalise_destination_amount, split_parent_violation, split_rule_violation,
    split_total, transfer_rule_violation, unknown_tags_detail, year_range,
};
use aurum_domain::validation::Patch;
use chrono::{Datelike, Local};
use sqlx::PgPool;

use crate::audit::log_destructive;
use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::repo::transactions::{ListFilter, NewTransaction, SplitRow, TransactionPatch, TransactionRow};

/// Python binds id filters as `INTEGER`; asyncpg refuses a value outside
/// int32 before the query runs, which surfaces as a 500.
fn int4(value: i64, what: &str) -> ApiResult<i32> {
    i32::try_from(value).map_err(|_| ApiError::internal(anyhow::anyhow!("{what} {value} is out of int32 range")))
}

fn int4_opt(value: Option<i64>, what: &str) -> ApiResult<Option<i32>> {
    value.map(|v| int4(v, what)).transpose()
}

// ---------------------------------------------------------------------------
// Write-path checks (shared by create, bulk and update)
// ---------------------------------------------------------------------------

/// `_ensure_category_matches_type`: the category must exist and, for income
/// or expense, have the matching kind. Returns the row (callers reuse it,
/// like the ORM's identity map does).
async fn ensure_category_matches_type(
    pool: &PgPool,
    category_id: Option<i64>,
    type_: Option<TransactionType>,
) -> ApiResult<Option<CategoryRead>> {
    let Some(category_id) = category_id else {
        return Ok(None);
    };
    let Some(category) = repo::categories::get(pool, category_id).await? else {
        return Err(ApiError::bad_request("Category not found"));
    };
    if let Some(detail) = category_kind_violation(&category, type_) {
        return Err(ApiError::bad_request(detail));
    }
    Ok(Some(category))
}

/// `_normalise_destination_amount` — the account lookup plus the pure rule.
async fn destination_amount(
    pool: &PgPool,
    type_: Option<TransactionType>,
    account_id: Option<i64>,
    transfer_account_id: Option<i64>,
    amount: Option<&Decimal>,
    destination: Option<&Decimal>,
) -> ApiResult<Option<Decimal>> {
    let mut currencies = HashMap::new();
    if destination_needs_currencies(type_, transfer_account_id) {
        let ids: Vec<i64> = account_id.into_iter().chain(transfer_account_id).collect();
        for (id, currency) in repo::accounts::currencies(pool, &ids).await? {
            currencies.insert(i64::from(id), currency);
        }
    }
    normalise_destination_amount(type_, account_id, transfer_account_id, amount, destination, &currencies)
        .map_err(ApiError::bad_request)
}

/// `_resolve_tags`: every id must exist (one 400 listing the unknown ones).
async fn resolve_tags(pool: &PgPool, tag_ids: &[i64]) -> ApiResult<Vec<TagRead>> {
    if tag_ids.is_empty() {
        return Ok(Vec::new());
    }
    let tags = repo::transactions::find_tags(pool, tag_ids).await?;
    if let Some(detail) = unknown_tags_detail(tag_ids, &tags) {
        return Err(ApiError::bad_request(detail));
    }
    Ok(tags)
}

/// `_build_splits`: every split's category must exist and match the type,
/// and all of them must share one top-level parent.
async fn build_splits(
    pool: &PgPool,
    splits: &[TransactionSplitInput],
    type_: Option<TransactionType>,
) -> ApiResult<Vec<(TransactionSplitInput, CategoryRead)>> {
    let mut built = Vec::with_capacity(splits.len());
    for split in splits {
        let category = ensure_category_matches_type(pool, Some(split.category_id), type_)
            .await?
            .ok_or_else(|| ApiError::internal(anyhow::anyhow!("split category vanished")))?;
        built.push((split.clone(), category));
    }
    if let Some(detail) = split_parent_violation(built.iter().map(|(_, c)| c)) {
        return Err(ApiError::bad_request(detail));
    }
    Ok(built)
}

/// A create's checks, in Python's order; returns what the insert needs.
struct PreparedCreate {
    row: NewTransaction,
    category: Option<CategoryRead>,
    tags: Vec<TagRead>,
    splits: Vec<(TransactionSplitInput, CategoryRead)>,
}

async fn prepare_create(
    pool: &PgPool,
    payload: TransactionCreate,
    category: Option<CategoryRead>,
) -> ApiResult<PreparedCreate> {
    let destination_amount = destination_amount(
        pool,
        Some(payload.type_),
        Some(payload.account_id),
        payload.transfer_account_id,
        Some(&payload.amount),
        payload.destination_amount.as_ref(),
    )
    .await?;
    let tags = resolve_tags(pool, &payload.tag_ids).await?;
    let splits = match payload.splits.as_deref() {
        Some(splits) if !splits.is_empty() => build_splits(pool, splits, Some(payload.type_)).await?,
        _ => Vec::new(),
    };
    let row = NewTransaction {
        account_id: payload.account_id,
        category_id: payload.category_id,
        transfer_account_id: payload.transfer_account_id,
        type_: payload.type_,
        amount: payload.amount,
        destination_amount,
        description: payload.description,
        merchant: payload.merchant,
        notes: payload.notes,
        date: payload.date,
    };
    Ok(PreparedCreate { row, category, tags, splits })
}

/// Inserts one prepared transaction with its splits and tags; returns the
/// new id and the split ids (in list order).
async fn insert_prepared(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    prepared: &PreparedCreate,
) -> ApiResult<(i32, Vec<i32>)> {
    let id = repo::transactions::insert(tx, &prepared.row).await?;
    let mut split_ids = Vec::with_capacity(prepared.splits.len());
    for (split, _) in &prepared.splits {
        split_ids.push(
            repo::transactions::insert_split(tx, id, split.category_id, &split.amount, split.note.as_deref()).await?,
        );
    }
    if !prepared.tags.is_empty() {
        let tag_ids: Vec<i32> = prepared.tags.iter().map(|t| t.id).collect();
        repo::transactions::set_tags(tx, id, &tag_ids).await?;
    }
    Ok((id, split_ids))
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

async fn account_for(pool: &PgPool, account_id: i32) -> ApiResult<AccountRead> {
    repo::accounts::get_many(pool, &[account_id])
        .await?
        .pop()
        .map(AccountRead::normalised)
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("account {account_id} of a transaction is missing")))
}

fn split_read(row: SplitRow, categories: &HashMap<i32, CategoryRead>) -> TransactionSplitRead {
    TransactionSplitRead {
        id: row.id,
        category_id: row.category_id,
        category: row.category_id.and_then(|id| categories.get(&id).cloned()),
        amount: row.amount,
        note: row.note,
    }
}

/// `TransactionRead` for stored rows, with `_EAGER`'s relations.
async fn hydrate(pool: &PgPool, rows: Vec<TransactionRow>) -> ApiResult<Vec<TransactionRead>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i32> = rows.iter().map(|r| r.id).collect();
    let mut account_ids: Vec<i32> = rows.iter().map(|r| r.account_id).collect();
    account_ids.dedup();
    let accounts: HashMap<i32, AccountRead> =
        repo::accounts::get_many(pool, &account_ids).await?.into_iter().map(|a| (a.id, a.normalised())).collect();
    let splits = repo::transactions::splits_of(pool, &ids).await?;
    let tags = repo::transactions::tags_of(pool, &ids).await?;
    let category_ids: Vec<i32> =
        rows.iter().filter_map(|r| r.category_id).chain(splits.iter().filter_map(|s| s.category_id)).collect();
    let categories: HashMap<i32, CategoryRead> = if category_ids.is_empty() {
        HashMap::new()
    } else {
        repo::categories::get_many(pool, &category_ids).await?.into_iter().map(|c| (c.id, c)).collect()
    };

    let mut splits_by_txn: HashMap<i32, Vec<TransactionSplitRead>> = HashMap::new();
    for split in splits {
        splits_by_txn.entry(split.transaction_id).or_default().push(split_read(split, &categories));
    }
    let mut tags_by_txn: HashMap<i32, Vec<TagRead>> = HashMap::new();
    for tag in tags {
        tags_by_txn.entry(tag.transaction_id).or_default().push(TagRead { id: tag.id, name: tag.name });
    }

    rows.into_iter()
        .map(|row| {
            let account = accounts.get(&row.account_id).cloned().ok_or_else(|| {
                ApiError::internal(anyhow::anyhow!("account {} of transaction {} is missing", row.account_id, row.id))
            })?;
            Ok(TransactionRead {
                account_id: row.account_id,
                category_id: row.category_id,
                transfer_account_id: row.transfer_account_id,
                type_: row.type_,
                amount: row.amount,
                destination_amount: row.destination_amount,
                // TransactionFields' field validator also runs on read.
                description: capitalize_first_letter(&row.description),
                merchant: row.merchant,
                notes: row.notes,
                date: row.date,
                id: row.id,
                account,
                category: row.category_id.and_then(|id| categories.get(&id).cloned()),
                tags: tags_by_txn.remove(&row.id).unwrap_or_default(),
                splits: splits_by_txn.remove(&row.id).unwrap_or_default(),
            })
        })
        .collect()
}

pub async fn list(pool: &PgPool, q: TransactionListQuery) -> ApiResult<TransactionPage> {
    let filter = ListFilter {
        year: q.year,
        month: q.month,
        start_date: q.start_date,
        end_date: q.end_date,
        account_id: int4_opt(q.account_id, "account_id")?,
        category_id: int4_opt(q.category_id, "category_id")?,
        tag_id: int4_opt(q.tag_id, "tag_id")?,
        type_: q.type_,
        // Lets the user find a transaction from any period by keyword (e.g. an
        // item bought months ago) without knowing which month to look in
        // first — matches description, merchant, and notes so any of those
        // fields can surface it.
        search_pattern: q.search.as_deref().map(|s| format!("%{}%", py_strip(s))),
    };
    let total = repo::transactions::count(pool, &filter).await?;
    // asyncpg sends OFFSET as int8 in practice: a page past int64 is a 500.
    let offset = (q.page - 1)
        .checked_mul(q.page_size)
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("page {} overflows the offset", q.page)))?;
    let limit = q.page_size;
    let rows = repo::transactions::page(pool, &filter, q.sort, limit, offset).await?;
    Ok(TransactionPage { items: hydrate(pool, rows).await?, total, page: q.page, page_size: q.page_size })
}

pub async fn years(pool: &PgPool) -> ApiResult<Vec<i32>> {
    let bounds = match repo::transactions::date_bounds(pool).await? {
        (Some(min), Some(max)) => Some((min, max)),
        _ => None,
    };
    Ok(year_range(bounds, Local::now().year()))
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

pub async fn create(pool: &PgPool, payload: TransactionCreate) -> ApiResult<TransactionRead> {
    let category = ensure_category_matches_type(pool, payload.category_id, Some(payload.type_)).await?;
    let prepared = prepare_create(pool, payload, category).await?;
    let mut tx = pool.begin().await?;
    let (id, split_ids) = insert_prepared(&mut tx, &prepared).await?;
    tx.commit().await?;

    let PreparedCreate { row, category, tags, splits } = prepared;
    let splits = splits
        .into_iter()
        .zip(split_ids)
        .map(|((split, category), split_id)| TransactionSplitRead {
            id: split_id,
            category_id: Some(category.id),
            category: Some(category),
            amount: split.amount,
            note: split.note,
        })
        .collect();
    let account_id = int4(row.account_id, "account_id")?;
    Ok(TransactionRead {
        account_id,
        category_id: int4_opt(row.category_id, "category_id")?,
        transfer_account_id: int4_opt(row.transfer_account_id, "transfer_account_id")?,
        type_: row.type_,
        amount: row.amount,
        destination_amount: row.destination_amount,
        description: row.description,
        merchant: row.merchant,
        notes: row.notes,
        date: row.date,
        id,
        account: account_for(pool, account_id).await?,
        category,
        tags,
        splits,
    })
}

/// CSV import. All rows are validated before any is added, so a bad row
/// 400s the whole request instead of leaving a half-imported statement
/// behind.
pub async fn bulk_create(pool: &PgPool, payload: TransactionBulkCreate) -> ApiResult<TransactionBulkCreateResult> {
    let mut categories = Vec::with_capacity(payload.items.len());
    for item in &payload.items {
        categories.push(ensure_category_matches_type(pool, item.category_id, Some(item.type_)).await?);
    }
    let mut prepared = Vec::with_capacity(payload.items.len());
    for (item, category) in payload.items.into_iter().zip(categories) {
        prepared.push(prepare_create(pool, item, category).await?);
    }
    let mut tx = pool.begin().await?;
    for item in &prepared {
        insert_prepared(&mut tx, item).await?;
    }
    tx.commit().await?;
    let created = prepared.len();
    // One request can add thousands of rows (CSV import) — worth a line so an
    // unexpected pile of transactions can be dated later.
    log_destructive("transactions.bulk_created", &[("count", &created)]);
    Ok(TransactionBulkCreateResult { created: created as i64 })
}

/// `updates.get(field, current)` for a PATCH field.
fn effective<T: Clone>(patch: &Patch<T>, current: Option<T>) -> Option<T> {
    match patch {
        Patch::Unset => current,
        Patch::Null => None,
        Patch::Value(v) => Some(v.clone()),
    }
}

/// The PATCH value of a field for the UPDATE (`None` = not sent).
fn written<T: Clone>(patch: &Patch<T>) -> Option<Option<T>> {
    patch.clone().into_option()
}

pub async fn update(pool: &PgPool, transaction_id: i64, payload: TransactionUpdate) -> ApiResult<TransactionRead> {
    let Some(current) = repo::transactions::get(pool, transaction_id).await? else {
        return Err(ApiError::not_found("Transaction not found"));
    };
    let current_tags = repo::transactions::tags_of(pool, &[current.id]).await?;
    let current_splits = repo::transactions::splits_of(pool, &[current.id]).await?;

    // Checks run against the row as it would look after the patch, not just
    // the fields sent: switching type alone can invalidate fields left
    // untouched.
    let type_ = effective(&payload.type_, Some(current.type_));
    let category_id = effective(&payload.category_id, current.category_id.map(i64::from));
    let account_id = effective(&payload.account_id, Some(i64::from(current.account_id)));
    let transfer_account_id = effective(&payload.transfer_account_id, current.transfer_account_id.map(i64::from));
    let amount = effective(&payload.amount, Some(current.amount.clone()));
    let destination = effective(&payload.destination_amount, current.destination_amount.clone());

    let category = ensure_category_matches_type(pool, category_id, type_).await?;
    if let Some(violation) =
        transfer_rule_violation(type_, account_id, transfer_account_id, category_id, destination.as_ref())
    {
        return Err(ApiError::bad_request(violation));
    }
    let destination_amount =
        destination_amount(pool, type_, account_id, transfer_account_id, amount.as_ref(), destination.as_ref()).await?;

    // Splits not touched by this patch are checked as they stand (their
    // category_id may be NULL if the category was since deleted — the sum
    // check needs none of that).
    let (split_count, total) = match &payload.splits {
        Patch::Value(splits) => (splits.len(), split_total(splits.iter().map(|s| &s.amount))),
        _ => (current_splits.len(), split_total(current_splits.iter().map(|s| &s.amount))),
    };
    if let Some(violation) = split_rule_violation(type_, amount.as_ref(), category_id, split_count, total.as_ref()) {
        return Err(ApiError::bad_request(violation));
    }
    let new_tags = match &payload.tag_ids {
        Patch::Value(tag_ids) => Some(resolve_tags(pool, tag_ids).await?),
        _ => None,
    };
    let new_splits = match &payload.splits {
        Patch::Value(splits) => Some(build_splits(pool, splits, type_).await?),
        _ => None,
    };

    let patch = TransactionPatch {
        account_id: written(&payload.account_id),
        category_id: written(&payload.category_id),
        transfer_account_id: written(&payload.transfer_account_id),
        type_: written(&payload.type_),
        amount: written(&payload.amount),
        destination_amount: destination_amount.clone(),
        description: written(&payload.description),
        merchant: written(&payload.merchant),
        notes: written(&payload.notes),
        date: written(&payload.date),
    };
    let mut tx = pool.begin().await?;
    repo::transactions::apply_patch(&mut tx, current.id, &patch).await?;
    if let Some(tags) = &new_tags {
        let tag_ids: Vec<i32> = tags.iter().map(|t| t.id).collect();
        repo::transactions::set_tags(&mut tx, current.id, &tag_ids).await?;
    }
    let mut new_split_ids = Vec::new();
    if let Some(splits) = &new_splits {
        repo::transactions::delete_splits(&mut tx, current.id).await?;
        for (split, _) in splits {
            new_split_ids.push(
                repo::transactions::insert_split(
                    &mut tx,
                    current.id,
                    split.category_id,
                    &split.amount,
                    split.note.as_deref(),
                )
                .await?,
            );
        }
    }
    tx.commit().await?;

    // The update went through, so every NOT NULL field is set.
    let missing = || ApiError::internal(anyhow::anyhow!("transaction {transaction_id} has a NULL required field"));
    let splits = match new_splits {
        Some(splits) => splits
            .into_iter()
            .zip(new_split_ids)
            .map(|((split, category), id)| TransactionSplitRead {
                id,
                category_id: Some(category.id),
                category: Some(category),
                amount: split.amount,
                note: split.note,
            })
            .collect(),
        None => {
            let ids: Vec<i32> = current_splits.iter().filter_map(|s| s.category_id).collect();
            let categories: HashMap<i32, CategoryRead> = if ids.is_empty() {
                HashMap::new()
            } else {
                repo::categories::get_many(pool, &ids).await?.into_iter().map(|c| (c.id, c)).collect()
            };
            current_splits.into_iter().map(|s| split_read(s, &categories)).collect()
        }
    };
    let tags =
        new_tags.unwrap_or_else(|| current_tags.into_iter().map(|t| TagRead { id: t.id, name: t.name }).collect());
    let account_id = int4(account_id.ok_or_else(missing)?, "account_id")?;
    Ok(TransactionRead {
        account_id,
        category_id: int4_opt(category_id, "category_id")?,
        transfer_account_id: int4_opt(transfer_account_id, "transfer_account_id")?,
        type_: type_.ok_or_else(missing)?,
        amount: amount.ok_or_else(missing)?,
        destination_amount,
        description: effective(&payload.description, Some(capitalize_first_letter(&current.description)))
            .ok_or_else(missing)?,
        merchant: effective(&payload.merchant, current.merchant),
        notes: effective(&payload.notes, current.notes),
        date: effective(&payload.date, Some(current.date)).ok_or_else(missing)?,
        id: current.id,
        account: account_for(pool, account_id).await?,
        category,
        tags,
        splits,
    })
}

pub async fn delete(pool: &PgPool, transaction_id: i64) -> ApiResult<()> {
    if !repo::transactions::exists(pool, transaction_id).await? {
        return Err(ApiError::not_found("Transaction not found"));
    }
    repo::transactions::delete(pool, transaction_id).await?;
    Ok(())
}
