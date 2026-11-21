//! Business logic for `/api/settings` — port of
//! `backend/app/services/settings_service.py` and the inline checks of
//! `backend/app/api/routes/settings.py`.

use aurum_domain::settings::{AppSettingsRead, AppSettingsRow, AppSettingsUpdate};
use aurum_domain::validation::Patch;
use sqlx::PgPool;

use crate::db::seed;
use crate::error::{ApiError, ApiResult};
use crate::repo;

/// Singleton row (id=1) — get-or-create so callers never hit a missing row
/// even if startup seeding hasn't run yet (e.g. a fresh test DB). Like
/// Python's `AppSettings(id=1)`, a row created here takes the *model*
/// default currency "USD", not AURUM_DEFAULT_CURRENCY.
pub async fn get_or_create_app_settings(pool: &PgPool) -> ApiResult<AppSettingsRow> {
    if let Some(row) = repo::settings::get(pool).await? {
        return Ok(row);
    }
    seed::insert_app_settings_if_missing(pool, "USD").await?;
    repo::settings::get(pool)
        .await?
        .ok_or_else(|| ApiError::internal(anyhow::anyhow!("app_settings row missing after insert")))
}

pub async fn read(pool: &PgPool) -> ApiResult<AppSettingsRead> {
    Ok(get_or_create_app_settings(pool).await?.into())
}

pub async fn update(pool: &PgPool, payload: AppSettingsUpdate) -> ApiResult<AppSettingsRead> {
    let current = get_or_create_app_settings(pool).await?;
    if let Patch::Value(currency) = &payload.currency {
        // Budgets/goals are denominated in the base currency, while crypto
        // valuations are fetched in it. Re-labelling those stored numbers
        // would silently corrupt their meaning, so changing the base remains
        // safe only until the first dependent row is created.
        if *currency != current.currency && repo::settings::has_currency_dependents(pool).await? {
            return Err(ApiError::conflict(
                "Base currency cannot be changed after transactions, budgets, goals, or assets have been created",
            ));
        }
    }
    Ok(repo::settings::apply_patch(pool, &payload).await?.into())
}
