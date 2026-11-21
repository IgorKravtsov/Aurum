"""Test harness wiring.

The suite is black-box: it talks HTTP to a real server process — the Python
backend or the Rust rewrite, picked by AURUM_TEST_SERVER=python|rust (see
tests/server.py for the process contract both implement).

Everything here talks to a dedicated ``aurum_test`` Postgres database on the
same server the app already uses — created fresh, migrated with the server's
own ``migrate`` command, and dropped again at the end of the run. The real
``aurum`` database (which holds the user's actual financial history) is
never opened: every process the harness starts gets
AURUM_POSTGRES_DB=aurum_test explicitly, so even the server's own startup
seeding only ever touches the test database. External APIs (NBP, CoinGecko)
are replaced by a local fake (tests/fake_providers.py). See tests/README.md
for how to run this.
"""
import asyncio
import shutil
import tempfile
from collections.abc import AsyncGenerator, Callable, Generator, Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

import asyncpg
import pytest
import pytest_asyncio
from httpx import AsyncClient
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker, create_async_engine

from tests.fake_providers import FakeCoinGecko, FakeNbp, FakeProviders
from tests.server import SERVER_KIND, TEST_DB_NAME, DbConfig, RunningServer, ServerRunner, rust_binary

# Tables the harness never wipes: migration bookkeeping of Alembic (Python)
# and sqlx (Rust).
MIGRATION_TABLES = ("alembic_version", "_sqlx_migrations")

_db = DbConfig.from_environment()


# --------------------------------------------------------------------------
# pytest hooks
# --------------------------------------------------------------------------


def pytest_configure(config: pytest.Config) -> None:
    config.addinivalue_line(
        "markers",
        "server_env(**env): run this test's `client` against a dedicated server started with these "
        "env overrides (a value of None leaves the variable unset)",
    )
    config.addinivalue_line(
        "markers",
        "python_internal: calls Python service code in-process — only runs with AURUM_TEST_SERVER=python",
    )


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    if SERVER_KIND == "python":
        return
    skip = pytest.mark.skip(reason=f"in-process Python internals; not applicable to AURUM_TEST_SERVER={SERVER_KIND}")
    for item in items:
        if item.get_closest_marker("python_internal"):
            item.add_marker(skip)


def pytest_report_header(config: pytest.Config) -> str:
    target = f"rust ({rust_binary()})" if SERVER_KIND == "rust" else "python (uvicorn app.main:app)"
    return f"aurum server under test: {target}; database {TEST_DB_NAME} on {_db.host}:{_db.port}"


# --------------------------------------------------------------------------
# Database bookkeeping (raw SQL only — nothing here knows the schema)
# --------------------------------------------------------------------------


async def _admin_execute(*statements: str) -> None:
    conn = await asyncpg.connect(_db.dsn("postgres"))
    try:
        for statement in statements:
            await conn.execute(statement)
    finally:
        await conn.close()


@dataclass(frozen=True)
class SeedSnapshot:
    """Every row the server's `seed` command writes into an empty database,
    captured once per session. Restoring it after a TRUNCATE ... RESTART
    IDENTITY gives exactly the post-seed state (same rows, same IDs, same
    sequence positions) in one round trip, instead of spawning the seed
    process before each of the ~150 tests."""

    tables: tuple[str, ...]
    rows: dict[str, str]  # table -> json array of its rows, in FK-safe insert order
    sequences: dict[str, int]  # sequence name -> value after seeding


async def _list_tables(conn: asyncpg.Connection) -> tuple[str, ...]:
    records = await conn.fetch(
        """
        SELECT table_name FROM information_schema.tables
        WHERE table_schema = 'public' AND table_type = 'BASE TABLE' AND table_name <> ALL($1::text[])
        ORDER BY table_name
        """,
        list(MIGRATION_TABLES),
    )
    return tuple(record["table_name"] for record in records)


def _truncate_sql(tables: tuple[str, ...]) -> str:
    return "TRUNCATE TABLE " + ", ".join(f'"{table}"' for table in tables) + " RESTART IDENTITY CASCADE"


async def _fk_safe_order(conn: asyncpg.Connection, tables: list[str]) -> list[str]:
    edges = await conn.fetch(
        """
        SELECT child.relname AS child, parent.relname AS parent
        FROM pg_constraint c
        JOIN pg_class child ON child.oid = c.conrelid
        JOIN pg_class parent ON parent.oid = c.confrelid
        WHERE c.contype = 'f' AND c.connamespace = 'public'::regnamespace
        """
    )
    parents = {table: set() for table in tables}
    for edge in edges:
        if edge["child"] in parents and edge["parent"] in parents and edge["child"] != edge["parent"]:
            parents[edge["child"]].add(edge["parent"])
    ordered: list[str] = []
    while parents:
        ready = sorted(table for table, deps in parents.items() if not deps - set(ordered))
        if not ready:
            raise RuntimeError(f"cyclic foreign keys between seeded tables: {sorted(parents)}")
        ordered.extend(ready)
        for table in ready:
            del parents[table]
    return ordered


