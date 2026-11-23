//! `/api/budgets` — port of the former Python backend's `app/api/routes/budgets.py`.
//!
//! Route-order caveat (see PORTING.md): Starlette tries `/status` before
//! `/{budget_id}`, so e.g. `PATCH /api/budgets/status` is a 422 (int
//! parsing) under Python but a 405 here.

use aurum_domain::budgets::{
    BudgetCreate, BudgetPath, BudgetRead, BudgetStatusQuery, BudgetStatusResponse, BudgetUpdate,
};
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use chrono::{Datelike, Local};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::extract::{ApiJson, ApiPath, ApiQuery, Input};
use crate::response::{created, no_content};
use crate::services;
use crate::state::AppState;

#[utoipa::path(get, path = "/api/budgets", tag = "budgets", responses((status = 200, body = Vec<BudgetRead>)))]
async fn read_budgets(State(state): State<AppState>) -> ApiResult<Json<Vec<BudgetRead>>> {
    Ok(Json(services::budgets::list(&state.pool).await?))
}

#[utoipa::path(
    get,
    path = "/api/budgets/status",
    tag = "budgets",
    params(("year" = Option<i64>, Query), ("month" = Option<i64>, Query)),
    responses((status = 200, body = BudgetStatusResponse), (status = 422, description = "Validation Error"))
)]
async fn read_budget_status(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<BudgetStatusQuery>,
) -> ApiResult<Json<BudgetStatusResponse>> {
    // Query(default_factory=lambda: date.today().year / .month); given
    // values are already range-checked (2000..=2100, 1..=12).
    let today = Local::now().date_naive();
    let year = query.year.map_or(today.year(), |y| y as i32);
    let month = query.month.map_or(today.month(), |m| m as u32);
    Ok(Json(services::budgets::get_budget_status(&state, year, month).await?))
}

#[utoipa::path(
    post,
    path = "/api/budgets",
    tag = "budgets",
    request_body = BudgetCreate,
    responses(
        (status = 201, body = BudgetRead),
        (status = 400, description = "Invalid category or duplicate budget"),
        (status = 422, description = "Validation Error")
    )
)]
async fn create_budget(State(state): State<AppState>, ApiJson(payload): ApiJson<BudgetCreate>) -> ApiResult<Response> {
    Ok(created(services::budgets::create(&state.pool, payload).await?))
}

#[utoipa::path(
    patch,
    path = "/api/budgets/{budget_id}",
    tag = "budgets",
    params(("budget_id" = i64, Path)),
    request_body = BudgetUpdate,
    responses(
        (status = 200, body = BudgetRead),
        (status = 404, description = "Budget not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn update_budget(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<BudgetPath, (), BudgetUpdate>,
) -> ApiResult<Json<BudgetRead>> {
    Ok(Json(services::budgets::update(&state.pool, path.budget_id, body).await?))
}

#[utoipa::path(
    delete,
    path = "/api/budgets/{budget_id}",
    tag = "budgets",
    params(("budget_id" = i64, Path)),
    responses(
        (status = 204),
        (status = 404, description = "Budget not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn delete_budget(State(state): State<AppState>, ApiPath(path): ApiPath<BudgetPath>) -> ApiResult<Response> {
    services::budgets::delete(&state.pool, path.budget_id).await?;
    Ok(no_content())
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(read_budgets, create_budget))
        .routes(routes!(read_budget_status))
        .routes(routes!(update_budget, delete_budget))
}
