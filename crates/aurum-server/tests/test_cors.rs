//! CORS defaults.
//!
//! The README tells self-hosters that leaving basic auth off is fine as long as
//! the instance is only reachable from localhost. That promise only holds if a
//! random page open in the same browser can't call the API on their behalf —
//! which is what these tests pin down.

use reqwest::Method;

use crate::common::TestApp;

const EVIL: &str = "https://evil.example";

/// Starlette answers a credentialed wildcard by echoing whatever Origin
/// asked, which is what makes the wildcard dangerous rather than merely
/// permissive — the answer must be `*` or nothing, never the caller's own
/// origin.
#[tokio::test]
async fn test_a_foreign_origin_is_never_echoed_back() {
    let client = TestApp::start().await;
    let resp = client.send(client.request(Method::GET, "/accounts").header("Origin", EVIL)).await;

    assert_ne!(resp.header("access-control-allow-origin"), Some(EVIL));
}

/// With allow-credentials, a page the user visits can call the instance
/// with their Basic Auth attached and read the response.
#[tokio::test]
async fn test_credentials_are_not_granted_to_every_origin() {
    let client = TestApp::start().await;
    let resp = client.send(client.request(Method::GET, "/accounts").header("Origin", EVIL)).await;

    assert_ne!(resp.header("access-control-allow-credentials"), Some("true"));
}

/// Nothing in the shipped setup needs CORS: compose serves UI and API
/// from one origin, and the Vite dev server proxies /api. So the default
/// should be closed, not `*`.
///
/// Checked on the wire: a server started with AURUM_CORS_ORIGINS unset
/// grants no origin anything, neither on a simple request nor on a
/// preflight.
#[tokio::test]
async fn test_shipped_default_allows_no_cross_origin_access() {
    let app = TestApp::start().await;
    // AURUM_CORS_ORIGINS unset = the shipped default.
    let server = app.spawn_server(|settings| settings.cors_origins = String::new()).await;
    let simple = server.send(server.request(Method::GET, "/accounts").header("Origin", EVIL)).await;
    let preflight = server
        .send(
            server
                .request(Method::OPTIONS, "/accounts")
                .header("Origin", EVIL)
                .header("Access-Control-Request-Method", "GET"),
        )
        .await;
    drop(server);

    assert!(!simple.headers.contains_key("access-control-allow-origin"));
    assert!(!preflight.headers.contains_key("access-control-allow-origin"));
}
