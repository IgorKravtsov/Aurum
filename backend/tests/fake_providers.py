"""Local stand-ins for the two external APIs the server calls — the NBP
exchange-rate API (services/fx_service.py) and CoinGecko
(services/crypto_service.py).

The suite is black-box: the server under test runs as a separate process
(Python or Rust, see conftest.py), so in-process monkeypatching can't reach
it any more. Instead the server is started with AURUM_NBP_BASE_URL /
AURUM_COINGECKO_BASE_URL pointing here, and tests configure what this fake
answers. Response shapes mirror the real APIs closely enough for any client
that parses the real ones:

- NBP: GET {base}/{table}/{code}/{start}/{end}/?format=json ->
  {"table", "currency", "code", "rates": [{"no", "effectiveDate", "mid"}]}
  (also {base}/{table}/{code}/{date}/), 404 "Brak danych" when nothing
  matches, 400 for ranges over 93 days — same as api.nbp.pl.
- CoinGecko: GET {base}/coins/markets, {base}/coins/{id}/market_chart,
  {base}/search — same JSON as api.coingecko.com/api/v3.

One fake server lives for the whole session; `reset()` runs before every
test so a test only ever sees what it configured itself (or the defaults
below).
"""
from __future__ import annotations

import json
import re
import threading
from datetime import date, timedelta
from decimal import Decimal
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import NamedTuple
from urllib.parse import parse_qs, urlsplit

NBP_PREFIX = "/nbp/api/exchangerates/rates"
COINGECKO_PREFIX = "/coingecko/api/v3"
NBP_MAX_RANGE_DAYS = 93

# Deterministic "healthy NBP" used when a test configures nothing: fixed
# PLN mid rates published on every business day, like the real table A.
# A test that mixes currencies without configuring rates therefore gets
# stable numbers instead of whatever the live API said that day.
DEFAULT_NBP_RATES: dict[str, Decimal] = {
    "USD": Decimal("4"),
    "EUR": Decimal("4.25"),
    "GBP": Decimal("5"),
    "CHF": Decimal("4.5"),
    "UAH": Decimal("0.1"),
    "RUB": Decimal("0.05"),
}


class MarketPoint(NamedTuple):
    """Same fields as crypto_service._MarketPoint — what one coin's
    /coins/markets row carries."""

    price: Decimal
    change_1h: Decimal | None
    change_24h: Decimal | None
    change_7d: Decimal | None
    change_30d: Decimal | None = None
    change_1y: Decimal | None = None


def _json_number(value: Decimal | float | None):
    if value is None:
        return None
    value = Decimal(str(value))
    return int(value) if value == value.to_integral_value() else float(value)


class FakeNbp:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        self.rates: dict[str, Decimal] = dict(DEFAULT_NBP_RATES)
        self.status: int | None = None
        self.requests: list[str] = []

    def serve_rates(self, rates: dict[str, str | Decimal]) -> None:
        """Replaces the whole rate table: listed codes are published with a
        constant PLN mid rate on every business day, any other code 404s
        (like a currency NBP doesn't quote)."""
        self.rates = {code.upper(): Decimal(str(rate)) for code, rate in rates.items()}

    def fail(self, status: int = 503) -> None:
        """Simulates an outage: every request answers `status`."""
        self.status = status

    def handle(self, path: str) -> tuple[int, object]:
        self.requests.append(path)
        if self.status is not None:
            return self.status, "Service Unavailable"
        match = re.fullmatch(
            r"/(?P<table>[abc])/(?P<code>[a-z]{3})/(?P<start>\d{4}-\d{2}-\d{2})(?:/(?P<end>\d{4}-\d{2}-\d{2}))?/?",
            path,
            flags=re.IGNORECASE,
        )
        if match is None:
            return 400, "400 BadRequest - Błędny zakres dat / Invalid date range"
        table = match["table"].lower()
        code = match["code"].upper()
        start = date.fromisoformat(match["start"])
        end = date.fromisoformat(match["end"]) if match["end"] else start
        if end < start or (end - start).days > NBP_MAX_RANGE_DAYS:
            return 400, "400 BadRequest - Przekroczony limit 93 dni / Limit of 93 days has been exceeded"
        rate = self.rates.get(code)
        # Everything configured is quoted in table A; table B has nothing.
        if table != "a" or rate is None:
            return 404, "404 NotFound - Not Found - Brak danych"
        rows = []
        day = start
        while day <= end:
            if day.weekday() < 5:
                rows.append(
                    {
                        "no": f"{len(rows) + 1:03d}/A/NBP/{day.year}",
                        "effectiveDate": day.isoformat(),
                        "mid": _json_number(rate),
                    }
                )
            day += timedelta(days=1)
        if not rows:
            return 404, "404 NotFound - Not Found - Brak danych"
        return 200, {"table": "A", "currency": code.lower(), "code": code, "rates": rows}


