use std::sync::atomic::AtomicBool;
use std::time::Duration;

use pw_application::{
    conversation::LlmClient,
    memory::{SelfReviewGenerator, is_safe_persistent_content, redact_persistent_content},
};
use pw_contracts::SelfReviewDto;
use pw_llm::{LlmClientConfig, OpenAiCompatClient, SamplingOptions};
use pw_platform::paths::AppDataLayout;
use pw_storage::Database;
use rusqlite::{OptionalExtension, params};
use tauri::State;

use crate::chat::{load_llm_api_key, load_llm_settings};

const CONVERSATION: &str = "default";
const MAX_TRANSCRIPT_MESSAGES: i64 = 50;
const MAX_REVIEW_CHARS: usize = 4_000;

fn get_self_review_at(layout: &AppDataLayout) -> Result<Option<SelfReviewDto>, String> {
    let database = Database::open(layout.main_database()).map_err(|error| error.to_string())?;
    database
        .connection()
        .query_row(
            "SELECT content,generated_at,source_message_id FROM self_reviews WHERE conversation_id=?1",
            [CONVERSATION],
            |row| {
                Ok(SelfReviewDto {
                    content: row.get(0)?,
                    generated_at: row.get(1)?,
                    source_message_id: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn latest_message_id(database: &Database) -> Result<Option<i64>, String> {
    database
        .connection()
        .query_row(
            "SELECT MAX(id) FROM messages WHERE conversation_id=?1",
            [CONVERSATION],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn transcript(database: &Database) -> Result<String, String> {
    let mut statement = database
        .connection()
        .prepare(
            "SELECT role,content FROM (
               SELECT id,role,content FROM messages
               WHERE conversation_id=?1 ORDER BY id DESC LIMIT ?2
             ) ORDER BY id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![CONVERSATION, MAX_TRANSCRIPT_MESSAGES], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut output = String::new();
    for row in rows {
        let (role, content) = row.map_err(|error| error.to_string())?;
        output.push_str(&role);
        output.push_str(": ");
        output.push_str(&content);
        output.push('\n');
    }
    Ok(output)
}

pub(crate) fn regenerate_self_review_at(
    layout: &AppDataLayout,
    only_if_stale: bool,
) -> Result<Option<SelfReviewDto>, String> {
    let database = Database::open(layout.main_database()).map_err(|error| error.to_string())?;
    let Some(source_message_id) = latest_message_id(&database)? else {
        return Ok(None);
    };
    if only_if_stale {
        let previous: Option<i64> = database
            .connection()
            .query_row(
                "SELECT source_message_id FROM self_reviews WHERE conversation_id=?1",
                [CONVERSATION],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .flatten();
        if previous.is_some_and(|value| value >= source_message_id) {
            return get_self_review_at(layout);
        }
    }
    let transcript = transcript(&database)?;
    drop(database);
    let settings = load_llm_settings(layout);
    let mut llm = OpenAiCompatClient::new(LlmClientConfig {
        base_url: settings.base_url,
        model: settings.model,
        api_key: load_llm_api_key(settings.provider)?,
        allow_remote: settings.allow_remote,
        timeout: Duration::from_secs(30),
        sampling: SamplingOptions {
            temperature: Some(0.4),
            max_tokens: Some(800),
            ..SamplingOptions::default()
        },
    })
    .map_err(|error| error.to_string())?;
    let prompt = SelfReviewGenerator::prompt(&transcript);
    let cancel = AtomicBool::new(false);
    let mut content = String::new();
    llm.stream_chat(&prompt, &cancel, &mut |delta| content.push_str(delta))
        .map_err(|error| error.to_string())?;
    let content = redact_persistent_content(content.trim());
    if content.is_empty()
        || content.chars().count() > MAX_REVIEW_CHARS
        || !is_safe_persistent_content(&content)
    {
        return Err("SELF_REVIEW_INVALID".into());
    }
    let generated_at = super::memory_center::now();
    let database = Database::open(layout.main_database()).map_err(|error| error.to_string())?;
    database
        .connection()
        .execute(
            "INSERT INTO self_reviews(conversation_id,content,generated_at,source_message_id)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(conversation_id) DO UPDATE SET
               content=excluded.content,
               generated_at=excluded.generated_at,
               source_message_id=excluded.source_message_id",
            params![CONVERSATION, content, generated_at, source_message_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(Some(SelfReviewDto {
        content,
        generated_at,
        source_message_id: Some(source_message_id),
    }))
}

/// Loads the latest generated self review, if one exists.
///
/// # Errors
/// Returns an error when the database cannot be queried.
#[tauri::command]
pub async fn get_self_review(
    layout: State<'_, AppDataLayout>,
) -> Result<Option<SelfReviewDto>, String> {
    let layout = layout.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_self_review_at(&layout))
        .await
        .map_err(|error| error.to_string())?
}

/// Generates and stores a fresh self review from recent conversation history.
///
/// # Errors
/// Returns an error when history, settings, the LLM, or storage is unavailable.
#[tauri::command]
pub async fn regenerate_self_review(
    layout: State<'_, AppDataLayout>,
) -> Result<Option<SelfReviewDto>, String> {
    let layout = layout.inner().clone();
    tauri::async_runtime::spawn_blocking(move || regenerate_self_review_at(&layout, false))
        .await
        .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_history_has_no_review_and_does_not_call_llm() {
        let root = std::env::temp_dir().join(format!("pw-self-review-{}", std::process::id()));
        let layout = AppDataLayout::under(root.clone());
        layout.create_all().unwrap();
        Database::open(layout.main_database()).unwrap();
        assert_eq!(regenerate_self_review_at(&layout, false).unwrap(), None);
        assert_eq!(get_self_review_at(&layout).unwrap(), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
