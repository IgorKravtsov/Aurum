//! Business logic for `/api/advice` — port of the former Python backend's `app/services/advice_service.py`.
//!
//! Gathers this month's and the trailing months' expense totals per
//! category (converted, split-aware — `category_rollup`), the budgeted
//! categories and two dashboard months, and hands them to the pure rules in
//! `aurum_domain::advice`.

use std::collections::HashMap;

use aurum_domain::Decimal;
use aurum_domain::advice::{
    AdviceResponse, TRAILING_MONTHS, accumulate, category_totals, rising_category_advice, savings_rate_trend_advice,
    unbudgeted_top_category_advice,
};
use aurum_domain::dashboard::month_bounds;
use aurum_domain::enums::TransactionType;
use aurum_domain::insights::previous_month;
use chrono::{Datelike, Local};

use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::services;
use crate::state::AppState;

/// `_category_expense_totals`: converted expense per category for one month,
/// in first-seen order.
async fn category_expense_totals(state: &AppState, year: i32, month: u32) -> ApiResult<Vec<(i32, Decimal)>> {
    let (start, end) =
        month_bounds(year, month).ok_or_else(|| ApiError::internal(anyhow::anyhow!("invalid month {year}-{month}")))?;
    let contributions =
        services::category_rollup::category_contributions(state, TransactionType::Expense, Some(start), Some(end))
            .await?;
    Ok(category_totals(&contributions))
}

/// `get_advice`: the three generators in Python's order, each skipped when
/// it has nothing to say.
pub async fn get_advice(state: &AppState) -> ApiResult<AdviceResponse> {
    let today = Local::now().date_naive();
    let (year, month) = (today.year(), today.month());
    let mut items = Vec::new();

    // Both category generators start from this month's totals and say
    // nothing without them.
    let current = category_expense_totals(state, year, month).await?;
    if !current.is_empty() {
        let mut trailing: HashMap<i32, Decimal> = HashMap::new();
        let (mut y, mut m) = (year, month);
        for _ in 0..TRAILING_MONTHS {
            (y, m) = previous_month(y, m);
            accumulate(&mut trailing, &category_expense_totals(state, y, m).await?);
        }
        let ids: Vec<i32> = current.iter().map(|(id, _)| *id).collect();
        let names = repo::advice::category_names(&state.pool, &ids).await?;
        items.extend(rising_category_advice(&current, &trailing, &names));

        let budgeted = repo::advice::budgeted_category_ids(&state.pool).await?;
        items.extend(unbudgeted_top_category_advice(&current, &budgeted, &names));
    }

    let current_summary = services::dashboard::get_dashboard_summary(state, year, month).await?;
    let (prev_year, prev_month) = previous_month(year, month);
    let previous_summary = services::dashboard::get_dashboard_summary(state, prev_year, prev_month).await?;
    items.extend(savings_rate_trend_advice(&current_summary, &previous_summary));

    Ok(AdviceResponse { items })
}
