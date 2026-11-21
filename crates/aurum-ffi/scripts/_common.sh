#!/usr/bin/env bash
# Shared helpers for the aurum-ffi scripts (sourced, not executed).

FFI_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_DIR="$(cd "$FFI_DIR/../.." && pwd)"
# Honour a custom CARGO_TARGET_DIR; otherwise the workspace-level `target/`.
TARGET_DIR="${CARGO_TARGET_DIR:-$WORKSPACE_DIR/target}"
LIB_NAME="aurum_ffi"

die() {
  echo "error: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "'$1' not found. $2"
}
