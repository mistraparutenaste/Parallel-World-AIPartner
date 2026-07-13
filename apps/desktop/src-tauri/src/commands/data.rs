use crate::chat::ChatService;
use pw_application::history::{ConversationHistory, MessageRole};
use pw_contracts::{
    ChatRoleDto, ConversationHistoryDeletedEventDto, ConversationMessageDto, SCHEMA_VERSION,
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
    Database::open(source)
        .map_err(|e| e.to_string())?
        .backup_to(destination)
        .map_err(|e| e.to_string())
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
    chat.with_exclusive_reset(|| {
        let mut db = Database::open(path(&layout)).map_err(|e| e.to_string())?;
        let tx = db
            .connection_mut()
            .transaction()
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM conversations WHERE id=?1", [CONVERSATION])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        app.emit(
            "conversation-history-deleted",
            ConversationHistoryDeletedEventDto {
                schema_version: SCHEMA_VERSION,
            },
        )
        .map_err(|e| e.to_string())
    })
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
    use super::validated_export_path;
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
}
