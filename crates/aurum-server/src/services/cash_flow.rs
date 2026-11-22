//! Business logic for `/api/cash-flow` — port of the former Python backend's `app/services/cash_flow_service.py`
//! (the pure part lives in `aurum_domain::cash_flow`).
//!
//! Month-by-month income vs. expense; transfers between the user's own
//! accounts are excluded from both totals, same as the Dashboard breakdown.
//!
//! Public API for other services: [`get_cash_flow`].

use aurum_domain::cash_flow::{CashFlowEntry, CashFlowResponse, build_cash_flow};
use aurum_domain::fx::MoneyAmount;
use chrono::NaiveDate;

use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::services;
use crate::state::AppState;

/// Income/expense per month between the bounds; a missing bound defaults
/// to the first/last income or expense transaction.
pub async fn get_cash_flow(
    state: &AppState,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
) -> ApiResult<CashFlowResponse> {
    let (min_date, max_date) = repo::cash_flow::bounds(&state.pool, start_date, end_date).await?;
    let (Some(start), Some(end)) = (start_date.or(min_date), end_date.or(max_date)) else {
        return Ok(CashFlowResponse::empty(start_date.or(min_date), end_date.or(max_date)));
    };

    let rows = repo::cash_flow::rows(&state.pool, start, end).await?;
    let settings = services::settings::get_or_create_app_settings(&state.pool).await?;
    let amounts: Vec<MoneyAmount<usize>> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| MoneyAmount::new(index, row.amount.clone(), row.currency.clone(), row.date))
        .collect();
    let mut converted = services::fx::convert_amounts(state, &amounts, &settings.currency).await?;
    let entries = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let amount = converted
                .remove(&index)
                .ok_or_else(|| ApiError::internal(anyhow::anyhow!("cash-flow row {index} was not converted")))?;
            Ok(CashFlowEntry { date: row.date, type_: row.type_, amount })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(build_cash_flow(start, end, &entries))
}
