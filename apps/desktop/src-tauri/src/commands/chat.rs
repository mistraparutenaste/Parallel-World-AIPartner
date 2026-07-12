//! Conversation commands.

use pw_contracts::LlmSettingsDto;
use pw_platform::paths::AppDataLayout;
use tauri::{AppHandle, Runtime, State};

use crate::chat::{ChatService, load_llm_settings, save_llm_settings};

/// Submits a user text message; the reply streams back as
/// `chat-message` / `conversation-state` events.
///
/// # Errors
///
/// Returns an error message when the message is empty or the
/// conversation worker cannot be started.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn send_chat_message<R: Runtime>(
    app: AppHandle<R>,
    service: State<'_, ChatService>,
    text: String,
) -> Result<(), String> {
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err("メッセージが空です".to_owned());
    }
    service.submit(&app, text)
}

/// Stops the in-flight generation and speech playback immediately
/// (生成途中で停止・発話割り込み).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn cancel_turn<R: Runtime>(
    app: AppHandle<R>,
    service: State<'_, ChatService>,
    tts: State<'_, crate::tts::TtsService>,
) {
    service.cancel();
    tts.stop(&app);
}

/// Returns the persisted LLM settings (defaults when unset).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
#[must_use]
pub fn get_llm_settings(layout: State<'_, AppDataLayout>) -> LlmSettingsDto {
    load_llm_settings(&layout)
}

/// Validates and persists LLM settings. The conversation worker is
/// rebuilt with the new settings on the next message.
///
/// # Errors
///
/// Returns an error message for invalid endpoints or write failures.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn set_llm_settings(
    layout: State<'_, AppDataLayout>,
    settings: LlmSettingsDto,
) -> Result<(), String> {
    save_llm_settings(&layout, &settings)
}
