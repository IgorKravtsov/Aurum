#!/usr/bin/env bash
# Generates the Swift and Kotlin bindings into crates/aurum-ffi/bindings/
# (gitignored) from the compiled host library.
#
# Usage: scripts/gen-bindings.sh [--release]
set -euo pipefail
# shellcheck source=_common.sh
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

profile_flag=()
profile_dir="debug"
if [[ "${1:-}" == "--release" ]]; then
  profile_flag=(--release)
  profile_dir="release"
elif [[ $# -gt 0 ]]; then
  die "unknown argument '$1' (only --release is supported)"
fi

need cargo "Install Rust: https://rustup.rs"

case "$(uname -s)" in
  Darwin) lib_file="lib${LIB_NAME}.dylib" ;;
  Linux) lib_file="lib${LIB_NAME}.so" ;;
  *) die "unsupported host OS '$(uname -s)' (use macOS or Linux)" ;;
esac

cd "$WORKSPACE_DIR"
cargo build -p aurum-ffi --lib "${profile_flag[@]}"
lib_path="$TARGET_DIR/$profile_dir/$lib_file"
[[ -f "$lib_path" ]] || die "library not found: $lib_path"

out_dir="$FFI_DIR/bindings"
rm -rf "$out_dir"
for language in swift kotlin; do
  cargo run -p aurum-ffi --bin uniffi-bindgen "${profile_flag[@]}" -- \
    generate --library "$lib_path" --language "$language" --out-dir "$out_dir/$language"
done

echo "Bindings written to $out_dir:"
find "$out_dir" -type f | sort
