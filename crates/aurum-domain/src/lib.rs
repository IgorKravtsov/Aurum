//! `aurum-domain` — the Aurum REST contract as Rust types, plus pure logic.
//!
//! Shared by the server (`aurum-server`), the HTTP client (`aurum-client`)
//! and through it the CLI and the mobile FFI layer (ADR 0001), so this crate
//! never does IO: no tokio, no network, no filesystem. Optional features
//! only add trait impls:
//!
//! * `sqlx` — `sqlx::Type`/`FromRow` impls so the server can read rows
//!   straight into these types (no queries live here);
//! * `openapi` — `utoipa::ToSchema` impls for the server's `/api/openapi.json`.
//!
//! One module per API route group, mirroring the former Python backend's `app/schemas/*.py`.

pub mod decimal;
pub mod enums;
pub mod pyfmt;
pub mod text;
pub mod validation;

pub mod accounts;
pub mod advice;
pub mod assets;
pub mod backup;
pub mod budgets;
pub mod cash_flow;
pub mod categories;
pub mod category_rollup;
pub mod crypto;
pub mod dashboard;
pub mod fx;
pub mod goals;
pub mod health;
pub mod insights;
pub mod net_worth;
pub mod recurring;
pub mod reports;
pub mod settings;
pub mod tags;
pub mod transactions;

pub use decimal::Decimal;
pub use pyfmt::{PyDateTime, PyFloat};

/// Single source of truth for the running app's version (Python:
/// `app/core/config.py::APP_VERSION`) — surfaced in the OpenAPI title, in
/// `GET /api/settings` as `app_version` (deliberately NOT on the
/// unauthenticated /api/health) and embedded in exported backups.
pub const APP_VERSION: &str = "2.0.0";
