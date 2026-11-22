//! Business logic for `/api/net-worth` — port of the former Python backend's `app/services/net_worth_service.py`
//! (the pure aggregation lives in `aurum_domain::net_worth`).
//!
//! Public API for other services (insights): [`get_net_worth_summary`].
//!
//! The steps run in Python's order — cash series, asset series, capital
//! roles, risk levels — so FX lookups (and the errors they can raise) hit
//! NBP / the `fx_rates` cache in the same sequence.

use std::collections::HashMap;

use aurum_domain::Decimal;
use aurum_domain::net_worth::{
    AssetTotals, NetWorthSummary, assemble_summary, asset_plan, capital_role_money, capital_role_summary, cash_plan,
    cash_today, requested_start, risk_level_summary,
};
use chrono::{Local, NaiveDate};

use crate::error::ApiResult;
use crate::repo;
use crate::services;
use crate::state::AppState;

/// `_cash_cumulative_events`: the converted cash total of every day from
/// the range start (or the first transaction) to today.
async fn cash_events(
    state: &AppState,
    series_start: Option<NaiveDate>,
    today: NaiveDate,
) -> ApiResult<Vec<(NaiveDate, Decimal)>> {
    let accounts = repo::net_worth::accounts(&state.pool).await?;
    let transactions = repo::net_worth::transactions(&state.pool).await?;
    let Some(plan) = cash_plan(&accounts, &transactions, series_start, today) else {
        return Ok(Vec::new());
    };
    let settings = services::settings::get_or_create_app_settings(&state.pool).await?;
    let converted = services::fx::convert_amounts(state, &plan.money, &settings.currency).await?;
    Ok(plan.events(&converted))
}

/// `_asset_events_and_class_totals`.
async fn asset_totals(state: &AppState, series_start: Option<NaiveDate>, today: NaiveDate) -> ApiResult<AssetTotals> {
    let assets = repo::net_worth::asset_meta(&state.pool).await?;
    let valuations = repo::net_worth::valuations(&state.pool).await?;
    let Some(plan) = asset_plan(&assets, &valuations, series_start, today) else {
        return Ok(AssetTotals::default());
    };
    let settings = services::settings::get_or_create_app_settings(&state.pool).await?;
    let converted = services::fx::convert_amounts(state, &plan.money, &settings.currency).await?;
    Ok(plan.totals(&converted))
}

/// Net worth over `range_key` (`"30d"`, `"90d"`, `"1y"`, `"5y"` or `"all"`;
/// any other key behaves like `"all"`, as in Python).
pub async fn get_net_worth_summary(state: &AppState, range_key: &str) -> ApiResult<NetWorthSummary> {
    let today = Local::now().date_naive();
    let start = requested_start(range_key, today);
    let cash = cash_events(state, start, today).await?;
    let assets = asset_totals(state, start, today).await?;

    let role_rows = repo::net_worth::capital_roles(&state.pool).await?;
    let settings = services::settings::get_or_create_app_settings(&state.pool).await?;
    let flows: HashMap<usize, Decimal> =
        services::fx::convert_amounts(state, &capital_role_money(&role_rows, today), &settings.currency).await?;
    let capital_roles = capital_role_summary(&role_rows, &flows, &assets);

    let risk_rows = repo::net_worth::risk_levels(&state.pool).await?;
    let risk_levels = risk_level_summary(&risk_rows, &assets, &cash_today(&cash));

    Ok(assemble_summary(range_key, today, &cash, &assets, capital_roles, risk_levels))
}
