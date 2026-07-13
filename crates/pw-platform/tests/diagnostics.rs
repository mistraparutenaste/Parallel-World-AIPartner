use pw_platform::diagnostics::{CrashInput, DiagnosticStore, RetentionPolicy};
use std::sync::atomic::{AtomicBool, Ordering};

static PREVIOUS_HOOK_CALLED: AtomicBool = AtomicBool::new(false);

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("pw-diagnostics-{name}-{}", std::process::id()))
}

#[test]
fn report_is_atomic_structured_and_contains_no_secret_text() {
    let root = temp_root("safe");
    let _ = std::fs::remove_dir_all(&root);
    let store = DiagnosticStore::new(&root, RetentionPolicy::default());
    let path = store
        .write(CrashInput::frontend(
            "unhandled_rejection",
            Some(10),
            Some(20),
        ))
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let serialized = value.to_string();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["payload_category"], "unhandled_rejection");
    assert!(value["timestamp_ms"].as_u64().is_some());
    assert!(!serialized.contains("prompt"));
    assert!(!std::fs::read_dir(&root).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .and_then(|value| value.to_str())
            == Some("tmp")
    }));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn concurrent_writes_have_unique_names_and_deterministic_retention() {
    let root = temp_root("concurrent");
    let _ = std::fs::remove_dir_all(&root);
    let store = std::sync::Arc::new(DiagnosticStore::new(
        &root,
        RetentionPolicy {
            max_files: 20,
            max_bytes: 20 * 1024 * 1024,
        },
    ));
    let threads = (0..32)
        .map(|index| {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .write(CrashInput::frontend("window_error", Some(index), None))
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    let entries = store.list().unwrap();
    assert_eq!(entries.len(), 20);
    let unique = entries
        .iter()
        .map(|entry| &entry.id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), entries.len());
    assert!(!std::fs::read_dir(&root).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .and_then(|value| value.to_str())
            == Some("tmp")
    }));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn export_conflict_preserves_old_file_and_leaves_no_predictable_temp() {
    let root = temp_root("export");
    let out = temp_root("export-out");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let store = DiagnosticStore::new(&root, RetentionPolicy::default());
    store
        .write(CrashInput::frontend("window_error", Some(1), None))
        .unwrap();
    let destination = out.join("reports.json");
    std::fs::write(&destination, b"old").unwrap();
    assert!(store.export(&destination, false).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"old");
    assert!(!std::fs::read_dir(&out).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp")
    }));
    store.export(&destination, true).unwrap();
    assert_ne!(std::fs::read(&destination).unwrap(), b"old");
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn concurrent_no_overwrite_has_exactly_one_creator() {
    let root = temp_root("export-race");
    let out = temp_root("export-race-out");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let store = std::sync::Arc::new(DiagnosticStore::new(&root, RetentionPolicy::default()));
    store
        .write(CrashInput::frontend("window_error", None, None))
        .unwrap();
    let destination = std::sync::Arc::new(out.join("reports.json"));
    let threads = (0..16)
        .map(|_| {
            let store = store.clone();
            let destination = destination.clone();
            std::thread::spawn(move || store.export(&destination, false).is_ok())
        })
        .collect::<Vec<_>>();
    let successes = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .filter(|success| *success)
        .count();
    assert_eq!(successes, 1);
    assert!(destination.exists());
    assert!(!std::fs::read_dir(&out).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .and_then(|value| value.to_str())
            == Some("tmp")
    }));
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn retention_enforces_count_and_total_bytes() {
    let root = temp_root("retention");
    let _ = std::fs::remove_dir_all(&root);
    let store = DiagnosticStore::new(
        &root,
        RetentionPolicy {
            max_files: 3,
            max_bytes: 850,
        },
    );
    for index in 0..10 {
        store
            .write(CrashInput::frontend("window_error", Some(index), None))
            .unwrap();
    }
    let entries = store.list().unwrap();
    assert!(entries.len() <= 3);
    assert!(entries.iter().map(|entry| entry.bytes).sum::<u64>() <= 850);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn maintenance_removes_orphaned_temporary_reports() {
    let root = temp_root("orphan-temp");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let orphan = root.join(".pw-crash-interrupted.tmp");
    std::fs::write(&orphan, vec![b'x'; 1024]).unwrap();

    let store = DiagnosticStore::new(&root, RetentionPolicy::default());
    store.recover_after_unclean_shutdown().unwrap();

    assert!(!orphan.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn installed_hook_never_calls_an_arbitrary_previous_hook() {
    let root = temp_root("panic-hook");
    let _ = std::fs::remove_dir_all(&root);
    PREVIOUS_HOOK_CALLED.store(false, Ordering::SeqCst);
    std::panic::set_hook(Box::new(|_| {
        PREVIOUS_HOOK_CALLED.store(true, Ordering::SeqCst);
    }));
    pw_platform::diagnostics::install_panic_hook(DiagnosticStore::new(
        &root,
        RetentionPolicy::default(),
    ));

    let _ = std::panic::catch_unwind(|| panic!("payload must be omitted"));

    assert!(!PREVIOUS_HOOK_CALLED.load(Ordering::SeqCst));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn list_and_export_ignore_unowned_or_unvalidated_json() {
    let root = temp_root("typed-only");
    let out = temp_root("typed-only-out");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let store = DiagnosticStore::new(&root, RetentionPolicy::default());
    store
        .write(CrashInput::frontend("window_error", Some(1), Some(2)))
        .unwrap();
    assert!(
        store
            .write(CrashInput::frontend("error", Some(1), Some(2)))
            .is_err()
    );
    std::fs::write(root.join("notes.json"), br#"{"prompt":"TOP_SECRET"}"#).unwrap();
    std::fs::write(
        root.join("crash-1-1-1.json"),
        br#"{
          "schema_version": 1,
          "timestamp_ms": 1,
          "build": "0.1.0",
          "source": "frontend",
          "payload_category": "window_error",
          "detail": "line=1;column=2",
          "thread": null,
          "location": null,
          "backtrace": null,
          "prompt": "TOP_SECRET"
        }"#,
    )
    .unwrap();
    std::fs::write(
        root.join("crash-2-1-1.json"),
        br#"{
          "schema_version": 0,
          "timestamp_ms": 2,
          "build": "0.1.0",
          "source": "frontend",
          "payload_category": "window_error",
          "detail": "line=1;column=2",
          "thread": null,
          "location": null,
          "backtrace": null
        }"#,
    )
    .unwrap();
    std::fs::write(
        root.join("crash-3-1-1.json"),
        br#"{
          "schema_version": 1,
          "timestamp_ms": 3,
          "build": "0.1.0",
          "source": "rust",
          "payload_category": "panic_string",
          "detail": "panic payload omitted",
          "thread": null,
          "location": null,
          "backtrace": "Bearer TOP_SECRET"
        }"#,
    )
    .unwrap();

    assert_eq!(store.list().unwrap().len(), 1);
    let destination = out.join("reports.json");
    store.export(&destination, false).unwrap();
    assert!(
        !std::fs::read_to_string(destination)
            .unwrap()
            .contains("TOP_SECRET")
    );
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(out);
}
