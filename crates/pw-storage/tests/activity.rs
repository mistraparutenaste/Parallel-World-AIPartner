use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pw_application::behavior::proactive::{
    FrequencyHistory, FrequencyPolicy, HistoryQuery, eligible_to_evaluate,
};
use pw_storage::activity::{
    ActivityDatabase, ActivityStorageError, FinalSpeakDecisionOutcome, FinalSpeakDecisionRequest,
    NewActivitySession, ProactiveDecision,
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

fn assert_file_excludes(path: &Path, sentinel: &[u8]) {
    let bytes = std::fs::read(path).unwrap();
    assert!(
        !bytes
            .windows(sentinel.len())
            .any(|window| window == sentinel),
        "plaintext sentinel reached {}",
        path.display()
    );
}

#[derive(Debug, Clone, Copy)]
enum V1SchemaDefect {
    MissingTopicHashUnique,
    MissingDecisionCheck,
    AlteredDecisionCheckValues,
    ProtectedContextAny,
    MissingProtectedContextNotNull,
    MissingProtectedContextNonemptyCheck,
    MissingTopicHashNotNull,
    MissingTopicHashNonemptyCheck,
    MissingActivityStartedAtIndex,
    AlteredActivityStartedAtExpressionIndex,
    MissingDecisionCreatedAtIndex,
}

fn v1_schema_with(defect: V1SchemaDefect) -> String {
    let protected_context_type = if matches!(defect, V1SchemaDefect::ProtectedContextAny) {
        "ANY"
    } else {
        "BLOB"
    };
    let protected_context_not_null =
        if matches!(defect, V1SchemaDefect::MissingProtectedContextNotNull) {
            ""
        } else {
            " NOT NULL"
        };
    let protected_context_check =
        if matches!(defect, V1SchemaDefect::MissingProtectedContextNonemptyCheck) {
            ""
        } else {
            " CHECK (length(protected_context) > 0)"
        };
    let decision_check = match defect {
        V1SchemaDefect::MissingDecisionCheck => "",
        V1SchemaDefect::AlteredDecisionCheckValues => " CHECK (decision IN ('SPEAK', 'SKIP'))",
        _ => " CHECK (decision IN ('speak', 'skip'))",
    };
    let topic_hash_not_null = if matches!(defect, V1SchemaDefect::MissingTopicHashNotNull) {
        ""
    } else {
        " NOT NULL"
    };
    let topic_hash_unique = if matches!(defect, V1SchemaDefect::MissingTopicHashUnique) {
        ""
    } else {
        " UNIQUE"
    };
    let topic_hash_check = if matches!(defect, V1SchemaDefect::MissingTopicHashNonemptyCheck) {
        ""
    } else {
        " CHECK (length(topic_hash) > 0)"
    };
    let activity_index = match defect {
        V1SchemaDefect::MissingActivityStartedAtIndex => "",
        V1SchemaDefect::AlteredActivityStartedAtExpressionIndex => {
            "CREATE INDEX activity_sessions_started_at_idx ON activity_sessions((started_at + 0));"
        }
        _ => "CREATE INDEX activity_sessions_started_at_idx ON activity_sessions(started_at);",
    };
    let decision_index = if matches!(defect, V1SchemaDefect::MissingDecisionCreatedAtIndex) {
        ""
    } else {
        "CREATE INDEX proactive_decisions_created_at_idx ON proactive_decisions(created_at);"
    };

    format!(
        "CREATE TABLE activity_sessions (
            id INTEGER PRIMARY KEY,
            started_at INTEGER NOT NULL CHECK (started_at >= 0),
            ended_at INTEGER CHECK (ended_at IS NULL OR (ended_at >= 0 AND ended_at >= started_at)),
            duration_seconds INTEGER NOT NULL CHECK (duration_seconds >= 0),
            category TEXT NOT NULL,
            payload_version INTEGER NOT NULL,
            protected_context {protected_context_type}{protected_context_not_null}{protected_context_check}
        ) STRICT;
        {activity_index}
        CREATE TABLE proactive_decisions (
            id INTEGER PRIMARY KEY,
            created_at INTEGER NOT NULL CHECK (created_at >= 0),
            candidate_kind TEXT NOT NULL,
            decision TEXT NOT NULL{decision_check},
            topic_hash BLOB{topic_hash_not_null}{topic_hash_unique}{topic_hash_check}
        ) STRICT;
        {decision_index}"
    )
}

