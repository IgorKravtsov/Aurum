//! Request extractors that validate like FastAPI.
//!
//! [`Input<P, Q, B>`] validates path params, query params and the JSON body
//! together and, on failure, answers one 422 whose `detail` lists the path
//! errors, then the query errors, then the body errors — the order FastAPI
//! uses. Each slot is a `FromInput` type from `aurum-domain`; `()` means
//! "this route has no such input" (the body is then not even read).
//!
//! ```ignore
//! async fn update(
//!     State(state): State<AppState>,
//!     Input { path, body, .. }: Input<CategoryPath, (), CategoryUpdate>,
//! ) -> ApiResult<Json<CategoryRead>> { ... }
//! ```
//!
//! [`ApiPath`], [`ApiQuery`], [`ApiJson`] are shorthands for routes that
//! only have one kind of input.
//!
//! Body handling mirrors `fastapi/routing.py`: an empty body counts as
//! missing; it is parsed as JSON only when the content type is
//! `application/json` / `application/*+json` (FastAPI's default
//! `strict_content_type=True`); anything else — including no content type —
//! is handed to validation as a string (→ `model_attributes_type`); invalid
//! JSON short-circuits with a single `json_invalid` error.

use aurum_domain::validation::{FromInput, Source, ValidationErrors, err};
use axum::body::{Body, Bytes};
use axum::extract::{FromRequest, FromRequestParts, RawPathParams, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use serde_json::{Map, Value};

use crate::error::ApiError;

/// Path + query + body, validated together. See the module docs.
#[derive(Debug, Clone)]
pub struct Input<P = (), Q = (), B = ()> {
    pub path: P,
    pub query: Q,
    pub body: B,
}

impl<S, P, Q, B> FromRequest<S> for Input<P, Q, B>
where
    S: Send + Sync,
    P: FromInput + Send,
    Q: FromInput + Send,
    B: FromInput + Send,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (mut parts, body) = req.into_parts();
        // FastAPI decodes the JSON body before validating anything else, so a
        // syntax error is reported alone.
        let body_value = if B::ABSENT { None } else { read_body(&parts.headers, body).await? };
        let path_value = path_object(&mut parts, state).await;
        let query_value = query_object(&parts);

        let mut errors = ValidationErrors::default();
        let path = collect(P::from_source(&path_value, Source::Path), &mut errors);
        let query = collect(Q::from_source(&query_value, Source::Query), &mut errors);
        let body = if B::ABSENT {
            collect(B::from_source(&Value::Null, Source::Body), &mut errors)
        } else {
            match body_value {
                // Python's json.loads("null") is None, i.e. "no body" too.
                None | Some(Value::Null) => {
                    errors.0.push(err::missing(vec![Source::Body.root()], Value::Null));
                    None
                }
                Some(value) => collect(B::from_source(&value, Source::Body), &mut errors),
            }
        };
        match (path, query, body) {
            (Some(path), Some(query), Some(body)) if errors.is_empty() => {
                reject_int4_overflow(&path_value)?;
                Ok(Self { path, query, body })
            }
            _ => Err(ApiError::Validation(errors)),
        }
    }
}

fn collect<T>(result: Result<T, ValidationErrors>, errors: &mut ValidationErrors) -> Option<T> {
    result.map_err(|e| errors.extend(e)).ok()
}

/// Only path parameters.
#[derive(Debug, Clone)]
pub struct ApiPath<P>(pub P);

impl<S, P> FromRequestParts<S> for ApiPath<P>
where
    S: Send + Sync,
    P: FromInput + Send,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let value = path_object(parts, state).await;
        let path = P::from_source(&value, Source::Path).map_err(ApiError::Validation)?;
        reject_int4_overflow(&value)?;
        Ok(ApiPath(path))
    }
}

/// Only query parameters.
#[derive(Debug, Clone)]
pub struct ApiQuery<Q>(pub Q);

impl<S, Q> FromRequestParts<S> for ApiQuery<Q>
where
    S: Send + Sync,
    Q: FromInput + Send,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let value = query_object(parts);
        Q::from_source(&value, Source::Query).map(ApiQuery).map_err(ApiError::Validation)
    }
}

/// Only a JSON body.
#[derive(Debug, Clone)]
pub struct ApiJson<B>(pub B);

impl<S, B> FromRequest<S> for ApiJson<B>
where
    S: Send + Sync,
    B: FromInput + Send,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        Input::<(), (), B>::from_request(req, state).await.map(|input| ApiJson(input.body))
    }
}

/// Path params as a JSON object of strings (`{"tag_id": "12"}`).
async fn path_object<S: Send + Sync>(parts: &mut Parts, state: &S) -> Value {
    let mut map = Map::new();
    if let Ok(params) = RawPathParams::from_request_parts(parts, state).await {
        for (key, value) in &params {
            map.insert(key.to_string(), Value::String(value.to_string()));
        }
    }
    Value::Object(map)
}

