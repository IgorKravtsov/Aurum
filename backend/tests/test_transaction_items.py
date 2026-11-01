"""Receipt line items (models/transaction.py's TransactionItem): creating,
reading, PATCH replace/untouched/clear semantics, the transfer rule, bulk
create, backup round trip and the per-product spending report.

Product names here are neutral examples, not real receipts.
"""
from datetime import date
from decimal import Decimal

import pytest
from httpx import AsyncClient

from app.services import fx_service
from tests.helpers import money, txn_payload

pytestmark = pytest.mark.asyncio

RECEIPT_ITEMS = [
    {"name": "Milk 2%", "quantity": "2", "unit": "szt", "unit_price": "3.50", "amount": "7.00"},
    {"name": "Apples", "quantity": "1.250", "unit": "kg", "unit_price": "4.00", "discount": "0.50", "amount": "4.50"},
    {"name": "Returnable packaging", "amount": "-0.50"},
]


@pytest.fixture(autouse=True)
def _offline_fx(monkeypatch):
    """Reports convert through the FX seam; keep these tests off the
    network with a constant rate (every account here shares one currency,
    so the rate value never changes a total)."""

    async def constant_rates(currency: str, requested_dates: set[date]):
        return {requested: (requested, Decimal("4")) for requested in requested_dates}

    monkeypatch.setattr(fx_service, "_fetch_nbp_currency", constant_rates)


async def _create(client: AsyncClient, account_id: int, **overrides) -> dict:
    resp = await client.post("/transactions", json=txn_payload(account_id, **overrides))
    assert resp.status_code == 201, resp.text
    return resp.json()


async def test_create_with_items_returns_them_in_order_with_total(client: AsyncClient, account_id, categories):
    txn = await _create(
        client, account_id, amount="12.00", category_id=categories["Groceries"]["id"], items=RECEIPT_ITEMS
    )

    assert [item["name"] for item in txn["items"]] == ["Milk 2%", "Apples", "Returnable packaging"]
    assert [item["position"] for item in txn["items"]] == [0, 1, 2]
    apples = txn["items"][1]
    assert money(apples["quantity"]) == money("1.250")
    assert apples["unit"] == "kg"
    assert money(apples["discount"]) == money("0.50")
    # Defaults: quantity 1, discount 0, no unit/unit price.
    deposit = txn["items"][2]
    assert money(deposit["quantity"]) == money("1")
    assert money(deposit["discount"]) == money("0")
    assert deposit["unit"] is None and deposit["unit_price"] is None
    # Partial receipts are fine: 11.00 of items on a 12.00 transaction.
    assert money(txn["items_total"]) == money("11.00")


async def test_transaction_without_items_has_null_items_total(client: AsyncClient, account_id):
    txn = await _create(client, account_id)
    assert txn["items"] == []
    assert txn["items_total"] is None


async def test_items_are_returned_by_the_list_endpoint(client: AsyncClient, account_id):
    created = await _create(client, account_id, items=RECEIPT_ITEMS[:1])
    listed = (await client.get("/transactions")).json()["items"]
    row = next(t for t in listed if t["id"] == created["id"])
    assert [item["name"] for item in row["items"]] == ["Milk 2%"]
    assert money(row["items_total"]) == money("7.00")


async def test_search_matches_item_names(client: AsyncClient, account_id):
    wanted = await _create(client, account_id, description="Weekly shop", items=RECEIPT_ITEMS[:2])
    await _create(client, account_id, description="Other shop")
    found = (await client.get("/transactions", params={"search": "apple"})).json()["items"]
    assert [t["id"] for t in found] == [wanted["id"]]


async def test_create_rejects_items_on_transfer(client: AsyncClient, account_id):
    other = (await client.post("/accounts", json={"name": "Savings", "type": "savings", "currency": "USD"})).json()
    resp = await client.post(
        "/transactions",
        json=txn_payload(account_id, type="transfer", transfer_account_id=other["id"], items=RECEIPT_ITEMS[:1]),
    )
    assert resp.status_code == 422
    assert "items are not valid for transfer transactions" in resp.text


async def test_create_rejects_unknown_or_mismatched_item_category(client: AsyncClient, account_id, categories):
    unknown = await client.post("/transactions", json=txn_payload(account_id, items=[{**RECEIPT_ITEMS[0], "category_id": 999999}]))
    assert unknown.status_code == 400
    assert "Category not found" in unknown.text

    wrong_kind = await client.post(
        "/transactions",
        json=txn_payload(account_id, items=[{**RECEIPT_ITEMS[0], "category_id": categories["Salary"]["id"]}]),
    )
    assert wrong_kind.status_code == 400


async def test_create_rejects_blank_name_and_missing_amount(client: AsyncClient, account_id):
    blank = await client.post("/transactions", json=txn_payload(account_id, items=[{"name": "  ", "amount": "1.00"}]))
    assert blank.status_code == 422
    missing_amount = await client.post("/transactions", json=txn_payload(account_id, items=[{"name": "Bread"}]))
    assert missing_amount.status_code == 422


