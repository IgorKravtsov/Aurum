//! SQL for `/api/settings`.

use aurum_domain::settings::{AppSettingsRow, AppSettingsUpdate};
use aurum_domain::validation::Patch;
use sqlx::PgExecutor;

pub async fn get(db: impl PgExecutor<'_>) -> sqlx::Result<Option<AppSettingsRow>> {
    sqlx::query_as(
        "SELECT currency, negative_cash_flow_threshold_months, net_worth_decline_threshold_months, \
         risky_allocation_threshold_percent, idle_cash_threshold_amount, idle_cash_threshold_days \
         FROM app_settings WHERE id = 1",
    )
    .fetch_optional(db)
    .await
}

/// Whether any row that is denominated in the base currency exists
/// (transactions, budgets, goals, assets — checked in that order in Python).
pub async fn has_currency_dependents(db: impl PgExecutor<'_>) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM transactions) OR EXISTS (SELECT 1 FROM budgets) \
         OR EXISTS (SELECT 1 FROM goals) OR EXISTS (SELECT 1 FROM assets)",
    )
    .fetch_one(db)
    .await
}

/// `setattr(settings, field, value)` for every *set* field, as one static
/// statement: `(flag, value)` per column, `CASE WHEN flag` keeps the others.
/// An explicit `null` is written as NULL — the NOT NULL constraint then
/// fails exactly like the Python commit did (500).
pub async fn apply_patch(db: impl PgExecutor<'_>, patch: &AppSettingsUpdate) -> sqlx::Result<AppSettingsRow> {
    fn parts<T: Clone>(p: &Patch<T>) -> (bool, Option<T>) {
        (p.is_set(), p.as_value().cloned())
    }
    let (set_currency, currency) = parts(&patch.currency);
    let (set_negative, negative) = parts(&patch.negative_cash_flow_threshold_months);
    let (set_decline, decline) = parts(&patch.net_worth_decline_threshold_months);
    let (set_risky, risky) = parts(&patch.risky_allocation_threshold_percent);
    let (set_amount, amount) = parts(&patch.idle_cash_threshold_amount);
    let (set_days, days) = parts(&patch.idle_cash_threshold_days);
    sqlx::query_as(
        "UPDATE app_settings SET \
           currency = CASE WHEN $1 THEN $2 ELSE currency END, \
           negative_cash_flow_threshold_months = CASE WHEN $3 THEN $4::int4 ELSE negative_cash_flow_threshold_months END, \
           net_worth_decline_threshold_months = CASE WHEN $5 THEN $6::int4 ELSE net_worth_decline_threshold_months END, \
           risky_allocation_threshold_percent = CASE WHEN $7 THEN $8::int4 ELSE risky_allocation_threshold_percent END, \
           idle_cash_threshold_amount = CASE WHEN $9 THEN $10 ELSE idle_cash_threshold_amount END, \
           idle_cash_threshold_days = CASE WHEN $11 THEN $12::int4 ELSE idle_cash_threshold_days END \
         WHERE id = 1 \
         RETURNING currency, negative_cash_flow_threshold_months, net_worth_decline_threshold_months, \
           risky_allocation_threshold_percent, idle_cash_threshold_amount, idle_cash_threshold_days",
    )
    .bind(set_currency)
    .bind(currency)
    .bind(set_negative)
    .bind(negative)
    .bind(set_decline)
    .bind(decline)
    .bind(set_risky)
    .bind(risky)
    .bind(set_amount)
    .bind(amount)
    .bind(set_days)
    .bind(days)
    .fetch_one(db)
    .await
}