class FakeCoinGecko:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        self.markets: dict[str, MarketPoint] = {}
        self.changes_90d: dict[str, float] = {}
        self.status: int | None = None
        self.forbidden_reason: str | None = None
        self.violations: list[str] = []
        self.market_calls: list[list[str]] = []
        self.chart_calls: list[str] = []

    def serve_markets(self, prices: dict[str, MarketPoint]) -> None:
        """What /coins/markets returns from now on: one row per requested id
        that's in `prices`; ids not listed are simply absent from the
        response (CoinGecko's own behavior for an unknown id)."""
        self.markets = dict(prices)
        self.status = None
        self.forbidden_reason = None

    def serve_90d_change(self, changes: dict[str, float]) -> None:
        """/coins/{id}/market_chart?days=90 returns a two-point price series
        whose first->last change is exactly `changes[id]` percent; any other
        id 404s."""
        self.changes_90d = dict(changes)
        self.status = None
        self.forbidden_reason = None

    def fail(self, status: int = 503) -> None:
        """Simulates an outage/rate limit: every request answers `status`."""
        self.status = status
        self.forbidden_reason = None

    def forbid(self, reason: str) -> None:
        """Any CoinGecko request from now on is a test failure: it's recorded
        in `violations` (checked after the test by conftest.py) and answered
        with HTTP 500."""
        self.forbidden_reason = reason

    def record_market_calls(self) -> list[list[str]]:
        """Starts a fresh log of /coins/markets calls (each entry is that
        call's list of ids) and returns it; it fills in as the server calls."""
        self.market_calls = []
        return self.market_calls

    def record_chart_calls(self) -> list[str]:
        """Same as record_market_calls, for /coins/{id}/market_chart (one
        coin id per call)."""
        self.chart_calls = []
        return self.chart_calls

    def handle(self, path: str, query: dict[str, list[str]]) -> tuple[int, object]:
        if self.forbidden_reason is not None:
            self.violations.append(f"{self.forbidden_reason} (got GET {path})")
            return 500, {"error": "forbidden by test"}
        if path == "/coins/markets":
            ids = [cid for cid in ",".join(query.get("ids", [])).split(",") if cid]
            self.market_calls.append(ids)
            if self.status is not None:
                return self.status, {"error": "simulated outage"}
            rows = []
            for cid in ids:
                point = self.markets.get(cid)
                if point is None:
                    continue
                rows.append(
                    {
                        "id": cid,
                        "symbol": cid[:3],
                        "name": cid.title(),
                        "current_price": _json_number(point.price),
                        "price_change_percentage_1h_in_currency": _json_number(point.change_1h),
                        "price_change_percentage_24h_in_currency": _json_number(point.change_24h),
                        "price_change_percentage_7d_in_currency": _json_number(point.change_7d),
                        "price_change_percentage_30d_in_currency": _json_number(point.change_30d),
                        "price_change_percentage_1y_in_currency": _json_number(point.change_1y),
                    }
                )
            return 200, rows
        chart = re.fullmatch(r"/coins/(?P<id>[^/]+)/market_chart", path)
        if chart is not None:
            cid = chart["id"]
            self.chart_calls.append(cid)
            if self.status is not None:
                return self.status, {"error": "simulated outage"}
            change = self.changes_90d.get(cid)
            if change is None:
                return 404, {"error": "coin not found"}
            first = 100.0
            last = first * (1 + change / 100)
            day_ms = 86_400_000
            return 200, {
                "prices": [[0, first], [90 * day_ms, last]],
                "market_caps": [],
                "total_volumes": [],
            }
        if path == "/search":
            if self.status is not None:
                return self.status, {"error": "simulated outage"}
            return 200, {"coins": [], "exchanges": [], "categories": [], "nfts": []}
        return 404, {"error": "Not Found"}


class FakeProviders:
    """Owns the fake HTTP server thread and both fakes' state."""

    def __init__(self) -> None:
        self.nbp = FakeNbp()
        self.coingecko = FakeCoinGecko()
        self._lock = threading.Lock()
        providers = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self) -> None:  # noqa: N802 — http.server's naming
                url = urlsplit(self.path)
                with providers._lock:
                    if url.path.startswith(NBP_PREFIX):
                        status, body = providers.nbp.handle(url.path[len(NBP_PREFIX):])
                    elif url.path.startswith(COINGECKO_PREFIX):
                        status, body = providers.coingecko.handle(
                            url.path[len(COINGECKO_PREFIX):], parse_qs(url.query)
                        )
                    else:
                        status, body = 404, "unknown fake provider"
                if isinstance(body, str):
                    payload, content_type = body.encode(), "text/plain; charset=utf-8"
                else:
                    payload, content_type = json.dumps(body).encode(), "application/json; charset=utf-8"
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def log_message(self, format, *args) -> None:  # noqa: A002 — keep pytest output clean
                pass

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._server.daemon_threads = True
        self._thread = threading.Thread(target=self._server.serve_forever, name="fake-providers", daemon=True)
        self._thread.start()

    @property
    def root_url(self) -> str:
        host, port = self._server.server_address[:2]
        return f"http://{host}:{port}"

    @property
    def nbp_base_url(self) -> str:
        return f"{self.root_url}{NBP_PREFIX}"

    @property
    def coingecko_base_url(self) -> str:
        return f"{self.root_url}{COINGECKO_PREFIX}"

    def reset(self) -> None:
        with self._lock:
            self.nbp.reset()
            self.coingecko.reset()

    def close(self) -> None:
        self._server.shutdown()
        self._server.server_close()
