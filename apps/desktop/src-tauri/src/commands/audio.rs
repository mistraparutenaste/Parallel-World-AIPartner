//! Microphone and STT commands.

use pw_audio::devices::list_input_devices;
use pw_contracts::{AudioDeviceDto, AudioDiagnosticsDto};
use pw_platform::paths::AppDataLayout;
use tauri::{AppHandle, Runtime, State};

use crate::speech::{SpeechService, SttModelPaths};

/// Lists selectable microphones.
#[tauri::command]
#[must_use]
pub fn list_microphones() -> Vec<AudioDeviceDto> {
    list_input_devices()
        .into_iter()
        .map(|device| AudioDeviceDto {
            id: device.id,
            name: device.name,
            is_default: device.is_default,
        })
        .collect()
}

/// Starts the speech pipeline (model loading happens asynchronously;
/// progress arrives via `stt-state` events).
///
/// # Errors
///
/// Returns an error message when a pipeline is already running or
/// the worker cannot be spawned.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn start_listening<R: Runtime>(
    app: AppHandle<R>,
    layout: State<'_, AppDataLayout>,
    service: State<'_, SpeechService>,
    device_id: Option<String>,
) -> Result<(), String> {
    let paths = SttModelPaths::under(&layout);
    service.start(app, paths, device_id)
}

/// Stops the running speech pipeline.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn stop_listening(service: State<'_, SpeechService>) {
    service.stop();
}

/// Mutes or unmutes capture without tearing the pipeline down
/// (also used while TTS is playing).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn set_capture_enabled(service: State<'_, SpeechService>, enabled: bool) {
    service.set_capture_enabled(enabled);
}

/// Returns the pipeline counters for the diagnostics panel.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
#[must_use]
pub fn get_audio_diagnostics(service: State<'_, SpeechService>) -> AudioDiagnosticsDto {
    service.diagnostics()
}
