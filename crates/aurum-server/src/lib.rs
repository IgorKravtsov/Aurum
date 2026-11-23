//! `aurum-server` — the Aurum REST API in Rust, a drop-in replacement for
//! the FastAPI backend: same paths, same JSON (down to Decimal/float/date
//! formatting and FastAPI's error bodies), same Postgres schema, same
//! `AURUM_*` settings. See PORTING.md for the conventions.
//!
//! Layout: `routes` (HTTP handlers, one module per route group) →
//! `services` (business logic, raises `ApiError`) → `repo` (SQL). Contract
//! types live in the `aurum-domain` crate.

pub mod audit;
pub mod config;
pub mod db;
pub mod error;
pub mod extract;
pub mod http;
mod pyjson;
pub mod repo;
pub mod response;
pub mod routes;
pub mod services;
pub mod state;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use utoipa::openapi::InfoBuilder;

use crate::state::AppState;

/// Builds the complete application: API routes, docs (when enabled), FastAPI
/// style 404/405/redirect handling and CORS (when configured).
pub fn build_app(state: AppState) -> Router {
    let settings = state.settings.clone();
    let (api, mut openapi) = routes::api_router().split_for_parts();
    openapi.info = InfoBuilder::new().title("Aurum API").version(aurum_domain::APP_VERSION).build();

    let mut router: Router<AppState> = api;
    if settings.enable_docs {
        router = router.merge(http::docs::router(openapi));
    }
    let router: Router = router
        .fallback(http::routing::not_found)
        .method_not_allowed_fallback(http::routing::method_not_allowed)
        .with_state(state);

    // Everything below wraps the *whole* router (an outer router whose only
    // route is the inner one as fallback), like Starlette middleware does —
    // `Router::layer` alone would run per matched route.
    let probe = router.clone();
    let head_paths: &'static [&'static str] = if settings.enable_docs {
        &[http::docs::OPENAPI_URL, http::docs::DOCS_URL, http::docs::REDOC_URL]
    } else {
        &[]
    };
    let mut app = Router::new()
        .fallback_service(router)
        .layer(axum::middleware::from_fn(move |request, next| {
            http::routing::starlette_routing(probe.clone(), head_paths, request, next)
        }))
        // FastAPI imposes no body limit (nginx caps uploads at 64m in front).
        .layer(DefaultBodyLimit::disable());
    if let Some(cors) = http::cors::Cors::from_origins(settings.cors_origins_list()) {
        app = app.layer(axum::middleware::from_fn_with_state(cors, http::cors::middleware));
    }
    app
}
