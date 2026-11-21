//! `/api/settings` — port of `backend/app/api/routes/settings.py`.
//!
//! Second reference implementation: Decimal output, a PATCH body with
//! `exclude_unset` semantics and a 409 raised from the service.

use aurum_domain::settings::{AppSettingsRead, AppSettingsUpdate};
use axum::Json;
use axum::extract::State;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::extract::ApiJson;
use crate::services;
use crate::state::AppState;

// Both routes return the stored row plus app_version, which isn't a stored
// column (see AppSettingsRead).
#[utoipa::path(get, path = "/api/settings", tag = "settings", responses((status = 200, body = AppSettingsRead)))]
async fn read_settings(State(state): State<AppState>) -> ApiResult<Json<AppSettingsRead>> {
    Ok(Json(services::settings::read(&state.pool).await?))
}

#[utoipa::path(
    patch,
    path = "/api/settings",
    tag = "settings",
    request_body = AppSettingsUpdate,
    responses(
        (status = 200, body = AppSettingsRead),
        (status = 409, description = "Base currency already in use"),
        (status = 422, description = "Validation Error")
    )
)]
async fn update_settings(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<AppSettingsUpdate>,
) -> ApiResult<Json<AppSettingsRead>> {
    Ok(Json(services::settings::update(&state.pool, payload).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(read_settings, update_settings))
}
