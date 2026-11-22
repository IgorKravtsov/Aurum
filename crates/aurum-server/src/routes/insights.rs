//! `/api/insights` — port of `backend/app/api/routes/insights.py`.
//!
//! Handlers are annotated with `#[utoipa::path(...)]` (full path) and
//! registered below with `.routes(routes!(...))`; see `routes/tags.rs` and PORTING.md.

use aurum_domain::insights::AlertsResponse;
use axum::Json;
use axum::extract::State;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::services;
use crate::state::AppState;

#[utoipa::path(get, path = "/api/insights/alerts", tag = "insights", responses((status = 200, body = AlertsResponse)))]
async fn read_financial_alerts(State(state): State<AppState>) -> ApiResult<Json<AlertsResponse>> {
    Ok(Json(services::insights::get_financial_alerts(&state).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(read_financial_alerts))
}
