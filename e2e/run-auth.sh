#!/usr/bin/env bash
# Runs the Basic Auth browser flow against an isolated throwaway stack.
# This is separate from run.sh so the normal auth-free E2E suite keeps
# exercising Aurum's default localhost-only installation mode.
set -euo pipefail
cd "$(dirname "$0")/.."

export AURUM_POSTGRES_DB=aurum_e2e_auth
export AURUM_POSTGRES_PASSWORD=aurum-e2e-auth-db-only
export AURUM_WEB_PORT=3101
export AURUM_BASIC_AUTH_USER=aurum-e2e
export AURUM_BASIC_AUTH_PASSWORD=auth-e2e-only

if docker compose version >/dev/null 2>&1; then
  COMPOSE=(docker compose -p aurum-e2e-auth)
elif command -v docker-compose >/dev/null 2>&1; then
  COMPOSE=(docker-compose -p aurum-e2e-auth)
else
  echo "Docker Compose is required" >&2
  exit 1
fi

cleanup() {
  "${COMPOSE[@]}" down -v
}
trap cleanup EXIT

"${COMPOSE[@]}" up -d --build

echo "waiting for http://localhost:${AURUM_WEB_PORT}/api/health ..."
for _ in $(seq 1 60); do
  if curl -sf "http://localhost:${AURUM_WEB_PORT}/api/health" >/dev/null; then
    echo "auth test stack is up"
    break
  fi
  sleep 1
done

cd e2e
npm install
AURUM_E2E_BASE_URL="http://localhost:${AURUM_WEB_PORT}" \
  AURUM_E2E_AUTH_USER="$AURUM_BASIC_AUTH_USER" \
  AURUM_E2E_AUTH_PASSWORD="$AURUM_BASIC_AUTH_PASSWORD" \
  npx playwright test tests/auth-login.spec.ts "$@"
