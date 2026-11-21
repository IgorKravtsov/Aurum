//! Starlette's routing edge cases: JSON 404/405 bodies and `redirect_slashes`.
//!
//! * Unknown path → 404 `{"detail":"Not Found"}`.
//! * Known path, wrong method → 405 `{"detail":"Method Not Allowed"}` with
//!   an `Allow` header.
//! * `HEAD` on an API route → 405 as well: FastAPI's `@router.get` registers
//!   GET only (unlike axum's `get`, which also answers HEAD). Only the docs
//!   pages, plain Starlette routes, answer HEAD.
//! * Unknown path whose trailing-slash twin exists (`/api/tags/` ↔
//!   `/api/tags`) → 307 to the twin, absolute URL built from the `Host`
//!   header, query string kept, empty body — for any method, exactly like
//!   Starlette's `Router.redirect_slashes`.
//!
//! The redirect check re-dispatches a *probe* request (an unused HTTP
//! method) for the twin path through the router: axum answers 405 when the
//! path exists and 404 when it doesn't, and no handler ever runs.

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tower::ServiceExt;

/// Marks a 404 produced by the router fallback (as opposed to a handler's
/// own 404 such as "Tag not found").
#[derive(Debug, Clone, Copy)]
pub struct NoRoute;

const PROBE_METHOD: &[u8] = b"AURUM-ROUTE-PROBE";

/// Router fallback: no route matches the path.
pub async fn not_found() -> Response {
    let mut response = (StatusCode::NOT_FOUND, axum::Json(json!({ "detail": "Not Found" }))).into_response();
    response.extensions_mut().insert(NoRoute);
    response
}

/// Method-not-allowed fallback. axum adds the `Allow` header itself.
pub async fn method_not_allowed() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, axum::Json(json!({ "detail": "Method Not Allowed" }))).into_response()
}

/// Answers whether `path` matches any route (any method), without running
/// a handler. `Some(allow)` carries the probed `Allow` header.
async fn probe(router: &Router, path: &str) -> Option<Option<HeaderValue>> {
    let uri = path.parse::<Uri>().ok()?;
    let method = Method::from_bytes(PROBE_METHOD).ok()?;
    let request = Request::builder().method(method).uri(uri).body(Body::empty()).ok()?;
    let Ok(response) = router.clone().oneshot(request).await;
    if response.extensions().get::<NoRoute>().is_some() {
        return None;
    }
    Some(response.headers().get(header::ALLOW).cloned())
}

/// Middleware implementing the Starlette behaviours above; `router` is the
/// fully built inner router (state applied) used for probing, and
/// `head_paths` are the routes that legitimately answer HEAD.
pub async fn starlette_routing(
    router: Router,
    head_paths: &'static [&'static str],
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.method().as_str().as_bytes() == PROBE_METHOD {
        return next.run(request).await;
    }
    let path = request.uri().path().to_string();

    if request.method() == Method::HEAD
        && !head_paths.contains(&path.as_str())
        && let Some(allow) = probe(&router, &path).await
    {
        let mut response = method_not_allowed().await;
        // axum lists e.g. "GET,HEAD,POST"; Starlette "GET, POST" (no HEAD).
        if let Some(allow) = allow.as_ref().and_then(|a| a.to_str().ok()) {
            let methods: Vec<&str> = allow.split(',').map(str::trim).filter(|m| *m != "HEAD").collect();
            if let Ok(value) = HeaderValue::from_str(&methods.join(", ")) {
                response.headers_mut().insert(header::ALLOW, value);
            }
        }
        return response;
    }

    let query = request.uri().query().map(String::from);
    let host = request.headers().get(header::HOST).and_then(|h| h.to_str().ok()).map(String::from);
    let response = next.run(request).await;
    if response.extensions().get::<NoRoute>().is_none() || path == "/" {
        return response;
    }
    let twin = match path.strip_suffix('/') {
        Some(stripped) => stripped.to_string(),
        None => format!("{path}/"),
    };
    if probe(&router, &twin).await.is_none() {
        return response;
    }
    // Starlette builds the URL from the ASGI scope: scheme (plain http
    // behind the bundled nginx), Host header, new path, original query.
    let host = host.unwrap_or_else(|| "localhost".into());
    let location = match query {
        Some(q) => format!("http://{host}{twin}?{q}"),
        None => format!("http://{host}{twin}"),
    };
    let mut redirect = StatusCode::TEMPORARY_REDIRECT.into_response();
    if let Ok(value) = HeaderValue::from_str(&location) {
        redirect.headers_mut().insert(header::LOCATION, value);
    }
    redirect.headers_mut().insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
    redirect
}
