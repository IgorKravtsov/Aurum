//! The exported API end to end: `AurumClient` against a local in-process server.

use std::net::TcpListener;

use aurum_ffi::{AurumClient, AurumError, QueryParam, contract_version, decimal_quantize};
use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};

/// Starts the mock API on a free port; returns its root (`.../api`).
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("non-blocking");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("rt");
        runtime.block_on(async move {
            let app = Router::new()
                .route("/api/health", get(|| async { r#"{"status":"ok"}"# }))
                // Echoes what the server saw: auth header, query string and raw body.
                .route(
                    "/api/echo",
                    post(|headers: HeaderMap, uri: axum::http::Uri, body: Bytes| async move {
                        let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
                        serde_json::json!({
                            "auth": auth,
                            "query": uri.query().unwrap_or(""),
                            "body": String::from_utf8_lossy(&body),
                        })
                        .to_string()
                    }),
                )
                .route("/api/missing", get(|| async { (StatusCode::NOT_FOUND, r#"{"detail":"Account not found"}"#) }));
            let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
            axum::serve(listener, app).await.expect("serve");
        });
    });
    format!("http://127.0.0.1:{port}/api")
}

#[tokio::test]
async fn health_is_decoded_into_the_domain_dto() {
    let client = AurumClient::new(serve(), None, None, None).unwrap();
    assert_eq!(client.health().await.unwrap().status, "ok");
}

#[tokio::test]
async fn request_forwards_auth_query_and_exact_body() {
    let client = AurumClient::new(serve(), Some("alice".into()), Some("secret".into()), Some(5_000)).unwrap();
    let query =
        vec![QueryParam { name: "a".into(), value: "1".into() }, QueryParam { name: "a".into(), value: "x y".into() }];
    let answer =
        client.request("POST".into(), "/echo".into(), query, Some(r#"{"amount": 1.10}"#.into())).await.unwrap();
    let echoed: serde_json::Value = serde_json::from_str(&answer).unwrap();
    // base64("alice:secret")
    assert_eq!(echoed["auth"], "Basic YWxpY2U6c2VjcmV0");
    assert_eq!(echoed["query"], "a=1&a=x+y");
    assert_eq!(echoed["body"], r#"{"amount": 1.10}"#);
}

#[tokio::test]
async fn api_errors_carry_status_message_and_body() {
    let client = AurumClient::new(serve(), None, None, None).unwrap();
    match client.request("GET".into(), "/missing".into(), vec![], None).await {
        Err(AurumError::Api { status, message, body_json }) => {
            assert_eq!(status, 404);
            assert_eq!(message, "Account not found");
            assert_eq!(body_json, r#"{"detail":"Account not found"}"#);
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn bad_input_is_rejected_before_any_request() {
    let client = AurumClient::new(serve(), None, None, None).unwrap();
    let invalid_body = client.request("POST".into(), "/echo".into(), vec![], Some("{oops".into())).await;
    assert!(matches!(invalid_body, Err(AurumError::InvalidArgument { .. })));
    assert!(matches!(
        client.request("NOT A METHOD".into(), "/echo".into(), vec![], None).await,
        Err(AurumError::InvalidArgument { .. })
    ));
    assert!(matches!(AurumClient::new("ftp://x".into(), None, None, None), Err(AurumError::InvalidArgument { .. })));
    assert!(matches!(
        AurumClient::new(serve(), Some("alice".into()), None, None),
        Err(AurumError::InvalidArgument { .. })
    ));
}

#[tokio::test]
async fn unreachable_server_is_a_network_error() {
    // Port 1 on localhost: nothing listens there.
    let client = AurumClient::new("http://127.0.0.1:1/api".into(), None, None, Some(2_000)).unwrap();
    assert!(matches!(client.health().await, Err(AurumError::Network { .. } | AurumError::Timeout)));
}

#[test]
fn pure_helpers_are_exported() {
    assert_eq!(contract_version(), aurum_domain::APP_VERSION);
    assert_eq!(decimal_quantize("2.675".into(), 2).unwrap(), "2.68");
}
