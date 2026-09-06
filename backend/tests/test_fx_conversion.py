"""Mixed-currency amounts are converted before any cross-account aggregate."""
from datetime import date, datetime, timedelta, timezone
from decimal import Decimal

import pytest
from fastapi import HTTPException

from app.models.fx import FxRate
from app.services import fx_service
from tests.helpers import money, txn_payload

pytestmark = pytest.mark.asyncio


async def test_mixed_currency_aggregates_use_historical_nbp_rates(
    client, test_sessionmaker, account_id, categories, monkeypatch
):
    async def constant_test_rates(currency: str, requested_dates: set[date]):
        rates = {"USD": Decimal("4"), "UAH": Decimal("0.1")}
        return {requested: (requested, rates[currency]) for requested in requested_dates}

    monkeypatch.setattr(fx_service, "_fetch_nbp_currency", constant_test_rates)
    await client.patch("/settings", json={"currency": "PLN"})
    usd_account = (await client.patch(f"/accounts/{account_id}", json={"currency": "USD"})).json()
    uah_account = (
        await client.post("/accounts", json={"name": "UAH card", "type": "checking", "currency": "UAH"})
    ).json()
    pln_account = (
        await client.post("/accounts", json={"name": "PLN account", "type": "checking", "currency": "PLN"})
    ).json()

    async with test_sessionmaker() as session:
        session.add_all(
            [
                FxRate(
                    currency="USD",
                    requested_date=date(2026, 1, 15),
                    effective_date=date(2026, 1, 15),
                    rate_to_pln=Decimal("4.000000000000000000"),
                    provider="test",
                ),
                FxRate(
                    currency="UAH",
                    requested_date=date(2026, 1, 15),
                    effective_date=date(2026, 1, 15),
                    rate_to_pln=Decimal("0.100000000000000000"),
                    provider="test",
                ),
            ]
        )
        await session.commit()

    assert (
        await client.post(
            "/transactions",
            json=txn_payload(
                usd_account["id"],
                type="income",
                amount="100.00",
                category_id=categories["Salary"]["id"],
            ),
        )
    ).status_code == 201
    assert (
        await client.post(
            "/transactions",
            json=txn_payload(
                uah_account["id"],
                amount="1000.00",
                category_id=categories["Groceries"]["id"],
            ),
        )
    ).status_code == 201
    assert (
        await client.post(
            "/transactions",
            json=txn_payload(
                pln_account["id"],
                amount="50.00",
                category_id=categories["Groceries"]["id"],
            ),
        )
    ).status_code == 201
    transfer_payload = txn_payload(
        usd_account["id"],
        type="transfer",
        amount="10.00",
        category_id=None,
        transfer_account_id=uah_account["id"],
    )
    missing_destination = await client.post("/transactions", json=transfer_payload)
    assert missing_destination.status_code == 400
    transfer_payload["destination_amount"] = "390.00"
    transfer = await client.post("/transactions", json=transfer_payload)
    assert transfer.status_code == 201
    assert money(transfer.json()["destination_amount"]) == Decimal("390")

    dashboard = (await client.get("/dashboard/summary", params={"year": 2026, "month": 1})).json()
    assert money(dashboard["real_income"]) == Decimal("400")
    assert money(dashboard["spent"]) == Decimal("150")
    assert money(dashboard["net"]) == Decimal("250")
    assert money(dashboard["spending_by_category"][0]["amount"]) == Decimal("150")

    cash_flow = (await client.get("/cash-flow", params={"start_date": "2026-01-01", "end_date": "2026-01-31"})).json()
    assert money(cash_flow["total_income"]) == Decimal("400")
    assert money(cash_flow["total_expense"]) == Decimal("150")

    await client.post(
        "/budgets",
        json={"category_id": categories["Groceries"]["id"], "monthly_limit": "200.00"},
    )
    budget = (await client.get("/budgets/status", params={"year": 2026, "month": 1})).json()["items"][0]
    assert money(budget["spent"]) == Decimal("150")
    assert budget["percent"] == 75.0

    report = (
        await client.get(
            "/reports/category-spending",
            params={"category_id": categories["Groceries"]["id"], "start_date": "2026-01-01", "end_date": "2026-01-31"},
        )
    ).json()
    assert money(report["total_amount"]) == Decimal("150")

    accounts = {item["name"]: item for item in (await client.get("/accounts")).json()}
    assert money(accounts["Main Account"]["balance"]) == Decimal("90")
    assert money(accounts["UAH card"]["balance"]) == Decimal("-610")
    assert money(accounts["PLN account"]["balance"]) == Decimal("-50")

    net_worth_response = await client.get("/net-worth/summary", params={"range": "30d"})
    assert net_worth_response.status_code == 200, net_worth_response.text
    net_worth = net_worth_response.json()
    assert money(net_worth["current"]) == Decimal("249")
    assert money({item["key"]: item["amount"] for item in net_worth["breakdown"]}["cash"]) == Decimal("249")

    currency_change = await client.patch("/settings", json={"currency": "USD"})
    assert currency_change.status_code == 409


