//! `/api/net-worth` — port of the former Python backend's `app/api/routes/net_worth.py`.

use aurum_domain::net_worth::{NetWorthQuery, NetWorthSummary};
use axum::Json;
use axum::extract::State;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::extract::ApiQuery;
use crate::services;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/net-worth/summary",
    tag = "net-worth",
    params(("range" = Option<String>, Query, pattern = r"^(1y|30d|5y|90d|all)$")),
    responses((status = 200, body = NetWorthSummary), (status = 422, description = "Validation Error"))
)]
async fn read_net_worth_summary(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<NetWorthQuery>,
) -> ApiResult<Json<NetWorthSummary>> {
    Ok(Json(services::net_worth::get_net_worth_summary(&state, &query.range).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(read_net_worth_summary))
}
