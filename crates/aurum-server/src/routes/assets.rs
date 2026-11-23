//! `/api/assets` — port of the former Python backend's `app/api/routes/assets.py`.

use aurum_domain::assets::{AssetCreate, AssetPath, AssetRead, AssetUpdate, AssetValuationCreate, AssetValuationRead};
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::extract::{ApiJson, ApiPath, Input};
use crate::response::{created, no_content};
use crate::services;
use crate::state::AppState;

#[utoipa::path(get, path = "/api/assets", tag = "assets", responses((status = 200, body = Vec<AssetRead>)))]
async fn list_assets(State(state): State<AppState>) -> ApiResult<Json<Vec<AssetRead>>> {
    Ok(Json(services::assets::list(&state.pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/assets",
    tag = "assets",
    request_body = AssetCreate,
    responses((status = 201, body = AssetRead), (status = 422, description = "Validation Error"))
)]
async fn create_asset(State(state): State<AppState>, ApiJson(payload): ApiJson<AssetCreate>) -> ApiResult<Response> {
    Ok(created(services::assets::create(&state.pool, payload).await?))
}

#[utoipa::path(
    patch,
    path = "/api/assets/{asset_id}",
    tag = "assets",
    params(("asset_id" = i64, Path)),
    request_body = AssetUpdate,
    responses(
        (status = 200, body = AssetRead),
        (status = 404, description = "Asset not found"),
        (status = 409, description = "Asset currency is locked"),
        (status = 422, description = "Validation Error")
    )
)]
async fn update_asset(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<AssetPath, (), AssetUpdate>,
) -> ApiResult<Json<AssetRead>> {
    Ok(Json(services::assets::update(&state.pool, path.asset_id, body).await?))
}

#[utoipa::path(
    delete,
    path = "/api/assets/{asset_id}",
    tag = "assets",
    params(("asset_id" = i64, Path)),
    responses((status = 204), (status = 404, description = "Asset not found"), (status = 422, description = "Validation Error"))
)]
async fn delete_asset(State(state): State<AppState>, ApiPath(path): ApiPath<AssetPath>) -> ApiResult<Response> {
    services::assets::delete(&state.pool, path.asset_id).await?;
    Ok(no_content())
}

/// Records (or corrects) an asset's value as of a date. Re-submitting the
/// same date updates that day's value instead of erroring, so users can fix
/// a typo without needing a separate edit flow.
#[utoipa::path(
    post,
    path = "/api/assets/{asset_id}/valuations",
    tag = "assets",
    params(("asset_id" = i64, Path)),
    request_body = AssetValuationCreate,
    responses(
        (status = 200, body = AssetRead),
        (status = 404, description = "Asset not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn add_asset_valuation(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<AssetPath, (), AssetValuationCreate>,
) -> ApiResult<Json<AssetRead>> {
    Ok(Json(services::assets::add_valuation(&state.pool, path.asset_id, body).await?))
}

#[utoipa::path(
    get,
    path = "/api/assets/{asset_id}/valuations",
    tag = "assets",
    params(("asset_id" = i64, Path)),
    responses(
        (status = 200, body = Vec<AssetValuationRead>),
        (status = 404, description = "Asset not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn list_asset_valuations(
    State(state): State<AppState>,
    ApiPath(path): ApiPath<AssetPath>,
) -> ApiResult<Json<Vec<AssetValuationRead>>> {
    Ok(Json(services::assets::valuations(&state.pool, path.asset_id).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_assets, create_asset))
        .routes(routes!(update_asset, delete_asset))
        .routes(routes!(add_asset_valuation, list_asset_valuations))
}
