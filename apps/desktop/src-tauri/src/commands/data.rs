use crate::chat::ChatService;
use pw_application::history::{ConversationHistory, MessageRole};
use pw_contracts::{
    ChatRoleDto, ConversationHistoryDeletedEventDto, ConversationLogPageDto,
    ConversationMessageDto, SCHEMA_VERSION,
};
use pw_platform::paths::AppDataLayout;
use pw_storage::{Database, SqliteConversationHistory};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Runtime, State};
const CONVERSATION: &str = "default";
fn path(layout: &AppDataLayout) -> PathBuf {
    layout.data.join("parallel-world.sqlite3")
}
fn validated_export_path(
    source: &std::path::Path,
    requested: &std::path::Path,
    allow_overwrite: bool,
) -> Result<PathBuf, String> {
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    if requested.exists() {
        if !requested
            .symlink_metadata()
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
        {
            return Err("保存先は通常ファイルである必要があります".into());
        }
        let destination = requested.canonicalize().map_err(|e| e.to_string())?;
        if same_file::is_same_file(&source, &destination).map_err(|e| e.to_string())? {
            return Err("保存先に使用中のデータベースは指定できません".into());
        }
        if !allow_overwrite {
            return Err("DESTINATION_EXISTS".into());
        }
        return Ok(destination);
    }
    let name = requested
        .file_name()
        .ok_or("保存先にはファイル名を指定してください")?;
    let parent = requested
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .canonicalize()
        .map_err(|_| "保存先ディレクトリが存在しません".to_owned())?;
    Ok(parent.join(name))
}
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Lists durable chat messages. Returns an error when `SQLite` is unavailable.
///
/// # Errors
/// Returns a storage error without preventing live chat from degrading normally.
pub fn list_conversation_history(
    layout: State<'_, AppDataLayout>,
) -> Result<Vec<ConversationMessageDto>, String> {
    let history =
        SqliteConversationHistory::new(Database::open(path(&layout)).map_err(|e| e.to_string())?);
    history
        .list_messages(CONVERSATION)
        .map_err(|e| e.to_string())
        .map(|xs| {
            xs.into_iter()
                .filter_map(|m| {
                    Some(ConversationMessageDto {
                        schema_version: SCHEMA_VERSION,
                        message_id: m.id?,
                        turn_id: m.turn_id,
                        role: match m.role {
                            MessageRole::User => ChatRoleDto::User,
                            MessageRole::Assistant => ChatRoleDto::Assistant,
                        },
                        text: m.content,
                        created_at: m.created_at,
                    })
                })
                .collect()
        })
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn list_conversation_log_at(
    database_path: &std::path::Path,
    before_message_id: Option<i64>,
    query: Option<&str>,
) -> Result<ConversationLogPageDto, String> {
    let query = query.map(str::trim).filter(|value| !value.is_empty());
    if query.is_some_and(|value| value.chars().count() > 200) {
        return Err("検索語は200文字以内で指定してください".into());
    }
    let pattern = query.map(|value| format!("%{}%", escape_like(value)));
    let database = Database::open(database_path).map_err(|error| error.to_string())?;
    let mut statement = database
        .connection()
        .prepare(
            "SELECT id, turn_id, role, content, created_at
             FROM messages
             WHERE conversation_id = ?1
               AND (?2 IS NULL OR content LIKE ?2 ESCAPE '\\')
               AND (?3 IS NULL OR id < ?3)
             ORDER BY id DESC
             LIMIT 101",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            (CONVERSATION, pattern.as_deref(), before_message_id),
            |row| {
                let role: String = row.get(2)?;
                let role = match role.as_str() {
                    "user" => ChatRoleDto::User,
                    "assistant" => ChatRoleDto::Assistant,
                    unknown => {
                        return Err(rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!("unknown role: {unknown}"),
                            )),
                        ));
                    }
                };
                let turn_id: Option<i64> = row.get(1)?;
                Ok(ConversationMessageDto {
                    schema_version: SCHEMA_VERSION,
                    message_id: row.get(0)?,
                    turn_id: turn_id.map(u64::try_from).transpose().map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Integer,
                            Box::new(error),
                        )
                    })?,
                    role,
                    text: row.get(3)?,
                    created_at: row.get(4)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut messages = rows;
    let has_more = messages.len() > 100;
    messages.truncate(100);
    messages.reverse();
    Ok(ConversationLogPageDto {
        schema_version: SCHEMA_VERSION,
        next_before_message_id: has_more
            .then(|| messages.first().map(|message| message.message_id))
            .flatten(),
        messages,
    })
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Lists one read-only page of durable conversation history.
///
/// # Errors
///
/// Returns an error when the history database cannot be queried.
pub async fn list_conversation_log(
    layout: State<'_, AppDataLayout>,
    before_message_id: Option<i64>,
    query: Option<String>,
) -> Result<ConversationLogPageDto, String> {
    let database_path = path(&layout);
    tauri::async_runtime::spawn_blocking(move || {
        list_conversation_log_at(&database_path, before_message_id, query.as_deref())
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Exports a consistent snapshot to an explicitly supplied path.
///
/// # Errors
/// Returns an error for an empty destination or failed `SQLite` backup.
pub fn export_user_data(
    layout: State<'_, AppDataLayout>,
    destination: String,
    allow_overwrite: bool,
) -> Result<(), String> {
    if destination.trim().is_empty() {
        return Err("保存先を指定してください".into());
    }
    let source = path(&layout);
    let destination = validated_export_path(
        &source,
        std::path::Path::new(destination.trim()),
        allow_overwrite,
    )?;
    export_database(&source, &destination)
}
fn export_database(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    Database::open(source)
        .map_err(|e| e.to_string())?
        .backup_to(destination)
        .map_err(|e| e.to_string())
}
fn delete_history_core(
    database_path: &std::path::Path,
) -> Result<ConversationHistoryDeletedEventDto, String> {
    let mut db = Database::open(database_path).map_err(|e| e.to_string())?;
    let tx = db
        .connection_mut()
        .transaction()
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM conversations WHERE id=?1", [CONVERSATION])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(ConversationHistoryDeletedEventDto {
        schema_version: SCHEMA_VERSION,
    })
}
fn execute_delete_history(
    reset: impl FnOnce(&mut dyn FnMut() -> Result<(), String>) -> Result<(), String>,
    delete: impl FnOnce() -> Result<ConversationHistoryDeletedEventDto, String>,
    emit: impl FnOnce(ConversationHistoryDeletedEventDto) -> Result<(), String>,
) -> Result<(), String> {
    let mut delete = Some(delete);
    let mut emit = Some(emit);
    let mut operation = || {
        let payload = delete.take().expect("operation called once")()?;
        emit.take().expect("operation called once")(payload)
    };
    reset(&mut operation)
}
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Stops chat and atomically deletes its durable history.
///
/// # Errors
/// Returns an error when shutdown or the transaction fails.
pub fn delete_conversation_history<R: Runtime>(
    app: AppHandle<R>,
    layout: State<'_, AppDataLayout>,
    chat: State<'_, ChatService>,
) -> Result<(), String> {
    execute_delete_history(
        |operation| chat.with_exclusive_reset(operation),
        || delete_history_core(&path(&layout)),
        |payload| {
            app.emit("conversation-history-deleted", payload)
                .map_err(|e| e.to_string())
        },
    )
}
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Stops chat and atomically deletes summaries and long-term memories.
///
/// # Errors
/// Returns an error when shutdown or the transaction fails.
pub fn delete_memories(
    layout: State<'_, AppDataLayout>,
    chat: State<'_, ChatService>,
) -> Result<(), String> {
    chat.with_exclusive_reset(|| {
        let mut db = Database::open(path(&layout)).map_err(|e| e.to_string())?;
        let tx = db
            .connection_mut()
            .transaction()
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM conversation_summaries", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM memories", [])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::{
        delete_history_core, execute_delete_history, export_database, list_conversation_log_at,
        validated_export_path,
    };
    use pw_application::history::{ConversationHistory, StoredConversation};
    use pw_contracts::{ConversationHistoryDeletedEventDto, SCHEMA_VERSION};
    use pw_storage::{Database, SqliteConversationHistory};

    #[test]
    fn conversation_log_pages_latest_messages_and_searches_literal_wildcards() {
        let root = std::env::temp_dir().join(format!("pw-conversation-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("history.sqlite3");
        let database = Database::open(&path).unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO conversations (id, created_at, updated_at) VALUES ('default', 1, 1)",
                [],
            )
            .unwrap();
        for index in 1..=105 {
            let content = if index == 42 {
                "needle%_literal".to_owned()
            } else {
                format!("message-{index}")
            };
            database
                .connection()
                .execute(
                    "INSERT INTO messages (conversation_id, turn_id, role, content, created_at)
                     VALUES ('default', ?1, 'user', ?2, ?1)",
                    (index, content),
                )
                .unwrap();
        }
        drop(database);

        let first = list_conversation_log_at(&path, None, None).unwrap();
        assert_eq!(first.messages.len(), 100);
        assert_eq!(first.messages.first().unwrap().message_id, 6);
        assert_eq!(first.messages.last().unwrap().message_id, 105);
        assert_eq!(first.next_before_message_id, Some(6));

        let older = list_conversation_log_at(&path, first.next_before_message_id, None).unwrap();
        assert_eq!(older.messages.len(), 5);
        assert_eq!(older.next_before_message_id, None);

        let searched = list_conversation_log_at(&path, None, Some("%_")).unwrap();
        assert_eq!(searched.messages.len(), 1);
        assert_eq!(searched.messages[0].text, "needle%_literal");
        assert!(list_conversation_log_at(&path, None, Some(&"x".repeat(201))).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn canonicalization_rejects_parent_alias_and_hardlink() {
        let root = std::env::temp_dir().join(format!("pw-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("child")).unwrap();
        let source = root.join("source");
        std::fs::write(&source, b"x").unwrap();
        assert!(validated_export_path(&source, &root.join("child/../source"), true).is_err());
        let hard = root.join("hard");
        std::fs::hard_link(&source, &hard).unwrap();
        assert!(validated_export_path(&source, &hard, true).is_err());
        assert_eq!(
            validated_export_path(&source, &root.join("child/../new"), false).unwrap(),
            root.canonicalize().unwrap().join("new")
        );
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn only_regular_files_may_be_explicitly_overwritten() {
        let root = std::env::temp_dir().join(format!("pw-export-kind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source");
        let other = root.join("other");
        std::fs::write(&source, b"x").unwrap();
        std::fs::write(&other, b"y").unwrap();
        assert!(validated_export_path(&source, &root, false).is_err());
        assert_eq!(
            validated_export_path(&source, &other, false).unwrap_err(),
            "DESTINATION_EXISTS"
        );
        assert!(validated_export_path(&source, &other, true).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn command_cores_export_then_delete_and_return_typed_notification() {
        let root = std::env::temp_dir().join(format!("pw-command-core-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source.sqlite3");
        let export = root.join("export.sqlite3");
        let mut history = SqliteConversationHistory::new(Database::open(&source).unwrap());
        history
            .upsert_conversation(&StoredConversation {
                id: "default".into(),
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        drop(history);
        export_database(&source, &export).unwrap();
        assert_eq!(
            Database::open(&export)
                .unwrap()
                .connection()
                .query_row("SELECT count(*) FROM conversations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            delete_history_core(&source).unwrap().schema_version,
            pw_contracts::SCHEMA_VERSION
        );
        assert_eq!(
            Database::open(&source)
                .unwrap()
                .connection()
                .query_row("SELECT count(*) FROM conversations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn delete_wrapper_orders_reset_delete_emit_and_propagates_failures() {
        use std::cell::RefCell;
        let order = RefCell::new(Vec::new());
        execute_delete_history(
            |op| {
                order.borrow_mut().push("reset");
                op()
            },
            || {
                order.borrow_mut().push("delete");
                Ok(ConversationHistoryDeletedEventDto {
                    schema_version: SCHEMA_VERSION,
                })
            },
            |payload| {
                assert_eq!(payload.schema_version, SCHEMA_VERSION);
                order.borrow_mut().push("emit");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*order.borrow(), ["reset", "delete", "emit"]);
        let touched = RefCell::new(Vec::new());
        assert!(
            execute_delete_history(
                |_| Err("reset".into()),
                || {
                    touched.borrow_mut().push("delete");
                    unreachable!()
                },
                |_| {
                    touched.borrow_mut().push("emit");
                    Ok(())
                }
            )
            .is_err()
        );
        assert!(touched.borrow().is_empty());
        assert!(
            execute_delete_history(
                |op| op(),
                || Ok(ConversationHistoryDeletedEventDto {
                    schema_version: SCHEMA_VERSION
                }),
                |_| Err("emit".into())
            )
            .is_err()
        );
    }
}
