#!/usr/bin/env bash
# Builds libaurum_ffi.so for Android ABIs into jniLibs and generates the
# Kotlin bindings next to them (needs cargo-ndk and an Android NDK).
#
# Usage: scripts/build-android.sh [--debug]   (release by default)
# Output: crates/aurum-ffi/build/android/jniLibs/<abi>/libaurum_ffi.so
#         crates/aurum-ffi/build/android/kotlin/uniffi/aurum_ffi/aurum_ffi.kt
set -euo pipefail
# shellcheck source=_common.sh
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

need cargo "Install Rust: https://rustup.rs"
need rustup "Install Rust: https://rustup.rs"
need cargo-ndk "Install it: cargo install cargo-ndk"
if [[ -z "${ANDROID_NDK_HOME:-}" && -z "${ANDROID_NDK_ROOT:-}" && -z "${ANDROID_HOME:-}" && -z "${ANDROID_SDK_ROOT:-}" ]]; then
  die "Android NDK not found: install it via Android Studio (SDK Manager > NDK) and set ANDROID_NDK_HOME"
fi

profile_flag=()
profile_dir="debug"
if [[ "${1:-}" == "--debug" ]]; then
  :
elif [[ $# -eq 0 ]]; then
  profile_flag=(--release)
  profile_dir="release"
else
  die "unknown argument '$1' (only --debug is supported)"
fi

# Rust target <-> Android ABI is handled by cargo-ndk's -t names.
abis=(arm64-v8a armeabi-v7a x86_64)
rust_targets=(aarch64-linux-android armv7-linux-androideabi x86_64-linux-android)
installed="$(rustup target list --installed)"
for target in "${rust_targets[@]}"; do
  grep -qx "$target" <<<"$installed" || die "Rust target missing; run: rustup target add ${rust_targets[*]}"
done

out_dir="$FFI_DIR/build/android"
rm -rf "$out_dir"
mkdir -p "$out_dir/jniLibs" "$out_dir/kotlin"

cd "$WORKSPACE_DIR"
abi_flags=()
for abi in "${abis[@]}"; do
  abi_flags+=(-t "$abi")
done
# --no-default-features: the uniffi CLI is host-only, keep it out of the device library.
cargo ndk "${abi_flags[@]}" -o "$out_dir/jniLibs" \
  build -p aurum-ffi --lib --no-default-features ${profile_flag[@]+"${profile_flag[@]}"}

# Kotlin sources come from the host library's metadata (same API on every ABI).
cargo build -p aurum-ffi --lib ${profile_flag[@]+"${profile_flag[@]}"}
case "$(uname -s)" in
  Darwin) host_lib="lib${LIB_NAME}.dylib" ;;
  Linux) host_lib="lib${LIB_NAME}.so" ;;
  *) die "unsupported host OS '$(uname -s)' (use macOS or Linux)" ;;
esac
cargo run -p aurum-ffi --bin uniffi-bindgen ${profile_flag[@]+"${profile_flag[@]}"} -- \
  generate --library "$TARGET_DIR/$profile_dir/$host_lib" --language kotlin --out-dir "$out_dir/kotlin"

echo "jniLibs: $out_dir/jniLibs"
echo "Kotlin sources: $out_dir/kotlin"
