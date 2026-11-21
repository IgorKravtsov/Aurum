# Backend tests

Real integration tests against a real Postgres database — a dedicated
`aurum_test` database created fresh, migrated, and dropped again on every
run. They never touch the real `aurum` database or its data: every process
the harness starts is given `AURUM_POSTGRES_DB=aurum_test` explicitly.

The suite is **black-box**: it starts the backend as a separate server
process and talks to it over HTTP only, so the same tests run unchanged
against the Python (FastAPI) backend and the Rust rewrite
(`crates/aurum-server`). Pick the implementation with `AURUM_TEST_SERVER`:

| `AURUM_TEST_SERVER` | Server under test |
|---|---|
| `python` (default) | `uvicorn app.main:app` from `backend/` |
| `rust` | `target/debug/aurum-server` (override with `AURUM_SERVER_BIN`) |

External APIs are never called: the server is pointed at a local fake NBP /
CoinGecko (`tests/fake_providers.py`) via `AURUM_NBP_BASE_URL` /
`AURUM_COINGECKO_BASE_URL`.

## Running

Postgres must be reachable with the usual `AURUM_POSTGRES_USER/PASSWORD/HOST/PORT`
(from the environment or `backend/.env`); the harness connects to the
`postgres` maintenance database to create/drop `aurum_test`.

### On the host (Python venv)

```bash
cd backend
pip install -r requirements-dev.txt
AURUM_POSTGRES_HOST=127.0.0.1 AURUM_POSTGRES_PORT=5432 pytest -v
```

### Against the Rust server

```bash
cargo build -p aurum-server          # from the repo root
cd backend
AURUM_TEST_SERVER=rust AURUM_POSTGRES_HOST=127.0.0.1 AURUM_POSTGRES_PORT=5432 pytest -v
```

Tests marked `python_internal` (they call Python service code in-process)
are skipped in this mode.

### Inside the compose stack

The stack must already be up (`docker compose up -d`). Test dependencies
aren't baked into the production image, so install them once per container
lifetime, then run pytest inside the running `backend` container:

```bash
docker compose exec backend pip install --user -r requirements-dev.txt
docker compose exec backend python -m pytest -v
```

Re-run just the `python -m pytest -v` line for subsequent runs; the
`pip install` only needs repeating after a container restart/rebuild.

Two details that both come from the container running as an unprivileged user
(`aurum`, uid 10001 — see backend/Dockerfile) rather than root:

- `--user` on the install: site-packages belongs to root and isn't writable.
  The packages land in `/home/aurum/.local`, which isn't on `PATH`, hence
  `python -m pytest` instead of a bare `pytest`.
- pytest prints a `PytestCacheWarning` about not being able to write
  `/app/.pytest_cache`. Expected — the source tree is read-only to the app
  user by design. Tests still pass; add `-p no:cacheprovider` to silence it.

## Server process contract

Both implementations provide three commands, configured only through
environment variables:

| Command | Python | Rust |
|---|---|---|
| `migrate` — create/upgrade the schema | `alembic upgrade head` | `aurum-server migrate` |
| `seed` — default categories, account, `app_settings` row (idempotent) | `python -m app.db.seed` | `aurum-server seed` |
| `serve` — HTTP API under `/api` | `uvicorn app.main:app --host $AURUM_HOST --port $AURUM_PORT` | `aurum-server serve` |

Variables: `AURUM_POSTGRES_USER/PASSWORD/HOST/PORT/DB`, `AURUM_DEFAULT_CURRENCY`,
`AURUM_CORS_ORIGINS`, `AURUM_COINGECKO_API_KEY`, `AURUM_ENABLE_DOCS`,
`AURUM_HOST` (default `0.0.0.0`), `AURUM_PORT` (default `8000`),
`AURUM_NBP_BASE_URL` (default `https://api.nbp.pl/api/exchangerates/rates`),
`AURUM_COINGECKO_BASE_URL` (default `https://api.coingecko.com/api/v3`).

The harness strips every other `AURUM_*` variable from the server's
environment and runs it outside `backend/`, so a developer's own `.env`
(real CoinGecko key, CORS origins, …) never leaks into a test server.

How a run goes (`tests/conftest.py`, `tests/server.py`):

1. Drop and create `aurum_test`, run `migrate`.
2. Truncate every table, run `seed` once and snapshot what it wrote.
3. Start one `serve` process on a free port for the whole session, with
   `AURUM_DEFAULT_CURRENCY=USD`, a dummy `AURUM_COINGECKO_API_KEY` and the
   fake provider URLs.
4. Before each test: `TRUNCATE` all tables (names discovered from
   `information_schema`, migration bookkeeping tables excluded)
   `RESTART IDENTITY CASCADE`, restore the seed snapshot, reset the fakes.

## Adding a test

- Use the `client` fixture (an `httpx.AsyncClient` whose base URL ends in
  `/api`) to hit the API the same way the frontend does — never reach into
  services/models directly, so tests keep verifying the actual contract and
  keep running against both implementations.
- `account_id` and `categories` fixtures give you the same default
  account/categories a real fresh install seeds.
- Every table is truncated and reseeded before each test (see
  `conftest.py::_clean_database`), so tests don't need to worry about
  leftover state or guess at IDs from a previous test.
- External data: configure the fakes per test instead of monkeypatching —
  `fake_nbp.serve_rates({"USD": "4"})` / `fake_nbp.fail(503)`,
  `fake_coingecko.serve_markets({...})`, `.serve_90d_change({...})`,
  `.fail(503)`, `.forbid("reason")` (any call fails the test),
  `.record_market_calls()` / `.record_chart_calls()` (call logs). With
  nothing configured, NBP publishes fixed rates for a few currencies
  (`DEFAULT_NBP_RATES`) on business days and CoinGecko knows no coins.
- A test that needs the server started with different env:
  `@pytest.mark.server_env(AURUM_X="...")` gives its `client` a dedicated
  server, or use the `spawn_server` fixture directly. `run_seed(**env)` runs
  the real `seed` command; `test_sessionmaker` gives raw SQL access to
  `aurum_test` (use `sqlalchemy.text`, not the app's ORM models).
- Mark anything that has to call Python code in-process with
  `@pytest.mark.python_internal`; it's skipped against the Rust server.
