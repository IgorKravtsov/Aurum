# Developing `aurum-server`: conventions of the Python-compatible API

`aurum-server` is the Rust backend of Aurum. Since 2.0.0 it is the only one:
the FastAPI implementation (`backend/`) it was ported from, and the
differential harness that proved the two byte-identical, were removed. The
HTTP contract did not change: same paths, same JSON byte for byte, same status
codes, same FastAPI-shaped error bodies, same Postgres schema (ADR 0002),
same `AURUM_*` settings, and backups exported by 1.x still restore. This
document keeps what that port learned — the conventions every route group
follows, the parity details that are easy to break, and the known divergences
from the former Python behaviour (section 10) — so a new or changed endpoint
stays consistent with the rest. The reference implementations are **tags**
(simple CRUD) and **settings** (Decimal, PATCH, 409).

Mentions of "Python"/"pydantic"/"SQLAlchemy" below describe the former
implementation whose observable behaviour is the contract; the web frontend,
`aurum-cli` and the mobile clients still depend on it.

## 1. Where things go — one set of files per group

| What | File (replace `<group>`, e.g. `goals`) | Former Python source |
|---|---|---|
| Contract types (request/response DTOs) | `crates/aurum-domain/src/<group>.rs` | `app/schemas/<group>.py` |
| HTTP handlers + router | `crates/aurum-server/src/routes/<group>.rs` | `app/api/routes/<group>.py` |
| Business logic | `crates/aurum-server/src/services/<group>.rs` | `app/services/*`, inline route logic |
| SQL | `crates/aurum-server/src/repo/<group>.rs` | the SQLAlchemy queries |
| Integration tests | `crates/aurum-server/tests/test_<group>.rs` | `tests/test_<group>.py` |

Every module is wired in (`routes/mod.rs` merges every `<group>::router()`),
so changing a group only touches its own files. A new group adds the same
set of files plus one `mod`/`merge` line in `routes/mod.rs`.
**Shared files** — `lib.rs`, `routes/mod.rs`, `extract.rs`, `error.rs`,
`response.rs`, `http/*`, `db/*`, `aurum-domain/src/{decimal,validation,pyfmt,enums,text}.rs`,
the `Cargo.toml`s — should not need changes for a route group change. If you
need a new shared helper, keep it private in your group module and promote it
once a second group needs it; if you need a new crate dependency, add it to
`[workspace.dependencies]` in the root `Cargo.toml` and reference it with
`{ workspace = true }`.

Cross-group reuse (e.g. transactions need the FX conversion of `fx_service`)
goes through the owning group's `services`/`repo` module as a `pub fn`.

## 2. Handlers

```rust
#[utoipa::path(post, path = "/api/tags", tag = "tags", request_body = TagCreate,
    responses((status = 201, body = TagRead), (status = 422, description = "Validation Error")))]
async fn create_tag(State(state): State<AppState>, ApiJson(payload): ApiJson<TagCreate>) -> ApiResult<Response> {
    Ok(created(services::tags::create(&state.pool, payload).await?))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list_tags, create_tag)).routes(routes!(delete_tag))
}
```

* `path` in `#[utoipa::path]` is the **full** path (`/api/...`, axum 0.8
  `{param}` syntax) and is what axum routes on. One `routes!(...)` per
  path; handlers for the same path go in the same `routes!`.
* Status: plain `Json(dto)` for 200, `response::created(dto)` for
  `status_code=201`, `response::no_content()` for 204 (FastAPI still sends
  `content-type: application/json` on it; the helper does too).
* Inputs (`crate::extract`): `ApiPath<P>`, `ApiQuery<Q>`, `ApiJson<B>` for a
  single source; **`Input<P, Q, B>` whenever a route has more than one**
  (e.g. `PATCH /x/{id}` with a body) so a bad id and a bad body are reported
  in one 422, path errors first, like FastAPI. `()` = "no such input".
* Every GET is GET-only: HEAD is answered with 405 by the routing layer,
  as FastAPI does. Unknown paths, 405s and trailing-slash 307 redirects are
  handled globally — don't add routes with a trailing slash.
