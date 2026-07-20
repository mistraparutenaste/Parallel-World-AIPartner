//! Tauri shell for Parallel World.
//!
//! This layer only wires windows, IPC commands and lifecycle; all
//! behaviour lives in the `pw-*` crates.

pub mod behavior;
pub mod bootstrap;
pub mod character;
pub mod chat;
pub mod commands;
pub mod diagnostics;
pub mod error;
pub mod speech;
pub mod stability_heartbeat;
pub mod supervisor;
pub mod tts;
pub mod ui;
pub mod updates;
pub mod windows;

use std::sync::Arc;

use tauri::Manager;

/// Builds and runs the Tauri application.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start.
#[allow(clippy::too_many_lines)] // Builder wiring is intentionally visible in one lifecycle function.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(commands::character::CharacterState::default())
        .manage(speech::SpeechService::default())
        .manage(chat::ChatService::default())
        .manage(tts::TtsService::default())
        .manage(behavior::DarkExpressionSafetyState::default())
        .manage(supervisor::FrontendRuntimeHealth::default())
        .invoke_handler(tauri::generate_handler![
            commands::app_status::get_app_status,
            commands::character::get_character_manifest,
            commands::character::get_character_settings,
            commands::character::get_character_setup,
            commands::character::import_character_asset,
            commands::character::set_active_character_renderer,
            commands::character::set_expression_idle_timeout,
            commands::character::set_expression,
            commands::character::start_motion,
            commands::character::set_click_through,
            commands::audio::list_microphones,
            commands::audio::start_listening,
            commands::audio::set_input_device,
            commands::audio::stop_listening,
            commands::audio::set_capture_enabled,
            commands::audio::set_speech_playback,
            commands::audio::get_audio_diagnostics,
            commands::audio::get_stt_state,
            commands::behavior::get_behavior_settings,
            commands::behavior::set_behavior_settings,
            commands::safety::get_dark_expression_safety_settings,
            commands::safety::set_safe_word,
            commands::safety::resume_dark_expression,
            commands::diagnostics::get_runtime_diagnostics,
            commands::diagnostics::report_frontend_error,
            commands::diagnostics::list_diagnostic_reports,
            commands::diagnostics::export_diagnostic_reports,
            commands::diagnostics::read_technical_log,
            commands::chat::send_chat_message,
            commands::chat::cancel_turn,
            commands::chat::get_llm_settings,
            commands::chat::set_llm_settings,
            commands::persona::get_persona_profile,
            commands::persona::set_persona_profile,
            commands::data::list_conversation_history,
            commands::data::list_conversation_log,
            commands::data::get_data_usage,
            commands::data::export_user_data,
            commands::data::delete_conversation_history,
            commands::data::delete_memories,
            commands::data::clear_tts_audio_cache,
            commands::memory_center::get_memory_center,
            commands::memory_center::set_memory_domain_control,
            commands::memory_center::set_temporary_conversation,
            commands::ui::get_ui_preferences,
            commands::ui::set_theme_preference,
            commands::ui::set_chat_placement,
            commands::tts::get_tts_settings,
            commands::tts::set_tts_settings,
            commands::tts::list_tts_voices,
            commands::tts::list_user_dict,
            commands::tts::add_user_dict_word,
            commands::tts::delete_user_dict_word,
            supervisor::rearm_managed_process,
            supervisor::report_runtime_failure,
            supervisor::report_runtime_success,
            supervisor::retry_character_renderer,
            supervisor::rearm_runtime_feature,
            commands::updates::get_update_state,
            commands::updates::check_for_updates,
            commands::updates::install_update
        ])
        .setup(|app| {
            app.manage(supervisor::ManagedProcesses::from_environment(
                app.handle().clone(),
            ));
            let layout = bootstrap::initialize(app.handle())?;
            let safety = behavior::load_dark_expression_safety(&layout);
            app.state::<behavior::DarkExpressionSafetyState>()
                .set_paused(safety.dark_expression_paused);
            let heartbeat_path = layout.logs.join("soak-heartbeat.json");
            app.manage(layout);
            app.manage(stability_heartbeat::StabilityHeartbeatService::start(
                app.handle().clone(),
                heartbeat_path,
            )?);
            let updater_configured = app
                .config()
                .plugins
                .0
                .get("updater")
                .and_then(|value| value.get("endpoints"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|endpoints| !endpoints.is_empty());
            if updater_configured {
                let flush_app = app.handle().clone();
                let flusher = updates::IdempotentFlusher::new(move || {
                    flush_app
                        .state::<stability_heartbeat::StabilityHeartbeatService>()
                        .shutdown();
                    flush_app.state::<supervisor::ManagedProcesses>().shutdown();
                });
                let backend = Arc::new(updates::TauriUpdateBackend::new(
                    app.handle().clone(),
                    flusher.clone(),
                ));
                app.manage(updates::UpdateService::enabled(
                    app.package_info().version.to_string(),
                    backend,
                    Arc::new(updates::SettingsUpdateEmitter(app.handle().clone())),
                    flusher,
                ));
                let update_app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = update_app.state::<updates::UpdateService>().check().await {
                        tracing::warn!(%error, "startup update check failed");
                    }
                });
            } else {
                app.manage(updates::UpdateService::disabled(
                    app.package_info().version.to_string(),
                ));
            }
            windows::create_missing_windows(app.handle())?;
            restore_window_states(app.handle());
            if let Err(error) = commands::ui::restore_chat_placement(
                app.handle(),
                &app.state::<pw_platform::paths::AppDataLayout>(),
            ) {
                tracing::warn!(%error, "failed to restore chat placement");
            }
            windows::spawn_cursor_watcher(app.handle().clone());
            // 会話ファーストUI: 起動と同時に音声認識を立ち上げる。モデル
            // 読み込みはワーカースレッド側で進み、進捗は stt-state
            // イベントとして各ウィンドウへ届く。
            let stt_paths =
                speech::SttModelPaths::under(&app.state::<pw_platform::paths::AppDataLayout>());
            if let Err(error) =
                app.state::<speech::SpeechService>()
                    .start(app.handle().clone(), stt_paths, None)
            {
                tracing::warn!(%error, "failed to start speech recognition at launch");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let mut visibilities = Vec::new();
                let mut visibility_complete = true;
                for (label, candidate) in app.webview_windows() {
                    match candidate.is_visible() {
                        Ok(visible) => visibilities.push((label, visible)),
                        Err(error) => {
                            tracing::warn!(%error, %label, "failed to inspect window visibility");
                            visibility_complete = false;
                            break;
                        }
                    }
                }
                if visibility_complete
                    && windows::should_exit_after_close(
                        window.label(),
                        visibilities
                            .iter()
                            .map(|(label, visible)| (label.as_str(), *visible)),
                    )
                {
                    app.exit(0);
                    return;
                }

                if window.label() == "chat" {
                    api.prevent_close();
                    let layout = app.state::<pw_platform::paths::AppDataLayout>();
                    if let Err(error) = commands::ui::dock_chat_on_close(app, &layout) {
                        tracing::warn!(%error, "failed to dock chat on close");
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building parallel-world")
        .run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                app.state::<stability_heartbeat::StabilityHeartbeatService>()
                    .shutdown();
                app.state::<supervisor::ManagedProcesses>().shutdown();
                app.state::<updates::UpdateService>().flush_before_exit();
            }
        });
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
