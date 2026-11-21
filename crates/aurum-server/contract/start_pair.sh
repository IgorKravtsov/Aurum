#!/usr/bin/env bash
# Starts the Python (FastAPI) and the Rust server side by side on two fresh,
# equivalent databases, for contract_diff.py:
#
#   crates/aurum-server/contract/start_pair.sh [cors_origins]   # start (restarts if running)
#   crates/aurum-server/contract/start_pair.sh --stop            # stop both
#
# Python: http://127.0.0.1:${PY_PORT:-8001} on database aurum_contract_py
# Rust:   http://127.0.0.1:${RS_PORT:-8002} on database aurum_contract_rs
#
# Needs: Postgres reachable via AURUM_POSTGRES_HOST/PORT/USER/PASSWORD
# (defaults 127.0.0.1:5432 aurum/aurum), `psql` on PATH or a Postgres docker
# container named by PG_CONTAINER, a Python env with backend/requirements.txt
# (PYTHON_BIN, default `python3`), and `cargo build -p aurum-server` done.
# Logs and pid files go to ${PAIR_DIR:-/tmp/aurum-contract-pair}.
set -u
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
DIR=${PAIR_DIR:-/tmp/aurum-contract-pair}
PY_PORT=${PY_PORT:-8001}
RS_PORT=${RS_PORT:-8002}
PYTHON_BIN=${PYTHON_BIN:-python3}
export AURUM_POSTGRES_HOST=${AURUM_POSTGRES_HOST:-127.0.0.1}
export AURUM_POSTGRES_PORT=${AURUM_POSTGRES_PORT:-5432}
export AURUM_POSTGRES_USER=${AURUM_POSTGRES_USER:-aurum}
export AURUM_POSTGRES_PASSWORD=${AURUM_POSTGRES_PASSWORD:-aurum}
mkdir -p "$DIR"

stop() {
  for f in "$DIR"/py.pid "$DIR"/rs.pid; do
    [ -f "$f" ] && kill "$(cat "$f")" 2>/dev/null
    rm -f "$f"
  done
  sleep 0.5
}

sql() {
  if [ -n "${PG_CONTAINER:-}" ]; then
    docker exec "$PG_CONTAINER" psql -U "$AURUM_POSTGRES_USER" -d postgres -qc "$1"
  else
    PGPASSWORD=$AURUM_POSTGRES_PASSWORD psql -h "$AURUM_POSTGRES_HOST" -p "$AURUM_POSTGRES_PORT" \
      -U "$AURUM_POSTGRES_USER" -d postgres -qc "$1"
  fi
}

stop
[ "${1:-}" = "--stop" ] && exit 0
export AURUM_CORS_ORIGINS=${1:-}

for db in aurum_contract_py aurum_contract_rs; do
  sql "DROP DATABASE IF EXISTS $db WITH (FORCE)" >/dev/null && sql "CREATE DATABASE $db" >/dev/null || exit 1
done

(cd "$REPO/backend" && AURUM_POSTGRES_DB=aurum_contract_py "$PYTHON_BIN" -m alembic upgrade head) >"$DIR/py-migrate.log" 2>&1 || {
  cat "$DIR/py-migrate.log"; exit 1; }
# Run both from $DIR so neither picks up a developer's backend/.env.
cd "$DIR" || exit 1
AURUM_POSTGRES_DB=aurum_contract_py setsid "$PYTHON_BIN" -m uvicorn app.main:app --app-dir "$REPO/backend" \
  --host 127.0.0.1 --port "$PY_PORT" >"$DIR/py.log" 2>&1 </dev/null &
echo $! >"$DIR/py.pid"
AURUM_POSTGRES_DB=aurum_contract_rs AURUM_HOST=127.0.0.1 AURUM_PORT=$RS_PORT \
  setsid "${AURUM_SERVER_BIN:-$REPO/target/debug/aurum-server}" >"$DIR/rs.log" 2>&1 </dev/null &
echo $! >"$DIR/rs.pid"

for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:$PY_PORT/api/health" >/dev/null && curl -sf "http://127.0.0.1:$RS_PORT/api/health" >/dev/null; then
    echo "python: http://127.0.0.1:$PY_PORT  rust: http://127.0.0.1:$RS_PORT  (logs in $DIR)"
    exit 0
  fi
  sleep 0.5
done
echo "servers did not come up; see $DIR/py.log and $DIR/rs.log" >&2
exit 1