* Route order: Starlette tries routes in registration order, axum prefers
  static segments. `http/route_table.rs` replays Starlette's decision from
  a table of every route in FastAPI's registration order, so e.g.
  `PATCH /api/transactions/years` reaches `PATCH /{transaction_id}` (422,
  `int_parsing` of `"years"`, plus any body errors) and a 405's `Allow`
  names the first matching route's method only. **Add every new route to
  `route_table::ROUTES`** at the position FastAPI registers it (a unit test
  fails when the table and the router disagree).

## 3. Errors

`ApiResult<T> = Result<T, ApiError>`:

* `raise HTTPException(404, "Tag not found")` → `Err(ApiError::not_found("Tag not found"))`
  (also `bad_request`, `conflict`, `bad_gateway`, `http(status, ..)`,
  `http_json(status, value)` for a non-string `detail`). Copy the `detail`
  text verbatim.
* `?` on `sqlx::Error`/`anyhow::Error` → 500 `Internal Server Error`
  (text/plain), like an uncaught Python exception. An IntegrityError that
  Python lets escape must stay a 500 — don't "improve" it.
* Validation errors come from the extractors; a service that needs to emit
  a 422 itself returns `ApiError::Validation(...)` built with `validation::err::*`.

## 4. Request DTOs (`FromInput`) — reproducing pydantic's 422s

Request bodies, query and path params are validated by hand-written
`FromInput` impls (serde's errors look nothing like pydantic's). Read the
pydantic model top to bottom and call the `Obj` methods **in field order**:

| pydantic | Rust (`o = v.object()?`) |
|---|---|
| `x: str = Field(min_length=1, max_length=50)` | `o.required("x", \|v\| v.str()?.min_len(1)?.max_len(50))` |
| `x: int \| None = None` | `o.optional("x", \|v\| v.int())` |
| `x: T = default` | `o.with_default("x", \|\| default, \|v\| ...)` |
| `x: T \| None` (no default) | `o.required_nullable("x", ...)` |
| PATCH model used with `exclude_unset=True` | `o.patch("x", ...)` → `Patch::{Unset, Null, Value}` |
| `Field(pattern=r"...")` | `.pattern(r"...")` (same regex engine as pydantic-core) |
| `Field(ge=1, le=24)` on int | `.ge(1)?.le(24)` |
| `Decimal = Field(ge=0, max_digits=14, decimal_places=2)` | `v.decimal()?.digits(Some(14), Some(2))?.ge("0")` (digits before bounds, like pydantic) |
| `float` | `v.float()` |
| `bool`, `date` | `v.bool()`, `v.date()` |
| enum / `Literal[...]` | `v.enum_::<AccountType>()` / `v.literal(&["a", "b"])` |
| `list[T]`, nested model | `v.list(\|v\| ...)`, `v.model::<T>()` |
| `@field_validator` transforming | `.map(\|s\| s.to_uppercase())` |
| `@field_validator` raising `ValueError(msg)` | `.check(\|x\| if .. { Err(msg.into()) } else { Ok(()) })` |
| `@model_validator(mode="after")` raising | after `o.finish()?`: `return Err(v.model_value_error(msg))` |

Then `o.finish()?;` and build the struct with `field.value()`. Query
parameters are a `FromInput` struct too (values arrive as strings; lax
coercion handles `"7"` → 7), e.g. `year: int = Query(default_factory=...,
ge=2000, le=2100)` → `o.with_default("year", || today.year() as i64, |v| v.int()?.ge(2000)?.le(2100))`.
Path ids: `i64` (`v.int()`).

Don't trust memory for pydantic wording: if an error type you need isn't in
`validation::err`, take the exact `msg`/`ctx` from pydantic v2's documented
error types (or an existing test that asserts it) and add a constructor with
them, plus a test in `crates/aurum-server/tests`.

## 5. Response DTOs

* `#[derive(Serialize, Deserialize)]` (+ `#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]`).
* **Field order = pydantic's JSON order**: inherited fields first, then the
  subclass's (`AccountWithBalance` = `name, type, currency, color, id, is_archived, balance`).
* Types: `Decimal` → `aurum_domain::Decimal` (JSON string, Python `str()`
  formatting); `float` → `PyFloat` (Python `repr` formatting: `1e-05`,
  `100.0`); `datetime` → `PyDateTime` (`...Z`, micros only when non-zero);
  `date` → `chrono::NaiveDate`; `int` ids → `i32` (DB `integer`).
  `Optional` → `Option<T>`, always serialised as `null` — never
  `skip_serializing_if` on a response.
