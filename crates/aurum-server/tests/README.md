# aurum-server API tests

Black-box tests of the HTTP API. They were ported one-to-one from the pytest
suite of the former Python backend (removed in 2.0.0): one module per former
pytest module (`test_accounts.rs` ↔ `test_accounts.py`), one `#[tokio::test]`
per pytest test with the same name, the same requests and the same
assertions. All modules build into a single test binary (`api`, see
`main.rs`). New tests are added here, in the module of their route group.

## Running

Postgres must be reachable; the user needs `CREATE DATABASE`.

```bash
AURUM_POSTGRES_HOST=127.0.0.1 AURUM_POSTGRES_PORT=5432 cargo test -p aurum-server
cargo test -p aurum-server --test api test_crypto::      # one module
cargo test -p aurum-server --test api -- --ignored        # the ignored (stale) test
```

`AURUM_POSTGRES_USER/PASSWORD/HOST/PORT` default to `aurum`/`aurum`/`127.0.0.1`/`5432`.
Without a reachable Postgres every test fails with a message saying so.

## How it works (`common/`)

- Once per run the harness creates a template database `aurum_it_<pid>_template`,
  runs the server's own migrations, truncates every table and runs the
  server's seed (the clean state every test starts from; the former pytest
  harness reset to the same state before each test).
- Each test gets its own database `aurum_it_<pid>_<n>` created from that
  template (`CREATE DATABASE ... TEMPLATE`), so tests run in parallel and
  never see each other's rows. It is dropped when the test ends; databases
  of test processes that no longer run are dropped at the start of the next
  run. The real `aurum` database is never opened.
- The app is `aurum_server::build_app` served in-process on a random port and
  called over real HTTP (base URL ends in `/api`).
- NBP and CoinGecko are fakes on a random port per test (`common/fakes.rs`,
  port of the former `fake_providers.py`): default NBP rates USD 4, EUR 4.25, GBP 5,
  CHF 4.5, UAH 0.1, RUB 0.05 PLN on business days; `serve_rates`, `fail`,
  `requests`; CoinGecko `serve_markets`, `serve_90d_change`, `fail`, `forbid`
  (a forbidden call fails the test), `record_market_calls`/`record_chart_calls`.

## Writing a test

```rust
#[tokio::test]
async fn test_something() {
    let client = TestApp::start().await;           // or start_with(|s| s.cors_origins = ...)
    let account_id = client.account_id().await;    // the seeded account
    let categories = client.categories().await;    // categories.id("Groceries")
    let resp = client.post("/transactions", txn_payload(account_id, json!({"category_id": categories.id("Groceries")}))).await;
    assert_eq!(resp.status, 201);
    assert_eq!(money(&resp.json()["amount"]), dec("10.00"));
}
```

`client.pool()` gives raw SQL access to the test's database,
`client.state` the app state for service-level tests, `client.spawn_server`
a second app with other settings on the same database, and `client.run_seed`
re-runs the seed with other settings.
