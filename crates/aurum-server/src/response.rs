//! Success responses shaped like FastAPI's.
//!
//! * `200` → return `Json(dto)` (axum sets `content-type: application/json`,
//!   like FastAPI's JSONResponse; serde_json's compact output and
//!   non-ASCII-as-UTF-8 match `json.dumps(..., ensure_ascii=False,
//!   separators=(",", ":"))`).
//! * `status_code=201` → [`created`].
//! * `status_code=204` → [`no_content`]: FastAPI still sends
//!   `content-type: application/json` on its empty 204, so we do too.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

pub fn created<T: Serialize>(body: T) -> Response {
    (StatusCode::CREATED, Json(body)).into_response()
}

pub fn no_content() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}
