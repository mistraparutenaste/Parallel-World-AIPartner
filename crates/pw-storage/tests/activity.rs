use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pw_storage::activity::{
    ActivityDatabase, ActivityStorageError, NewActivitySession, ProactiveDecision,
};

fn unique_database_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "parallel-world-activity-{label}-{}-{nonce}.sqlite3",
        std::process::id()
    ))
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", path.display()))
}

fn remove_database_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(sidecar_path(path, "-wal"));
    let _ = std::fs::remove_file(sidecar_path(path, "-shm"));
}

fn new_session(started_at: i64, context: &[u8]) -> NewActivitySession {
    NewActivitySession {
        started_at,
        ended_at: None,
        duration_seconds: 0,
        category: "development".to_owned(),
        payload_version: 1,
        protected_context: context.to_vec(),
    }
}

#[test]
fn activity_database_has_isolated_v1_strict_schema_and_connection_settings() {
    let database = ActivityDatabase::open_in_memory().expect("activity database opens");
    let connection = database.connection();

    assert_eq!(
        connection
            .pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "busy_timeout", |row| row.get::<_, i64>(0))
            .unwrap(),
        5_000
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );

    for table in ["activity_sessions", "proactive_decisions"] {
        let schema: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            schema.ends_with("STRICT"),
            "{table} must be STRICT: {schema}"
        );
    }
    for forbidden in ["conversations", "memories"] {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                [forbidden],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!exists, "activity DB unexpectedly contains {forbidden}");
    }
}

#[test]
fn activity_file_database_uses_wal_and_rejects_future_or_invalid_v1_schema() {
    let wal_path = unique_database_path("wal");
    let database = ActivityDatabase::open(&wal_path).expect("file database opens");
    let mode: String = database
        .connection()
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    drop(database);
    remove_database_files(&wal_path);

    let future_path = unique_database_path("future");
    let connection = rusqlite::Connection::open(&future_path).unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    drop(connection);
    assert!(matches!(
        ActivityDatabase::open(&future_path),
        Err(ActivityStorageError::FutureSchema {
            found: 2,
            supported: 1
        })
    ));
    let connection = rusqlite::Connection::open(&future_path).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    drop(connection);
    remove_database_files(&future_path);

    let invalid_path = unique_database_path("invalid-schema");
    let connection = rusqlite::Connection::open(&invalid_path).unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);
    assert!(matches!(
        ActivityDatabase::open(&invalid_path),
        Err(ActivityStorageError::InvalidSchema)
    ));
    remove_database_files(&invalid_path);
}

#[test]
fn activity_sessions_insert_update_and_page_by_descending_id_cursor() {
    let mut database = ActivityDatabase::open_in_memory().unwrap();
    let first_id = database
        .insert_session(&new_session(10, b"cipher-1"))
        .unwrap();
    let second_id = database
        .insert_session(&new_session(20, b"cipher-2"))
        .unwrap();
    let third_id = database
        .insert_session(&new_session(30, b"cipher-3"))
        .unwrap();

    assert!(
        database
            .update_session(first_id, Some(18), 8)
            .expect("session updates")
    );
    assert!(!database.update_session(99_999, Some(40), 1).unwrap());

    let first_page = database.page_sessions(2, None).unwrap();
    assert_eq!(
        first_page
            .sessions
            .iter()
            .map(|session| session.id)
            .collect::<Vec<_>>(),
        vec![third_id, second_id]
    );
    assert_eq!(first_page.next_before_id, Some(second_id));
    assert_eq!(first_page.sessions[0].protected_context, b"cipher-3");

    let second_page = database
        .page_sessions(2, first_page.next_before_id)
        .unwrap();
    assert_eq!(second_page.sessions.len(), 1);
    assert_eq!(second_page.sessions[0].id, first_id);
    assert_eq!(second_page.sessions[0].ended_at, Some(18));
    assert_eq!(second_page.sessions[0].duration_seconds, 8);
    assert_eq!(second_page.next_before_id, None);
}

#[test]
fn activity_sessions_delete_selected_then_delete_all() {
    let mut database = ActivityDatabase::open_in_memory().unwrap();
    let first = database
        .insert_session(&new_session(1, b"cipher-1"))
        .unwrap();
    let second = database
        .insert_session(&new_session(2, b"cipher-2"))
        .unwrap();
    let third = database
        .insert_session(&new_session(3, b"cipher-3"))
        .unwrap();

    assert_eq!(
        database.delete_selected_sessions(&[first, third]).unwrap(),
        2
    );
    assert_eq!(
        database
            .page_sessions(10, None)
            .unwrap()
            .sessions
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![second]
    );
    assert_eq!(database.delete_all_sessions().unwrap(), 1);
    assert!(
        database
            .page_sessions(10, None)
            .unwrap()
            .sessions
            .is_empty()
    );
}

