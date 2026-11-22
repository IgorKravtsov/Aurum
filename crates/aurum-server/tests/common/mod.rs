//! Test harness wiring — port of the former Python backend's `tests/conftest.py`,
//! `server.py` and `helpers.py`.
//!
//! Every test calls [`TestApp::start`] (or [`TestApp::start_with`] for a
//! different configuration — the pytest `server_env` marker) and gets:
//!
//! - its own fresh Postgres database, created from a template that was
//!   migrated and seeded once per test run (`CREATE DATABASE ... TEMPLATE`),
//!   so tests never see each other's rows, never guess at auto-incremented
//!   IDs and can run in parallel;
//! - the real `aurum_server::build_app` router served in-process on a random
//!   local port, talked to over real HTTP (base URL ends in `/api`, same
//!   paths the frontend calls);
//! - its own fake NBP/CoinGecko (see `fakes.rs`).
//!
//! The database is dropped again when the `TestApp` goes out of scope.
//! Databases are named `aurum_it_<pid>_*`; ones left behind by a test
//! process that no longer runs (e.g. killed with Ctrl-C) are dropped at the
//! start of the next run. The real `aurum` database is never opened: the
//! harness only ever connects to the `postgres` maintenance database and to
//! the databases it created itself.
//!
//! Connection parameters come from `AURUM_POSTGRES_USER/PASSWORD/HOST/PORT`
//! (defaults aurum/aurum/127.0.0.1/5432 — what the CI service container
//! uses). If Postgres isn't reachable the tests fail with a message saying
//! so, rather than silently passing.

#![allow(dead_code)]

pub mod fakes;

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use aurum_domain::Decimal;
use aurum_server::config::Settings;
use aurum_server::state::AppState;
use reqwest::Method;
use reqwest::header::HeaderMap;
use serde_json::{Map, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, PgConnection, PgPool};
use tokio::sync::OnceCell;

pub use fakes::{FakeCoinGecko, FakeNbp, FakeProviders, MarketPoint};

/// Prefix of every database the harness creates.
const DB_PREFIX: &str = "aurum_it_";

/// Aborts a spawned task (a server) when dropped.
pub struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// --------------------------------------------------------------------------
// Configuration
// --------------------------------------------------------------------------

/// The configuration every test's app starts from, like the session server
/// of the pytest harness: shipped defaults (no `AURUM_*` from the caller's
/// shell leaks in except the Postgres connection), `AURUM_DEFAULT_CURRENCY=USD`
/// and a dummy CoinGecko key — the fake CoinGecko doesn't check it, but the
/// server refuses to call CoinGecko at all without one.
fn base_settings() -> Settings {
    let env = |name: &str, default: &str| {
        std::env::var(format!("AURUM_POSTGRES_{name}")).ok().filter(|v| !v.is_empty()).unwrap_or(default.into())
    };
    let port = env("PORT", "5432");
    Settings {
        postgres_user: env("USER", "aurum"),
        postgres_password: env("PASSWORD", "aurum"),
        postgres_host: env("HOST", "127.0.0.1"),
        postgres_port: port.parse().unwrap_or_else(|_| panic!("AURUM_POSTGRES_PORT is not a port: {port:?}")),
        postgres_db: "postgres".into(),
        default_currency: "USD".into(),
        coingecko_api_key: "test-key".into(),
        ..Settings::default()
    }
}

// --------------------------------------------------------------------------
// Database bookkeeping
// --------------------------------------------------------------------------

async fn admin_connection(settings: &Settings) -> PgConnection {
    PgConnection::connect_with(&settings.connect_options_for("postgres")).await.unwrap_or_else(|err| {
        panic!(
            "cannot reach Postgres at {}:{} as {:?} ({err}). The API tests need a running Postgres: \
             set AURUM_POSTGRES_HOST/PORT/USER/PASSWORD (the user must be allowed to CREATE DATABASE).",
            settings.postgres_host, settings.postgres_port, settings.postgres_user
        )
    })
}

/// Runs a statement built from harness-generated database names only
/// (`aurum_it_<pid>_<n>`), never from outside input.
async fn admin_execute(admin: &mut PgConnection, statement: &str) {
    sqlx::query(sqlx::AssertSqlSafe(statement.to_string()))
        .execute(&mut *admin)
        .await
        .unwrap_or_else(|err| panic!("`{statement}` failed: {err}"));
}