async def test_item_category_is_independent_of_splits_and_category(client: AsyncClient, account_id, categories):
    txn = await _create(
        client,
        account_id,
        category_id=categories["Groceries"]["id"],
        items=[{**RECEIPT_ITEMS[0], "category_id": categories["Shopping"]["id"]}],
    )
    assert txn["category_id"] == categories["Groceries"]["id"]
    assert txn["splits"] == []
    assert txn["items"][0]["category"]["id"] == categories["Shopping"]["id"]


async def test_patch_without_items_leaves_them_untouched(client: AsyncClient, account_id):
    txn = await _create(client, account_id, items=RECEIPT_ITEMS)
    resp = await client.patch(f"/transactions/{txn['id']}", json={"description": "Renamed"})
    assert resp.status_code == 200, resp.text
    assert [item["id"] for item in resp.json()["items"]] == [item["id"] for item in txn["items"]]


async def test_patch_with_items_replaces_the_full_set(client: AsyncClient, account_id):
    txn = await _create(client, account_id, items=RECEIPT_ITEMS)
    resp = await client.patch(
        f"/transactions/{txn['id']}", json={"items": [{"name": "Bread", "amount": "2.20"}, {"name": "Butter", "amount": "5.10"}]}
    )
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert [item["name"] for item in body["items"]] == ["Bread", "Butter"]
    assert [item["position"] for item in body["items"]] == [0, 1]
    assert money(body["items_total"]) == money("7.30")


async def test_patch_with_empty_list_clears_items(client: AsyncClient, account_id):
    txn = await _create(client, account_id, items=RECEIPT_ITEMS)
    resp = await client.patch(f"/transactions/{txn['id']}", json={"items": []})
    assert resp.status_code == 200, resp.text
    assert resp.json()["items"] == []
    assert resp.json()["items_total"] is None


async def test_patch_rejects_turning_a_transaction_with_items_into_a_transfer(client: AsyncClient, account_id):
    other = (await client.post("/accounts", json={"name": "Savings", "type": "savings", "currency": "USD"})).json()
    txn = await _create(client, account_id, items=RECEIPT_ITEMS[:1])
    resp = await client.patch(
        f"/transactions/{txn['id']}", json={"type": "transfer", "transfer_account_id": other["id"]}
    )
    assert resp.status_code == 400
    assert "items are not valid for transfer transactions" in resp.text

    # Clearing the items in the same patch makes the conversion valid.
    resp = await client.patch(
        f"/transactions/{txn['id']}", json={"type": "transfer", "transfer_account_id": other["id"], "items": []}
    )
    assert resp.status_code == 200, resp.text


async def test_patch_rejects_items_on_existing_transfer(client: AsyncClient, account_id):
    other = (await client.post("/accounts", json={"name": "Savings", "type": "savings", "currency": "USD"})).json()
    transfer = await _create(client, account_id, type="transfer", transfer_account_id=other["id"])
    resp = await client.patch(f"/transactions/{transfer['id']}", json={"items": RECEIPT_ITEMS[:1]})
    assert resp.status_code == 400
    assert "items are not valid for transfer transactions" in resp.text


async def test_deleting_a_transaction_cascades_its_items(client: AsyncClient, account_id):
    txn = await _create(client, account_id, items=RECEIPT_ITEMS)
    assert (await client.delete(f"/transactions/{txn['id']}")).status_code == 204
    report = (await client.get("/reports/item-spending")).json()
    assert report["items"] == []


async def test_bulk_create_accepts_items(client: AsyncClient, account_id):
    resp = await client.post(
        "/transactions/bulk",
        json={
            "items": [
                txn_payload(account_id, description="With receipt", items=RECEIPT_ITEMS),
                txn_payload(account_id, description="Without receipt"),
            ]
        },
    )
    assert resp.status_code == 201, resp.text
    listed = {t["description"]: t for t in (await client.get("/transactions")).json()["items"]}
    assert len(listed["With receipt"]["items"]) == 3
    assert listed["Without receipt"]["items"] == []


async def test_bulk_create_rejects_items_on_transfer(client: AsyncClient, account_id):
    other = (await client.post("/accounts", json={"name": "Savings", "type": "savings", "currency": "USD"})).json()
    resp = await client.post(
        "/transactions/bulk",
        json={"items": [txn_payload(account_id, type="transfer", transfer_account_id=other["id"], items=RECEIPT_ITEMS)]},
    )
    assert resp.status_code == 422
    assert (await client.get("/transactions")).json()["total"] == 0