#[test]
fn activity_retention_deletes_only_sessions_started_before_the_cutoff() {
    let mut database = ActivityDatabase::open_in_memory().unwrap();
    let older = database.insert_session(&new_session(99, b"old")).unwrap();
    let boundary = database.insert_session(&new_session(100, b"edge")).unwrap();
    let newer = database.insert_session(&new_session(101, b"new")).unwrap();

    assert_eq!(database.delete_sessions_before(100).unwrap(), 1);
    assert_eq!(
        database
            .page_sessions(10, None)
            .unwrap()
            .sessions
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![newer, boundary]
    );
    assert_ne!(older, boundary);

    database
        .connection()
        .execute("DROP TABLE activity_sessions", [])
        .unwrap();
    assert!(database.delete_sessions_before(200).is_err());
}

#[test]
fn activity_proactive_decision_queries_deduplicate_topics_and_rate_limit_only_speech() {
    let mut database = ActivityDatabase::open_in_memory().unwrap();
    database
        .insert_proactive_decision(100, "return", ProactiveDecision::Speak, b"hash-1")
        .unwrap();
    database
        .insert_proactive_decision(200, "long_session", ProactiveDecision::Skip, b"hash-2")
        .unwrap();
    database
        .insert_proactive_decision(300, "category", ProactiveDecision::Speak, b"hash-3")
        .unwrap();

    assert!(database.has_topic_hash(b"hash-1").unwrap());
    assert!(!database.has_topic_hash(b"unknown").unwrap());
    assert_eq!(database.count_decisions_since(100).unwrap(), 2);
    assert_eq!(database.count_decisions_since(101).unwrap(), 1);
    assert_eq!(database.count_decisions_since(301).unwrap(), 0);
    assert_eq!(database.latest_spoken_decision_at().unwrap(), Some(300));
    assert!(
        database
            .insert_proactive_decision(400, "return", ProactiveDecision::Skip, b"hash-1")
            .is_err()
    );

    let empty = ActivityDatabase::open_in_memory().unwrap();
    assert_eq!(empty.latest_spoken_decision_at().unwrap(), None);
}

#[test]
fn activity_repository_rejects_invalid_times_durations_and_empty_sensitive_blobs() {
    let mut database = ActivityDatabase::open_in_memory().unwrap();

    let mut session = new_session(1, b"cipher");
    session.protected_context.clear();
    assert!(matches!(
        database.insert_session(&session),
        Err(ActivityStorageError::EmptyProtectedContext)
    ));
    let mut session = new_session(-1, b"cipher");
    session.duration_seconds = -1;
    assert!(matches!(
        database.insert_session(&session),
        Err(ActivityStorageError::InvalidTimestamp)
    ));
    let id = database.insert_session(&new_session(5, b"cipher")).unwrap();
    assert!(matches!(
        database.update_session(id, Some(4), 1),
        Err(ActivityStorageError::EndedBeforeStarted)
    ));
    assert!(matches!(
        database.update_session(id, Some(6), -1),
        Err(ActivityStorageError::InvalidDuration)
    ));
    assert!(matches!(
        database.delete_sessions_before(-1),
        Err(ActivityStorageError::InvalidTimestamp)
    ));
    assert!(matches!(
        database.insert_proactive_decision(1, "return", ProactiveDecision::Speak, b""),
        Err(ActivityStorageError::EmptyTopicHash)
    ));
    assert!(matches!(
        database.count_decisions_since(-1),
        Err(ActivityStorageError::InvalidTimestamp)
    ));
    assert!(matches!(
        database.has_topic_hash(b""),
        Err(ActivityStorageError::EmptyTopicHash)
    ));
}

#[cfg(windows)]
#[test]
fn activity_database_files_never_contain_dpapi_plaintext_sentinel() {
    use pw_platform::activity::{DataProtector, DpapiProtector};

    let path = unique_database_path("plaintext-sentinel");
    let sentinel = format!(
        "ACTIVITY-PLAINTEXT-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let protected = DpapiProtector
        .protect(sentinel.as_bytes())
        .expect("sentinel is protected");
    let mut database = ActivityDatabase::open(&path).unwrap();
    database
        .insert_session(&new_session(1, &protected))
        .unwrap();
    database
        .connection()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(database);

    for candidate in [
        path.clone(),
        sidecar_path(&path, "-wal"),
        sidecar_path(&path, "-shm"),
    ] {
        if candidate.exists() {
            let bytes = std::fs::read(&candidate).unwrap();
            assert!(
                !bytes
                    .windows(sentinel.len())
                    .any(|window| window == sentinel.as_bytes()),
                "plaintext sentinel reached {}",
                candidate.display()
            );
        }
    }
    remove_database_files(&path);
}
