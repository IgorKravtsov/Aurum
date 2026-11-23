//! `GET /api/health`.

use aurum_domain::health::Health;
use axum::Json;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// Status only. This is the one route nginx serves without auth (Docker's
/// HEALTHCHECK and uptime monitors need it — see frontend/nginx.conf), so
/// anything returned here is world-readable. The running version used to
/// ride along, which told an unauthenticated caller exactly which release
/// they were looking at, and therefore which known issues it still has;
/// it now comes back from GET /api/settings, which is behind auth.
#[utoipa::path(get, path = "/api/health", responses((status = 200, body = Health)))]
async fn health() -> Json<Health> {
    Json(Health { status: "ok".into() })
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(health))
}
