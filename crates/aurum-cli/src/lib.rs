//! `aurum` — command-line client for the Aurum REST API.
//!
//! Rust port of the former Node CLI with identical user-visible behaviour
//! (flags, output, exit codes, config file, skill texts). Layout:
//!
//! - `args`, `commands`, `help`: command line -> command model
//! - `config`: config file and base-URL resolution (disk IO lives here)
//! - `request`, `transport`: command -> HTTP request -> `aurum-client`
//! - `skills`: the embedded agent guide
//! - `js`: the JavaScript semantics the Node CLI relied on, see its docs

pub mod args;
pub mod commands;
pub mod config;
pub mod env;
pub mod error;
pub mod help;
pub mod js;
pub mod request;
pub mod skills;
pub mod transport;

use std::path::Path;

use serde_json::Value;

use crate::args::parse_arguments;
use crate::commands::find_command;
use crate::env::Env;
use crate::error::{CliError, Result};
use crate::help::{VERSION, help};
use crate::js::fs;

/// What a command prints: strings verbatim, everything else as JSON.
fn render(value: &Value, compact: bool) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => js::stringify(other, if compact { 0 } else { 2 }),
    }
}

/// Runs one invocation (`argv` without the program name) and returns the
/// text for stdout. Errors are printed by the caller to stderr with exit code 1.
pub async fn run(argv: &[String], env: &Env) -> Result<String> {
    let parsed = parse_arguments(argv)?;
    let compact = parsed.globals.get("compact").is_some_and(js::truthy);

    if parsed.globals.get("version").is_some_and(js::truthy) {
        return Ok(format!("{VERSION}\n"));
    }
    if parsed.globals.get("help").is_some_and(js::truthy) || parsed.positionals.is_empty() {
        return Ok(format!("{}\n", help()));
    }

    match parsed.positionals[0].as_str() {
        "skills" => {
            // Flags win over globals, e.g. `--no-json` switches `--json` off.
            let mut merged = parsed.globals.clone();
            merged.extend(parsed.flags.clone());
            let output = skills::run_skills(&parsed.positionals, &merged, env)?;
            return Ok(format!("{}\n", render(&output, compact)));
        }
        "config" => {
            let output = config::run_config(&parsed.positionals, env)?;
            return Ok(format!("{}\n", render(&output, compact)));
        }
        _ => {}
    }

    let built = if parsed.positionals[0] == "request" {
        request::build_generic_request(&parsed.positionals, &parsed.flags, &parsed.globals, env)?
    } else {
        let command = find_command(&parsed.positionals).ok_or_else(|| {
            CliError::new(format!("Неизвестная команда: {}\nЗапустите aurum --help", parsed.positionals.join(" ")))
        })?;
        request::build_request(command, &parsed.positionals, &parsed.flags, &parsed.globals, env)?
    };
    let result = transport::execute_request(&built, &parsed.globals, env).await?;
    let output = format!("{}\n", render(&result, compact));

    match parsed.globals.get("output").filter(|path| js::truthy(path)) {
        Some(path) => {
            // The response may contain private financial data: owner-only file.
            let path = js::to_js_string(path);
            fs::write_private(Path::new(&path), output.as_bytes())
                .map_err(|error| CliError::new(fs::error_message(&error, "open", Some(&path))))?;
            Ok(String::new())
        }
        None => Ok(output),
    }
}
