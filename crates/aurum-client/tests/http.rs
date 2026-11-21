//! HTTP behaviour of `AurumClient` against a local in-process server.

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aurum_client::{AurumClient, BasicAuth, ClientConfig, ClientError};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, Response, StatusCode};
use serde_json::json;

#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    path_and_query: String,
    headers: BTreeMap<String, String>,
    body: String,
}

/// `(status, body, delay)` the mock answers with.
type Reply = (u16, String, Duration);

/// Starts a server answering every request with `reply(request)`;
/// returns its API root (`.../api`) and the request log.
fn serve(reply: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<Recorded>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("non-blocking");
    let port = listener.local_addr().expect("addr").port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let (shared_log, reply) = (Arc::clone(&log), Arc::new(reply));
    std::thread::spawn(move || {
        let runtime =
            tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("runtime");
        runtime.block_on(async move {
            let app = Router::new().fallback(move |request: Request<Body>| {
                let (log, reply) = (Arc::clone(&shared_log), Arc::clone(&reply));
                async move {
                    let (parts, body) = request.into_parts();
                    let bytes = to_bytes(body, 1 << 20).await.unwrap_or_default();
                    let recorded = Recorded {
                        method: parts.method.to_string(),
                        path_and_query: parts.uri.path_and_query().map(ToString::to_string).unwrap_or_default(),
                        headers: parts
                            .headers
                            .iter()
                            .map(|(name, value)| {
                                (name.to_string(), String::from_utf8_lossy(value.as_bytes()).into_owned())
                            })
                            .collect(),
                        body: String::from_utf8_lossy(&bytes).into_owned(),
                    };
                    let (status, text, delay) = reply(&recorded);
                    log.lock().expect("log").push(recorded);
                    tokio::time::sleep(delay).await;
                    Response::builder()
                        .status(StatusCode::from_u16(status).expect("status"))
                        .body(Body::from(text))
                        .expect("response")
                }
            });
            let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
            axum::serve(listener, app).await.expect("serve");
        });
    });
    (format!("http://127.0.0.1:{port}/api"), log)
}

fn ok(body: &str) -> Reply {
    (200, body.to_string(), Duration::ZERO)
}