async def test_backup_roundtrip_preserves_items(client: AsyncClient, account_id, categories):
    txn = await _create(
        client, account_id, items=[{**RECEIPT_ITEMS[0], "category_id": categories["Groceries"]["id"]}, *RECEIPT_ITEMS[1:]]
    )
    payload = (await client.get("/backup/export")).json()
    assert len(payload["transaction_items"]) == 3

    resp = await client.post("/backup/import", json=payload)
    assert resp.status_code == 200, resp.text
    restored = (await client.get("/transactions")).json()["items"][0]
    assert restored["id"] == txn["id"]
    assert [item["name"] for item in restored["items"]] == ["Milk 2%", "Apples", "Returnable packaging"]
    assert restored["items"][0]["category_id"] == categories["Groceries"]["id"]
    assert money(restored["items_total"]) == money("11.00")

    # The id sequence was advanced: a new item doesn't collide with restored ids.
    added = await client.patch(f"/transactions/{txn['id']}", json={"items": [*RECEIPT_ITEMS, {"name": "Bread", "amount": "2.00"}]})
    assert added.status_code == 200, added.text


async def test_backup_without_transaction_items_still_imports(client: AsyncClient, account_id):
    await _create(client, account_id, items=RECEIPT_ITEMS)
    payload = (await client.get("/backup/export")).json()
    del payload["transaction_items"]
    resp = await client.post("/backup/import", json=payload)
    assert resp.status_code == 200, resp.text
    assert (await client.get("/transactions")).json()["items"][0]["items"] == []


async def test_backup_rejects_item_with_unknown_transaction(client: AsyncClient, account_id):
    await _create(client, account_id, items=RECEIPT_ITEMS[:1])
    payload = (await client.get("/backup/export")).json()
    payload["transaction_items"][0]["transaction_id"] = 999999
    resp = await client.post("/backup/import", json=payload)
    assert resp.status_code == 400
    assert "unknown transaction_id" in resp.text


async def test_item_spending_report_groups_by_normalized_name(client: AsyncClient, account_id, categories):
    await _create(
        client,
        account_id,
        date="2026-01-10",
        items=[
            {"name": "Milk 2%", "quantity": "2", "unit": "szt", "unit_price": "3.50", "amount": "7.00"},
            {"name": "Bread", "amount": "2.00"},
        ],
    )
    await _create(
        client,
        account_id,
        date="2026-02-05",
        items=[{"name": "  MILK 2% ", "quantity": "1", "unit": "szt", "unit_price": "3.60", "discount": "0.10", "amount": "3.50"}],
    )
    # Income items are excluded from a spending report.
    await _create(
        client,
        account_id,
        type="income",
        category_id=categories["Salary"]["id"],
        date="2026-02-06",
        items=[{"name": "Milk 2%", "amount": "100.00"}],
    )

    report = (await client.get("/reports/item-spending")).json()

    assert report["product_count"] == 2
    assert report["purchase_count"] == 2
    assert money(report["total_amount"]) == money("12.50")
    milk, bread = report["items"]
    assert milk["name"] == "MILK 2%"  # spelling from the latest purchase, trimmed
    assert money(milk["total_amount"]) == money("10.50")
    assert money(milk["total_quantity"]) == money("3")
    assert milk["unit"] == "szt"
    assert milk["purchase_count"] == 2
    assert money(milk["avg_unit_price"]) == money("3.50")
    assert milk["last_date"] == "2026-02-05"
    assert bread["name"] == "Bread"
    assert bread["unit"] is None


async def test_item_spending_report_filters(client: AsyncClient, account_id):
    other = (await client.post("/accounts", json={"name": "Card", "type": "checking", "currency": "USD"})).json()
    await _create(client, account_id, date="2026-01-10", items=[{"name": "Coffee beans", "amount": "20.00"}])
    await _create(client, account_id, date="2026-03-10", items=[{"name": "Coffee filters", "amount": "5.00"}])
    await _create(client, other["id"], date="2026-03-11", items=[{"name": "Tea", "amount": "8.00"}])

    by_search = (await client.get("/reports/item-spending", params={"search": "COFFEE"})).json()
    assert {p["name"] for p in by_search["items"]} == {"Coffee beans", "Coffee filters"}

    by_dates = (
        await client.get("/reports/item-spending", params={"start_date": "2026-03-01", "end_date": "2026-03-31"})
    ).json()
    assert {p["name"] for p in by_dates["items"]} == {"Coffee filters", "Tea"}

    by_account = (await client.get("/reports/item-spending", params={"account_id": other["id"]})).json()
    assert [p["name"] for p in by_account["items"]] == ["Tea"]

    limited = (await client.get("/reports/item-spending", params={"limit": 1})).json()
    assert [p["name"] for p in limited["items"]] == ["Coffee beans"]
    # Totals still cover every product, not just the returned page.
    assert limited["product_count"] == 3
    assert money(limited["total_amount"]) == money("33.00")


async def test_item_spending_report_empty(client: AsyncClient):
    report = (await client.get("/reports/item-spending")).json()
    assert report["items"] == []
    assert report["product_count"] == 0
    assert money(report["total_amount"]) == money("0")