/// Every path parameter is an `integer` id. Python validates it as an
/// unbounded `int` and only fails when asyncpg encodes it for the `int4`
/// column (`DataError` → 500), i.e. after validation succeeded; here the
/// same happens up front for a valid integer outside the `int4` range.
/// (Integers beyond `i64` fail validation here: 422 vs Python's 500.)
fn reject_int4_overflow(path: &Value) -> Result<(), ApiError> {
    let Value::Object(params) = path else { return Ok(()) };
    for value in params.values().filter_map(Value::as_str) {
        let digits = value.trim().replace('_', "");
        let parsed = digits.parse::<i64>().ok().or_else(|| digits.parse::<f64>().ok().map(|f| f as i64));
        if parsed.is_some_and(|n| i32::try_from(n).is_err()) {
            return Err(ApiError::internal(anyhow::anyhow!("path parameter {value:?} is out of int4 range")));
        }
    }
    Ok(())
}

/// Query params as a JSON object of strings. Like Starlette's
/// `QueryParams.get`, a repeated key keeps its last value; blank values are
/// kept (`?year=` is the string `""`, which then fails int parsing).
fn query_object(parts: &Parts) -> Value {
    let mut map = Map::new();
    if let Some(query) = parts.uri.query()
        && let Ok(pairs) = serde_urlencoded::from_str::<Vec<(String, String)>>(query)
    {
        for (key, value) in pairs {
            map.insert(key, Value::String(value));
        }
    }
    Value::Object(map)
}

/// Reads and decodes the body the way FastAPI does. `Ok(None)` = no body.
async fn read_body(headers: &HeaderMap, body: Body) -> Result<Option<Value>, ApiError> {
    let bytes: Bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(ApiError::internal)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    // FastAPI's default `strict_content_type=True`: without a content-type
    // the body is *not* parsed as JSON.
    let is_json = match headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) {
        None => false,
        Some(content_type) => {
            let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
            match mime.split_once('/') {
                Some(("application", subtype)) => subtype == "json" || subtype.ends_with("+json"),
                _ => false,
            }
        }
    };
    if !is_json {
        return Ok(Some(Value::String(String::from_utf8_lossy(&bytes).into_owned())));
    }
    // json.loads(bytes) decodes a leading UTF-8 BOM away ('utf-8-sig').
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf".as_slice()).unwrap_or(&bytes);
    let Ok(text) = std::str::from_utf8(bytes) else {
        // json.loads() raising something other than JSONDecodeError ends up
        // in FastAPI's generic `except Exception` branch.
        return Err(ApiError::bad_request("There was an error parsing the body"));
    };
    serde_json::from_str::<Value>(text).map(Some).map_err(|e| {
        // CPython's exact message and position; the serde mapping is the
        // fallback for text Python accepts but serde doesn't (NaN, ...).
        let item = match crate::pyjson::decode_error(text) {
            Some((pos, msg)) => err::json_invalid(pos, msg),
            None => json_decode_error(bytes, &e),
        };
        ApiError::Validation(item.into())
    })
}

/// Maps a serde_json syntax error to Python's `json.JSONDecodeError`
/// message and character position (best effort: the common cases match
/// exactly, exotic ones keep the right shape with Python's nearest wording).
/// Only used when `pyjson` finds no syntax error, i.e. for input CPython
/// accepts but serde rejects (`NaN`, lone surrogates, huge exponents).
fn json_decode_error(bytes: &[u8], e: &serde_json::Error) -> aurum_domain::validation::ErrorItem {
    let text = String::from_utf8_lossy(bytes);
    // serde_json reports 1-based line/column of the offending byte.
    let mut offset = 0usize;
    for (i, line) in text.split('\n').enumerate() {
        if i + 1 == e.line() {
            offset += e.column().saturating_sub(1).min(line.len());
            break;
        }
        offset += line.len() + 1;
    }
    let offset = offset.min(text.len());
    let char_pos = text.get(..offset).map(|s| s.chars().count()).unwrap_or(offset);
    let message = e.to_string();
    let python = if message.starts_with("key must be a string") {
        "Expecting property name enclosed in double quotes"
    } else if message.starts_with("expected `:`") {
        "Expecting ':' delimiter"
    } else if message.starts_with("expected `,` or") {
        "Expecting ',' delimiter"
    } else if message.starts_with("trailing characters") {
        "Extra data"
    } else if message.starts_with("EOF while parsing a string") {
        "Unterminated string starting at"
    } else if message.starts_with("control character") {
        "Invalid control character at"
    } else if message.starts_with("invalid escape") {
        "Invalid \\escape"
    } else {
        "Expecting value"
    };
    err::json_invalid(char_pos, python)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_errors_look_like_python() {
        let e = serde_json::from_slice::<Value>(b"{bad").unwrap_err();
        let item = json_decode_error(b"{bad", &e);
        assert_eq!(
            serde_json::to_string(&item).unwrap(),
            r#"{"type":"json_invalid","loc":["body",1],"msg":"JSON decode error","input":{},"ctx":{"error":"Expecting property name enclosed in double quotes"}}"#
        );
    }
}
