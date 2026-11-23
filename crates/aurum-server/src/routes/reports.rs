//! `/api/reports` — port of the former Python backend's `app/api/routes/reports.py`.

use aurum_domain::reports::{
    CategoryRankingQuery, CategoryRankingReport, CategorySpendingQuery, CategorySpendingReport,
};
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
    path = "/api/reports/category-spending",
    tag = "reports",
    params(
        ("category_id" = i64, Query),
        ("start_date" = Option<chrono::NaiveDate>, Query),
        ("end_date" = Option<chrono::NaiveDate>, Query)
    ),
    responses(
        (status = 200, body = CategorySpendingReport),
        (status = 404, description = "Category not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn read_category_spending_report(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<CategorySpendingQuery>,
) -> ApiResult<Json<CategorySpendingReport>> {
    Ok(Json(services::reports::get_category_spending_report(&state, q.category_id, q.start_date, q.end_date).await?))
}

#[utoipa::path(
    get,
    path = "/api/reports/category-ranking",
    tag = "reports",
    params(
        ("kind" = Option<aurum_domain::enums::CategoryKind>, Query),
        ("start_date" = Option<chrono::NaiveDate>, Query),
        ("end_date" = Option<chrono::NaiveDate>, Query)
    ),
    responses((status = 200, body = CategoryRankingReport), (status = 422, description = "Validation Error"))
)]
async fn read_category_ranking_report(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<CategoryRankingQuery>,
) -> ApiResult<Json<CategoryRankingReport>> {
    Ok(Json(services::reports::get_category_ranking_report(&state, q.kind, q.start_date, q.end_date).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(read_category_spending_report)).routes(routes!(read_category_ranking_report))
}
