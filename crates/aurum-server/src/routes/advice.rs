//! `/api/advice` — port of the former Python backend's `app/api/routes/advice.py`.
//!
//! Handlers are annotated with `#[utoipa::path(...)]` (full path) and
//! registered below with `.routes(routes!(...))`; see `routes/tags.rs` and PORTING.md.

use aurum_domain::advice::AdviceResponse;
use axum::Json;
use axum::extract::State;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::services;
use crate::state::AppState;

#[utoipa::path(get, path = "/api/advice", tag = "advice", responses((status = 200, body = AdviceResponse)))]
async fn read_advice(State(state): State<AppState>) -> ApiResult<Json<AdviceResponse>> {
    Ok(Json(services::advice::get_advice(&state).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(read_advice))
}
