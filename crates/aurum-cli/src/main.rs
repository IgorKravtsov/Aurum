//! Aurum CLI — Rust port of the Node CLI in `cli/` (later phase).
//!
//! Skeleton only: it compiles and links against `aurum-client` so the
//! workspace graph is in place; commands are ported in a later phase.

fn main() {
    // Referencing the client keeps the dependency edge honest until the
    // first real command lands.
    let _ = std::any::type_name::<aurum_client::AurumClient>();
    eprintln!("aurum-cli {}: not implemented yet — use the Node CLI in cli/", env!("CARGO_PKG_VERSION"));
    std::process::exit(2);
}
