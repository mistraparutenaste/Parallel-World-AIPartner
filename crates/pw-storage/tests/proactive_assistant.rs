use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pw_application::history::{
    ConversationHistory, MessageRole, ProactiveAssistantHistory, ProactiveAssistantMessage,
    StoredTurn,
};
use pw_storage::{Database, SqliteConversationHistory};

fn unique_database_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "parallel-world-proactive-assistant-{label}-{}-{nonce}.sqlite3",
        std::process::id()
    ))
}

fn remove_database_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

fn message(conversation_id: &str, content: &str, created_at: i64) -> ProactiveAssistantMessage {
    ProactiveAssistantMessage {
        conversation_id: conversation_id.to_owned(),
        content: content.to_owned(),
        created_at,
    }
}

fn table_count(history: &SqliteConversationHistory, table: &str) -> i64 {
    history
        .database()
        .connection()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn proactive_assistant_append_creates_only_one_assistant_message_at_turn_one() {
    let mut history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
    let persisted = history
        .append_proactive_assistant(&message("chat", "hello", 100))
        .unwrap();
    assert_eq!(persisted.turn_id, 1);
    assert_eq!(persisted.message_id, 1);
    assert_eq!(table_count(&history, "conversations"), 1);
    assert_eq!(table_count(&history, "messages"), 1);
    let messages = history.list_messages("chat").unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, MessageRole::Assistant);
    assert_eq!(messages[0].turn_id, Some(1));
    assert_eq!(messages[0].content, "hello");
}

#[test]
fn proactive_assistant_appends_have_monotonic_turn_and_message_ids() {
    let mut history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
    let first = history
        .append_proactive_assistant(&message("chat", "first", 10))
        .unwrap();
    let second = history
        .append_proactive_assistant(&message("chat", "second", 20))
        .unwrap();
    assert_eq!((first.turn_id, second.turn_id), (1, 2));
    assert!(second.message_id > first.message_id);
}

#[test]
fn proactive_and_normal_turn_allocation_interoperate_across_reopen() {
    let path = unique_database_path("interoperate");
    {
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        assert_eq!(history.reserve_turn_id("chat", 1).unwrap(), 1);
        let proactive = history
            .append_proactive_assistant(&message("chat", "proactive", 2))
            .unwrap();
        assert_eq!(proactive.turn_id, 2);
        let normal_turn = history.reserve_turn_id("chat", 3).unwrap();
        assert_eq!(normal_turn, 3);
        history
            .store_completed_turn(&StoredTurn {
                conversation_id: "chat".into(),
                turn_id: normal_turn,
                user_content: "user".into(),
                assistant_content: "assistant".into(),
                created_at: 3,
            })
            .unwrap();
    }
    let mut reopened = SqliteConversationHistory::new(Database::open(&path).unwrap());
    let fourth = reopened
        .append_proactive_assistant(&message("chat", "after reopen", 4))
        .unwrap();
    assert_eq!(fourth.turn_id, 4);
    let turns = reopened
        .list_messages("chat")
        .unwrap()
        .into_iter()
        .filter_map(|message| message.turn_id)
        .collect::<Vec<_>>();
    assert_eq!(turns, vec![2, 3, 3, 4]);
    drop(reopened);
    remove_database_files(&path);
}

