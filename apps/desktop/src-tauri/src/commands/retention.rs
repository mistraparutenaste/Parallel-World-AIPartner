use std::path::Path;

use pw_contracts::{RetentionSettingsDto, SCHEMA_VERSION};
use pw_platform::{
    config_io::{JsonFormat, read_json_lenient, write_atomic_json},
    paths::AppDataLayout,
};
use pw_storage::{Database, SqliteConversationHistory};
use tauri::State;

const FILE_NAME: &str = "retention.json";
const DEFAULT_KEEP_MESSAGES: u32 = 30;
const MAX_KEEP_MESSAGES: u32 = 10_000;
const DEFAULT_CONVERSATION: &str = "default";

fn defaults() -> RetentionSettingsDto {
    RetentionSettingsDto {
        schema_version: SCHEMA_VERSION,
        keep_messages: DEFAULT_KEEP_MESSAGES,
    }
}

pub(crate) fn load_retention_settings(layout: &AppDataLayout) -> RetentionSettingsDto {
    read_json_lenient::<RetentionSettingsDto>(&layout.config.join(FILE_NAME))
        .filter(|settings| (1..=MAX_KEEP_MESSAGES).contains(&settings.keep_messages))
        .unwrap_or_else(defaults)
}

fn save_retention_settings(
    layout: &AppDataLayout,
    keep_messages: u32,
) -> Result<RetentionSettingsDto, String> {
    if !(1..=MAX_KEEP_MESSAGES).contains(&keep_messages) {
        return Err("保持件数は1から10000の範囲で指定してください".into());
    }
    let settings = RetentionSettingsDto {
        schema_version: SCHEMA_VERSION,
        keep_messages,
    };
    write_atomic_json(&layout.config, FILE_NAME, &settings, JsonFormat::Pretty)
        .map_err(|error| error.to_string())?;
    Ok(settings)
}

fn layout_from_database(database_path: &Path) -> Option<AppDataLayout> {
    let root = database_path.parent()?.parent()?.to_path_buf();
    Some(AppDataLayout::under(root))
}

pub(crate) fn prune_messages_at(database_path: &Path) -> Result<usize, String> {
    let layout = layout_from_database(database_path)
        .ok_or_else(|| "database path has no application root".to_owned())?;
    let settings = load_retention_settings(&layout);
    let database = Database::open(database_path).map_err(|error| error.to_string())?;
    SqliteConversationHistory::new(database)
        .prune_summarized_messages(
            DEFAULT_CONVERSATION,
            usize::try_from(settings.keep_messages).unwrap_or(usize::MAX),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn get_retention_settings(layout: State<'_, AppDataLayout>) -> RetentionSettingsDto {
    load_retention_settings(&layout)
}

/// # Errors
/// Returns an error for an out-of-range value or an atomic write failure.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn set_retention_settings(
    layout: State<'_, AppDataLayout>,
    keep_messages: u32,
) -> Result<RetentionSettingsDto, String> {
    save_retention_settings(&layout, keep_messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_to_thirty_and_reject_zero() {
        let root = std::env::temp_dir().join(format!("pw-retention-{}", std::process::id()));
        let layout = AppDataLayout::under(root.clone());
        layout.create_all().unwrap();
        assert_eq!(load_retention_settings(&layout).keep_messages, 30);
        assert!(save_retention_settings(&layout, 0).is_err());
        assert_eq!(
            save_retention_settings(&layout, 42).unwrap().keep_messages,
            42
        );
        assert_eq!(load_retention_settings(&layout).keep_messages, 42);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pruning_keeps_newest_rows_and_never_deletes_the_summary_cursor() {
        let root = std::env::temp_dir().join(format!("pw-retention-prune-{}", std::process::id()));
        let layout = AppDataLayout::under(root.clone());
        layout.create_all().unwrap();
        let database_path = layout.main_database();
        let database = Database::open(&database_path).unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO conversations(id,created_at,updated_at) VALUES('default',1,1)",
                [],
            )
            .unwrap();
        for id in 1..=40 {
            database
                .connection()
                .execute(
                    "INSERT INTO messages(id,conversation_id,role,content,created_at) VALUES(?1,'default','user','message',?1)",
                    [id],
                )
                .unwrap();
        }
        database
            .connection()
            .execute(
                "INSERT INTO conversation_summaries(conversation_id,content,through_message_id,updated_at) VALUES('default','summary',35,1)",
                [],
            )
            .unwrap();
        drop(database);
        assert_eq!(prune_messages_at(&database_path).unwrap(), 10);
        let database = Database::open(&database_path).unwrap();
        let remaining: (i64, i64) = database
            .connection()
            .query_row("SELECT COUNT(*),MIN(id) FROM messages", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(remaining, (30, 11));
        assert_eq!(
            database
                .connection()
                .query_row(
                    "SELECT through_message_id FROM conversation_summaries",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            35
        );
        drop(database);
        std::fs::remove_dir_all(root).unwrap();
    }
}
