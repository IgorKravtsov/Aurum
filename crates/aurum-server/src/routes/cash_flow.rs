//! `/api/cash-flow` — port of `backend/app/api/routes/cash_flow.py`.

use aurum_domain::cash_flow::{CashFlowQuery, CashFlowResponse};
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
    path = "/api/cash-flow",
    tag = "cash-flow",
    params(("start_date" = Option<chrono::NaiveDate>, Query), ("end_date" = Option<chrono::NaiveDate>, Query)),
    responses((status = 200, body = CashFlowResponse), (status = 422, description = "Validation Error"))
)]
async fn read_cash_flow(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<CashFlowQuery>,
) -> ApiResult<Json<CashFlowResponse>> {
    Ok(Json(services::cash_flow::get_cash_flow(&state, query.start_date, query.end_date).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(read_cash_flow))
}
