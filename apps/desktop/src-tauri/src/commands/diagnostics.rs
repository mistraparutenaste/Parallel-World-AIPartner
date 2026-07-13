use pw_contracts::{
    DiagnosticReportDto, FrontendDiagnosticDto, FrontendErrorKindDto, RuntimeDiagnosticsDto,
    SCHEMA_VERSION,
};
use pw_platform::{
    diagnostics::{CrashInput, DiagnosticStore, RetentionPolicy},
    paths::AppDataLayout,
};
use std::path::Path;
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

fn store(layout: &AppDataLayout) -> DiagnosticStore {
    DiagnosticStore::new(&layout.crashes, RetentionPolicy::default())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Stores bounded metadata for a frontend error without raw message or stack text.
/// # Errors
/// Returns an error for an unsupported schema or failed durable write.
pub fn report_frontend_error(
    layout: State<'_, AppDataLayout>,
    report: FrontendDiagnosticDto,
) -> Result<(), String> {
    if report.schema_version != SCHEMA_VERSION {
        return Err("unsupported diagnostic schema".into());
    }
    store(&layout)
        .write(CrashInput::frontend(
            match report.kind {
                FrontendErrorKindDto::WindowError => "window_error",
                FrontendErrorKindDto::UnhandledRejection => "unhandled_rejection",
            },
            report.line,
            report.column,
        ))
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Lists the retained, already-redacted diagnostic reports.
/// # Errors
/// Returns an error if the crash directory cannot be inspected.
pub fn list_diagnostic_reports(
    layout: State<'_, AppDataLayout>,
) -> Result<Vec<DiagnosticReportDto>, String> {
    store(&layout)
        .list()
        .map(|items| {
            items
                .into_iter()
                .map(|item| DiagnosticReportDto {
                    schema_version: SCHEMA_VERSION,
                    id: item.id,
                    timestamp_ms: item.timestamp_ms,
                    category: item.category,
                    bytes: item.bytes,
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
/// Exports retained redacted reports to an explicitly supplied file path.
/// # Errors
/// Returns an error for an unsafe destination, overwrite conflict, or failed I/O.
pub fn export_diagnostic_reports(
    layout: State<'_, AppDataLayout>,
    destination: String,
    allow_overwrite: bool,
) -> Result<(), String> {
    if destination.trim().is_empty() {
        return Err("destination is required".into());
    }
    store(&layout)
        .export(Path::new(destination.trim()), allow_overwrite)
        .map_err(|error| error.to_string())
}
