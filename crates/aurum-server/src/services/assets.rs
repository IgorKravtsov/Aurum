//! Business logic for `/api/assets` (port of the inline logic of
//! `backend/app/api/routes/assets.py`).
//!
//! Parity note — Python's session uses `expire_on_commit=False`, so the
//! Asset object it built or patched from the request keeps its Python-side
//! values in the response instead of the database's: `monthly_cash_flow`
//! echoes the payload (`"12.5"`, not `"12.50"`). Valuations are always
//! reloaded from the database, so `current_value` carries the column's
//! scale (`"1000.00"`, even for a created asset posted as `"1E+3"`).

use aurum_domain::assets::{AssetCreate, AssetRead, AssetRow, AssetUpdate, AssetValuationCreate, AssetValuationRead};
use aurum_domain::validation::Patch;
use chrono::{Local, NaiveDate};
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use crate::repo;

const NOT_FOUND: &str = "Asset not found";

fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Every asset, by name, with its current value.
pub async fn list(pool: &PgPool) -> ApiResult<Vec<AssetRead>> {
    Ok(repo::assets::list(pool).await?.into_iter().map(AssetRow::into_read).collect())
}

/// Creates the asset and its first valuation in one transaction.
pub async fn create(pool: &PgPool, payload: AssetCreate) -> ApiResult<AssetRead> {
    let as_of_date = payload.as_of_date.unwrap_or_else(today);
    let mut tx = pool.begin().await?;
    let id = repo::assets::insert(&mut *tx, &payload).await?;
    repo::assets::insert_valuation(&mut *tx, id, &payload.value, as_of_date).await?;
    tx.commit().await?;
    let mut row = repo::assets::get(pool, i64::from(id))
        .await?
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("asset {id} vanished after insert")))?;
    // The Asset object keeps the payload's own Decimal.
    row.monthly_cash_flow = payload.monthly_cash_flow;
    Ok(row.into_read())
}

pub async fn update(pool: &PgPool, asset_id: i64, payload: AssetUpdate) -> ApiResult<AssetRead> {
    let Some(asset) = repo::assets::get(pool, asset_id).await? else {
        return Err(ApiError::not_found(NOT_FOUND));
    };
    if let Patch::Value(currency) = &payload.currency
        && *currency != asset.currency
        && asset.latest_date.is_some()
    {
        return Err(ApiError::conflict("Asset currency cannot be changed after valuations have been recorded"));
    }
    repo::assets::apply_patch(pool, asset_id, &payload).await?;
    let mut row = repo::assets::get(pool, asset_id)
        .await?
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("asset {asset_id} vanished during update")))?;
    // setattr() left the patched Python value on the object.
    if let Some(flow) = payload.monthly_cash_flow.into_option() {
        row.monthly_cash_flow = flow;
    }
    Ok(row.into_read())
}

/// Records (or corrects) an asset's value as of a date. Re-submitting the
/// same date updates that day's value instead of erroring, so users can fix
/// a typo without needing a separate edit flow.
pub async fn add_valuation(pool: &PgPool, asset_id: i64, payload: AssetValuationCreate) -> ApiResult<AssetRead> {
    if !repo::assets::exists(pool, asset_id).await? {
        return Err(ApiError::not_found(NOT_FOUND));
    }
    let as_of_date = payload.as_of_date.unwrap_or_else(today);
    repo::assets::upsert_valuation(pool, asset_id, &payload.value, as_of_date).await?;
    let row = repo::assets::get(pool, asset_id)
        .await?
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("asset {asset_id} vanished after a valuation")))?;
    Ok(row.into_read())
}

pub async fn valuations(pool: &PgPool, asset_id: i64) -> ApiResult<Vec<AssetValuationRead>> {
    if !repo::assets::exists(pool, asset_id).await? {
        return Err(ApiError::not_found(NOT_FOUND));
    }
    Ok(repo::assets::valuations(pool, asset_id).await?)
}

pub async fn delete(pool: &PgPool, asset_id: i64) -> ApiResult<()> {
    if !repo::assets::exists(pool, asset_id).await? {
        return Err(ApiError::not_found(NOT_FOUND));
    }
    repo::assets::delete(pool, asset_id).await?;
    Ok(())
}
