//! `GET /api/health`.

use serde::{Deserialize, Serialize};

/// Status only — this is the one route nginx serves without auth, so
/// anything returned here is world-readable (the version deliberately is
/// not; it comes back from `GET /api/settings`, which is behind auth).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Health {
    pub status: String,
}
