//! `/api/crypto` — port of the former Python backend's `app/api/routes/crypto.py`.
//!
//! Handlers are annotated with `#[utoipa::path(...)]` (full path, e.g.
//! `path = "/api/crypto/holdings"`) and registered below with
//! `.routes(routes!(...))`, one `routes!` per path; see PORTING.md.

use aurum_domain::crypto::{
    CryptoHistoryQuery, CryptoHistoryResponse, CryptoHoldingCreate, CryptoHoldingRead, CryptoHoldingUpdate,
    CryptoPerformanceResponse, CryptoPortfolioCreate, CryptoPortfolioRead, CryptoPortfolioUpdate, CryptoSearchQuery,
    CryptoSearchResult, CryptoSyncResult, CryptoTransactionCreate, CryptoTransactionPath, CryptoTransactionRead,
    CryptoTransactionUpdate, HoldingPath, PortfolioFilterQuery, PortfolioListQuery, PortfolioPath,
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
    path = "/api/crypto/portfolios",
    tag = "crypto",
    params(("include_archived" = Option<bool>, Query)),
    responses((status = 200, body = Vec<CryptoPortfolioRead>), (status = 422, description = "Validation Error"))
)]
async fn read_portfolios(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<PortfolioListQuery>,
) -> ApiResult<Json<Vec<CryptoPortfolioRead>>> {
    Ok(Json(services::crypto::list_portfolios(&state.pool, query.include_archived).await?))
}

#[utoipa::path(
    post,
    path = "/api/crypto/portfolios",
    tag = "crypto",
    request_body = CryptoPortfolioCreate,
    responses((status = 201, body = CryptoPortfolioRead), (status = 422, description = "Validation Error"))
)]
async fn create_portfolio(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<CryptoPortfolioCreate>,
) -> ApiResult<Response> {
    Ok(created(services::crypto::create_portfolio(&state.pool, payload).await?))
}

#[utoipa::path(
    patch,
    path = "/api/crypto/portfolios/{portfolio_id}",
    tag = "crypto",
    params(("portfolio_id" = i64, Path)),
    request_body = CryptoPortfolioUpdate,
    responses(
        (status = 200, body = CryptoPortfolioRead),
        (status = 404, description = "Crypto portfolio not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn update_portfolio(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<PortfolioPath, (), CryptoPortfolioUpdate>,
) -> ApiResult<Json<CryptoPortfolioRead>> {
    Ok(Json(services::crypto::update_portfolio(&state.pool, path.portfolio_id, body).await?))
}

#[utoipa::path(
    delete,
    path = "/api/crypto/portfolios/{portfolio_id}",
    tag = "crypto",
    params(("portfolio_id" = i64, Path)),
    responses(
        (status = 204),
        (status = 400, description = "Portfolio still has coins"),
        (status = 404, description = "Crypto portfolio not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn delete_portfolio(State(state): State<AppState>, ApiPath(path): ApiPath<PortfolioPath>) -> ApiResult<Response> {
    services::crypto::delete_portfolio(&state.pool, path.portfolio_id).await?;
    Ok(no_content())
}

/// Opening the Crypto tab lands here — this is also where the lazy
/// once-a-day auto-refresh happens: prices are only re-fetched from
/// CoinGecko if 24h have passed since the last sync.
#[utoipa::path(
    get,
    path = "/api/crypto/holdings",
    tag = "crypto",
    params(("portfolio_id" = Option<i64>, Query)),
    responses(
        (status = 200, body = CryptoSyncResult),
        (status = 400, description = "CoinGecko API key not configured"),
        (status = 422, description = "Validation Error")
    )
)]
async fn read_holdings(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<PortfolioFilterQuery>,
) -> ApiResult<Json<CryptoSyncResult>> {
    Ok(Json(services::crypto::refresh_prices(&state, false, query.portfolio_id).await?))
}

#[utoipa::path(
    post,
    path = "/api/crypto/holdings",
    tag = "crypto",
    request_body = CryptoHoldingCreate,
    responses(
        (status = 201, body = CryptoHoldingRead),
        (status = 400, description = "Unknown portfolio or CoinGecko API key not configured"),
        (status = 422, description = "Validation Error")
    )
)]
async fn create_holding(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<CryptoHoldingCreate>,
) -> ApiResult<Response> {
    Ok(created(services::crypto::create_holding(&state, payload).await?))
}

/// The "Refresh prices" button — always hits CoinGecko regardless of the
/// once-a-day window.
#[utoipa::path(
    post,
    path = "/api/crypto/refresh",
    tag = "crypto",
    params(("portfolio_id" = Option<i64>, Query)),
    responses(
        (status = 200, body = CryptoSyncResult),
        (status = 400, description = "CoinGecko API key not configured"),
        (status = 422, description = "Validation Error")
    )
)]
async fn refresh_holdings(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<PortfolioFilterQuery>,
) -> ApiResult<Json<CryptoSyncResult>> {
    Ok(Json(services::crypto::refresh_prices(&state, true, query.portfolio_id).await?))
}

