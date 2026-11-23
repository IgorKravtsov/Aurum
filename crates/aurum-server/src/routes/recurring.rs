//! `/api/recurring` — port of the former Python backend's `app/api/routes/recurring.py`.

use aurum_domain::recurring::{
    RecurringPath, RecurringTransactionCreate, RecurringTransactionRead, RecurringTransactionUpdate,
};
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

#[utoipa::path(
    get,
    path = "/api/recurring",
    tag = "recurring",
    responses((status = 200, body = Vec<RecurringTransactionRead>))
)]
async fn read_recurring(State(state): State<AppState>) -> ApiResult<Json<Vec<RecurringTransactionRead>>> {
    Ok(Json(services::recurring::list(&state.pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/recurring",
    tag = "recurring",
    request_body = RecurringTransactionCreate,
    responses(
        (status = 201, body = RecurringTransactionRead),
        (status = 400, description = "Invalid category or transfer"),
        (status = 422, description = "Validation Error")
    )
)]
async fn create_recurring(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<RecurringTransactionCreate>,
) -> ApiResult<Response> {
    Ok(created(services::recurring::create(&state.pool, payload).await?))
}

#[utoipa::path(
    patch,
    path = "/api/recurring/{recurring_id}",
    tag = "recurring",
    params(("recurring_id" = i64, Path)),
    request_body = RecurringTransactionUpdate,
    responses(
        (status = 200, body = RecurringTransactionRead),
        (status = 400, description = "Invalid category or transfer"),
        (status = 404, description = "Recurring transaction not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn update_recurring(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<RecurringPath, (), RecurringTransactionUpdate>,
) -> ApiResult<Json<RecurringTransactionRead>> {
    Ok(Json(services::recurring::update(&state.pool, path.recurring_id, body).await?))
}

#[utoipa::path(
    delete,
    path = "/api/recurring/{recurring_id}",
    tag = "recurring",
    params(("recurring_id" = i64, Path)),
    responses(
        (status = 204),
        (status = 404, description = "Recurring transaction not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn delete_recurring(State(state): State<AppState>, ApiPath(path): ApiPath<RecurringPath>) -> ApiResult<Response> {
    services::recurring::delete(&state.pool, path.recurring_id).await?;
    Ok(no_content())
}

/// "Post now": creates a real transaction from the template, dated today.
#[utoipa::path(
    post,
    path = "/api/recurring/{recurring_id}/post",
    tag = "recurring",
    params(("recurring_id" = i64, Path)),
    responses(
        (status = 201, body = RecurringTransactionRead),
        (status = 400, description = "Invalid transfer"),
        (status = 404, description = "Recurring transaction not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn post_recurring(State(state): State<AppState>, ApiPath(path): ApiPath<RecurringPath>) -> ApiResult<Response> {
    Ok(created(services::recurring::post(&state.pool, path.recurring_id).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(read_recurring, create_recurring))
        .routes(routes!(update_recurring, delete_recurring))
        .routes(routes!(post_recurring))
}
