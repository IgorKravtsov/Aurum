#!/usr/bin/env bash
# Builds AurumFFI.xcframework (device + simulator slices) and the Swift
# bindings. macOS + Xcode only.
#
# Usage: scripts/build-ios.sh [--debug]   (release by default)
# Output: crates/aurum-ffi/build/ios/AurumFFI.xcframework
#         crates/aurum-ffi/build/ios/swift/aurum_ffi.swift
set -euo pipefail
# shellcheck source=_common.sh
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

[[ "$(uname -s)" == "Darwin" ]] || die "build-ios.sh only runs on macOS (needs Xcode); current OS: $(uname -s)"
need cargo "Install Rust: https://rustup.rs"
need rustup "Install Rust: https://rustup.rs"
need xcodebuild "Install Xcode and run: xcode-select --install"
need lipo "Install the Xcode command line tools: xcode-select --install"

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

targets=(aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios)
installed="$(rustup target list --installed)"
for target in "${targets[@]}"; do
  grep -qx "$target" <<<"$installed" || die "Rust target missing; run: rustup target add ${targets[*]}"
done

cd "$WORKSPACE_DIR"
for target in "${targets[@]}"; do
  # --no-default-features: the uniffi CLI is host-only, keep it out of the device library.
  cargo build -p aurum-ffi --lib --no-default-features ${profile_flag[@]+"${profile_flag[@]}"} --target "$target"
done

out_dir="$FFI_DIR/build/ios"
rm -rf "$out_dir"
mkdir -p "$out_dir/swift" "$out_dir/headers" "$out_dir/sim"

# Swift sources + C header/modulemap come from the host library's metadata.
cargo build -p aurum-ffi --lib ${profile_flag[@]+"${profile_flag[@]}"}
cargo run -p aurum-ffi --bin uniffi-bindgen ${profile_flag[@]+"${profile_flag[@]}"} -- \
  generate --library "$TARGET_DIR/$profile_dir/lib${LIB_NAME}.dylib" --language swift --out-dir "$out_dir/swift"
cp "$out_dir/swift/"*.h "$out_dir/headers/"
# xcodebuild expects the modulemap to be called module.modulemap.
cp "$out_dir/swift/"*.modulemap "$out_dir/headers/module.modulemap"

# Both simulator architectures go into one fat library (one slice per platform).
lipo -create \
  "$TARGET_DIR/aarch64-apple-ios-sim/$profile_dir/lib${LIB_NAME}.a" \
  "$TARGET_DIR/x86_64-apple-ios/$profile_dir/lib${LIB_NAME}.a" \
  -output "$out_dir/sim/lib${LIB_NAME}.a"

xcodebuild -create-xcframework \
  -library "$TARGET_DIR/aarch64-apple-ios/$profile_dir/lib${LIB_NAME}.a" -headers "$out_dir/headers" \
  -library "$out_dir/sim/lib${LIB_NAME}.a" -headers "$out_dir/headers" \
  -output "$out_dir/AurumFFI.xcframework"

echo "XCFramework: $out_dir/AurumFFI.xcframework"
echo "Swift sources: $out_dir/swift/${LIB_NAME}.swift"
