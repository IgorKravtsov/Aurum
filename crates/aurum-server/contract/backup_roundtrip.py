#!/usr/bin/env python3
"""Cross-server backup round trip: a backup file must restore on either
server and an export right after the restore must reproduce it byte for
byte (ADR 0002 — backup files are compatible in both directions).

    python3 backup_roundtrip.py --python http://127.0.0.1:8001 --rust http://127.0.0.1:8002

Needs a fresh pair from start_pair.sh (it writes data into both). The only
byte that may differ between two exports is the `exported_at` timestamp,
which is normalised before comparing — except after a restore done by the
*Python* server: SQLAlchemy writes a transaction's `transaction_tags` rows
in an arbitrary order (it walks the Tag side of the many-to-many through a
set), so its export lists a multi-tag transaction's `tag_ids` in a random
permutation even when restoring its own file. Those comparisons sort each
`tag_ids` array; comparisons after a Rust restore (which keeps file order)
are strict.

1. Python → Rust: seed rich data through the Python API (every table: tags
   on transactions, splits, subcategories, transfers, assets, budgets,
   goals, recurring, settings; crypto and FX rows, which need upstream
   providers to create, go in through a restore), then mutate it so the
   heap order isn't just insertion order. Export P1 from Python, restore it
   into Rust, export R1: R1 == P1. Restore R1 back into Python: P2 == P1.
2. Rust → Python: mutate the restored data through the Rust API, export R2,
   restore it into Python, export P3: P3 == R2; restore P3 into Rust and
   export: == P3. Finally a Rust restore of its own export is stable.

Stdlib only; exits 1 on any difference. Independent of the Rust code: it
only speaks HTTP to both servers.
"""
import argparse
import http.client
import json
import re
import sys
from urllib.parse import urlsplit

EXPORTED_AT = re.compile(rb'"exported_at":"[^"]*"')
TAG_IDS = re.compile(rb'"tag_ids":\[([0-9,]*)\]')


class Server:
    def __init__(self, name: str, base: str):
        self.name = name
        self.url = urlsplit(base)

    def raw(self, method: str, path: str, body=None) -> tuple[int, bytes]:
        conn = http.client.HTTPConnection(self.url.hostname, self.url.port, timeout=60)
        headers = {}
        data = None
        if body is not None:
            data = body if isinstance(body, bytes) else json.dumps(body).encode()
            headers["content-type"] = "application/json"
        conn.request(method, path, body=data, headers=headers)
        response = conn.getresponse()
        payload = response.read()
        conn.close()
        return response.status, payload

    def call(self, method: str, path: str, body=None, expect=(200, 201, 204)):
        status, payload = self.raw(method, path, body)
        if status not in expect:
            sys.exit(f"{self.name}: {method} {path} -> {status}: {payload[:500]!r}")
        return json.loads(payload) if payload else None

    def export(self) -> bytes:
        status, payload = self.raw("GET", "/api/backup/export")
        if status != 200:
            sys.exit(f"{self.name}: export -> {status}: {payload[:500]!r}")
        return payload

    def restore(self, backup: bytes) -> None:
        status, payload = self.raw("POST", "/api/backup/import", backup)
        if status != 200 or json.loads(payload) != {"status": "ok"}:
            sys.exit(f"{self.name}: import -> {status}: {payload[:500]!r}")


def normalised(export: bytes, sort_tags: bool) -> bytes:
    export = EXPORTED_AT.sub(b'"exported_at":"<exported_at>"', export, count=1)
    if sort_tags:
        export = TAG_IDS.sub(
            lambda m: b'"tag_ids":[' + b",".join(sorted(m.group(1).split(b","), key=lambda x: int(x or 0))) + b"]",
            export,
        )
    return export


def compare(label: str, expected: bytes, actual: bytes, python_restored: bool = False) -> bool:
    a, b = normalised(expected, python_restored), normalised(actual, python_restored)
    if python_restored:
        label += " [tag_ids order normalised]"
    if a == b:
        print(f"ok    {label} ({len(a)} bytes)")
        return True
    at = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
    print(f"FAIL  {label}: first difference at byte {at}")
    print(f"  expected: …{a[max(0, at - 120):at + 120].decode(errors='replace')}…")
    print(f"  actual:   …{b[max(0, at - 120):at + 120].decode(errors='replace')}…")
    return False


