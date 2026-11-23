//! `/api/tags` — port of the former Python backend's `app/api/routes/tags.py`.
//!
//! Reference implementation for the porting agents (see PORTING.md): thin
//! handlers that validate input with the FastAPI-compatible extractors, call
//! the service, and pick the status code the Python decorator declared.

use aurum_domain::tags::{TagCreate, TagPath, TagRead};
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::extract::{ApiJson, ApiPath};
use crate::response::{created, no_content};
use crate::services;
use crate::state::AppState;

#[utoipa::path(get, path = "/api/tags", tag = "tags", responses((status = 200, body = Vec<TagRead>)))]
async fn list_tags(State(state): State<AppState>) -> ApiResult<Json<Vec<TagRead>>> {
    Ok(Json(services::tags::list(&state.pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/tags",
    tag = "tags",
    request_body = TagCreate,
    responses((status = 201, body = TagRead), (status = 422, description = "Validation Error"))
)]
async fn create_tag(State(state): State<AppState>, ApiJson(payload): ApiJson<TagCreate>) -> ApiResult<Response> {
    Ok(created(services::tags::create(&state.pool, payload).await?))
}

#[utoipa::path(
    delete,
    path = "/api/tags/{tag_id}",
    tag = "tags",
    params(("tag_id" = i64, Path)),
    responses((status = 204), (status = 404, description = "Tag not found"), (status = 422, description = "Validation Error"))
)]
async fn delete_tag(State(state): State<AppState>, ApiPath(path): ApiPath<TagPath>) -> ApiResult<Response> {
    services::tags::delete(&state.pool, path.tag_id).await?;
    Ok(no_content())
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list_tags, create_tag)).routes(routes!(delete_tag))
}
