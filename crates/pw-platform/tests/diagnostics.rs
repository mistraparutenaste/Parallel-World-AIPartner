use pw_platform::diagnostics::{CrashInput, DiagnosticStore, RetentionPolicy};

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
                    .write(CrashInput::frontend("error", Some(index), None))
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
        .write(CrashInput::frontend("error", Some(1), None))
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
        .write(CrashInput::frontend("error", None, None))
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
            .write(CrashInput::frontend("error", Some(index), None))
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
