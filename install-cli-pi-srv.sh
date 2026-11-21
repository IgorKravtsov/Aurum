#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REMOTE_HOST=${1:-${AURUM_CLI_REMOTE_HOST:-pi-srv}}
# Где лежат собранные релизы (см. .github/workflows/release-cli.yml).
AURUM_REPO=${AURUM_REPO:-IgorKravtsov/Aurum}
LOCAL_TMP=""
REMOTE_TMP=""

usage() {
  echo "Использование: $0 [SSH_HOST]"
  echo "По умолчанию SSH_HOST: pi-srv (или AURUM_CLI_REMOTE_HOST)."
  echo "Версия берётся из Cargo.toml (или AURUM_CLI_VERSION); репозиторий релизов — AURUM_REPO."
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

for required_command in ssh scp tar; do
  if ! command -v "$required_command" >/dev/null 2>&1; then
    echo "Не найдена локальная команда: $required_command" >&2
    exit 1
  fi
done

if [[ ! -f "$SCRIPT_DIR/crates/aurum-cli/Cargo.toml" || ! -f "$SCRIPT_DIR/skills/aurum/SKILL.md" ]]; then
  echo "Запускайте скрипт из репозитория Aurum с каталогами crates/ и skills/." >&2
  exit 1
fi

# Версия workspace: строка version = "x.y.z" в секции [workspace.package].
VERSION=${AURUM_CLI_VERSION:-$(awk '/^\[workspace\.package\]/{in_section=1; next} /^\[/{in_section=0} in_section && /^version *=/{gsub(/[" ]/, "", $3); print $3; exit}' "$SCRIPT_DIR/Cargo.toml")}
if [[ -z "$VERSION" ]]; then
  echo "Не удалось определить версию Aurum CLI из Cargo.toml." >&2
  exit 1
fi

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Платформа сервера -> target-имя релизного архива (см. release-cli.yml).
target_for() {
  case "$1" in
    "Linux x86_64") echo "x86_64-unknown-linux-gnu" ;;
    "Linux aarch64" | "Linux arm64") echo "aarch64-unknown-linux-gnu" ;;
    "Darwin x86_64") echo "x86_64-apple-darwin" ;;
    "Darwin arm64" | "Darwin aarch64") echo "aarch64-apple-darwin" ;;
    *) echo "" ;;
  esac
}

# 1) Готовый бинарник из GitHub Releases. Возвращает 1, если его нет.
download_release_binary() {
  local target=$1 url archive expected actual
  command -v curl >/dev/null 2>&1 || return 1
  url="https://github.com/$AURUM_REPO/releases/download/v$VERSION/aurum-$target.tar.gz"
  archive="$LOCAL_TMP/aurum-$target.tar.gz"
  curl -fsSL --retry 2 -o "$archive" "$url" || return 1
  if curl -fsSL -o "$archive.sha256" "$url.sha256" 2>/dev/null; then
    expected=$(cut -d' ' -f1 <"$archive.sha256")
    actual=$(sha256_of "$archive")
    if [[ "$expected" != "$actual" ]]; then
      echo "Контрольная сумма $url не совпала." >&2
      return 1
    fi
  fi
  tar -xzf "$archive" -C "$LOCAL_TMP" aurum
}

trap cleanup EXIT
LOCAL_TMP=$(mktemp -d)
REMOTE_TMP=$(ssh "$REMOTE_HOST" 'mktemp -d /tmp/aurum-cli-install.XXXXXX')

REMOTE_PLATFORM=$(ssh "$REMOTE_HOST" 'uname -sm')
TARGET=$(target_for "$REMOTE_PLATFORM")

INSTALL_MODE=""
if [[ -n "$TARGET" ]] && download_release_binary "$TARGET"; then
  INSTALL_MODE="binary"
  scp -q "$LOCAL_TMP/aurum" "$REMOTE_HOST:$REMOTE_TMP/aurum"
  echo "Найден релиз v$VERSION для $TARGET."
elif [[ "$REMOTE_PLATFORM" == "$(uname -sm)" ]] && command -v cargo >/dev/null 2>&1; then
  # 2) Платформы совпадают: собираем локально и копируем готовый бинарник.
  echo "Релиз v$VERSION не найден; собираю aurum локально."
  cargo build --release --locked -p aurum-cli --manifest-path "$SCRIPT_DIR/Cargo.toml" --target-dir "$LOCAL_TMP/target"
  INSTALL_MODE="binary"
  scp -q "$LOCAL_TMP/target/release/aurum" "$REMOTE_HOST:$REMOTE_TMP/aurum"
else
  # 3) Последний вариант: исходники на сервер и cargo install там.
  echo "Релиз v$VERSION для '$REMOTE_PLATFORM' не найден; собираю на сервере через cargo install."
  INSTALL_MODE="source"
  tar -czf "$LOCAL_TMP/source.tgz" -C "$SCRIPT_DIR" Cargo.toml Cargo.lock LICENSE crates
  scp -q "$LOCAL_TMP/source.tgz" "$REMOTE_HOST:$REMOTE_TMP/source.tgz"
fi
scp -q "$SCRIPT_DIR/skills/aurum/SKILL.md" "$REMOTE_HOST:$REMOTE_TMP/aurum-skill.md"

ssh "$REMOTE_HOST" bash -s -- "$REMOTE_TMP" "$INSTALL_MODE" <<'REMOTE_INSTALL'
set -Eeuo pipefail

REMOTE_TMP=$1
INSTALL_MODE=$2
# Единственная системная зависимость на сервере — cargo, и то лишь для сборки из исходников.
export PATH="$HOME/.cargo/bin:$PATH"

# CLI и skills устанавливаются только в домашний каталог пользователя, без sudo.
install -d -m 0755 "$HOME/.local/bin" "$HOME/.agents/skills/aurum" "$HOME/.claude/skills/aurum"
if [[ "$INSTALL_MODE" == "binary" ]]; then
  install -m 0755 "$REMOTE_TMP/aurum" "$HOME/.local/bin/aurum"
else
  if ! command -v cargo >/dev/null 2>&1; then
    echo "Для сборки Aurum CLI из исходников на сервере нужен cargo (rustup)." >&2
    exit 1
  fi
  mkdir -p "$REMOTE_TMP/source"
  tar -xzf "$REMOTE_TMP/source.tgz" -C "$REMOTE_TMP/source"
  (cd "$REMOTE_TMP/source" && cargo install --locked --path crates/aurum-cli --root "$HOME/.local" --force)
fi
install -m 0644 "$REMOTE_TMP/aurum-skill.md" "$HOME/.agents/skills/aurum/SKILL.md"
install -m 0644 "$REMOTE_TMP/aurum-skill.md" "$HOME/.claude/skills/aurum/SKILL.md"

AURUM_BIN="$HOME/.local/bin/aurum"
"$AURUM_BIN" --version
"$AURUM_BIN" skills get core | grep -q 'Обязательное сопоставление перед внесением операций'
echo "Aurum CLI установлен: $AURUM_BIN"
REMOTE_INSTALL
