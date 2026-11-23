//! Spend/income per top-level category, split-aware — port of
//! the former Python backend's `app/services/category_rollup.py` (the pure rollup lives in
//! `aurum_domain::category_rollup`).
//!
//! Public API for other services (dashboard, reports, budgets, insights):
//! [`category_contributions`] (converted per-category contributions) and
//! [`rollup_spending_by_top_level_category`] (totals per top-level
//! category, sorted by amount desc).

use aurum_domain::enums::TransactionType;
use aurum_domain::fx::MoneyAmount;
use chrono::NaiveDate;

pub use aurum_domain::category_rollup::{
    CategoryContribution, CategoryRollupChildItem, CategoryRollupItem, RollupCategory, rollup,
};

use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::services;
use crate::state::AppState;

/// Converted contributions for every category a transaction fed.
///
/// A plain transaction contributes one row; a split contributes one row per
/// split line. Currency is inherited from the parent transaction's account,
/// and all values leave this seam in the app's configured display currency.
/// Order: plain rows first, then split rows, each in Postgres' order (as
/// Python), which decides tie order downstream.
pub async fn category_contributions(
    state: &AppState,
    transaction_type: TransactionType,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
) -> ApiResult<Vec<CategoryContribution>> {
    let pool = &state.pool;
    let mut rows = repo::category_rollup::plain_contributions(pool, transaction_type, start_date, end_date).await?;
    rows.extend(repo::category_rollup::split_contributions(pool, transaction_type, start_date, end_date).await?);
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let settings = services::settings::get_or_create_app_settings(pool).await?;
    let amounts: Vec<MoneyAmount<usize>> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| MoneyAmount::new(index, row.amount.clone(), row.currency.clone(), row.date))
        .collect();
    let mut converted = services::fx::convert_amounts(state, &amounts, &settings.currency).await?;
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            let amount = converted
                .remove(&index)
                .ok_or_else(|| ApiError::internal(anyhow::anyhow!("contribution {index} was not converted")))?;
            Ok(CategoryContribution {
                transaction_id: row.transaction_id,
                category_id: row.category_id,
                amount,
                transaction_date: row.date,
                currency: settings.currency.clone(),
            })
        })
        .collect()
}

/// Every top-level category's total for the period, sorted by amount desc
/// (category sort_order as tiebreak).
pub async fn rollup_spending_by_top_level_category(
    state: &AppState,
    transaction_type: TransactionType,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
) -> ApiResult<Vec<CategoryRollupItem>> {
    let contributions = category_contributions(state, transaction_type, start_date, end_date).await?;
    if contributions.is_empty() {
        return Ok(Vec::new());
    }
    let categories = repo::categories::all_for_rollup(&state.pool).await?;
    Ok(rollup(&contributions, &categories))
}
