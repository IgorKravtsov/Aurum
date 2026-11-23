//! Business logic for `/api/dashboard` — port of the former Python backend's `app/services/dashboard_service.py`
//! (aggregation logic behind the Overview dashboard; the pure part lives in
//! `aurum_domain::dashboard`).
//!
//! Public API for other services (insights, advice): [`get_dashboard_summary`].

use aurum_domain::dashboard::{DashboardSummary, MonthTotals, build_summary, month_bounds};
use aurum_domain::enums::TransactionType;
use aurum_domain::fx::MoneyAmount;

use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::services;
use crate::state::AppState;

/// One month's income, spending, transfers and spending by top-level
/// category, all converted into the display currency. `month` must be
/// 1..=12 (the route validates it).
pub async fn get_dashboard_summary(state: &AppState, year: i32, month: u32) -> ApiResult<DashboardSummary> {
    let (start, end) =
        month_bounds(year, month).ok_or_else(|| ApiError::internal(anyhow::anyhow!("invalid month {year}-{month}")))?;

    let rows = repo::dashboard::month_rows(&state.pool, start, end).await?;
    let settings = services::settings::get_or_create_app_settings(&state.pool).await?;
    let amounts: Vec<MoneyAmount<usize>> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| MoneyAmount::new(index, row.amount.clone(), row.currency.clone(), row.date))
        .collect();
    let converted = services::fx::convert_amounts(state, &amounts, &settings.currency).await?;
    let mut totals = MonthTotals::default();
    for (index, row) in rows.iter().enumerate() {
        if let Some(amount) = converted.get(&index) {
            totals.add(row.type_, amount);
        }
    }

    // A subcategory's spending rolls up into its parent's slice, and a split
    // transaction's category_id=NULL means its category lives on its split
    // lines instead — rollup_spending_by_top_level_category handles both
    // the same way a plain transaction's category already was.
    let rollup = services::category_rollup::rollup_spending_by_top_level_category(
        state,
        TransactionType::Expense,
        Some(start),
        Some(end),
    )
    .await?;

    Ok(build_summary(year, month as i32, totals, rollup))
}
