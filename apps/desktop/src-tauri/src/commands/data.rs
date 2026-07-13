use crate::chat::ChatService;
use pw_application::history::{ConversationHistory, MessageRole};
use pw_contracts::{ChatRoleDto, ConversationMessageDto, SCHEMA_VERSION};
use pw_platform::paths::AppDataLayout;
use pw_storage::{Database, SqliteConversationHistory};
use std::path::PathBuf;
use tauri::State;
const CONVERSATION: &str = "chat";
fn path(layout: &AppDataLayout) -> PathBuf {
    layout.data.join("parallel-world.sqlite3")
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
    let destination = PathBuf::from(destination.trim());
    if source == destination {
        return Err("保存先に使用中のデータベースは指定できません".into());
    }
    if destination.is_dir() {
        return Err("保存先にはファイルを指定してください".into());
    }
    if destination.exists() && !allow_overwrite {
        return Err("保存先は既に存在します".into());
    }
    if destination.parent().is_none_or(|parent| !parent.is_dir()) {
        return Err("保存先ディレクトリが存在しません".into());
    }
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
pub fn delete_conversation_history(
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
        tx.commit().map_err(|e| e.to_string())
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
