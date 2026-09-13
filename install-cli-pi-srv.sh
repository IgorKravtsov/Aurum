#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REMOTE_HOST=${1:-${AURUM_CLI_REMOTE_HOST:-pi-srv}}
LOCAL_TMP=""
REMOTE_TMP=""

usage() {
  echo "Использование: $0 [SSH_HOST]"
  echo "По умолчанию SSH_HOST: pi-srv (или AURUM_CLI_REMOTE_HOST)."
}

cleanup() {
  if [[ -n "$LOCAL_TMP" ]]; then
    rm -rf -- "$LOCAL_TMP"
  fi
  if [[ -n "$REMOTE_TMP" ]]; then
    ssh "$REMOTE_HOST" bash -s -- "$REMOTE_TMP" <<'REMOTE_CLEANUP' >/dev/null 2>&1 || true
set -eu
rm -rf -- "$1"
REMOTE_CLEANUP
  fi
}

if [[ $# -gt 1 ]]; then
  usage >&2
  exit 2
fi

for required_command in npm ssh scp; do
  if ! command -v "$required_command" >/dev/null 2>&1; then
    echo "Не найдена локальная команда: $required_command" >&2
    exit 1
  fi
done

if [[ ! -f "$SCRIPT_DIR/cli/package.json" || ! -f "$SCRIPT_DIR/skills/aurum/SKILL.md" ]]; then
  echo "Запускайте скрипт из репозитория Aurum с каталогами cli/ и skills/." >&2
  exit 1
fi

trap cleanup EXIT
LOCAL_TMP=$(mktemp -d)
REMOTE_TMP=$(ssh "$REMOTE_HOST" 'mktemp -d /tmp/aurum-cli-install.XXXXXX')

PACKAGE_NAME=$(npm pack "$SCRIPT_DIR/cli" --pack-destination "$LOCAL_TMP")
PACKAGE_PATH="$LOCAL_TMP/$PACKAGE_NAME"

scp -q "$PACKAGE_PATH" "$REMOTE_HOST:$REMOTE_TMP/aurum-cli.tgz"
scp -q "$SCRIPT_DIR/skills/aurum/SKILL.md" "$REMOTE_HOST:$REMOTE_TMP/aurum-skill.md"

ssh "$REMOTE_HOST" bash -s -- "$REMOTE_TMP" <<'REMOTE_INSTALL'
set -Eeuo pipefail

REMOTE_TMP=$1
NODE_MAJOR=$(node --version 2>/dev/null | sed -E 's/^v([0-9]+).*/\1/')

if [[ -z "$NODE_MAJOR" || "$NODE_MAJOR" -lt 20 ]]; then
  echo "Для Aurum CLI требуется Node.js 20 или новее." >&2
  exit 1
fi
if ! command -v npm >/dev/null 2>&1; then
  echo "На сервере не найден npm." >&2
  exit 1
fi

# CLI и skills устанавливаются только в домашний каталог пользователя, без sudo.
npm install -g --prefix "$HOME/.local" "$REMOTE_TMP/aurum-cli.tgz"
install -d -m 0755 "$HOME/.agents/skills/aurum" "$HOME/.claude/skills/aurum"
install -m 0644 "$REMOTE_TMP/aurum-skill.md" "$HOME/.agents/skills/aurum/SKILL.md"
install -m 0644 "$REMOTE_TMP/aurum-skill.md" "$HOME/.claude/skills/aurum/SKILL.md"

AURUM_BIN="$HOME/.local/bin/aurum"
"$AURUM_BIN" --version
"$AURUM_BIN" skills get core | grep -q 'Обязательное сопоставление перед внесением операций'
echo "Aurum CLI установлен: $AURUM_BIN"
REMOTE_INSTALL
