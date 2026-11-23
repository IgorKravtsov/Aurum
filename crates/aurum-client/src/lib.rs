//! `aurum-client` — async client for the Aurum REST API.
//!
//! A single generic [`AurumClient::request`] covers every endpoint (the CLI
//! maps each command onto it); typed shortcuts such as
//! [`AurumClient::health`] reuse the DTOs from `aurum-domain` so the client
//! can never drift from the server's contract. Credentials, timeouts and
//! the base URL are passed in by the caller (CLI config, iOS Keychain,
//! Android Keystore); this crate never reads or persists anything (ADR 0001).

mod client;
mod config;
mod error;

pub use client::AurumClient;
pub use config::{BasicAuth, ClientConfig, DEFAULT_TIMEOUT};
pub use error::ClientError;
/// Re-exported so callers can pre-validate URLs with the exact same parser.
pub use reqwest::Url;
