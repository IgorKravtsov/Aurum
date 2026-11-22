# Porting a route group from FastAPI to `aurum-server`

The Rust server must be a drop-in replacement for `backend/app`: same paths,
same JSON byte for byte, same status codes, same FastAPI error bodies, same
Postgres schema (ADR 0002), same `AURUM_*` settings. This document is the
contract for everyone porting a route group. The reference implementations
are **tags** (simple CRUD) and **settings** (Decimal, PATCH, 409).

## 1. Where things go — one set of files per group

| What | File (replace `<group>`, e.g. `goals`) | Python source |
|---|---|---|
| Contract types (request/response DTOs) | `crates/aurum-domain/src/<group>.rs` | `app/schemas/<group>.py` |
| HTTP handlers + router | `crates/aurum-server/src/routes/<group>.rs` | `app/api/routes/<group>.py` |
| Business logic | `crates/aurum-server/src/services/<group>.rs` | `app/services/*`, inline route logic |
| SQL | `crates/aurum-server/src/repo/<group>.rs` | the SQLAlchemy queries |
| Differential test cases | `crates/aurum-server/contract/cases/<group>.json` | — |

Every module already exists and is wired in (`routes/mod.rs` merges every
`<group>::router()`), so porting a group only touches its own files.
**Shared files** — `lib.rs`, `routes/mod.rs`, `extract.rs`, `error.rs`,
`response.rs`, `http/*`, `db/*`, `aurum-domain/src/{decimal,validation,pyfmt,enums,text}.rs`,
the `Cargo.toml`s — should not need changes. If you need a new shared helper,
keep it private in your group module and say so in your report so it can be
promoted once; if you need a new crate dependency, add it to
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
`validation::err`, capture the real response from the Python server
(section 8) and add a constructor with the exact `msg`/`ctx`.

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

## 9. Comparing against Python

1. Build: `cargo build -p aurum-server`.
2. Start both servers on fresh, equivalent databases (Python on :8001,
   Rust on :8002):
   `PYTHON_BIN=/path/to/venv/bin/python AURUM_POSTGRES_PORT=5432 crates/aurum-server/contract/start_pair.sh`
   (`PG_CONTAINER=<name>` to run psql through docker; `--stop` to stop).
3. Replay your cases and diff status, body bytes and contract headers:
   `python3 crates/aurum-server/contract/contract_diff.py --python http://127.0.0.1:8001 --rust http://127.0.0.1:8002 crates/aurum-server/contract/cases/<group>.json`
   Cases run in order and share state; restart the pair before a re-run.
   Every file also passes when all of them run on one pair (in the order
   of `routes/mod.rs`, then `backup`, `shared`). Point
   `AURUM_NBP_BASE_URL`/`AURUM_COINGECKO_BASE_URL` at the fakes
   (`cases/crypto_fake.py` serves both) — dashboard/insights/net-worth
   cases need deterministic FX rates. `cases/shared.json` covers the shared
   layer (JSON syntax errors, route order, escapes, `int4` overflow).
   Use `"mask": ["created_at"]` for genuinely time-dependent fields only.
   Cover the happy path, every `HTTPException` branch and a few 422s.
4. The black-box pytest suite runs unchanged against the Rust server:
   `cd backend && AURUM_TEST_SERVER=rust pytest -v` (see `backend/tests/README.md`;
   the harness drives `aurum-server migrate|seed|serve`).
5. The same suite, ported to Rust, runs in-process with `cargo test -p aurum-server`
   (`crates/aurum-server/tests/`, one module per pytest module; see its `README.md`).
   New behaviour gets its test there.

## 10. Known gaps (shared layer)

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
