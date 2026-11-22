#!/usr/bin/env python3
"""Fake CoinGecko for the crypto contract cases (cases/crypto*.json).

Both servers of the pair must talk to the *same* fake so they see the same
market data. This starts backend/tests/fake_providers.py (the fake the
pytest suite uses) plus a tiny control endpoint that switches scenarios
between case files:

    python3 crypto_fake.py --ctl-port 8049 --url-file /tmp/cg.url &
    export AURUM_COINGECKO_BASE_URL=$(cat /tmp/cg.url) AURUM_COINGECKO_API_KEY=test-key
    PY_PORT=8041 RS_PORT=8042 ... contract/start_pair.sh
    contract_diff.py ... cases/crypto.json
    curl -s http://127.0.0.1:8049/outage
    contract_diff.py ... cases/crypto_outage.json
    # restart the pair with AURUM_COINGECKO_API_KEY= for cases/crypto_nokey.json

Scenarios: /healthy (the default: bitcoin, ethereum, solana quoted;
90-day charts for bitcoin and ethereum only) and /outage (every request 503).
"""
import argparse
import importlib.util
import threading
from decimal import Decimal
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
_spec = importlib.util.spec_from_file_location("fake_providers", REPO / "backend" / "tests" / "fake_providers.py")
fake_providers = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fake_providers)
MarketPoint = fake_providers.MarketPoint


def healthy(providers) -> None:
    with providers._lock:
        providers.coingecko.reset()
        providers.coingecko.serve_markets(
            {
                "bitcoin": MarketPoint(
                    Decimal("50000"), Decimal("0.5"), Decimal("-1.25"), Decimal("3"), Decimal("10.5"), Decimal("120")
                ),
                "ethereum": MarketPoint(Decimal("2500.123456789"), None, Decimal("2.333333"), None),
                "solana": MarketPoint(Decimal("0.000012345"), Decimal("-0.00001"), None, None, None, Decimal("-75.5")),
            }
        )
        providers.coingecko.serve_90d_change({"bitcoin": 25.0, "ethereum": -12.345})


def outage(providers) -> None:
    with providers._lock:
        providers.coingecko.fail(503)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--ctl-port", type=int, default=8049)
    parser.add_argument("--url-file", required=True, help="where to write the CoinGecko base URL")
    args = parser.parse_args()

    providers = fake_providers.FakeProviders()
    healthy(providers)
    Path(args.url_file).write_text(providers.coingecko_base_url)
    actions = {"/healthy": healthy, "/outage": outage}

    class Control(BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802 — http.server's naming
            action = actions.get(self.path)
            if action is not None:
                action(providers)
            self.send_response(200 if action else 404)
            self.end_headers()

        def log_message(self, format, *args) -> None:  # noqa: A002
            pass

    server = ThreadingHTTPServer(("127.0.0.1", args.ctl_port), Control)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    threading.Event().wait()


if __name__ == "__main__":
    main()