#[tokio::test]
async fn request_sends_basic_auth_and_json_and_decodes_the_answer() {
    let (url, log) = serve(|_| (201, r#"{"id":1}"#.to_string(), Duration::ZERO));
    let client =
        AurumClient::new(url, Some(BasicAuth { username: "alice".into(), password: "secret".into() })).unwrap();

    let result = client.post("/accounts", &json!({"name": "Cash"})).await.unwrap();

    assert_eq!(result, json!({"id": 1}));
    let request = log.lock().unwrap()[0].clone();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path_and_query, "/api/accounts");
    assert_eq!(request.body, r#"{"name":"Cash"}"#);
    // base64("alice:secret")
    assert_eq!(request.headers["authorization"], "Basic YWxpY2U6c2VjcmV0");
    assert_eq!(request.headers["accept"], "application/json");
    assert_eq!(request.headers["content-type"], "application/json");
}

#[tokio::test]
async fn query_is_form_encoded_and_the_path_gets_a_leading_slash() {
    let (url, log) = serve(|_| ok("[]"));
    let client = AurumClient::new(format!("{url}/"), None).unwrap();

    client.get("transactions", &[("search".into(), "a b&c".into()), ("page".into(), "2".into())]).await.unwrap();

    let request = log.lock().unwrap()[0].clone();
    assert_eq!(request.path_and_query, "/api/transactions?search=a+b%26c&page=2");
    assert!(!request.headers.contains_key("authorization"));
    assert!(!request.headers.contains_key("content-type"), "no body, no content type");
}

#[tokio::test]
async fn empty_and_non_json_bodies_are_preserved() {
    let (url, _) = serve(|request| match request.path_and_query.as_str() {
        "/api/empty" => (204, String::new(), Duration::ZERO),
        "/api/text" => ok("plain text"),
        _ => ok("\u{feff}{\"a\":1}"),
    });
    let client = AurumClient::new(url, None).unwrap();

    assert_eq!(client.get("/empty", &[]).await.unwrap(), json!({"ok": true, "status": 204}));
    assert_eq!(client.get("/text", &[]).await.unwrap(), json!("plain text"));
    assert_eq!(client.get("/bom", &[]).await.unwrap(), json!({"a": 1}));
}

#[tokio::test]
async fn non_success_statuses_become_api_errors_with_the_decoded_payload() {
    let (url, _) = serve(|request| match request.path_and_query.as_str() {
        "/api/missing" => (404, r#"{"detail":"Not found"}"#.to_string(), Duration::ZERO),
        "/api/proxy" => (502, "Bad gateway".to_string(), Duration::ZERO),
        _ => (500, String::new(), Duration::ZERO),
    });
    let client = AurumClient::new(url, None).unwrap();

    match client.delete("/missing").await {
        Err(ClientError::Api { status: 404, reason, payload }) => {
            assert_eq!(reason, "Not Found");
            assert_eq!(payload, json!({"detail": "Not found"}));
        }
        other => panic!("unexpected: {other:?}"),
    }
    match client.get("/proxy", &[]).await {
        Err(ClientError::Api { status: 502, payload, .. }) => assert_eq!(payload, json!("Bad gateway")),
        other => panic!("unexpected: {other:?}"),
    }
    match client.get("/empty", &[]).await {
        Err(ClientError::Api { status: 500, payload, .. }) => assert_eq!(payload, json!({"ok": true, "status": 500})),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn extra_headers_replace_same_spelling_and_merge_case_variants() {
    let (url, log) = serve(|_| ok("{}"));
    let mut config = ClientConfig::new(url);
    config.headers = vec![
        ("X-Custom".into(), "one".into()),
        ("X-Custom".into(), "two".into()),
        ("accept".into(), "text/plain".into()),
    ];
    let client = AurumClient::with_config(config).unwrap();

    client.get("/h", &[]).await.unwrap();

    let headers = log.lock().unwrap()[0].headers.clone();
    assert_eq!(headers["x-custom"], "two");
    // `Accept` and `accept` are two entries; like fetch, they are joined.
    assert_eq!(headers["accept"], "application/json, text/plain");
}

#[tokio::test]
async fn invalid_headers_are_rejected_before_sending() {
    let (url, log) = serve(|_| ok("{}"));
    let mut config = ClientConfig::new(url);
    config.headers = vec![("bad name".into(), "x".into())];
    let client = AurumClient::with_config(config).unwrap();
    assert!(matches!(client.get("/h", &[]).await, Err(ClientError::InvalidHeaderName(name)) if name == "bad name"));

    let mut config = ClientConfig::new("http://127.0.0.1:1/api");
    config.headers = vec![("X-A".into(), "line\nbreak".into())];
    let client = AurumClient::with_config(config).unwrap();
    assert!(matches!(client.get("/h", &[]).await, Err(ClientError::InvalidHeaderValue { .. })));
    assert!(log.lock().unwrap().is_empty());
}

#[tokio::test]
async fn slow_answers_time_out() {
    let (url, _) = serve(|_| (200, "{}".to_string(), Duration::from_millis(800)));
    let mut config = ClientConfig::new(url);
    config.timeout = Duration::from_millis(100);
    let client = AurumClient::with_config(config).unwrap();

    assert!(matches!(client.get("/slow", &[]).await, Err(ClientError::Timeout)));
}

#[tokio::test]
async fn unreachable_servers_are_transport_errors() {
    let client = AurumClient::new("http://127.0.0.1:1/api", None).unwrap();
    assert!(matches!(client.get("/health", &[]).await, Err(ClientError::Transport(_))));
}

#[test]
fn base_url_must_be_http_or_https() {
    assert!(matches!(AurumClient::new("localhost:8000", None), Err(ClientError::InvalidUrl(_))));
    assert!(matches!(AurumClient::new("ftp://x", None), Err(ClientError::InvalidUrl(_))));
    assert_eq!(AurumClient::new("http://localhost:8000/api//", None).unwrap().base_url(), "http://localhost:8000/api");
}

#[tokio::test]
async fn health_is_typed_and_the_method_is_validated() {
    let (url, log) = serve(|_| ok(r#"{"status":"ok"}"#));
    let client = AurumClient::new(url, None).unwrap();

    assert_eq!(client.health().await.unwrap().status, "ok");
    assert_eq!(log.lock().unwrap()[0].path_and_query, "/api/health");
    assert!(matches!(client.request("GE T", "/x", &[], None).await, Err(ClientError::InvalidMethod(_))));
    assert!(matches!(client.request_as::<Vec<u32>>("GET", "/health", &[], None).await, Err(ClientError::Decode(_))));
}

#[test]
fn credentials_never_show_up_in_debug_output() {
    let auth = BasicAuth { username: "alice".into(), password: "hunter2".into() };
    let printed = format!("{auth:?}");
    assert!(printed.contains("alice") && !printed.contains("hunter2"));
}
