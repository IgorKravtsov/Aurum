//! `/api/categories` — port of `backend/app/api/routes/categories.py`.

use aurum_domain::categories::{CategoryCreate, CategoryListQuery, CategoryPath, CategoryRead, CategoryUpdate};
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::extract::{ApiJson, ApiPath, ApiQuery, Input};
use crate::response::{created, no_content};
use crate::services;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/categories",
    tag = "categories",
    params(("kind" = Option<aurum_domain::enums::CategoryKind>, Query)),
    responses((status = 200, body = Vec<CategoryRead>), (status = 422, description = "Validation Error"))
)]
async fn list_categories(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<CategoryListQuery>,
) -> ApiResult<Json<Vec<CategoryRead>>> {
    Ok(Json(services::categories::list(&state.pool, query.kind).await?))
}

#[utoipa::path(
    post,
    path = "/api/categories",
    tag = "categories",
    request_body = CategoryCreate,
    responses(
        (status = 201, body = CategoryRead),
        (status = 400, description = "Invalid parent category"),
        (status = 422, description = "Validation Error")
    )
)]
async fn create_category(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<CategoryCreate>,
) -> ApiResult<Response> {
    Ok(created(services::categories::create(&state.pool, payload).await?))
}

#[utoipa::path(
    patch,
    path = "/api/categories/{category_id}",
    tag = "categories",
    params(("category_id" = i64, Path)),
    request_body = CategoryUpdate,
    responses(
        (status = 200, body = CategoryRead),
        (status = 400, description = "Invalid parent category"),
        (status = 404, description = "Category not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn update_category(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<CategoryPath, (), CategoryUpdate>,
) -> ApiResult<Json<CategoryRead>> {
    Ok(Json(services::categories::update(&state.pool, path.category_id, body).await?))
}

#[utoipa::path(
    delete,
    path = "/api/categories/{category_id}",
    tag = "categories",
    params(("category_id" = i64, Path)),
    responses(
        (status = 204),
        (status = 400, description = "Default category still in use"),
        (status = 404, description = "Category not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn delete_category(State(state): State<AppState>, ApiPath(path): ApiPath<CategoryPath>) -> ApiResult<Response> {
    services::categories::delete(&state.pool, path.category_id).await?;
    Ok(no_content())
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_categories, create_category))
        .routes(routes!(update_category, delete_category))
}
