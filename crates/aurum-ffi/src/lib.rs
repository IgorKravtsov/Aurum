//! `aurum-ffi` — mobile bindings (ADR 0001: UniFFI over `aurum-client`).
//!
//! Proc-macro style UniFFI (no UDL): the exported surface is
//! [`AurumClient`] (async, tokio-backed), the flat [`AurumError`] and a few
//! pure helpers from `aurum-domain`. Swift/Kotlin bindings are generated from
//! the compiled library (`scripts/gen-bindings.sh`). The core keeps no
//! secrets: the platform keychain/keystore owns the credentials and passes
//! them to the constructor.

mod client;
mod error;
mod helpers;

pub use aurum_domain as domain;
pub use client::{AurumClient, QueryParam};
pub use error::AurumError;
pub use helpers::{contract_version, decimal_quantize};

uniffi::setup_scaffolding!();
