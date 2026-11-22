//! Business logic for `/api/insights` — port of `backend/app/services/insights_service.py`.
//!
//! Gathers the inputs (dashboard months, the 5-year net worth summary, this
//! month's budget status, idle depository balances) and hands them to the
//! pure rules in `aurum_domain::insights`, which decide what fires. Calls
//! run in Python's order, so an upstream FX failure surfaces from the same
//! step.

use aurum_domain::Decimal;
use aurum_domain::fx::MoneyAmount;
use aurum_domain::insights::{
    AlertInputs, AlertThresholds, AlertsResponse, IDLE_CASH_ACCOUNT_TYPES, MAX_LOOKBACK_MONTHS, financial_alerts,
    idle_cash_candidates, idle_cash_count, net_worth_decline_streak, previous_month, risky_percent,
};
use chrono::{Datelike, Days, Local, NaiveDate};

use crate::error::{ApiError, ApiResult};
use crate::repo;
use crate::services;
use crate::state::AppState;

/// `_negative_cash_flow_streak`: consecutive fully-elapsed months (walking
/// back from last month) with a negative net. Months are fetched lazily, as
/// in Python, so no FX lookup happens past the first non-negative month.
async fn negative_cash_flow_streak(state: &AppState, today: NaiveDate) -> ApiResult<usize> {
    let (mut year, mut month) = previous_month(today.year(), today.month());
    let mut streak = 0;
    for _ in 0..MAX_LOOKBACK_MONTHS {
        let summary = services::dashboard::get_dashboard_summary(state, year, month).await?;
        if !summary.net.is_negative() {
            break;
        }
        streak += 1;
        (year, month) = previous_month(year, month);
    }
    Ok(streak)
}

/// `_idle_cash_account_count`: non-archived depository accounts holding at
/// least `threshold_amount` (in the display currency) and untouched for
/// `threshold_days`.
async fn idle_cash_account_count(
    state: &AppState,
    today: NaiveDate,
    threshold_amount: &Decimal,
    threshold_days: i32,
) -> ApiResult<usize> {
    let eligible: Vec<(i32, String)> = repo::insights::active_accounts(&state.pool)
        .await?
        .into_iter()
        .filter(|(_, _, type_)| IDLE_CASH_ACCOUNT_TYPES.contains(type_))
        .map(|(id, currency, _)| (id, currency))
        .collect();
    if eligible.is_empty() {
        return Ok(0);
    }

    // Same derivation the accounts service uses — balance is never stored,
    // only ever summed from the full transaction history — plus the most
    // recent date that touched each account.
    let balances = services::accounts::balances(&state.pool).await?;
    let last_activity = repo::insights::last_activity(&state.pool).await?;

    let settings = services::settings::get_or_create_app_settings(&state.pool).await?;
    let amounts: Vec<MoneyAmount<i32>> = eligible
        .iter()
        .map(|(id, currency)| {
            let balance = balances.get(id).cloned().unwrap_or_else(Decimal::zero);
            MoneyAmount::new(*id, balance, currency.clone(), today)
        })
        .collect();
    let converted = services::fx::convert_amounts(state, &amounts, &settings.currency).await?;

    // `date.today() - timedelta(days=...)` raises OverflowError (→ 500)
    // when it leaves the date range.
    let cutoff = u64::try_from(threshold_days)
        .ok()
        .and_then(|days| today.checked_sub_days(Days::new(days)))
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("idle cash cutoff out of range")))?;
    let ids: Vec<i32> = eligible.iter().map(|(id, _)| *id).collect();
    Ok(idle_cash_count(&idle_cash_candidates(&ids, &converted, &last_activity), threshold_amount, cutoff))
}

/// `get_financial_alerts`.
pub async fn get_financial_alerts(state: &AppState) -> ApiResult<AlertsResponse> {
    let today = Local::now().date_naive();
    let settings = services::settings::get_or_create_app_settings(&state.pool).await?;

    let negative_cash_flow_streak = negative_cash_flow_streak(state, today).await?;

    // The configurable threshold is capped at 24 months; five years leaves
    // ample comparison history without loading every historical FX day.
    let net_worth_summary = services::net_worth::get_net_worth_summary(state, "5y").await?;

    let budget_status = services::budgets::get_budget_status(state, today.year(), today.month()).await?;
    let over_budget_count = budget_status.items.iter().filter(|item| item.is_over_budget).count();

    let idle_cash_count =
        idle_cash_account_count(state, today, &settings.idle_cash_threshold_amount, settings.idle_cash_threshold_days)
            .await?;

    let inputs = AlertInputs {
        negative_cash_flow_streak,
        net_worth_decline_streak: net_worth_decline_streak(&net_worth_summary.series, today),
        risky_percent: risky_percent(&net_worth_summary.risk_levels),
        over_budget_count,
        idle_cash_count,
    };
    let thresholds = AlertThresholds {
        negative_cash_flow_months: settings.negative_cash_flow_threshold_months,
        net_worth_decline_months: settings.net_worth_decline_threshold_months,
        risky_allocation_percent: settings.risky_allocation_threshold_percent,
        idle_cash_days: settings.idle_cash_threshold_days,
    };
    Ok(financial_alerts(&inputs, &thresholds))
}