/// Drops databases left behind by test processes that are gone. Liveness
/// is checked through /proc, so this is a no-op where /proc doesn't exist.
async fn drop_stale_databases(admin: &mut PgConnection) {
    if !Path::new("/proc/self").exists() {
        return;
    }
    let names: Vec<String> = sqlx::query_scalar("SELECT datname FROM pg_database WHERE starts_with(datname, $1)")
        .bind(DB_PREFIX)
        .fetch_all(&mut *admin)
        .await
        .expect("list test databases");
    for name in names {
        let pid =
            name.strip_prefix(DB_PREFIX).and_then(|rest| rest.split('_').next()).and_then(|p| p.parse::<u32>().ok());
        if let Some(pid) = pid
            && pid != std::process::id()
            && !Path::new(&format!("/proc/{pid}")).exists()
        {
            admin_execute(admin, &format!(r#"DROP DATABASE IF EXISTS "{name}" WITH (FORCE)"#)).await;
        }
    }
}

/// Tables the harness never wipes: migration bookkeeping of Alembic (Python)
/// and sqlx (Rust).
const MIGRATION_TABLES: [&str; 2] = ["alembic_version", "_sqlx_migrations"];

/// `TRUNCATE ... RESTART IDENTITY CASCADE` of every table (names discovered
/// from information_schema, migration bookkeeping excluded) — exactly what
/// the pytest harness did between `migrate` and the `seed` it snapshotted,
/// so tests start from the seed's rows only (no data a migration inserted,
/// e.g. the default crypto portfolio of 0002) with fresh sequences.
async fn truncate_all_tables(pool: &PgPool) {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE' AND table_name <> ALL($1::text[]) \
         ORDER BY table_name",
    )
    .bind(MIGRATION_TABLES)
    .fetch_all(pool)
    .await
    .expect("list tables");
    let list = tables.iter().map(|table| format!(r#""{table}""#)).collect::<Vec<_>>().join(", ");
    // Table names come from the catalog, not from outside input.
    sqlx::query(sqlx::AssertSqlSafe(format!("TRUNCATE TABLE {list} RESTART IDENTITY CASCADE")))
        .execute(pool)
        .await
        .expect("truncate the template database");
}

/// The per-run template: created fresh, migrated and seeded with the
/// server's own code (`db::migrate::run` + `db::seed::run`, i.e. what
/// `aurum-server migrate` / `seed` do; tables truncated in between like the
/// pytest harness did), once for the whole test binary.
async fn template_database() -> &'static str {
    static TEMPLATE: OnceCell<String> = OnceCell::const_new();
    TEMPLATE
        .get_or_init(|| async {
            let settings = base_settings();
            let mut admin = admin_connection(&settings).await;
            drop_stale_databases(&mut admin).await;
            let name = format!("{DB_PREFIX}{}_template", std::process::id());
            admin_execute(&mut admin, &format!(r#"DROP DATABASE IF EXISTS "{name}" WITH (FORCE)"#)).await;
            admin_execute(&mut admin, &format!(r#"CREATE DATABASE "{name}""#)).await;
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .connect_with(settings.connect_options_for(&name))
                .await
                .expect("connect to the template database");
            aurum_server::db::migrate::run(&pool).await.expect("migrate the template database");
            truncate_all_tables(&pool).await;
            aurum_server::db::seed::run(&pool, &settings).await.expect("seed the template database");
            // CREATE DATABASE ... TEMPLATE needs nobody connected to it.
            pool.close().await;
            let _ = admin.close().await;
            name
        })
        .await
}

/// One test's own database, dropped (WITH FORCE) when this goes away.
struct TestDatabase {
    name: String,
    settings: Settings,
}

impl TestDatabase {
    async fn create() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let template = template_database().await;
        let settings = base_settings();
        let name = format!("{DB_PREFIX}{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
        let mut admin = admin_connection(&settings).await;
        admin_execute(&mut admin, &format!(r#"CREATE DATABASE "{name}" TEMPLATE "{template}""#)).await;
        let _ = admin.close().await;
        Self { name, settings }
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        // Drop runs inside the test's runtime, which can't be blocked on;
        // a short-lived thread with its own runtime does the async work.
        let (name, settings) = (self.name.clone(), self.settings.clone());
        let dropper = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
            runtime.block_on(async {
                if let Ok(mut admin) = PgConnection::connect_with(&settings.connect_options_for("postgres")).await {
                    admin_execute(&mut admin, &format!(r#"DROP DATABASE IF EXISTS "{name}" WITH (FORCE)"#)).await;
                    let _ = admin.close().await;
                }
            });
        });
        let _ = dropper.join();
    }
}

// --------------------------------------------------------------------------
// HTTP
// --------------------------------------------------------------------------

/// A fully read response.
#[derive(Debug)]
pub struct Resp {
    pub status: u16,
    pub headers: HeaderMap,
    pub text: String,
}

impl Resp {
    /// The body as JSON (panics with the raw body if it isn't).
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text)
            .unwrap_or_else(|err| panic!("response {} is not JSON ({err}): {}", self.status, self.text))
    }

    /// A response header, if present and valid UTF-8.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

/// HTTP client for one running server (the equivalent of the pytest
/// `client` fixture: an httpx client whose base URL ends in `/api`).
pub struct Api {
    base_url: String,
    http: reqwest::Client,
}

impl Api {
    fn new(base_url: String) -> Self {
        let http = reqwest::Client::builder()
            // httpx doesn't follow redirects by default; neither do we.
            .redirect(reqwest::redirect::Policy::none())
            // The server lives on 127.0.0.1 — a proxy configured in the
            // shell must never sit in between.
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("http client");
        Self { base_url, http }
    }

    /// e.g. `http://127.0.0.1:PORT/api`.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// A request builder for `path` (relative to `/api`), for anything the
    /// shortcuts below don't cover (custom headers, raw bodies, methods).
    pub fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.http.request(method, format!("{}{path}", self.base_url))
    }

    pub async fn send(&self, request: reqwest::RequestBuilder) -> Resp {
        let response = request.send().await.unwrap_or_else(|err| panic!("request failed: {err}"));
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let text = response.text().await.unwrap_or_else(|err| panic!("reading the response failed: {err}"));
        Resp { status, headers, text }
    }

    pub async fn get(&self, path: &str) -> Resp {
        self.send(self.request(Method::GET, path)).await
    }

    /// GET with query parameters given as a JSON object, like httpx's
    /// `params={"year": 2026, "month": 1}`.
    pub async fn get_with(&self, path: &str, params: Value) -> Resp {
        self.get(&format!("{path}?{}", query_string(&params))).await
    }

    pub async fn post(&self, path: &str, body: Value) -> Resp {
        self.send(self.request(Method::POST, path).json(&body)).await
    }

    /// POST without a body (httpx's `client.post(path)`).
    pub async fn post_empty(&self, path: &str) -> Resp {
        self.send(self.request(Method::POST, path)).await
    }

    pub async fn patch(&self, path: &str, body: Value) -> Resp {
        self.send(self.request(Method::PATCH, path).json(&body)).await
    }

    pub async fn delete(&self, path: &str) -> Resp {
        self.send(self.request(Method::DELETE, path)).await
    }
}

fn query_string(params: &Value) -> String {
    let object = params.as_object().expect("query params must be a JSON object");
    let pairs: Vec<(String, String)> = object
        .iter()
        .map(|(key, value)| {
            let value = match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            (key.clone(), value)
        })
        .collect();
    serde_urlencoded::to_string(pairs).expect("encode query")
}

/// Serves `build_app(state)` on a random local port.
async fn serve(state: AppState) -> (Api, AbortOnDrop) {
    let app = aurum_server::build_app(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind test server");
    let addr = listener.local_addr().expect("test server address");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (Api::new(format!("http://{addr}/api")), AbortOnDrop(server))
}

// --------------------------------------------------------------------------
// Per test
// --------------------------------------------------------------------------

/// One test's world: database, fakes, and the app serving both. Derefs to
/// [`Api`], so `app.get("/accounts")` talks to it.
pub struct TestApp {
    api: Api,
    /// The settings the app was built with.
    pub settings: Settings,
    /// The same state the app's handlers get — for service-level tests and
    /// direct SQL (`sqlx::query(...).execute(&app.state.pool)`).
    pub state: AppState,
    pub nbp: FakeNbp,
    pub coingecko: FakeCoinGecko,
    // Drop order matters: the server goes before the database.
    _server: AbortOnDrop,
    _fakes: FakeProviders,
    _database: TestDatabase,
}

impl std::ops::Deref for TestApp {
    type Target = Api;

    fn deref(&self) -> &Api {
        &self.api
    }
}

impl TestApp {
    /// The default configuration (see [`base_settings`]).
    pub async fn start() -> Self {
        Self::start_with(|_| {}).await
    }

    /// For a test that needs the app built with different settings (CORS,
    /// default currency, CoinGecko key, ...) — the pytest `server_env`
    /// marker.
    pub async fn start_with(configure: impl FnOnce(&mut Settings)) -> Self {
        let database = TestDatabase::create().await;
        let fakes = FakeProviders::start().await;
        let mut settings = base_settings();
        settings.postgres_db = database.name.clone();
        settings.nbp_base_url = fakes.nbp_base_url();
        settings.coingecko_base_url = fakes.coingecko_base_url();
        configure(&mut settings);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(settings.connect_options())
            .await
            .unwrap_or_else(|err| panic!("connect to test database {}: {err}", database.name));
        let state = AppState::new(pool, settings.clone()).expect("app state");
        let (api, server) = serve(state.clone()).await;
        Self {
            api,
            settings,
            state,
            nbp: fakes.nbp.clone(),
            coingecko: fakes.coingecko.clone(),
            _server: server,
            _fakes: fakes,
            _database: database,
        }
    }

    /// Raw SQL access to this test's database.
    pub fn pool(&self) -> &PgPool {
        &self.state.pool
    }

    /// A second app on the same database (and the same fakes) built with
    /// other settings — the pytest `spawn_server` fixture. It stops when the
    /// returned value is dropped.
    pub async fn spawn_server(&self, configure: impl FnOnce(&mut Settings)) -> SpawnedServer {
        let mut settings = self.settings.clone();
        configure(&mut settings);
        let state = AppState::new(self.state.pool.clone(), settings).expect("app state");
        let (api, server) = serve(state).await;
        SpawnedServer { api, _server: server }
    }

    /// Runs the server's real seed (what `aurum-server seed` does) against
    /// this test's database now, with the given settings changes (e.g.
    /// `default_currency = "GBP"`) — the pytest `run_seed` fixture.
    pub async fn run_seed(&self, configure: impl FnOnce(&mut Settings)) {
        let mut settings = self.settings.clone();
        configure(&mut settings);
        aurum_server::db::seed::run(self.pool(), &settings).await.expect("seed");
    }

    /// The default seeded account (see db/seed.rs) — every transaction
    /// needs one, and the app itself always seeds exactly this one on first
    /// boot, so tests build on the same shape real usage does.
    pub async fn account_id(&self) -> i64 {
        let accounts = self.get("/accounts").await.json();
        let first = accounts.as_array().and_then(|list| list.first()).cloned();
        let first = first.expect("seed_default_account should have created exactly one account");
        first["id"].as_i64().expect("account id")
    }

    /// Default seeded categories, e.g. `categories.id("Groceries")`.
    pub async fn categories(&self) -> Categories {
        let list = self.get("/categories").await.json();
        Categories(list.as_array().cloned().expect("categories list"))
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        // Fails the test afterwards if the server made a CoinGecko call the
        // test had forbidden (the pytest `_clean_database` fixture's check).
        let violations = self.coingecko.violations();
        if !violations.is_empty() && !std::thread::panicking() {
            panic!("{}", violations.join("; "));
        }
    }
}

/// An extra app started by [`TestApp::spawn_server`].
pub struct SpawnedServer {
    api: Api,
    _server: AbortOnDrop,
}

impl std::ops::Deref for SpawnedServer {
    type Target = Api;

    fn deref(&self) -> &Api {
        &self.api
    }
}

/// The seeded categories in API order (a dict keyed by name in pytest).
pub struct Categories(pub Vec<Value>);

impl Categories {
    pub fn get(&self, name: &str) -> &Value {
        self.0.iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("no category named {name:?}"))
    }

    pub fn id(&self, name: &str) -> i64 {
        self.get(name)["id"].as_i64().expect("category id")
    }

    pub fn values(&self) -> &[Value] {
        &self.0
    }
}

// --------------------------------------------------------------------------
// helpers.py
// --------------------------------------------------------------------------

/// Compares amounts by value regardless of whether the API serializes
/// Decimal as a JSON string or number — that's a wire-format detail, not
/// business behavior worth pinning a test to.
pub fn money(value: &Value) -> Decimal {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        other => panic!("not an amount: {other}"),
    };
    text.parse().unwrap_or_else(|_| panic!("not a decimal: {text:?}"))
}

/// `Decimal("...")`.
pub fn dec(text: &str) -> Decimal {
    text.parse().unwrap_or_else(|_| panic!("not a decimal: {text:?}"))
}

/// A valid expense transaction on `account_id`, with `overrides` (a JSON
/// object) layered on top — `txn_payload(account_id, **overrides)`.
pub fn txn_payload(account_id: i64, overrides: Value) -> Value {
    let mut payload = Map::new();
    payload.insert("account_id".into(), account_id.into());
    payload.insert("type".into(), "expense".into());
    payload.insert("amount".into(), "10.00".into());
    payload.insert("description".into(), "test transaction".into());
    payload.insert("date".into(), "2026-01-15".into());
    if let Value::Object(overrides) = overrides {
        payload.extend(overrides);
    } else {
        assert!(overrides.is_null(), "txn_payload overrides must be a JSON object");
    }
    Value::Object(payload)
}

/// Today's date in the server's local time zone (Python's `date.today()`).
pub fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}
