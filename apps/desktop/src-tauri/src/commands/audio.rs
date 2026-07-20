//! Microphone and STT commands.

use pw_audio::devices::list_input_devices;
use pw_contracts::{AudioDeviceDto, AudioDiagnosticsDto, SttStateEventDto};
use pw_platform::paths::AppDataLayout;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::speech::{STATE_EVENT, SpeechService, SttModelPaths};

/// Lists selectable microphones.
///
/// # Errors
///
/// Returns an error when the blocking command worker cannot be joined.
#[tauri::command]
pub async fn list_microphones() -> Result<Vec<AudioDeviceDto>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        list_input_devices()
            .into_iter()
            .map(|device| AudioDeviceDto {
                id: device.id,
                name: device.name,
                is_default: device.is_default,
            })
            .collect()
    })
    .await
    .map_err(|error| format!("audio command worker failed: {error}"))
}

/// Starts the speech pipeline (model loading happens asynchronously;
/// progress arrives via `stt-state` events).
///
/// # Errors
///
/// Returns an error message when a pipeline is already running or
/// the worker cannot be spawned.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri injects an owned AppHandle.
pub fn start_listening<R: Runtime>(
    app: AppHandle<R>,
    device_id: Option<String>,
) -> Result<(), String> {
    let paths = SttModelPaths::under(&app.state::<AppDataLayout>());
    let service = app.state::<SpeechService>();
    service.start(app.clone(), paths, device_id)
}

/// Requests a microphone switch while keeping the speech pipeline running.
/// The worker replaces the capture session and retains the loaded STT models.
#[tauri::command]
pub async fn set_input_device<R: Runtime>(
    app: AppHandle<R>,
    device_id: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<SpeechService>().set_input_device(device_id);
    })
    .await
    .map_err(|error| format!("audio command worker failed: {error}"))
}

/// Stops the running speech pipeline.
///
/// # Errors
///
/// Returns an error when the stopped state cannot be emitted.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri injects an owned AppHandle.
pub fn stop_listening<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let state = app.state::<SpeechService>().stop();
    app.emit(STATE_EVENT, state)
        .map_err(|error| format!("failed to emit stopped speech state: {error}"))
}

/// Mutes or unmutes capture without tearing the pipeline down
/// (also used while TTS is playing).
///
/// # Errors
///
/// Returns an error when the blocking command worker cannot be joined.
#[tauri::command]
pub async fn set_capture_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<SpeechService>().set_capture_enabled(enabled);
    })
    .await
    .map_err(|error| format!("audio command worker failed: {error}"))
}

/// Reported by the character window around speech playback: capture
/// is muted while TTS audio is playing so the assistant does not hear
/// itself (基本設計 Phase 2完了条件).
///
/// # Errors
///
/// Returns an error when the blocking command worker cannot be joined.
#[tauri::command]
pub async fn set_speech_playback<R: Runtime>(
    app: AppHandle<R>,
    active: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<SpeechService>().set_capture_enabled(!active);
    })
    .await
    .map_err(|error| format!("audio command worker failed: {error}"))
}

/// Returns the pipeline counters for the diagnostics panel.
///
/// # Errors
///
/// Returns an error when the blocking command worker cannot be joined.
#[tauri::command]
pub async fn get_audio_diagnostics<R: Runtime>(
    app: AppHandle<R>,
) -> Result<AudioDiagnosticsDto, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<SpeechService>().diagnostics())
        .await
        .map_err(|error| format!("audio command worker failed: {error}"))
}

/// Returns the latest STT phase so windows mounted after an event can hydrate.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri injects an owned AppHandle.
pub fn get_stt_state<R: Runtime>(app: AppHandle<R>) -> SttStateEventDto {
    app.state::<SpeechService>().current_state()
}
