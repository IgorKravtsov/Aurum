//! `cargo run -p aurum-ffi --bin uniffi-bindgen generate --library <cdylib> ...`
//! — generates the Swift/Kotlin bindings (see `scripts/gen-bindings.sh`).

fn main() {
    uniffi::uniffi_bindgen_main()
}
