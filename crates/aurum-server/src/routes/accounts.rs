//! `/api/accounts` — port of `backend/app/api/routes/accounts.py`.

use aurum_domain::accounts::{AccountCreate, AccountListQuery, AccountPath, AccountUpdate, AccountWithBalance};
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::audit::log_destructive;
use crate::error::ApiResult;
use crate::extract::{ApiJson, ApiPath, ApiQuery, Input};
use crate::response::{created, no_content};
use crate::services;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/accounts",
    tag = "accounts",
    params(("include_archived" = Option<bool>, Query)),
    responses((status = 200, body = Vec<AccountWithBalance>), (status = 422, description = "Validation Error"))
)]
async fn read_accounts(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<AccountListQuery>,
) -> ApiResult<Json<Vec<AccountWithBalance>>> {
    Ok(Json(services::accounts::list(&state.pool, query.include_archived).await?))
}

#[utoipa::path(
    post,
    path = "/api/accounts",
    tag = "accounts",
    request_body = AccountCreate,
    responses((status = 201, body = AccountWithBalance), (status = 422, description = "Validation Error"))
)]
async fn create_account(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<AccountCreate>,
) -> ApiResult<Response> {
    Ok(created(services::accounts::create(&state.pool, payload).await?))
}

#[utoipa::path(
    patch,
    path = "/api/accounts/{account_id}",
    tag = "accounts",
    params(("account_id" = i64, Path)),
    request_body = AccountUpdate,
    responses(
        (status = 200, body = AccountWithBalance),
        (status = 404, description = "Account not found"),
        (status = 409, description = "Account currency is locked"),
        (status = 422, description = "Validation Error")
    )
)]
async fn update_account(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<AccountPath, (), AccountUpdate>,
) -> ApiResult<Json<AccountWithBalance>> {
    Ok(Json(services::accounts::update(&state.pool, path.account_id, body).await?))
}

#[utoipa::path(
    delete,
    path = "/api/accounts/{account_id}",
    tag = "accounts",
    params(("account_id" = i64, Path)),
    responses((status = 204), (status = 404, description = "Account not found"), (status = 422, description = "Validation Error"))
)]
async fn delete_account(State(state): State<AppState>, ApiPath(path): ApiPath<AccountPath>) -> ApiResult<Response> {
    // Takes every transaction on the account with it, so it's worth a line in
    // the log even though the UI asks for confirmation first.
    services::accounts::delete(&state.pool, path.account_id).await?;
    log_destructive("account.deleted", &[("account_id", &path.account_id)]);
    Ok(no_content())
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(read_accounts, create_account)).routes(routes!(update_account, delete_account))
}
