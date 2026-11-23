//! Every API endpoint must have a named CLI command (and no two commands
//! may share an endpoint).
//!
//! The reference is the server's own route table
//! (`aurum_server::http::route_table::ROUTES`, which a unit test of
//! `aurum-server` keeps equal to the real router), so a new endpoint without
//! a CLI command fails here instead of being silently skipped.

use std::collections::BTreeSet;

use aurum_cli::commands::COMMANDS;
use aurum_server::http::route_table::ROUTES;

/// `METHOD path` of every server route, in the CLI's notation: the `/api`
/// prefix is the base URL's job (`--api-base-url`), not part of a command's path.
fn server_endpoints() -> BTreeSet<String> {
    ROUTES
        .iter()
        .map(|(method, path)| {
            let path = path.strip_prefix("/api").unwrap_or_else(|| panic!("маршрут без /api: {path}"));
            format!("{method} {path}")
        })
        .collect()
}

#[test]
fn every_backend_endpoint_has_a_named_cli_command() {
    let backend = server_endpoints();
    assert!(backend.len() > 50, "таблица маршрутов сервера подозрительно мала: {}", backend.len());

    let cli: Vec<String> = COMMANDS.iter().map(|command| format!("{} {}", command.method, command.path)).collect();
    let unique: BTreeSet<String> = cli.iter().cloned().collect();
    assert_eq!(unique, backend);
    assert_eq!(cli.len(), unique.len(), "у двух команд не должно быть одного endpoint");
}
