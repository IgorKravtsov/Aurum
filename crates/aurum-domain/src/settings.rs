//! API contract types for `/api/settings` — port of the former Python backend's `app/schemas/settings.py`.
//!
//! Second reference implementation: Decimal fields, numeric bounds and a
//! PATCH model with `exclude_unset` semantics ([`Patch`]).

use serde::{Deserialize, Serialize};

use crate::APP_VERSION;
use crate::decimal::Decimal;
use crate::validation::{FromInput, Patch, Val, ValidationErrors};

/// `AppSettingsRead`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppSettingsRead {
    pub currency: String,
    pub negative_cash_flow_threshold_months: i32,
    pub net_worth_decline_threshold_months: i32,
    pub risky_allocation_threshold_percent: i32,
    pub idle_cash_threshold_amount: Decimal,
    pub idle_cash_threshold_days: i32,
    // Not a stored column: the default fills itself in when FastAPI validates
    // the ORM row against this model, so neither route has to assemble it.
    // It lives on this (authenticated) response rather than on /api/health,
    // which is deliberately unauthenticated and so would hand the running
    // version to anyone who can reach the instance.
    pub app_version: String,
}

/// The stored columns of the singleton `app_settings` row (id = 1).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct AppSettingsRow {
    pub currency: String,
    pub negative_cash_flow_threshold_months: i32,
    pub net_worth_decline_threshold_months: i32,
    pub risky_allocation_threshold_percent: i32,
    pub idle_cash_threshold_amount: Decimal,
    pub idle_cash_threshold_days: i32,
}

impl From<AppSettingsRow> for AppSettingsRead {
    fn from(row: AppSettingsRow) -> Self {
        Self {
            currency: row.currency,
            negative_cash_flow_threshold_months: row.negative_cash_flow_threshold_months,
            net_worth_decline_threshold_months: row.net_worth_decline_threshold_months,
            risky_allocation_threshold_percent: row.risky_allocation_threshold_percent,
            idle_cash_threshold_amount: row.idle_cash_threshold_amount,
            idle_cash_threshold_days: row.idle_cash_threshold_days,
            app_version: APP_VERSION.to_string(),
        }
    }
}

/// `AppSettingsUpdate` — all fields optional, the route applies a partial
/// update (`exclude_unset`), so a single-field PATCH from e.g. CurrencyCard
/// doesn't need to resend the alert thresholds.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppSettingsUpdate {
    // ISO 4217 code, e.g. "USD"/"UAH"/"EUR" — the frontend only ever sends
    // values from its curated currency list, but validate the shape anyway
    // since Intl.NumberFormat would otherwise silently accept garbage.
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub currency: Patch<String>,
    // Consecutive complete months before the corresponding alert fires.
    // Capped at 24 to match the insights service's MAX_LOOKBACK_MONTHS.
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub negative_cash_flow_threshold_months: Patch<i64>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub net_worth_decline_threshold_months: Patch<i64>,
    // Max % of capital allowed in medium/high risk tiers (the 80/20 rule's
    // "20% at most exposed" half) before risky_allocation_exceeded fires.
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub risky_allocation_threshold_percent: Patch<i64>,
    // Balance (in the app's display currency) and days of no activity a
    // depository account needs to hit before idle_cash fires.
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub idle_cash_threshold_amount: Patch<Decimal>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub idle_cash_threshold_days: Patch<i64>,
}

impl FromInput for AppSettingsUpdate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let currency = o.patch("currency", |v| v.str()?.min_len(3)?.max_len(3)?.pattern(r"^[A-Z]{3}$"));
        let negative = o.patch("negative_cash_flow_threshold_months", |v| v.int()?.ge(1)?.le(24));
        let decline = o.patch("net_worth_decline_threshold_months", |v| v.int()?.ge(1)?.le(24));
        let risky = o.patch("risky_allocation_threshold_percent", |v| v.int()?.ge(1)?.le(100));
        let amount = o.patch("idle_cash_threshold_amount", |v| v.decimal()?.digits(Some(14), Some(2))?.ge("0"));
        let days = o.patch("idle_cash_threshold_days", |v| v.int()?.ge(1)?.le(365));
        o.finish()?;
        Ok(Self {
            currency: currency.value(),
            negative_cash_flow_threshold_months: negative.value(),
            net_worth_decline_threshold_months: decline.value(),
            risky_allocation_threshold_percent: risky.value(),
            idle_cash_threshold_amount: amount.value(),
            idle_cash_threshold_days: days.value(),
        })
    }
}
