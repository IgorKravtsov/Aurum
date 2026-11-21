//! `execute_request` (Basic Auth, headers, timeout, error texts) against a mock server.

mod common;

use aurum_cli::args::Flags;
use aurum_cli::env::Env;
use aurum_cli::request::BuiltRequest;
use aurum_cli::transport::execute_request;
use common::{MockServer, Reply};
use serde_json::{Value, json};
use std::time::Duration;

fn request(server: &MockServer, method: &str, path: &str, body: Option<Value>) -> BuiltRequest {
    BuiltRequest {
        method: method.to_string(),
        base_url: server.api_url(),
        path: path.to_string(),
        query: Vec::new(),
        body,
    }
}

fn flags(value: Value) -> Flags {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn adds_basic_auth_and_returns_json() {
    let server = MockServer::start(|_| Reply::json(201, r#"{"id":1}"#));
    let built = request(&server, "POST", "/accounts", Some(json!({"name": "Cash"})));

    let result =
        execute_request(&built, &flags(json!({"user": "alice", "password": "secret"})), &Env::default()).await.unwrap();

    assert_eq!(result, json!({"id": 1}));
    let sent = &server.requests()[0];
    assert_eq!(sent.headers["authorization"], "Basic YWxpY2U6c2VjcmV0");
    assert_eq!(sent.body, r#"{"name":"Cash"}"#);
    assert_eq!(sent.headers["content-type"], "application/json");
}

#[tokio::test]
async fn password_is_read_from_a_file_without_the_trailing_newline() {
    let directory = tempfile::tempdir().unwrap();
    let password_file = directory.path().join("password");
    std::fs::write(&password_file, "local-secret\n").unwrap();
    let server = MockServer::start(|_| Reply::json(200, "{}"));

    execute_request(
        &request(&server, "GET", "/settings", None),
        &flags(json!({"user": "alice", "password_file": password_file.to_str().unwrap()})),
        &Env::default(),
    )
    .await
    .unwrap();

    // base64("alice:local-secret")
    assert_eq!(server.requests()[0].headers["authorization"], "Basic YWxpY2U6bG9jYWwtc2VjcmV0");
    // The password file itself is never modified.
    assert_eq!(std::fs::read_to_string(&password_file).unwrap(), "local-secret\n");
}

#[tokio::test]
async fn credentials_may_come_from_the_environment_and_must_come_in_pairs() {
    let server = MockServer::start(|_| Reply::json(200, "{}"));
    let built = request(&server, "GET", "/settings", None);

    let env = Env::from_pairs([("AURUM_USER", "bob"), ("AURUM_PASSWORD", "p")]);
    execute_request(&built, &Flags::new(), &env).await.unwrap();
    assert_eq!(server.requests()[0].headers["authorization"], "Basic Ym9iOnA=");

    let lonely_user = Env::from_pairs([("AURUM_USER", "bob")]);
    let error = execute_request(&built, &Flags::new(), &lonely_user).await.unwrap_err();
    assert_eq!(error.0, "Для Basic Auth нужны и имя пользователя, и пароль");
    let lonely_password = flags(json!({"password": "x"}));
    assert!(execute_request(&built, &lonely_password, &Env::default()).await.is_err());
    assert_eq!(server.requests().len(), 1, "invalid credentials never reach the network");
}

#[tokio::test]
async fn headers_and_timeout_are_validated() {
    let server = MockServer::start(|_| Reply::json(200, "{}"));
    let built = request(&server, "GET", "/x", None);

    let error = execute_request(&built, &flags(json!({"header": "nocolon"})), &Env::default()).await.unwrap_err();
    assert_eq!(error.0, "--header ожидает значение в формате 'Name: value'");
    let error = execute_request(&built, &flags(json!({"timeout": "abc"})), &Env::default()).await.unwrap_err();
    assert_eq!(error.0, "--timeout должен быть положительным числом миллисекунд");
    let error = execute_request(&built, &Flags::new(), &Env::from_pairs([("AURUM_TIMEOUT", "0")])).await.unwrap_err();
    assert_eq!(error.0, "--timeout должен быть положительным числом миллисекунд");

    execute_request(&built, &flags(json!({"header": ["X-A: 1:2 ", "X-B:3"]})), &Env::default()).await.unwrap();
    let sent = &server.requests()[0];
    assert_eq!((sent.headers["x-a"].as_str(), sent.headers["x-b"].as_str()), ("1:2", "3"));
}

#[tokio::test]
async fn failures_read_like_the_node_cli() {
    let server = MockServer::start(|request| match request.path_and_query.as_str() {
        "/api/missing" => Reply::json(404, r#"{"detail":"Not found"}"#),
        "/api/proxy" => Reply::json(502, "Bad gateway"),
        "/api/empty" => Reply::json(204, ""),
        _ => Reply { status: 200, body: "{}".to_string(), delay: Duration::from_millis(800) },
    });
    let run = |path: &'static str, globals: Value| {
        let built = request(&server, "GET", path, None);
        async move { execute_request(&built, &flags(globals), &Env::default()).await }
    };

    assert_eq!(
        run("/missing", json!({})).await.unwrap_err().0,
        "Aurum API вернул 404 Not Found\n{\n  \"detail\": \"Not found\"\n}"
    );
    assert_eq!(run("/proxy", json!({})).await.unwrap_err().0, "Aurum API вернул 502 Bad Gateway\nBad gateway");
    assert_eq!(run("/empty", json!({})).await.unwrap(), json!({"ok": true, "status": 204}));
    assert_eq!(
        run("/slow", json!({"timeout": "100"})).await.unwrap_err().0,
        "The operation was aborted due to timeout"
    );

    let unreachable =
        BuiltRequest { base_url: "http://127.0.0.1:1/api".to_string(), ..request(&server, "GET", "/x", None) };
    assert_eq!(execute_request(&unreachable, &Flags::new(), &Env::default()).await.unwrap_err().0, "fetch failed");
}