* `#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]` when a DTO is a
  plain projection of a row; otherwise a private row struct in `repo` and a
  `From` impl.

## 6. Decimal and number rules

* Never use `f64` for money or quantities. Python's `Decimal` keeps its
  exponent and prints it (`"1000.00"` vs `"1000"`), so does ours.
* `numeric` columns decode with their scale (`numeric(14,2)` → `"1000.00"`,
  zero in `numeric(38,18)` → `"0E-18"`, exactly as asyncpg). Values of any
  size fit (it's arbitrary precision).
* Arithmetic: use `a.add(&b)`, `.sub`, `.mul`, `.div` (→ `Option`, `None` on
  zero divisor), `.quantize(-2)` (= `.quantize(Decimal("0.01"))`, ROUND_HALF_EVEN),
  `.round_to_int()`, `.normalize()`, `.abs()`, `.neg()`. They implement
  Python's default context (28 significant digits, ideal exponents), so
  `amount / spent * 100` gives the same digits as Python.
* `float(d)` → `d.to_f64()` (correctly rounded, like CPython).
* Starting values matter for the printed exponent: `sum([])` and
  `Decimal("0")` print `"0"` (`Decimal::zero()`); `Decimal("0.00")` prints
  `"0.00"`. SQL `SUM(numeric(14,2))` keeps scale 2; `COALESCE(SUM(x), 0)`
  is `"0"` when there are no rows. Mirror where Python summed (SQL vs Python).
* Python ints are unbounded; we use `i64` for inputs. A valid *path* id
  outside `int4` is a 500 in both (asyncpg can't encode it; here the
  extractor answers it once validation passed). Beyond `i64` a path id is a
  422 here (500 in Python), and out-of-range ids in query/body still give
  404/400 here vs 500 in Python — accepted divergences.

## 7. Database

* Runtime-checked queries only: `sqlx::query_as::<_, Row>("…")`,
  `query_scalar`, `query` with `&'static str` SQL (no `query!` macros → the
  build never needs a database). Dynamic SQL needs `sqlx::AssertSqlSafe` or
  `QueryBuilder`; prefer static SQL (see `repo::settings::apply_patch` for a
  static partial update).
* **Enums are stored by NAME** (`'EXPENSE'`), the API uses the value
  (`"expense"`). Bind the enum itself (`.bind(kind)`) and decode into it;
  never bind `kind.as_str()`. In raw SQL literals use the uppercase name.
* **ORM-side defaults are not DB defaults.** SQLAlchemy `default=` values
  must be written explicitly in every INSERT: `accounts.is_archived=false,
  type='CHECKING', currency='USD'`; `assets.currency='USD',
  capital_role='NEUTRAL', risk_level='MEDIUM'`; `categories.sort_order=0,
  is_default=false`; `crypto_portfolios.is_archived=false`;
  `recurring_transactions.is_active=true`; `fx_rates.provider='nbp'`;
  `app_settings` (see `db::seed`). `created_at/updated_at` *are* DB defaults.
* `TimestampMixin` tables (accounts, assets, budgets, crypto*, goals,
  recurring_transactions, transactions) have `onupdate=func.now()`: every
  UPDATE must also `SET updated_at = now()`.
* ORM cascades (`cascade="all, delete-orphan"` on Account.transactions,
  Asset.valuations, Goal.contributions, Transaction.splits,
  CryptoHolding.transactions) match the FK `ON DELETE CASCADE`s, so a plain
  `DELETE` of the parent is equivalent. Relationship `order_by`s
  (`AssetValuation.as_of_date`, `GoalContribution.date`,
  `TransactionSplit.id`, `CryptoTransaction.date`) must be reproduced in
  the SQL that loads children.
* Ordering: copy SQLAlchemy's `order_by` into SQL verbatim (collation-
  dependent orderings like `ORDER BY name` must stay in SQL). Where Python
  sorts in Python (`sorted(..., key=...)`), sort in Rust with the same key —
  both sorts are stable; Rust `String` order equals Python `str` order.
