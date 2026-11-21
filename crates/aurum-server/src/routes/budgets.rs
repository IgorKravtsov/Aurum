//! `/api/budgets` — port of `backend/app/api/routes/budgets.py`.
//!
//! Not ported yet. Add handlers annotated with `#[utoipa::path(...)]` (full
//! path, e.g. `path = "/api/budgets"`) and register them below with
//! `.routes(routes!(...))`; see `routes/tags.rs` and PORTING.md.

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}
