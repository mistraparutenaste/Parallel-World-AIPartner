//! Exports the TypeScript side of the IPC contracts.
//!
//! Run from the repository root:
//! `cargo run -p pw-contracts --bin export-bindings`

use std::fs;
use std::path::Path;

use pw_contracts::{AppStatusDto, ConversationStateDto};
use ts_rs::{Config, TS};

fn main() {
    let out_dir = Path::new("packages/contracts/src/generated");
    fs::create_dir_all(out_dir).expect("create bindings output directory");

    let config = Config::new().with_out_dir(out_dir);
    AppStatusDto::export_all(&config).expect("export AppStatusDto bindings");
    ConversationStateDto::export_all(&config).expect("export ConversationStateDto bindings");

    println!("TypeScript bindings exported to {}", out_dir.display());
}