fn assert_invalid_v1_schema(defect: V1SchemaDefect) {
    let path = unique_database_path("invalid-v1-contract");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch(&v1_schema_with(defect)).unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);

    match ActivityDatabase::open(&path) {
        Err(ActivityStorageError::InvalidSchema) => {}
        Err(error) => panic!("{defect:?} returned unexpected error: {error:?}"),
        Ok(_) => panic!("{defect:?} was accepted as schema v1"),
    }
    remove_database_files(&path);
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

fn topic_hash(seed: u8) -> [u8; 32] {
    [seed; 32]
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
fn activity_schema_rejects_missing_topic_hash_unique_constraint() {
    assert_invalid_v1_schema(V1SchemaDefect::MissingTopicHashUnique);
}

#[test]
fn activity_schema_rejects_missing_decision_check_constraint() {
    assert_invalid_v1_schema(V1SchemaDefect::MissingDecisionCheck);
}

#[test]
fn activity_schema_rejects_altered_decision_check_values() {
    assert_invalid_v1_schema(V1SchemaDefect::AlteredDecisionCheckValues);
}

#[test]
fn activity_schema_rejects_any_protected_context_column() {
    assert_invalid_v1_schema(V1SchemaDefect::ProtectedContextAny);
}

#[test]
fn activity_schema_rejects_missing_protected_context_not_null_constraint() {
    assert_invalid_v1_schema(V1SchemaDefect::MissingProtectedContextNotNull);
}

#[test]
fn activity_schema_rejects_missing_protected_context_nonempty_check() {
    assert_invalid_v1_schema(V1SchemaDefect::MissingProtectedContextNonemptyCheck);
}

#[test]
fn activity_schema_rejects_missing_topic_hash_not_null_constraint() {
    assert_invalid_v1_schema(V1SchemaDefect::MissingTopicHashNotNull);
}

#[test]
fn activity_schema_rejects_missing_topic_hash_nonempty_check() {
    assert_invalid_v1_schema(V1SchemaDefect::MissingTopicHashNonemptyCheck);
}

#[test]
fn activity_schema_rejects_missing_started_at_index() {
    assert_invalid_v1_schema(V1SchemaDefect::MissingActivityStartedAtIndex);
}

#[test]
fn activity_schema_rejects_expression_instead_of_started_at_index() {
    assert_invalid_v1_schema(V1SchemaDefect::AlteredActivityStartedAtExpressionIndex);
}

#[test]
fn activity_schema_rejects_missing_created_at_index() {
    assert_invalid_v1_schema(V1SchemaDefect::MissingDecisionCreatedAtIndex);
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
    let first = topic_hash(1);
    let second = topic_hash(2);
    let third = topic_hash(3);
    let unknown = topic_hash(4);
    database
        .insert_proactive_decision(100, "return", ProactiveDecision::Speak, &first)
        .unwrap();
    database
        .insert_proactive_decision(200, "long_session", ProactiveDecision::Skip, &second)
        .unwrap();
    database
        .insert_proactive_decision(300, "category_change", ProactiveDecision::Speak, &third)
        .unwrap();

    assert!(database.has_topic_hash(&first).unwrap());
    assert!(!database.has_topic_hash(&unknown).unwrap());
    assert_eq!(database.count_decisions_since(100).unwrap(), 2);
    assert_eq!(database.count_decisions_since(101).unwrap(), 1);
    assert_eq!(database.count_decisions_since(301).unwrap(), 0);
    assert_eq!(database.latest_spoken_decision_at().unwrap(), Some(300));
    assert!(
        database
            .insert_proactive_decision(400, "return", ProactiveDecision::Skip, &first)
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

#[test]
fn activity_frequency_snapshot_is_single_consistent_speak_only_view_with_inclusive_cutoffs() {
    let mut database = ActivityDatabase::open_in_memory().unwrap();
    let queried = topic_hash(9);
    database
        .insert_proactive_decision(0, "return", ProactiveDecision::Speak, &topic_hash(1))
        .unwrap();
    database
        .insert_proactive_decision(100, "long_session", ProactiveDecision::Skip, &topic_hash(2))
        .unwrap();
    database
        .insert_proactive_decision(200, "category_change", ProactiveDecision::Speak, &queried)
        .unwrap();
    database
        .insert_proactive_decision(300, "return", ProactiveDecision::Speak, &topic_hash(3))
        .unwrap();

    let snapshot = database.frequency_snapshot(&queried, 300, 200).unwrap();
    assert!(snapshot.topic_exists);
    assert_eq!(snapshot.latest_spoken_at, Some(300));
    assert_eq!(snapshot.spoken_last_hour, 1);
    assert_eq!(snapshot.spoken_last_day, 2);

    let epoch = database.frequency_snapshot(&topic_hash(8), 0, 0).unwrap();
    assert_eq!(epoch.spoken_last_hour, 3);
    assert_eq!(epoch.spoken_last_day, 3);
}

#[test]
fn activity_frequency_boundaries_validate_before_writes_and_never_echo_input() {
    let mut database = ActivityDatabase::open_in_memory().unwrap();
    let sentinel = "SENSITIVE-WINDOW-TITLE";
    for invalid in [Vec::new(), vec![1; 31], vec![1; 33]] {
        let error = database.frequency_snapshot(&invalid, 0, 0).unwrap_err();
        assert!(!format!("{error:?}{error}").contains(sentinel));
    }
    assert!(database.frequency_snapshot(&topic_hash(1), 1, 2).is_err());

    for candidate in ["Return", "category", sentinel] {
        let error = database
            .insert_proactive_decision(1, candidate, ProactiveDecision::Skip, &topic_hash(7))
            .unwrap_err();
        assert!(!format!("{error:?}{error}").contains(sentinel));
    }
    let count: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM proactive_decisions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

fn speak_request<'a>(
    created_at: i64,
    candidate_kind: &'a str,
    topic_hash: &'a [u8],
) -> FinalSpeakDecisionRequest<'a> {
    FinalSpeakDecisionRequest {
        created_at,
        candidate_kind,
        topic_hash,
        minimum_interval_seconds: 900,
        hour_since: created_at.checked_sub(3_600).unwrap_or(0).max(0),
        day_since: created_at.checked_sub(86_400).unwrap_or(0).max(0),
        max_per_hour: 3,
        max_per_day: 16,
    }
}

#[test]
fn final_speak_recheck_honors_interval_future_and_count_boundaries() {
    let mut database = ActivityDatabase::open_in_memory().unwrap();
    database
        .insert_proactive_decision(100, "return", ProactiveDecision::Speak, &topic_hash(1))
        .unwrap();

    assert_eq!(
        database
            .record_final_speak(speak_request(999, "long_session", &topic_hash(2)))
            .unwrap(),
        FinalSpeakDecisionOutcome::RateLimited
    );
    assert!(matches!(
        database
            .record_final_speak(speak_request(1_000, "long_session", &topic_hash(2)))
            .unwrap(),
        FinalSpeakDecisionOutcome::Inserted { .. }
    ));

    let mut future = ActivityDatabase::open_in_memory().unwrap();
    future
        .insert_proactive_decision(2_000, "return", ProactiveDecision::Speak, &topic_hash(3))
        .unwrap();
    assert_eq!(
        future
            .record_final_speak(speak_request(1_000, "return", &topic_hash(4)))
            .unwrap(),
        FinalSpeakDecisionOutcome::RateLimited
    );

    let mut counts = ActivityDatabase::open_in_memory().unwrap();
    for (at, seed) in [(100, 10), (200, 11), (300, 12)] {
        counts
            .insert_proactive_decision(at, "return", ProactiveDecision::Speak, &topic_hash(seed))
            .unwrap();
    }
    let thirteenth = topic_hash(13);
    let mut request = speak_request(1_200, "category_change", &thirteenth);
    request.minimum_interval_seconds = 1;
    request.hour_since = 100;
    request.day_since = 0;
    assert_eq!(
        counts.record_final_speak(request).unwrap(),
        FinalSpeakDecisionOutcome::RateLimited
    );
    let mut request = speak_request(1_200, "category_change", &thirteenth);
    request.minimum_interval_seconds = 1;
    request.hour_since = 101;
    request.day_since = 0;
    assert!(matches!(
        counts.record_final_speak(request).unwrap(),
        FinalSpeakDecisionOutcome::Inserted { .. }
    ));

    let mut daily = ActivityDatabase::open_in_memory().unwrap();
    for (at, seed) in [(0, 20), (50, 21), (100, 22)] {
        daily
            .insert_proactive_decision(at, "return", ProactiveDecision::Speak, &topic_hash(seed))
            .unwrap();
    }
    let daily_hash = topic_hash(23);
    let mut request = speak_request(1_000, "long_session", &daily_hash);
    request.hour_since = 500;
    request.day_since = 0;
    request.max_per_day = 3;
    assert_eq!(
        daily.record_final_speak(request).unwrap(),
        FinalSpeakDecisionOutcome::RateLimited
    );
    request.max_per_day = 4;
    assert!(matches!(
        daily.record_final_speak(request).unwrap(),
        FinalSpeakDecisionOutcome::Inserted { .. }
    ));
}

#[test]
fn final_speak_duplicate_precedes_rate_limit_and_does_not_insert_twice() {
    let mut database = ActivityDatabase::open_in_memory().unwrap();
    let duplicate = topic_hash(1);
    database
        .insert_proactive_decision(100, "return", ProactiveDecision::Speak, &duplicate)
        .unwrap();
    let mut request = speak_request(101, "return", &duplicate);
    request.max_per_hour = 1;
    request.max_per_day = 1;
    assert_eq!(
        database.record_final_speak(request).unwrap(),
        FinalSpeakDecisionOutcome::DuplicateTopic
    );
    let count: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM proactive_decisions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn final_speak_rejects_invalid_inputs_before_transaction() {
    let mut database = ActivityDatabase::open_in_memory().unwrap();
    let first = topic_hash(1);
    for candidate in ["Return", "category", "SENSITIVE-WINDOW-TITLE"] {
        assert!(
            database
                .record_final_speak(speak_request(1_000, candidate, &topic_hash(1)))
                .is_err()
        );
    }
    for hash in [vec![], vec![0; 31], vec![0; 33]] {
        assert!(
            database
                .record_final_speak(speak_request(1_000, "return", &hash))
                .is_err()
        );
    }
    let mut invalid = speak_request(1_000, "return", &first);
    invalid.day_since = 10;
    invalid.hour_since = 9;
    assert!(database.record_final_speak(invalid).is_err());
    let mut invalid = speak_request(1_000, "return", &first);
    invalid.hour_since = 1_001;
    assert!(database.record_final_speak(invalid).is_err());
    let mut invalid = speak_request(1_000, "return", &first);
    invalid.minimum_interval_seconds = 0;
    assert!(database.record_final_speak(invalid).is_err());
    let mut invalid = speak_request(1_000, "return", &first);
    invalid.max_per_hour = 0;
    assert!(database.record_final_speak(invalid).is_err());
    let mut invalid = speak_request(1_000, "return", &first);
    invalid.max_per_day = i64::MAX.cast_unsigned() + 1;
    assert!(database.record_final_speak(invalid).is_err());
    assert!(database.frequency_snapshot(&first, -1, -1).is_err());
    assert_eq!(
        database
            .connection()
            .query_row("SELECT COUNT(*) FROM proactive_decisions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn two_connections_atomically_enforce_one_speak_limit() {
    use std::sync::{Arc, Barrier};

    let path = unique_database_path("atomic-speak-race");
    let first = ActivityDatabase::open(&path).unwrap();
    let second = ActivityDatabase::open(&path).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let run = |mut database: ActivityDatabase, seed: u8, barrier: Arc<Barrier>| {
        std::thread::spawn(move || {
            let hash = topic_hash(seed);
            let mut request = speak_request(1_000, "return", &hash);
            request.minimum_interval_seconds = 1;
            request.hour_since = 0;
            request.day_since = 0;
            request.max_per_hour = 1;
            request.max_per_day = 1;
            barrier.wait();
            database.record_final_speak(request)
        })
    };
    let left = run(first, 1, Arc::clone(&barrier));
    let right = run(second, 2, barrier);
    let outcomes = [
        left.join().unwrap().unwrap(),
        right.join().unwrap().unwrap(),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, FinalSpeakDecisionOutcome::Inserted { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == FinalSpeakDecisionOutcome::RateLimited)
            .count(),
        1
    );
    let reopened = ActivityDatabase::open(&path).unwrap();
    assert_eq!(reopened.count_decisions_since(0).unwrap(), 1);
    drop(reopened);
    remove_database_files(&path);
}

#[test]
fn frequency_history_adapter_returns_snapshot_and_has_opaque_errors() {
    let mut database = ActivityDatabase::open_in_memory().unwrap();
    let hash = topic_hash(1);
    database
        .insert_proactive_decision(10, "return", ProactiveDecision::Speak, &hash)
        .unwrap();
    let snapshot = FrequencyHistory::snapshot(
        &database,
        HistoryQuery {
            topic_hash: hash,
            hour_since: 0,
            day_since: 0,
        },
    )
    .unwrap();
    assert!(snapshot.topic_exists);
    assert_eq!(snapshot.latest_spoken_at, Some(10));
    database
        .connection()
        .execute("DROP TABLE proactive_decisions", [])
        .unwrap();
    let error = FrequencyHistory::snapshot(
        &database,
        HistoryQuery {
            topic_hash: hash,
            hour_since: 0,
            day_since: 0,
        },
    )
    .unwrap_err();
    assert_eq!(format!("{error}"), "activity frequency history unavailable");
    assert_eq!(format!("{error:?}"), "ActivityFrequencyHistoryError");
    assert!(std::error::Error::source(&error).is_none());
    assert!(!eligible_to_evaluate(
        &database,
        hash,
        10,
        FrequencyPolicy::default()
    ));
}

#[test]
fn pre_task5_v1_activity_database_reopens_without_schema_changes() {
    let path = unique_database_path("pre-task5-v1");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../activity-migrations/0001_initial.sql"))
        .unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);
    let database = ActivityDatabase::open(&path).unwrap();
    let schema: String = database
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='proactive_decisions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(schema.contains("length(topic_hash) > 0"));
    assert!(!schema.contains("length(topic_hash) = 32"));
    drop(database);
    remove_database_files(&path);
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

    let wal_path = sidecar_path(&path, "-wal");
    let shm_path = sidecar_path(&path, "-shm");
    assert!(wal_path.exists(), "test must inspect a live WAL sidecar");
    assert!(
        std::fs::metadata(&wal_path).unwrap().len() > 0,
        "test must inspect a nonempty WAL sidecar"
    );
    assert_file_excludes(&path, sentinel.as_bytes());
    assert_file_excludes(&wal_path, sentinel.as_bytes());
    if shm_path.exists() {
        assert_file_excludes(&shm_path, sentinel.as_bytes());
    }

    database
        .connection()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(database);

    assert_file_excludes(&path, sentinel.as_bytes());
    remove_database_files(&path);
}