async def _capture_seed_snapshot(runner: ServerRunner) -> SeedSnapshot:
    conn = await asyncpg.connect(_db.dsn(TEST_DB_NAME))
    try:
        tables = await _list_tables(conn)
        await conn.execute(_truncate_sql(tables))
        await asyncio.to_thread(runner.run, "seed")
        rows: dict[str, str] = {}
        for table in tables:
            payload = await conn.fetchval(f'SELECT json_agg(t) FROM "{table}" t')
            if payload is not None:
                rows[table] = payload
        ordered = await _fk_safe_order(conn, list(rows))
        sequences: dict[str, int] = {}
        columns = await conn.fetch(
            """
            SELECT table_name, column_name,
                   pg_get_serial_sequence(quote_ident(table_name::text), column_name::text) AS sequence
            FROM information_schema.columns
            WHERE table_schema = 'public' AND table_name = ANY($1::text[])
            """,
            list(rows),
        )
        for column in columns:
            if column["sequence"] is None:
                continue
            value = await conn.fetchval(f'SELECT max("{column["column_name"]}") FROM "{column["table_name"]}"')
            if value is not None:
                sequences[column["sequence"]] = int(value)
        return SeedSnapshot(tables=tables, rows={table: rows[table] for table in ordered}, sequences=sequences)
    finally:
        await conn.close()


async def _reset_database(snapshot: SeedSnapshot) -> None:
    conn = await asyncpg.connect(_db.dsn(TEST_DB_NAME))
    try:
        async with conn.transaction():
            await conn.execute(_truncate_sql(snapshot.tables))
            for table, payload in snapshot.rows.items():
                await conn.execute(
                    f'INSERT INTO "{table}" OVERRIDING SYSTEM VALUE '
                    f'SELECT * FROM json_populate_recordset(NULL::"{table}", $1::json)',
                    payload,
                )
            for sequence, value in snapshot.sequences.items():
                await conn.execute("SELECT setval($1::regclass, $2)", sequence, value)
    finally:
        await conn.close()


# --------------------------------------------------------------------------
# Session-wide: database, fake providers, server process
# --------------------------------------------------------------------------


@pytest.fixture(scope="session")
def fake_providers() -> Generator[FakeProviders, None, None]:
    providers = FakeProviders()
    yield providers
    providers.close()


@pytest.fixture(scope="session")
def _workdir() -> Generator[Path, None, None]:
    path = Path(tempfile.mkdtemp(prefix="aurum-tests-"))
    yield path
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture(scope="session")
def server_runner(_workdir: Path, fake_providers: FakeProviders) -> ServerRunner:
    if SERVER_KIND == "rust" and not rust_binary().exists():
        pytest.exit(
            f"Rust server binary not found at {rust_binary()} — build it with "
            "`cargo build -p aurum-server` from the repo root, or point AURUM_SERVER_BIN at it.",
            returncode=4,
        )
    return ServerRunner(
        workdir=_workdir,
        base_env={
            **_db.env(TEST_DB_NAME),
            "AURUM_DEFAULT_CURRENCY": "USD",
            # Any non-empty key: the fake CoinGecko doesn't check it, but the
            # server refuses to call CoinGecko at all without one.
            "AURUM_COINGECKO_API_KEY": "test-key",
            "AURUM_NBP_BASE_URL": fake_providers.nbp_base_url,
            "AURUM_COINGECKO_BASE_URL": fake_providers.coingecko_base_url,
        },
    )


@pytest.fixture(scope="session")
def _test_database(server_runner: ServerRunner) -> Generator[SeedSnapshot, None, None]:
    """Create aurum_test from scratch and run the server's own `migrate`
    against it. Connects to Postgres' always-present `postgres` maintenance
    database to issue CREATE/DROP DATABASE — the real `aurum` database is
    never opened by this fixture.

    Deliberately a *sync* fixture that drives asyncio.run() itself rather
    than an async one: session-scoped async fixtures need to share a loop
    with function-scoped async tests, which pytest-asyncio doesn't do by
    default and led to "attached to a different loop" errors here. A plain
    sync fixture sidesteps the whole question — asyncio.run() opens and
    cleanly closes its own throwaway loop for each call below.
    """
    asyncio.run(
        _admin_execute(
            f'DROP DATABASE IF EXISTS "{TEST_DB_NAME}" WITH (FORCE)',
            f'CREATE DATABASE "{TEST_DB_NAME}"',
        )
    )
    server_runner.run("migrate")
    snapshot = asyncio.run(_capture_seed_snapshot(server_runner))

    yield snapshot

    asyncio.run(_admin_execute(f'DROP DATABASE IF EXISTS "{TEST_DB_NAME}" WITH (FORCE)'))


