#!/usr/bin/env python3
"""Differential contract check: replay the same HTTP requests against the
Python (FastAPI) server and the Rust server and diff the answers byte for
byte — status, body and the headers that are part of the contract.

Both servers must start from equivalent databases (fresh, migrated and
seeded), because cases run in order and create rows: ids only line up if
both sides see the same history. See PORTING.md ("Comparing against
Python") for how to start the two servers.

    python3 contract_diff.py --python http://127.0.0.1:8001 \
        --rust http://127.0.0.1:8002 cases/tags.json [cases/other.json ...]

A case file is a JSON list of requests:

    {"name": "create tag", "method": "POST", "path": "/api/tags",
     "headers": {"content-type": "application/json"},
     "json": {"name": "Trip"}            # or "body": "<raw text>"
     "mask": ["created_at"]}             # optional: JSON keys to ignore

One file per route group (cases/<group>.json), so porting agents never edit
the same file. Stdlib only; exits 1 on any difference.
"""
import argparse
import http.client
import json
import sys
from urllib.parse import urlsplit

CONTRACT_HEADERS = (
    "content-type",
    "location",
    "allow",
    "vary",
    "access-control-allow-origin",
    "access-control-allow-credentials",
    "access-control-allow-methods",
    "access-control-allow-headers",
    "access-control-max-age",
)


def send(base: str, case: dict) -> tuple[int, dict, bytes]:
    url = urlsplit(base)
    conn = http.client.HTTPConnection(url.hostname, url.port, timeout=30)
    headers = dict(case.get("headers", {}))
    if "json" in case:
        body = json.dumps(case["json"]).encode()
        headers.setdefault("content-type", "application/json")
    elif "body" in case:
        body = case["body"].encode()
    else:
        body = None
    conn.request(case.get("method", "GET"), case["path"], body=body, headers=headers)
    response = conn.getresponse()
    data = response.read()
    picked = {}
    for name in CONTRACT_HEADERS:
        value = response.getheader(name)
        if value is not None:
            picked[name] = value.replace(base, "<BASE>").replace(url.netloc, "<HOST>")
    if response.status == 405:
        # Allow lists the methods of the matched path; ordering differs.
        picked.pop("allow", None)
    conn.close()
    return response.status, picked, data


def mask(value, keys):
    if isinstance(value, dict):
        return {k: ("<masked>" if k in keys else mask(v, keys)) for k, v in value.items()}
    if isinstance(value, list):
        return [mask(v, keys) for v in value]
    return value


def comparable(body: bytes, keys) -> bytes:
    if not keys:
        return body
    try:
        return json.dumps(mask(json.loads(body), set(keys)), separators=(",", ":"), ensure_ascii=False).encode()
    except ValueError:
        return body


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--python", required=True, help="base URL of the FastAPI server")
    parser.add_argument("--rust", required=True, help="base URL of the Rust server")
    parser.add_argument("files", nargs="+")
    args = parser.parse_args()

    failures = 0
    total = 0
    for path in args.files:
        with open(path, encoding="utf-8") as fh:
            cases = json.load(fh)
        for case in cases:
            total += 1
            label = f"{path}: {case.get('name') or case['method'] + ' ' + case['path']}"
            py = send(args.python, case)
            rs = send(args.rust, case)
            keys = case.get("mask", [])
            same = py[0] == rs[0] and py[1] == rs[1] and comparable(py[2], keys) == comparable(rs[2], keys)
            if same:
                print(f"ok    {label}")
                continue
            failures += 1
            print(f"DIFF  {label}")
            print(f"  python: {py[0]} {py[1]}\n          {py[2][:2000]!r}")
            print(f"  rust:   {rs[0]} {rs[1]}\n          {rs[2][:2000]!r}")
    print(f"\n{total - failures}/{total} identical")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
