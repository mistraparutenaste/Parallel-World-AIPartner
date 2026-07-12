//! TTS commands: settings, speaker list, user dictionary.

use std::time::Duration;

use pw_contracts::{TtsSettingsDto, TtsSpeakerDto, UserDictWordDto};
use pw_platform::paths::AppDataLayout;
use pw_tts::{AivisSpeechClient, TtsClientConfig};
use tauri::State;

use crate::tts::{load_tts_settings, save_tts_settings};

fn engine_client(layout: &AppDataLayout) -> Result<AivisSpeechClient, String> {
    let settings = load_tts_settings(layout);
    AivisSpeechClient::new(&TtsClientConfig {
        base_url: settings.base_url,
        timeout: Duration::from_secs(10),
    })
    .map_err(|error| error.to_string())
}

/// Returns the persisted TTS settings (defaults when unset).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
#[must_use]
pub fn get_tts_settings(layout: State<'_, AppDataLayout>) -> TtsSettingsDto {
    load_tts_settings(&layout)
}

/// Validates and persists TTS settings. The synthesis worker is
/// rebuilt with the new settings on the next sentence.
///
/// # Errors
///
/// Returns an error message for invalid endpoints or write failures.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn set_tts_settings(
    layout: State<'_, AppDataLayout>,
    settings: TtsSettingsDto,
) -> Result<(), String> {
    save_tts_settings(&layout, &settings)
}

/// Lists voice styles from the engine, flattened for the dropdown.
///
/// # Errors
///
/// Returns an error message when the engine is unreachable.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn list_tts_speakers(layout: State<'_, AppDataLayout>) -> Result<Vec<TtsSpeakerDto>, String> {
    let client = engine_client(&layout)?;
    let speakers = client.speakers().map_err(|error| error.to_string())?;
    Ok(speakers
        .into_iter()
        .flat_map(|speaker| {
            speaker.styles.into_iter().map(move |style| TtsSpeakerDto {
                name: speaker.name.clone(),
                style_name: style.name,
                style_id: style.id,
            })
        })
        .collect())
}

/// Lists the engine's user dictionary.
///
/// # Errors
///
/// Returns an error message when the engine is unreachable.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn list_user_dict(layout: State<'_, AppDataLayout>) -> Result<Vec<UserDictWordDto>, String> {
    let client = engine_client(&layout)?;
    let words = client.user_dict().map_err(|error| error.to_string())?;
    Ok(words
        .into_iter()
        .map(|word| UserDictWordDto {
            uuid: word.uuid,
            surface: word.surface,
            pronunciation: word.pronunciation,
            accent_type: word.accent_type,
        })
        .collect())
}

/// Adds a pronunciation override, returning its UUID.
///
/// # Errors
///
/// Returns an error message when the input is empty or the engine is
/// unreachable.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn add_user_dict_word(
    layout: State<'_, AppDataLayout>,
    surface: String,
    pronunciation: String,
    accent_type: u32,
) -> Result<String, String> {
    let surface = surface.trim();
    let pronunciation = pronunciation.trim();
    if surface.is_empty() || pronunciation.is_empty() {
        return Err("単語と読みを入力してください".to_owned());
    }
    let client = engine_client(&layout)?;
    client
        .add_user_dict_word(surface, pronunciation, accent_type)
        .map_err(|error| error.to_string())
}

/// Deletes a pronunciation override.
///
/// # Errors
///
/// Returns an error message when the engine is unreachable.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn delete_user_dict_word(layout: State<'_, AppDataLayout>, uuid: String) -> Result<(), String> {
    let client = engine_client(&layout)?;
    client
        .delete_user_dict_word(&uuid)
        .map_err(|error| error.to_string())
}