def seed_python(py: Server) -> None:
    """Every table, through the public API where it can be done offline."""
    acc_main = py.call("POST", "/api/accounts", {"name": "Main", "type": "checking", "currency": "usd"})["id"]
    acc_eur = py.call("POST", "/api/accounts", {"name": "Euro savings", "type": "savings", "currency": "EUR"})["id"]
    acc_old = py.call("POST", "/api/accounts", {"name": "Old card", "type": "credit_card", "color": "#112233"})["id"]
    py.call("PATCH", f"/api/accounts/{acc_old}", {"is_archived": True})

    parent = py.call("POST", "/api/categories", {"name": "Hobby", "kind": "expense", "color": "#e34948"})["id"]
    child = py.call(
        "POST", "/api/categories", {"name": "Books", "kind": "expense", "color": "#7a869a", "parent_id": parent}
    )["id"]
    salary = py.call("POST", "/api/categories", {"name": "Bonus", "kind": "income", "color": "#2a78d6"})["id"]

    trip = py.call("POST", "/api/tags", {"name": "trip"})["id"]
    work = py.call("POST", "/api/tags", {"name": "work"})["id"]
    gift = py.call("POST", "/api/tags", {"name": "gift"})["id"]

    def txn(**fields):
        body = {"type": "expense", "amount": "10.00", "description": "item", "date": "2026-01-15"}
        body.update(fields)
        return py.call("POST", "/api/transactions", body)["id"]

    first = txn(account_id=acc_main, category_id=child, tag_ids=[work, trip], merchant="Shop", notes="multi-line\nnote")
    txn(account_id=acc_main, type="income", amount="2500.50", category_id=salary, description="Bonus", date="2026-02-01")
    txn(
        account_id=acc_main,
        type="transfer",
        transfer_account_id=acc_eur,
        amount="100.00",
        destination_amount="92.37",
        description="To savings",
        date="2026-02-03",
    )
    split = txn(
        account_id=acc_main,
        amount="100.00",
        description="Receipt",
        splits=[
            {"category_id": child, "amount": "70.00"},
            {"category_id": parent, "amount": "30.00", "note": "snacks"},
        ],
        tag_ids=[gift],
    )
    doomed = txn(account_id=acc_eur, amount="1.00", description="Deleted later", tag_ids=[trip])
    txn(account_id=acc_old, amount="0.01", description="Ünïcödé ✓ \"quoted\"", tag_ids=[gift, trip, work])
    # Updates and deletes move rows around in the heap and the association
    # table — the export must carry that order, not the insertion order.
    py.call("PATCH", f"/api/transactions/{first}", {"amount": "12.34", "tag_ids": [trip, gift]})
    py.call("PATCH", f"/api/transactions/{split}", {"description": "Receipt (edited)"})
    py.call("DELETE", f"/api/transactions/{doomed}")

    house = py.call(
        "POST",
        "/api/assets",
        {"name": "Flat", "asset_class": "real_estate", "value": "250000.00", "as_of_date": "2025-12-31",
         "capital_role": "income", "monthly_cash_flow": "900.00", "risk_level": "low"},
    )["id"]
    py.call("POST", f"/api/assets/{house}/valuations", {"value": "255000.00", "as_of_date": "2026-03-31"})
    py.call("POST", "/api/assets", {"name": "Car", "asset_class": "vehicles", "value": "9000", "currency": "EUR"})

    py.call("POST", "/api/budgets", {"category_id": parent, "monthly_limit": "300"})
    goal = py.call("POST", "/api/goals", {"name": "Holiday", "target_amount": "5000", "target_date": "2026-08-01"})["id"]
    py.call("POST", f"/api/goals/{goal}/contributions", {"amount": "250.00", "date": "2026-01-20", "note": "start"})
    py.call("POST", f"/api/goals/{goal}/contributions", {"amount": "-50.00", "date": "2026-02-20"})
    py.call(
        "POST",
        "/api/recurring",
        {"account_id": acc_main, "category_id": child, "type": "expense", "amount": "15.99",
         "description": "Subscription", "frequency": "monthly", "anchor_date": "2026-01-05"},
    )
    py.call("PATCH", "/api/settings", {"negative_cash_flow_threshold_months": 3, "idle_cash_threshold_amount": "2500.5"})

    # Crypto holdings and FX rates are created from upstream providers
    # (CoinGecko, NBP) — they go in through a restore of the export instead.
    backup = json.loads(py.export())
    asset_id = max(a["id"] for a in backup["assets"]) + 1
    backup["assets"].append(
        {"id": asset_id, "name": "Bitcoin", "asset_class": "crypto", "currency": "USD", "notes": None,
         "capital_role": "neutral", "monthly_cash_flow": None, "risk_level": "high"}
    )
    backup["crypto_portfolios"] = [
        {"id": 4, "name": "Long term", "color": "#2a78d6", "is_archived": False},
        {"id": 2, "name": "Archived", "color": None, "is_archived": True},
    ]
    backup["crypto_holdings"] = [
        {"asset_id": asset_id, "portfolio_id": 4, "coingecko_id": "bitcoin", "symbol": "btc", "name": "Bitcoin",
         "thumb_url": "https://example.invalid/btc.png", "last_price": "64000.123456789012345678",
         "price_change_1h": "-0.1234", "price_change_24h": "2.5", "price_change_7d": None,
         "price_change_30d": "10", "price_change_1y": "-3.0001"}
    ]
    backup["crypto_transactions"] = [
        {"id": 7, "asset_id": asset_id, "type": "buy", "quantity": "0.000000000000000001",
         "price_per_unit": "60000", "date": "2026-01-02", "note": None},
        {"id": 3, "asset_id": asset_id, "type": "sell", "quantity": "1E-8", "price_per_unit": "61000.5",
         "date": "2026-01-03", "note": "partial"},
    ]
    backup["fx_rates"] = [
        {"id": 10, "currency": "EUR", "requested_date": "2026-01-04", "effective_date": "2026-01-02",
         "rate_to_pln": "4.2512", "provider": "nbp", "fetched_at": "2026-01-04T10:11:12.123456Z"},
        {"id": 11, "currency": "USD", "requested_date": "2026-01-04", "effective_date": "2026-01-02",
         "rate_to_pln": "3.987654321", "provider": "nbp", "fetched_at": "2026-01-04T12:00:00+02:00"},
    ]
    py.restore(json.dumps(backup).encode())
    # More history after the restore, so ids and sequences keep moving.
    txn(account_id=acc_eur, amount="5.00", description="After restore", tag_ids=[work])