#[test]
fn proactive_assistant_older_timestamp_does_not_move_conversation_backward() {
    let mut history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
    history
        .append_proactive_assistant(&message("chat", "newer", 200))
        .unwrap();
    history
        .append_proactive_assistant(&message("chat", "older", 100))
        .unwrap();
    let updated_at: i64 = history
        .database()
        .connection()
        .query_row(
            "SELECT updated_at FROM conversations WHERE id='chat'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(updated_at, 200);
}

#[test]
fn proactive_assistant_content_contract_uses_utf8_byte_length() {
    let mut history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
    let boundary = "a".repeat(65_536);
    history
        .append_proactive_assistant(&message("chat", &boundary, 1))
        .unwrap();
    let over = "b".repeat(65_537);
    assert!(
        history
            .append_proactive_assistant(&message("chat", &over, 2))
            .is_err()
    );
    let multibyte_boundary = "界".repeat(21_845);
    assert_eq!(multibyte_boundary.len(), 65_535);
    history
        .append_proactive_assistant(&message("chat", &multibyte_boundary, 3))
        .unwrap();
    let multibyte_over = "界".repeat(21_846);
    assert!(multibyte_over.len() > 65_536);
    assert!(
        history
            .append_proactive_assistant(&message("chat", &multibyte_over, 4))
            .is_err()
    );
}

#[test]
fn proactive_assistant_invalid_inputs_write_nothing() {
    let mut history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
    for invalid in [
        message("", "content", 1),
        message("chat", "", 1),
        message("chat", "content", -1),
    ] {
        assert!(history.append_proactive_assistant(&invalid).is_err());
    }
    assert_eq!(table_count(&history, "conversations"), 0);
    assert_eq!(table_count(&history, "conversation_turn_sequences"), 0);
    assert_eq!(table_count(&history, "messages"), 0);
}

#[test]
fn proactive_assistant_message_and_final_update_failures_roll_back_unconsumed_turn() {
    for (label, trigger) in [
        (
            "message",
            "CREATE TRIGGER fail_proactive_message BEFORE INSERT ON messages
             WHEN NEW.content='ROLLBACK-SENTINEL'
             BEGIN SELECT RAISE(ABORT, 'ROLLBACK-SENTINEL'); END;",
        ),
        (
            "update",
            "CREATE TRIGGER fail_proactive_update BEFORE UPDATE OF updated_at ON conversations
             WHEN NEW.id='chat'
             BEGIN SELECT RAISE(ABORT, 'ROLLBACK-SENTINEL'); END;",
        ),
    ] {
        let mut history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
        history
            .database()
            .connection()
            .execute_batch(trigger)
            .unwrap();
        let error = history
            .append_proactive_assistant(&message("chat", "ROLLBACK-SENTINEL", 1))
            .unwrap_err();
        let exposed = format!("{error}{error:?}");
        assert!(!exposed.contains("ROLLBACK-SENTINEL"));
        assert!(std::error::Error::source(&error).is_none());
        assert_eq!(table_count(&history, "conversations"), 0, "{label}");
        assert_eq!(
            table_count(&history, "conversation_turn_sequences"),
            0,
            "{label}"
        );
        assert_eq!(table_count(&history, "messages"), 0, "{label}");
        history
            .database()
            .connection()
            .execute_batch("DROP TRIGGER IF EXISTS fail_proactive_message; DROP TRIGGER IF EXISTS fail_proactive_update;")
            .unwrap();
        let persisted = history
            .append_proactive_assistant(&message("chat", "recovered", 2))
            .unwrap();
        assert_eq!(persisted.turn_id, 1, "{label}");
    }
}

#[test]
fn proactive_assistant_sequence_overflow_is_stable_and_never_promotes_to_real() {
    let path = unique_database_path("overflow");
    {
        let database = Database::open(&path).unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO conversations(id,created_at,updated_at) VALUES('chat',1,1)",
                [],
            )
            .unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO conversation_turn_sequences(conversation_id,next_turn_id) VALUES('chat',?1)",
                [i64::MAX],
            )
            .unwrap();
    }
    let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
    assert!(history.reserve_turn_id("chat", 2).is_err());
    let error = history
        .append_proactive_assistant(&message("chat", "content", 2))
        .unwrap_err();
    assert_eq!(
        format!("{error}"),
        "proactive assistant history unavailable"
    );
    assert_eq!(format!("{error:?}"), "ProactiveAssistantHistoryError");
    assert!(std::error::Error::source(&error).is_none());
    drop(history);

    let reopened = Database::open(&path).unwrap();
    let sequence: (String, i64) = reopened
        .connection()
        .query_row(
            "SELECT typeof(next_turn_id),next_turn_id FROM conversation_turn_sequences WHERE conversation_id='chat'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(sequence, ("integer".into(), i64::MAX));
    assert_eq!(
        reopened
            .connection()
            .query_row(
                "SELECT updated_at FROM conversations WHERE id='chat'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        reopened
            .connection()
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(reopened);
    remove_database_files(&path);
}