@pytest.fixture(scope="session")
def server(_test_database: SeedSnapshot, server_runner: ServerRunner) -> Generator[RunningServer, None, None]:
    """The one long-lived server process every test's `client` talks to."""
    running = server_runner.start()
    yield running
    running.stop()


@pytest.fixture
def spawn_server(server_runner: ServerRunner, _test_database) -> Callable[..., object]:
    """For a test that needs the server started with different env (CORS,
    default currency, ...): `with spawn_server(AURUM_X="y") as base_url:`
    runs a short-lived extra server against the same test database. A value
    of None leaves that variable unset."""

    @contextmanager
    def spawn(**overrides: str | None) -> Iterator[str]:
        running = server_runner.start(overrides)
        try:
            yield running.base_url
        finally:
            running.stop()

    return spawn


@pytest.fixture
def run_seed(server_runner: ServerRunner, _test_database) -> Callable[..., None]:
    """Runs the server's real `seed` command now, with optional env
    overrides (e.g. AURUM_DEFAULT_CURRENCY="GBP")."""

    def run(**overrides: str | None) -> None:
        server_runner.run("seed", overrides)

    return run


# --------------------------------------------------------------------------
# Per-test
# --------------------------------------------------------------------------


@pytest_asyncio.fixture
async def test_sessionmaker(_test_database) -> AsyncGenerator[async_sessionmaker[AsyncSession], None]:
    """Direct SQL access to aurum_test for the few tests that set up or
    inspect rows no endpoint exposes (raw SQL via sqlalchemy.text — the
    harness doesn't depend on the Python app's ORM models)."""
    # Function-scoped, not session-scoped: pytest-asyncio gives each test
    # function its own event loop, and an asyncpg engine/pool created under
    # one loop can't be reused from another ("attached to a different
    # loop"). Recreating the engine per test keeps it bound to whichever
    # loop is actually running.
    engine = create_async_engine(_db.sqlalchemy_url(TEST_DB_NAME), pool_pre_ping=True)
    yield async_sessionmaker(bind=engine, expire_on_commit=False, autoflush=False)
    await engine.dispose()


@pytest_asyncio.fixture(autouse=True)
async def _clean_database(_test_database: SeedSnapshot, fake_providers: FakeProviders):
    """Wipe every table and reseed the default categories/account/app
    settings before each test, so tests never see leftovers from a previous
    one and never have to guess at auto-incremented IDs from prior runs.
    The reseed restores what the server's own `seed` command wrote at
    session start (see SeedSnapshot). Also resets the fake NBP/CoinGecko to
    their defaults, and fails the test afterwards if the server made a
    CoinGecko call the test had forbidden."""
    await _reset_database(_test_database)
    fake_providers.reset()
    yield
    violations = list(fake_providers.coingecko.violations)
    if violations:
        pytest.fail("; ".join(violations))


@pytest.fixture
def fake_nbp(fake_providers: FakeProviders) -> FakeNbp:
    return fake_providers.nbp


@pytest.fixture
def fake_coingecko(fake_providers: FakeProviders) -> FakeCoinGecko:
    return fake_providers.coingecko


@pytest_asyncio.fixture
async def client(request: pytest.FixtureRequest, server: RunningServer, server_runner: ServerRunner) -> AsyncGenerator[AsyncClient, None]:
    """httpx client for the API (base URL ends in /api, same paths the
    frontend calls). Talks to the session's server, or — for a test marked
    @pytest.mark.server_env(...) — to a dedicated server started with those
    env overrides for just this test."""
    marker = request.node.get_closest_marker("server_env")
    dedicated = await asyncio.to_thread(server_runner.start, marker.kwargs) if marker else None
    base_url = dedicated.base_url if dedicated else server.base_url
    try:
        async with AsyncClient(base_url=base_url, timeout=30, trust_env=False) as ac:
            yield ac
    finally:
        if dedicated:
            dedicated.stop()


@pytest_asyncio.fixture
async def account_id(client: AsyncClient) -> int:
    """The default seeded account (see app/db/seed.py) — every transaction
    needs one, and the app itself always seeds exactly this one on first
    boot, so tests build on the same shape real usage does."""
    resp = await client.get("/accounts")
    accounts = resp.json()
    assert accounts, "seed_default_account should have created exactly one account"
    return accounts[0]["id"]


@pytest_asyncio.fixture
async def categories(client: AsyncClient) -> dict[str, dict]:
    """Default seeded categories keyed by name, e.g. categories["Groceries"]["id"]."""
    resp = await client.get("/categories")
    return {c["name"]: c for c in resp.json()}
