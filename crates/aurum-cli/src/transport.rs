//! Sending a built request: Basic Auth, extra headers and the timeout are
//! resolved here (flags, environment, password file) and handed to
//! `aurum-client` as plain data. Port of `executeRequest` in `cli/src/client.js`.

use std::path::Path;
use std::time::Duration;

use aurum_client::{AurumClient, BasicAuth, ClientConfig, ClientError};
use serde_json::Value;

use crate::args::Flags;
use crate::env::Env;
use crate::error::{CliError, Result};
use crate::js::{self, fs};
use crate::request::BuiltRequest;

const DEFAULT_TIMEOUT_MS: f64 = 30_000.0;

/// `--password-file`/`AURUM_PASSWORD_FILE` (trailing newlines dropped),
/// else `--password`/`AURUM_PASSWORD`.
fn read_password(globals: &Flags, env: &Env) -> Result<Option<Value>> {
    let password_file = globals
        .get("password_file")
        .cloned()
        .or_else(|| env.get("AURUM_PASSWORD_FILE").map(|value| Value::String(value.to_string())));
    if let Some(password_file) = password_file.filter(js::truthy) {
        let path = js::to_js_string(&password_file);
        let content = fs::read_to_string(Path::new(&path))
            .map_err(|error| CliError::new(fs::read_error_message(&error, &path)))?;
        return Ok(Some(Value::String(content.trim_end_matches(['\r', '\n']).to_string())));
    }
    Ok(globals
        .get("password")
        .cloned()
        .or_else(|| env.get("AURUM_PASSWORD").map(|value| Value::String(value.to_string()))))
}

/// `--header 'Name: value'` entries, in order.
fn parse_headers(globals: &Flags) -> Result<Vec<(String, String)>> {
    let entries = match globals.get("header") {
        None => Vec::new(),
        Some(Value::Array(items)) => items.iter().collect(),
        Some(other) => vec![other],
    };
    entries
        .into_iter()
        .map(|entry| {
            let text = js::to_js_string(entry);
            match text.find(':') {
                Some(separator) if separator >= 1 => {
                    Ok((text[..separator].trim().to_string(), text[separator + 1..].trim().to_string()))
                }
                _ => Err(CliError::new("--header ожидает значение в формате 'Name: value'")),
            }
        })
        .collect()
}

fn resolve_timeout(globals: &Flags, env: &Env) -> Result<Duration> {
    let milliseconds = match (globals.get("timeout"), env.get("AURUM_TIMEOUT")) {
        (Some(value), _) => js::to_js_number(value),
        (None, Some(value)) => js::string_to_number(value),
        (None, None) => DEFAULT_TIMEOUT_MS,
    };
    if !milliseconds.is_finite() || milliseconds <= 0.0 {
        return Err(CliError::new("--timeout должен быть положительным числом миллисекунд"));
    }
    // Fractions are truncated and absurd values clamped, as for AbortSignal.timeout.
    Ok(Duration::from_millis(milliseconds.min(9_007_199_254_740_991.0) as u64))
}

/// How the Node CLI's `fetch` failures read to the user.
fn describe_client_error(error: ClientError) -> CliError {
    match error {
        ClientError::Api { status, reason, payload } => {
            // Objects/arrays are shown as pretty JSON, anything else verbatim.
            let detail = match &payload {
                Value::Object(_) | Value::Array(_) | Value::Null => js::stringify(&payload, 2),
                other => js::to_js_string(other),
            };
            CliError::new(format!("Aurum API вернул {status} {reason}\n{detail}"))
        }
        ClientError::Timeout => CliError::new("The operation was aborted due to timeout"),
        ClientError::Transport(_) => CliError::new("fetch failed"),
        // Texts of the `Headers` constructor in Node's fetch.
        ClientError::InvalidHeaderName(name) => {
            CliError::new(format!("Headers.append: \"{name}\" is an invalid header name."))
        }
        ClientError::InvalidHeaderValue { value, .. } => {
            CliError::new(format!("Headers.append: \"{value}\" is an invalid header value."))
        }
        other => CliError::new(other.to_string()),
    }
}

pub async fn execute_request(request: &BuiltRequest, globals: &Flags, env: &Env) -> Result<Value> {
    let headers = parse_headers(globals)?;

    let user =
        globals.get("user").cloned().or_else(|| env.get("AURUM_USER").map(|value| Value::String(value.to_string())));
    let password = read_password(globals, env)?;
    let has_user = user.as_ref().is_some_and(js::truthy);
    if (has_user && password.is_none()) || (!has_user && password.is_some()) {
        return Err(CliError::new("Для Basic Auth нужны и имя пользователя, и пароль"));
    }
    let auth = match (user, password) {
        (Some(user), Some(password)) if js::truthy(&user) => {
            Some(BasicAuth { username: js::to_js_string(&user), password: js::to_js_string(&password) })
        }
        _ => None,
    };

    let mut config = ClientConfig::new(request.base_url.clone());
    config.auth = auth;
    config.headers = headers;
    config.timeout = resolve_timeout(globals, env)?;
    let client = AurumClient::with_config(config).map_err(describe_client_error)?;

    // Serialised here (not by the client) so the body bytes are exactly what
    // JSON.stringify produced for the Node CLI.
    let body = request.body.as_ref().map(|body| js::stringify(body, 0).into_bytes());
    client.request_raw(&request.method, &request.path, &request.query, body).await.map_err(describe_client_error)
}