/// Updates holding-only metadata (currently just `network`) — quantity/
/// price/date go through the /transactions routes instead.
#[utoipa::path(
    patch,
    path = "/api/crypto/holdings/{asset_id}",
    tag = "crypto",
    params(("asset_id" = i64, Path)),
    request_body = CryptoHoldingUpdate,
    responses(
        (status = 200, body = CryptoHoldingRead),
        (status = 404, description = "Crypto holding not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn update_holding(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<HoldingPath, (), CryptoHoldingUpdate>,
) -> ApiResult<Json<CryptoHoldingRead>> {
    Ok(Json(services::crypto::update_holding(&state.pool, path.asset_id, body).await?))
}

/// Buy more of, or sell some of, a coin already being tracked.
#[utoipa::path(
    post,
    path = "/api/crypto/holdings/{asset_id}/transactions",
    tag = "crypto",
    params(("asset_id" = i64, Path)),
    request_body = CryptoTransactionCreate,
    responses(
        (status = 201, body = CryptoHoldingRead),
        (status = 400, description = "Cannot sell more than you currently hold"),
        (status = 404, description = "Crypto holding not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn add_transaction(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<HoldingPath, (), CryptoTransactionCreate>,
) -> ApiResult<Response> {
    Ok(created(services::crypto::add_transaction(&state.pool, path.asset_id, body).await?))
}

#[utoipa::path(
    get,
    path = "/api/crypto/holdings/{asset_id}/transactions",
    tag = "crypto",
    params(("asset_id" = i64, Path)),
    responses(
        (status = 200, body = Vec<CryptoTransactionRead>),
        (status = 404, description = "Crypto holding not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn list_transactions(
    State(state): State<AppState>,
    ApiPath(path): ApiPath<HoldingPath>,
) -> ApiResult<Json<Vec<CryptoTransactionRead>>> {
    Ok(Json(services::crypto::list_transactions(&state.pool, path.asset_id).await?))
}

#[utoipa::path(
    patch,
    path = "/api/crypto/transactions/{transaction_id}",
    tag = "crypto",
    params(("transaction_id" = i64, Path)),
    request_body = CryptoTransactionUpdate,
    responses(
        (status = 200, body = CryptoHoldingRead),
        (status = 400, description = "Cannot sell more than you currently hold"),
        (status = 404, description = "Crypto transaction not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn update_transaction(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<CryptoTransactionPath, (), CryptoTransactionUpdate>,
) -> ApiResult<Json<CryptoHoldingRead>> {
    Ok(Json(services::crypto::update_transaction(&state.pool, path.transaction_id, body).await?))
}

#[utoipa::path(
    delete,
    path = "/api/crypto/transactions/{transaction_id}",
    tag = "crypto",
    params(("transaction_id" = i64, Path)),
    responses(
        (status = 204),
        (status = 404, description = "Crypto transaction not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn delete_transaction(
    State(state): State<AppState>,
    ApiPath(path): ApiPath<CryptoTransactionPath>,
) -> ApiResult<Response> {
    services::crypto::delete_transaction(&state.pool, path.transaction_id).await?;
    Ok(no_content())
}

#[utoipa::path(
    get,
    path = "/api/crypto/search",
    tag = "crypto",
    params(("q" = String, Query)),
    responses(
        (status = 200, body = Vec<CryptoSearchResult>),
        (status = 400, description = "CoinGecko API key not configured"),
        (status = 422, description = "Validation Error")
    )
)]
async fn search_coins(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<CryptoSearchQuery>,
) -> ApiResult<Json<Vec<CryptoSearchResult>>> {
    Ok(Json(services::crypto::search_coins(&state, &query.q).await?))
}

#[utoipa::path(
    get,
    path = "/api/crypto/history",
    tag = "crypto",
    params(("range" = Option<String>, Query), ("portfolio_id" = Option<i64>, Query)),
    responses((status = 200, body = CryptoHistoryResponse), (status = 422, description = "Validation Error"))
)]
async fn read_crypto_history(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<CryptoHistoryQuery>,
) -> ApiResult<Json<CryptoHistoryResponse>> {
    Ok(Json(services::crypto::history(&state.pool, &query.range, query.portfolio_id).await?))
}

/// Backs the Best/Worst Performer stat only while the 90d range is
/// selected — a separate on-demand call instead of a cached field like
/// 7d/30d/1y.
#[utoipa::path(
    get,
    path = "/api/crypto/performance/90d",
    tag = "crypto",
    params(("portfolio_id" = Option<i64>, Query)),
    responses(
        (status = 200, body = CryptoPerformanceResponse),
        (status = 400, description = "CoinGecko API key not configured"),
        (status = 422, description = "Validation Error")
    )
)]
async fn read_90d_performance(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<PortfolioFilterQuery>,
) -> ApiResult<Json<CryptoPerformanceResponse>> {
    Ok(Json(services::crypto::performance_90d(&state, query.portfolio_id).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(read_portfolios, create_portfolio))
        .routes(routes!(update_portfolio, delete_portfolio))
        .routes(routes!(read_holdings, create_holding))
        .routes(routes!(refresh_holdings))
        .routes(routes!(update_holding))
        .routes(routes!(add_transaction, list_transactions))
        .routes(routes!(update_transaction, delete_transaction))
        .routes(routes!(search_coins))
        .routes(routes!(read_crypto_history))
        .routes(routes!(read_90d_performance))
}
