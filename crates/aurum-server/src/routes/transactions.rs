//! `/api/transactions` — port of the former Python backend's `app/api/routes/transactions.py`.
//!
//! Route-order caveat (see PORTING.md): Starlette tries `/years` and
//! `/bulk` before `/{transaction_id}`, so e.g. `PATCH /api/transactions/years`
//! is a 422 (int parsing) under Python but a 405 here.

use aurum_domain::transactions::{
    TransactionBulkCreate, TransactionBulkCreateResult, TransactionCreate, TransactionListQuery, TransactionPage,
    TransactionPath, TransactionRead, TransactionUpdate,
};
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
    path = "/api/transactions",
    tag = "transactions",
    params(
        ("year" = Option<i64>, Query), ("month" = Option<i64>, Query),
        ("start_date" = Option<chrono::NaiveDate>, Query), ("end_date" = Option<chrono::NaiveDate>, Query),
        ("account_id" = Option<i64>, Query), ("category_id" = Option<i64>, Query), ("tag_id" = Option<i64>, Query),
        ("type" = Option<aurum_domain::enums::TransactionType>, Query), ("search" = Option<String>, Query),
        ("sort" = Option<String>, Query), ("page" = Option<i64>, Query), ("page_size" = Option<i64>, Query)
    ),
    responses((status = 200, body = TransactionPage), (status = 422, description = "Validation Error"))
)]
async fn list_transactions(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<TransactionListQuery>,
) -> ApiResult<Json<TransactionPage>> {
    Ok(Json(services::transactions::list(&state.pool, query).await?))
}

/// Full range of years to offer in the year picker — from the earliest
/// transaction through the current year.
#[utoipa::path(get, path = "/api/transactions/years", tag = "transactions", responses((status = 200, body = Vec<i32>)))]
async fn list_transaction_years(State(state): State<AppState>) -> ApiResult<Json<Vec<i32>>> {
    Ok(Json(services::transactions::years(&state.pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/transactions",
    tag = "transactions",
    request_body = TransactionCreate,
    responses(
        (status = 201, body = TransactionRead),
        (status = 400, description = "Invalid category, tags, splits or transfer"),
        (status = 422, description = "Validation Error")
    )
)]
async fn create_transaction(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<TransactionCreate>,
) -> ApiResult<Response> {
    Ok(created(services::transactions::create(&state.pool, payload).await?))
}

/// CSV import lands here — all-or-nothing.
#[utoipa::path(
    post,
    path = "/api/transactions/bulk",
    tag = "transactions",
    request_body = TransactionBulkCreate,
    responses(
        (status = 201, body = TransactionBulkCreateResult),
        (status = 400, description = "Invalid row"),
        (status = 422, description = "Validation Error")
    )
)]
async fn bulk_create_transactions(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<TransactionBulkCreate>,
) -> ApiResult<Response> {
    Ok(created(services::transactions::bulk_create(&state.pool, payload).await?))
}

#[utoipa::path(
    patch,
    path = "/api/transactions/{transaction_id}",
    tag = "transactions",
    params(("transaction_id" = i64, Path)),
    request_body = TransactionUpdate,
    responses(
        (status = 200, body = TransactionRead),
        (status = 400, description = "Invalid category, tags, splits or transfer"),
        (status = 404, description = "Transaction not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn update_transaction(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<TransactionPath, (), TransactionUpdate>,
) -> ApiResult<Json<TransactionRead>> {
    Ok(Json(services::transactions::update(&state.pool, path.transaction_id, body).await?))
}

#[utoipa::path(
    delete,
    path = "/api/transactions/{transaction_id}",
    tag = "transactions",
    params(("transaction_id" = i64, Path)),
    responses(
        (status = 204),
        (status = 404, description = "Transaction not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn delete_transaction(
    State(state): State<AppState>,
    ApiPath(path): ApiPath<TransactionPath>,
) -> ApiResult<Response> {
    services::transactions::delete(&state.pool, path.transaction_id).await?;
    Ok(no_content())
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_transactions, create_transaction))
        .routes(routes!(list_transaction_years))
        .routes(routes!(bulk_create_transactions))
        .routes(routes!(update_transaction, delete_transaction))
}
