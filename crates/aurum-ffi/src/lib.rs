//! `aurum-ffi` — mobile bindings (ADR 0001: UniFFI over `aurum-client`).
//!
//! Skeleton only. A later phase adds `uniffi`, the `cdylib`/`staticlib`
//! crate types and the exported async API; until then this crate just pins
//! the dependency graph (`ffi` → `client` → `domain`).

pub use aurum_client::{AurumClient, BasicAuth, ClientError};
pub use aurum_domain as domain;
