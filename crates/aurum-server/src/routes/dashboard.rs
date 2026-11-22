//! `/api/dashboard` — port of `backend/app/api/routes/dashboard.py`.

use aurum_domain::dashboard::{DashboardQuery, DashboardSummary};
use axum::Json;
use axum::extract::State;
use chrono::{Datelike, Local};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::extract::ApiQuery;
use crate::services;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/dashboard/summary",
    tag = "dashboard",
    params(("year" = Option<i64>, Query, minimum = 2000, maximum = 2100), ("month" = Option<i64>, Query, minimum = 1, maximum = 12)),
    responses((status = 200, body = DashboardSummary), (status = 422, description = "Validation Error"))
)]
async fn read_dashboard_summary(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<DashboardQuery>,
) -> ApiResult<Json<DashboardSummary>> {
    // `default_factory=lambda: date.today().year/.month`.
    let today = Local::now().date_naive();
    let year = query.year.map_or(today.year(), |y| y as i32);
    let month = query.month.map_or(today.month(), |m| m as u32);
    Ok(Json(services::dashboard::get_dashboard_summary(&state, year, month).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(read_dashboard_summary))
}
