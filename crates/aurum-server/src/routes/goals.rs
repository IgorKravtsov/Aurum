//! `/api/goals` — port of the former Python backend's `app/api/routes/goals.py`.

use aurum_domain::goals::{GoalContributionCreate, GoalCreate, GoalPath, GoalRead, GoalUpdate};
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::extract::{ApiJson, ApiPath, Input};
use crate::response::{created, no_content};
use crate::services;
use crate::state::AppState;

#[utoipa::path(get, path = "/api/goals", tag = "goals", responses((status = 200, body = Vec<GoalRead>)))]
async fn read_goals(State(state): State<AppState>) -> ApiResult<Json<Vec<GoalRead>>> {
    Ok(Json(services::goals::list(&state.pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/goals",
    tag = "goals",
    request_body = GoalCreate,
    responses((status = 201, body = GoalRead), (status = 422, description = "Validation Error"))
)]
async fn create_goal(State(state): State<AppState>, ApiJson(payload): ApiJson<GoalCreate>) -> ApiResult<Response> {
    Ok(created(services::goals::create(&state.pool, payload).await?))
}

#[utoipa::path(
    patch,
    path = "/api/goals/{goal_id}",
    tag = "goals",
    params(("goal_id" = i64, Path)),
    request_body = GoalUpdate,
    responses(
        (status = 200, body = GoalRead),
        (status = 404, description = "Goal not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn update_goal(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<GoalPath, (), GoalUpdate>,
) -> ApiResult<Json<GoalRead>> {
    Ok(Json(services::goals::update(&state.pool, path.goal_id, body).await?))
}

#[utoipa::path(
    delete,
    path = "/api/goals/{goal_id}",
    tag = "goals",
    params(("goal_id" = i64, Path)),
    responses(
        (status = 204),
        (status = 404, description = "Goal not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn delete_goal(State(state): State<AppState>, ApiPath(path): ApiPath<GoalPath>) -> ApiResult<Response> {
    services::goals::delete(&state.pool, path.goal_id).await?;
    Ok(no_content())
}

#[utoipa::path(
    post,
    path = "/api/goals/{goal_id}/contributions",
    tag = "goals",
    params(("goal_id" = i64, Path)),
    request_body = GoalContributionCreate,
    responses(
        (status = 201, body = GoalRead),
        (status = 404, description = "Goal not found"),
        (status = 422, description = "Validation Error")
    )
)]
async fn add_contribution(
    State(state): State<AppState>,
    Input { path, body, .. }: Input<GoalPath, (), GoalContributionCreate>,
) -> ApiResult<Response> {
    Ok(created(services::goals::add_contribution(&state.pool, path.goal_id, body).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(read_goals, create_goal))
        .routes(routes!(update_goal, delete_goal))
        .routes(routes!(add_contribution))
}
