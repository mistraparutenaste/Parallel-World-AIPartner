//! Tauri shell for Parallel World.
//!
//! This layer only wires windows, IPC commands and lifecycle; all
//! behaviour lives in the `pw-*` crates.

pub mod bootstrap;
pub mod character;
pub mod chat;
pub mod commands;
pub mod error;
pub mod speech;
pub mod tts;
pub mod windows;

use tauri::Manager;

/// Builds and runs the Tauri application.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(commands::character::CharacterState::default())
        .manage(speech::SpeechService::default())
        .manage(chat::ChatService::default())
        .manage(tts::TtsService::default())
        .invoke_handler(tauri::generate_handler![
            commands::app_status::get_app_status,
            commands::character::get_character_manifest,
            commands::character::set_expression,
            commands::character::start_motion,
            commands::character::set_click_through,
            commands::audio::list_microphones,
            commands::audio::start_listening,
            commands::audio::stop_listening,
            commands::audio::set_capture_enabled,
            commands::audio::set_speech_playback,
            commands::audio::get_audio_diagnostics,
            commands::chat::send_chat_message,
            commands::chat::cancel_turn,
            commands::chat::get_llm_settings,
            commands::chat::set_llm_settings,
            commands::data::list_conversation_history,
            commands::data::export_user_data,
            commands::data::delete_conversation_history,
            commands::data::delete_memories,
            commands::tts::get_tts_settings,
            commands::tts::set_tts_settings,
            commands::tts::list_tts_speakers,
            commands::tts::list_user_dict,
            commands::tts::add_user_dict_word,
            commands::tts::delete_user_dict_word
        ])
        .setup(|app| {
            let layout = bootstrap::initialize(app.handle())?;
            app.manage(layout);
            windows::create_missing_windows(app.handle())?;
            restore_window_states(app.handle());
            windows::spawn_cursor_watcher(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running parallel-world");
}

/// Restores saved position and size for every window (best effort).
fn restore_window_states<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri_plugin_window_state::{StateFlags, WindowExt};
    for window in app.webview_windows().values() {
        if let Err(error) = window.restore_state(StateFlags::all()) {
            tracing::warn!(%error, label = window.label(), "failed to restore window state");
        }
    }
}
