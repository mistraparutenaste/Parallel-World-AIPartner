//! Guards against `packages/contracts` drifting from
//! `crates/pw-contracts/src/dto/*.rs`.
//!
//! `generated_output_matches_committed_snapshot` regenerates the bindings
//! into a scratch directory with [`pw_contracts::bindings::export_all`] and
//! diffs the result against the committed `packages/contracts/src/generated`
//! and `packages/contracts/src/index.ts`. Bumping a `pub const` (for example
//! `CHARACTER_SETTINGS_SCHEMA_VERSION`) without re-running
//! `cargo run -p pw-contracts --bin export-bindings` fails this test, so the
//! TypeScript side can never silently drift from the Rust value.
//!
//! `every_ts_derive_type_is_exported` scans the DTO source for every
//! `#[derive(TS)]` type and asserts each one actually produced a `.ts` file,
//! so pruning `export_bindings.rs` down to root types (relying on `ts-rs`'s
//! transitive `export_all`) can never silently stop covering a type.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn scratch_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "pw-contracts-bindings-test-{label}-{}",
        std::process::id()
    ))
}

fn list_ts_files(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| Path::new(name).extension().and_then(|ext| ext.to_str()) == Some("ts"))
        .collect()
}

#[test]
fn generated_output_matches_committed_snapshot() {
    let root = repo_root();
    let committed_generated = root.join("packages/contracts/src/generated");
    let committed_index = root.join("packages/contracts/src/index.ts");

    let scratch = scratch_dir("snapshot");
    let _ = fs::remove_dir_all(&scratch);
    let generated_dir = scratch.join("generated");
    pw_contracts::bindings::export_all(&generated_dir);

    let committed_files = list_ts_files(&committed_generated);
    let fresh_files = list_ts_files(&generated_dir);
    assert_eq!(
        committed_files, fresh_files,
        "packages/contracts/src/generated is out of date; run \
         `cargo run -p pw-contracts --bin export-bindings` and commit the diff"
    );
    for name in &fresh_files {
        let committed = fs::read_to_string(committed_generated.join(name))
            .unwrap_or_else(|error| panic!("read committed {name}: {error}"));
        let fresh = fs::read_to_string(generated_dir.join(name))
            .unwrap_or_else(|error| panic!("read fresh {name}: {error}"));
        assert_eq!(
            committed, fresh,
            "packages/contracts/src/generated/{name} is out of date; run \
             `cargo run -p pw-contracts --bin export-bindings` and commit the diff"
        );
    }

    let committed_index_src = fs::read_to_string(&committed_index)
        .unwrap_or_else(|error| panic!("read {}: {error}", committed_index.display()));
    let fresh_index_src = fs::read_to_string(scratch.join("index.ts"))
        .expect("bindings::export_all writes index.ts next to generated_dir");
    assert_eq!(
        committed_index_src, fresh_index_src,
        "packages/contracts/src/index.ts is out of date; run \
         `cargo run -p pw-contracts --bin export-bindings` and commit the diff"
    );

    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn every_ts_derive_type_is_exported() {
    let root = repo_root();
    let dto_dir = root.join("crates/pw-contracts/src/dto");
    let expected = ts_derive_type_names(&dto_dir);
    assert!(
        expected.len() >= 86,
        "expected at least 86 #[derive(TS)] types under {}; found {}: {expected:?}",
        dto_dir.display(),
        expected.len(),
    );

    let scratch = scratch_dir("coverage");
    let _ = fs::remove_dir_all(&scratch);
    let generated_dir = scratch.join("generated");
    pw_contracts::bindings::export_all(&generated_dir);

    let missing: Vec<&String> = expected
        .iter()
        .filter(|name| !generated_dir.join(format!("{name}.ts")).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "these #[derive(TS)] types never became reachable from a root \
         `export_all` call in crates/pw-contracts/src/bindings.rs: {missing:?}",
    );

    let _ = fs::remove_dir_all(&scratch);
}

/// Scans `crates/pw-contracts/src/dto/*.rs` for `pub struct`/`pub enum`
/// declarations whose immediately preceding `#[derive(...)]` attribute lists
/// `TS`. This is a source scan, not a real parser, but every DTO in this
/// crate follows the same `#[derive(..., TS)]` then zero or more other
/// attributes then `pub struct`/`pub enum Name` shape, so it is a reliable
/// proxy for "this type derives `ts_rs::TS`".
fn ts_derive_type_names(dto_dir: &Path) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(dto_dir).expect("read dto directory") {
        let entry = entry.expect("read dto directory entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        let mut pending_derive_ts = false;
        for line in source.lines() {
            let trimmed = line.trim();
            if let Some(derive_body) = trimmed
                .strip_prefix("#[derive(")
                .and_then(|rest| rest.strip_suffix(")]"))
            {
                pending_derive_ts = derive_body.split(',').any(|token| token.trim() == "TS");
                continue;
            }
            if !pending_derive_ts {
                continue;
            }
            if trimmed.starts_with("#[") || trimmed.starts_with("///") || trimmed.is_empty() {
                continue;
            }
            if let Some(rest) = trimmed
                .strip_prefix("pub struct ")
                .or_else(|| trimmed.strip_prefix("pub enum "))
            {
                let name = rest
                    .split(|c: char| c == '{' || c == '(' || c.is_whitespace())
                    .next()
                    .unwrap_or_default();
                if !name.is_empty() {
                    names.insert(name.to_owned());
                }
            }
            pending_derive_ts = false;
        }
    }
    names
}