def mutate_rust(rs: Server) -> None:
    """Changes made through the Rust server's own API before exporting from it."""
    accounts = rs.call("GET", "/api/accounts")
    main = next(a["id"] for a in accounts if a["name"] == "Main")
    tags = {t["name"]: t["id"] for t in rs.call("GET", "/api/tags")}
    fresh = rs.call("POST", "/api/tags", {"name": "rust-made"})["id"]
    created = rs.call(
        "POST",
        "/api/transactions",
        {"account_id": main, "type": "expense", "amount": "3.21", "description": "From Rust", "date": "2026-03-01",
         "tag_ids": [fresh, tags["trip"]]},
    )["id"]
    items = rs.call("GET", "/api/transactions")["items"]
    edited = next(t["id"] for t in items if t["description"] == "Bonus")
    rs.call("PATCH", f"/api/transactions/{edited}", {"notes": "edited in Rust", "tag_ids": [tags["work"]]})
    rs.call("PATCH", f"/api/transactions/{created}", {"tag_ids": [tags["gift"], fresh]})
    rs.call("POST", "/api/categories", {"name": "Rust child", "kind": "expense", "color": "#123456",
                                         "parent_id": next(c["id"] for c in rs.call("GET", "/api/categories")
                                                           if c["name"] == "Hobby")})


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--python", required=True, help="base URL of the Python server")
    parser.add_argument("--rust", required=True, help="base URL of the Rust server")
    args = parser.parse_args()
    py, rs = Server("python", args.python), Server("rust", args.rust)

    ok = True
    seed_python(py)
    p1 = py.export()
    rs.restore(p1)
    r1 = rs.export()
    ok &= compare("python export -> rust restore -> rust export", p1, r1)
    py.restore(r1)
    ok &= compare("rust export -> python restore -> python export", p1, py.export(), python_restored=True)

    mutate_rust(rs)
    r2 = rs.export()
    py.restore(r2)
    p3 = py.export()
    ok &= compare("rust-mutated export -> python restore -> python export", r2, p3, python_restored=True)
    rs.restore(p3)
    ok &= compare("python export -> rust restore -> rust export (2)", p3, rs.export())
    # And the Rust restore is stable on its own output.
    r3 = rs.export()
    rs.restore(r3)
    ok &= compare("rust export -> rust restore -> rust export", r3, rs.export())
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