async def test_cached_weekend_rate_survives_backup_without_provider(client, test_sessionmaker, monkeypatch):
    saturday = date(2026, 1, 17)
    async with test_sessionmaker() as session:
        session.add(
            FxRate(
                currency="UAH",
                requested_date=date(2026, 1, 16),
                effective_date=date(2026, 1, 16),
                rate_to_pln=Decimal("0.083000000000000000"),
                provider="test",
            )
        )
        await session.commit()

        async def unavailable(*args, **kwargs):
            raise HTTPException(status_code=503, detail="provider unavailable")

        monkeypatch.setattr(fx_service, "_fetch_nbp_currency", unavailable)
        converted = await fx_service.convert_amounts(
            session,
            [fx_service.MoneyAmount("weekend", Decimal("1000"), "UAH", saturday)],
            "PLN",
        )

    assert converted["weekend"] == Decimal("83")

    backup = (await client.get("/backup/export")).json()
    assert backup["fx_rates"][0]["effective_date"] == "2026-01-16"
    assert (await client.post("/backup/import", json=backup)).status_code == 200
    async with test_sessionmaker() as session:
        restored = await fx_service.convert_amounts(
            session,
            [fx_service.MoneyAmount("restored", Decimal("1000"), "UAH", saturday)],
            "PLN",
        )
    assert restored["restored"] == Decimal("83")


async def test_expired_provisional_rate_is_refreshed(test_sessionmaker, monkeypatch):
    today = date.today()
    async with test_sessionmaker() as session:
        session.add(
            FxRate(
                currency="USD",
                requested_date=today,
                effective_date=today - timedelta(days=1),
                rate_to_pln=Decimal("4"),
                provider="test",
                fetched_at=datetime.now(timezone.utc) - timedelta(hours=2),
            )
        )
        await session.commit()

        async def refreshed_rate(currency: str, requested_dates: set[date]):
            assert currency == "USD"
            assert requested_dates == {today}
            return {today: (today, Decimal("5"))}

        monkeypatch.setattr(fx_service, "_fetch_nbp_currency", refreshed_rate)
        converted = await fx_service.convert_amounts(
            session,
            [fx_service.MoneyAmount("today", Decimal("2"), "USD", today)],
            "PLN",
        )

    assert converted["today"] == Decimal("10")


async def test_recurring_transfer_locks_both_account_currencies(client, account_id):
    destination = (
        await client.post("/accounts", json={"name": "Recurring destination", "type": "savings", "currency": "USD"})
    ).json()
    rule = await client.post(
        "/recurring",
        json={
            "account_id": account_id,
            "category_id": None,
            "transfer_account_id": destination["id"],
            "type": "transfer",
            "amount": "25.00",
            "description": "Monthly transfer",
            "frequency": "monthly",
            "anchor_date": "2026-01-15",
        },
    )
    assert rule.status_code == 201
    assert money(rule.json()["destination_amount"]) == Decimal("25")

    response = await client.patch(f"/accounts/{destination['id']}", json={"currency": "UAH"})

    assert response.status_code == 409


async def test_base_currency_is_immutable_after_transaction_history(client, account_id, categories):
    await client.post(
        "/transactions",
        json=txn_payload(account_id, category_id=categories["Groceries"]["id"]),
    )

    response = await client.patch("/settings", json={"currency": "PLN"})

    assert response.status_code == 409


async def test_account_currency_is_immutable_after_history(client, account_id, categories):
    await client.post(
        "/transactions",
        json=txn_payload(account_id, category_id=categories["Groceries"]["id"]),
    )

    response = await client.patch(f"/accounts/{account_id}", json={"currency": "PLN"})

    assert response.status_code == 409
    assert "cannot be changed" in response.json()["detail"]
