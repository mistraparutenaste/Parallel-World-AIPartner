//! Exports the TypeScript side of the IPC contracts.
//!
//! Run from the repository root:
//! `cargo run -p pw-contracts --bin export-bindings`

use std::path::Path;

fn main() {
    let out_dir = Path::new("packages/contracts/src/generated");
    pw_contracts::bindings::export_all(out_dir);
    println!("TypeScript bindings exported to {}", out_dir.display());
}
