//! A line-for-line port of Starlette's `CORSMiddleware`, configured the way
//! `backend/app/main.py` configures it.
//!
//! CORS stays off unless someone deliberately opens it, and credentials are
//! only granted to a pinned list. Starlette answers a credentialed "*" by
//! echoing back whatever Origin asked instead of a literal "*" — so the two
//! together turn any page the user happens to have open into an authenticated
//! client of their instance, which is exactly what the README's "fine if it's
//! only reachable from localhost" advice assumes can't happen. Hence
//! `allow_credentials = "*" not in origins`.
//!
//! tower-http's `CorsLayer` differs in many small ways (which headers, when
//! `Vary` is added, the 400 on a disallowed preflight), so it isn't used.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Starlette's ALL_METHODS (what `allow_methods=["*"]` expands to).
const ALL_METHODS: &str = "DELETE, GET, HEAD, OPTIONS, PATCH, POST, PUT, QUERY";
const ALL_METHOD_LIST: [&str; 8] = ["DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT", "QUERY"];
const PREFLIGHT_VARY: &str =
    "Origin, Access-Control-Request-Method, Access-Control-Request-Headers, Access-Control-Request-Private-Network";
const MAX_AGE: &str = "600";

#[derive(Debug, Clone)]
pub struct Cors {
    origins: Vec<String>,
    allow_all_origins: bool,
    allow_credentials: bool,
}

impl Cors {
    /// `None` when the origin list is empty — main.py doesn't install the
    /// middleware at all then (so not even `Vary: Origin` is added).
    pub fn from_origins(origins: Vec<String>) -> Option<Arc<Self>> {
        if origins.is_empty() {
            return None;
        }
        let allow_all_origins = origins.iter().any(|o| o == "*");
        Some(Arc::new(Self { allow_credentials: !allow_all_origins, allow_all_origins, origins }))
    }

    fn is_allowed_origin(&self, origin: &str) -> bool {
        self.allow_all_origins || self.origins.iter().any(|o| o == origin)
    }

    /// `preflight_explicit_allow_origin` in Starlette.
    fn explicit_preflight_origin(&self) -> bool {
        !self.allow_all_origins || self.allow_credentials
    }

    fn preflight(&self, headers: &HeaderMap) -> Response {
        let origin = header_str(headers, header::ORIGIN).unwrap_or_default();
        let method = header_str(headers, header::ACCESS_CONTROL_REQUEST_METHOD).unwrap_or_default();
        let requested_headers = header_str(headers, header::ACCESS_CONTROL_REQUEST_HEADERS);
        let private_network = headers.get("access-control-request-private-network");

        let mut out = HeaderMap::new();
        set(&mut out, header::VARY, PREFLIGHT_VARY);
        if !self.explicit_preflight_origin() {
            set(&mut out, header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
        }
        set(&mut out, header::ACCESS_CONTROL_ALLOW_METHODS, ALL_METHODS);
        set(&mut out, header::ACCESS_CONTROL_MAX_AGE, MAX_AGE);
        if self.allow_credentials {
            set(&mut out, header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true");
        }

        let mut failures: Vec<&str> = Vec::new();
        if self.is_allowed_origin(&origin) {
            if self.explicit_preflight_origin() {
                set(&mut out, header::ACCESS_CONTROL_ALLOW_ORIGIN, &origin);
            }
        } else {
            failures.push("origin");
        }
        if !ALL_METHOD_LIST.contains(&method.as_str()) {
            failures.push("method");
        }
        // allow_headers=["*"]: mirror back whatever was requested.
        if let Some(requested) = requested_headers {
            set(&mut out, header::ACCESS_CONTROL_ALLOW_HEADERS, &requested);
        }
        if private_network.is_some() {
            // allow_private_network is False in main.py.
            failures.push("private-network");
        }

        let (status, text) = if failures.is_empty() {
            (StatusCode::OK, "OK".to_string())
        } else {
            (StatusCode::BAD_REQUEST, format!("Disallowed CORS {}", failures.join(", ")))
        };
        let mut response = (status, text).into_response();
        response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
        response.headers_mut().extend(out);
        response
    }

    fn decorate(&self, origin: Option<&str>, headers: &mut HeaderMap) {
        if origin.is_some() {
            if self.allow_all_origins {
                set(headers, header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
            }
            if self.allow_credentials {
                set(headers, header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true");
            }
        }
        match origin {
            Some(origin) if self.allow_all_origins && self.allow_credentials => explicit_origin(headers, origin),
            Some(origin) if !self.allow_all_origins && self.is_allowed_origin(origin) => {
                explicit_origin(headers, origin)
            }
            _ => append_vary_origin(headers),
        }
    }
}

fn explicit_origin(headers: &mut HeaderMap, origin: &str) {
    set(headers, header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    append_vary_origin(headers);
}

/// `headers["Vary"] = ", ".join([*headers.getlist("Vary"), "Origin"])`
fn append_vary_origin(headers: &mut HeaderMap) {
    let mut values: Vec<String> =
        headers.get_all(header::VARY).iter().filter_map(|v| v.to_str().ok().map(String::from)).collect();
    values.push("Origin".into());
    set(headers, header::VARY, &values.join(", "));
}

fn set(headers: &mut HeaderMap, name: HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

fn header_str(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers.get(name).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
}

/// Middleware entry point (`axum::middleware::from_fn_with_state`).
pub async fn middleware(State(cors): State<Arc<Cors>>, request: Request<Body>, next: Next) -> Response {
    let origin = header_str(request.headers(), header::ORIGIN);
    if origin.is_some()
        && request.method() == Method::OPTIONS
        && request.headers().contains_key(header::ACCESS_CONTROL_REQUEST_METHOD)
    {
        return cors.preflight(request.headers());
    }
    let mut response = next.run(request).await;
    if response.extensions().get::<crate::error::Uncaught>().is_some() {
        return response;
    }
    cors.decorate(origin.as_deref(), response.headers_mut());
    response
}
