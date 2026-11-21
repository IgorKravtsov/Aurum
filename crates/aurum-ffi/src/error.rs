//! Flat error type crossing the FFI boundary (Swift `throws`, Kotlin exceptions).

use aurum_client::ClientError;
use serde_json::Value;

/// [`aurum_client::ClientError`] flattened for Swift/Kotlin. Only plain
/// strings/integers cross the boundary, so the platform side can `switch` on
/// the case and show `message` without knowing any Rust type.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AurumError {
    /// Bad input from the app: malformed URL/method/header, invalid JSON body,
    /// half-specified credentials, unparsable decimal.
    #[error("invalid argument: {message}")]
    InvalidArgument { message: String },
    /// The request did not finish within the configured timeout.
    #[error("request timed out")]
    Timeout,
    /// Network/TLS failure before a complete answer arrived.
    #[error("network error: {message}")]
    Network { message: String },
    /// A 2xx answer whose JSON does not fit the expected type.
    #[error("unexpected response: {message}")]
    Decode { message: String },
    /// The server answered with a non-2xx status. `message` is the server's
    /// human-readable `detail` when it has one (else the HTTP reason phrase);
    /// `body_json` is the whole decoded payload as a JSON string.
    #[error("server answered {status}: {message}")]
    Api { status: u16, message: String, body_json: String },
}

/// Best human-readable text of an error payload: FastAPI-style `{"detail": "..."}`,
/// a plain-text body, or — failing both — the HTTP reason phrase.
fn api_message(reason: &str, payload: &Value) -> String {
    match payload {
        Value::String(text) if !text.is_empty() => text.clone(),
        Value::Object(map) => match map.get("detail") {
            Some(Value::String(detail)) => detail.clone(),
            Some(detail) => detail.to_string(),
            None => reason.to_string(),
        },
        _ => reason.to_string(),
    }
}

impl From<ClientError> for AurumError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::InvalidUrl(_)
            | ClientError::InvalidMethod(_)
            | ClientError::InvalidHeaderName(_)
            | ClientError::InvalidHeaderValue { .. } => Self::InvalidArgument { message: error.to_string() },
            ClientError::Timeout => Self::Timeout,
            ClientError::Decode(message) => Self::Decode { message },
            ClientError::Transport(source) => Self::Network { message: source.to_string() },
            ClientError::Api { status, reason, payload } => {
                Self::Api { status, message: api_message(&reason, &payload), body_json: payload.to_string() }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn api_message_prefers_detail_then_text_then_reason() {
        assert_eq!(api_message("Not Found", &json!({"detail": "Account not found"})), "Account not found");
        assert_eq!(api_message("Unprocessable", &json!({"detail": [{"msg": "x"}]})), r#"[{"msg":"x"}]"#);
        assert_eq!(api_message("Bad Gateway", &json!("upstream down")), "upstream down");
        assert_eq!(api_message("Not Found", &json!({"ok": true})), "Not Found");
    }
}
