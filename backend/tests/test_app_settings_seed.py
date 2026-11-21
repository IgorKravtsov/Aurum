"""Regression coverage for seed_default_app_settings() respecting
AURUM_DEFAULT_CURRENCY. An earlier version of the 9f3a2d7c5e11 migration
hardcoded the singleton app_settings row to currency='USD' at migration
time — since this function only creates the row when it's missing, that
pre-existing row silently defeated a non-USD AURUM_DEFAULT_CURRENCY on a
fresh install. The migration no longer inserts the row at all; this
function is now the sole place it gets created.

Exercised black-box: the server's own `seed` command (see tests/server.py)
is run with AURUM_DEFAULT_CURRENCY set, and the row is read back with raw
SQL.
"""
from sqlalchemy import text


async def test_seed_creates_the_row_with_the_configured_currency(test_sessionmaker, run_seed):
    async with test_sessionmaker() as session:
        await session.execute(text("DELETE FROM app_settings"))
        await session.commit()

        # Was: monkeypatch default_currency to GBP + seed_default_app_settings(session).
        run_seed(AURUM_DEFAULT_CURRENCY="GBP")

        settings_row = (await session.execute(text("SELECT currency FROM app_settings WHERE id = 1"))).first()
        assert settings_row is not None
        assert settings_row.currency == "GBP"


async def test_seed_never_overwrites_an_existing_row(test_sessionmaker, run_seed):
    """Self-healing only kicks in when the row is missing — an existing
    row's currency (e.g. one the user changed via Settings after install)
    must never be silently reset just because the app restarted."""
    async with test_sessionmaker() as session:
        await session.execute(text("UPDATE app_settings SET currency = 'EUR' WHERE id = 1"))
        await session.commit()

    # Was: monkeypatch default_currency to GBP + seed_default_app_settings(session).
    run_seed(AURUM_DEFAULT_CURRENCY="GBP")

    async with test_sessionmaker() as session:
        settings_row = (await session.execute(text("SELECT currency FROM app_settings WHERE id = 1"))).first()
        assert settings_row.currency == "EUR"
