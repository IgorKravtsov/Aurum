//! A tiny in-process HTTP server for black-box tests: it records every
//! request and answers with whatever the test's closure returns.

#![allow(dead_code)] // each test binary uses a different part of this module

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, Response, StatusCode};

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path_and_query: String,
    /// Header names are lower-cased.
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

pub struct Reply {
    pub status: u16,
    pub body: String,
    pub delay: Duration,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Self {
        Self { status, body: body.to_string(), delay: Duration::ZERO }
    }
}

pub struct MockServer {
    port: u16,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl MockServer {
    pub fn start(reply: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        listener.set_nonblocking(true).expect("non-blocking listener");
        let port = listener.local_addr().expect("local addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        let reply = Arc::new(reply);

        // A dedicated thread with its own runtime keeps the server alive no
        // matter which runtime (or none) the test itself uses. It is simply
        // left running: the test process exits right after the tests.
        std::thread::spawn(move || {
            let runtime =
                tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("runtime");
            runtime.block_on(async move {
                let app = Router::new().fallback(move |request: Request<Body>| {
                    let (log, reply) = (Arc::clone(&log), Arc::clone(&reply));
                    async move {
                        let (parts, body) = request.into_parts();
                        let bytes = to_bytes(body, 10 * 1024 * 1024).await.unwrap_or_default();
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
                        let answer = reply(&recorded);
                        log.lock().expect("request log").push(recorded);
                        if !answer.delay.is_zero() {
                            tokio::time::sleep(answer.delay).await;
                        }
                        Response::builder()
                            .status(StatusCode::from_u16(answer.status).expect("valid status"))
                            .header("content-type", "application/json")
                            .body(Body::from(answer.body))
                            .expect("response")
                    }
                });
                let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");
                axum::serve(listener, app).await.expect("serve");
            });
        });
        Self { port, requests }
    }

    /// API root as the CLI is configured with, e.g. `http://127.0.0.1:41234/api`.
    pub fn api_url(&self) -> String {
        format!("http://127.0.0.1:{}/api", self.port)
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().expect("request log").clone()
    }
}
