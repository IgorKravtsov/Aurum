//! Errors as FastAPI renders them.
//!
//! * `HTTPException(status, detail)` → [`ApiError::Http`] →
//!   `{"detail": <detail>}` with that status (`application/json`).
//! * `RequestValidationError` → [`ApiError::Validation`] → 422
//!   `{"detail": [ {type, loc, msg, input, ctx?}, ... ]}`.
//! * Anything unhandled (a DB error, a bug) → [`ApiError::Internal`] → 500
//!   `Internal Server Error` as `text/plain; charset=utf-8`, which is what
//!   Starlette's ServerErrorMiddleware answers. The cause is logged, never
//!   sent to the client.
//!
//! Handlers return [`ApiResult<T>`]; `?` on a `sqlx::Error` or an
//! `anyhow::Error` turns into a 500, same as an uncaught Python exception.

use aurum_domain::validation::ValidationErrors;
use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

#[derive(Debug)]
pub enum ApiError {
    /// `raise HTTPException(status_code=..., detail=...)`.
    Http { status: StatusCode, detail: Value },
    /// A 422 from request validation.
    Validation(ValidationErrors),
    /// An unexpected failure — 500.
    Internal(anyhow::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    /// `HTTPException(status_code=status, detail="...")`.
    pub fn http(status: StatusCode, detail: impl Into<String>) -> Self {
        Self::Http { status, detail: Value::String(detail.into()) }
    }

    /// `HTTPException` with a structured (non-string) detail.
    pub fn http_json(status: StatusCode, detail: Value) -> Self {
        Self::Http { status, detail }
    }

    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::http(StatusCode::BAD_REQUEST, detail)
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::http(StatusCode::NOT_FOUND, detail)
    }

    pub fn conflict(detail: impl Into<String>) -> Self {
        Self::http(StatusCode::CONFLICT, detail)
    }

    pub fn bad_gateway(detail: impl Into<String>) -> Self {
        Self::http(StatusCode::BAD_GATEWAY, detail)
    }

    pub fn internal(err: impl Into<anyhow::Error>) -> Self {
        Self::Internal(err.into())
    }
}

impl From<ValidationErrors> for ApiError {
    fn from(value: ValidationErrors) -> Self {
        Self::Validation(value)
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(value: sqlx::Error) -> Self {
        Self::Internal(value.into())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        Self::Internal(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Http { status, detail } => (status, Json(json!({ "detail": detail }))).into_response(),
            ApiError::Validation(errors) => {
                (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "detail": errors.0 }))).into_response()
            }
            ApiError::Internal(err) => {
                tracing::error!(error = ?err, "unhandled error while serving request");
                internal_server_error()
            }
        }
    }
}

/// Marks the 500 of an uncaught error. In Starlette it comes from
/// `ServerErrorMiddleware`, *outside* the CORS middleware, so it carries no
/// CORS headers (not even `Vary: Origin`); the CORS layer checks this.
#[derive(Debug, Clone, Copy)]
pub struct Uncaught;

/// Starlette's bare 500.
pub fn internal_server_error() -> Response {
    let mut response = (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    response.extensions_mut().insert(Uncaught);
    response
}
