use pw_contracts::{RuntimeDiagnosticsDto, SCHEMA_VERSION};
use tauri::State;

use crate::{chat::ChatService, tts::TtsService};

#[tauri::command]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn get_runtime_diagnostics(
    chat: State<'_, ChatService>,
    tts: State<'_, TtsService>,
) -> RuntimeDiagnosticsDto {
    let mut queues = chat.queue_metrics();
    queues.push(tts.queue_metrics());
    RuntimeDiagnosticsDto {
        schema_version: SCHEMA_VERSION,
        queues,
    }
}