* Text: `str.strip()` → `aurum_domain::text::py_strip` (Python treats
  U+001C..U+001F as whitespace, Rust doesn't); `capitalize_first_letter`
  is ported there too.
* Transactions: where Python does several writes before one `commit()`,
  use `pool.begin()` so it stays atomic.

## 8. Upstream providers, audit log

* HTTP: `state.http` (shared `reqwest::Client`), base URLs from
  `state.settings.nbp_base_url` / `coingecko_base_url` (tests point them at
  a fake), timeouts per request like Python (`.timeout(12s)` NBP, `10s`
  CoinGecko), API key `state.settings.coingecko_api_key`.
* Destructive operations: `crate::audit::log_destructive("account.deleted", &[("account_id", &id)])`.

## 9. Running and testing

Prerequisites: a Rust toolchain (stable, MSRV 1.88, see `rust-version` in the
root `Cargo.toml`) and a reachable Postgres 16 whose user may `CREATE DATABASE`
(e.g. `docker run -d -p 5432:5432 -e POSTGRES_USER=aurum -e POSTGRES_PASSWORD=aurum postgres:16-alpine`).

```bash
cargo build -p aurum-server
# Run the server against a database: migrate + seed + listen on AURUM_HOST:AURUM_PORT
AURUM_POSTGRES_HOST=127.0.0.1 cargo run -p aurum-server -- serve
# Or step by step: `aurum-server migrate`, `aurum-server seed`, `aurum-server serve`

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
AURUM_POSTGRES_HOST=127.0.0.1 cargo test --workspace
```

1. The black-box API suite lives in `crates/aurum-server/tests/` (one module
   per route group, see its `README.md`): every test builds the app in-process
   on a fresh database cloned from a migrated + seeded template and talks to
   it over real HTTP. **New or changed behaviour gets its test there** — cover
   the happy path, every `HTTPException` branch and a few 422s.
2. Unit tests of the pure logic sit next to the code in `aurum-domain`
   (validation, `Decimal`, `pyfmt`, backup checks, ...). Tests that need
   deterministic FX rates or CoinGecko data point
   `AURUM_NBP_BASE_URL` / `AURUM_COINGECKO_BASE_URL` at the fakes in
   `tests/common/fakes.rs` (the harness does this for you).
3. `aurum-cli`'s `tests/coverage.rs` compares the CLI commands with
   `http/route_table.rs`: a new endpoint needs a CLI command (and the
   built-in skill/docs update, see `AGENTS.md`) or that test fails.
4. Migrations: schema changes are new numbered files in
   `crates/aurum-server/migrations/` (never edit an applied one — sqlx
   verifies their checksums). They continue from the frozen Alembic baseline
   (ADR 0002): `0001_baseline.sql` is the schema of the last Alembic
   revision, so a database created by 1.x upgrades in place when the new
   image starts, and an existing `alembic_version` table is left untouched.
5. A backup is compatible across releases when the `aurum_backup_version`
   (format) matches; `app_version` is informational only. Fields added later
   must be optional with a default in `FromInput` (see `aurum_domain::backup`)
   so older files — including those exported by 1.x — keep restoring;
   `test_backup_from_1_3_2_restores_in_current_version` pins this.

## 10. Known divergences from the former Python backend

Accepted differences from the 1.x behaviour (everything else was verified
byte-identical against the Python server, response bodies, status codes and
the contract headers, when the port was finished):

* `openapi.json` is generated by utoipa: same paths, not byte-identical to FastAPI's.
* Python `Decimal("-0.00")` (negative zero) can't be represented; prints `0.00`.
* JSON syntax errors: `pyjson.rs` ports CPython's scanner, so every
  `json.JSONDecodeError` message and position matches (fuzzed against
  CPython 3.14). Input Python *accepts* but serde rejects still differs:
  `NaN`/`Infinity` literals, lone `\udxxx` surrogates, floats overflowing
  `f64`; also UTF-16/32 bodies (Python auto-detects, here a 400).
* A 500 carries no CORS headers in either server (Starlette's
  `ServerErrorMiddleware` sits outside `CORSMiddleware`).
* Row order of an export without `ORDER BY` (Postgres heap order) can
  differ after a long shared history (e.g. `fx_rates` inserted through
  different pooled connections); on fresh databases it matches.
* A path id beyond `i64` is a 422 (500 in Python); out-of-range ids in a
  query/body still give 404/400 here vs 500 in Python (section 6).
* One test of the old suite is `#[ignore]`d in `crates/aurum-server/tests`
  because it was already stale in Python (`cargo test -p aurum-server --test
  api -- --ignored` runs it).
