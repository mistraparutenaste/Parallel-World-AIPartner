//! Application-level error type for the Tauri shell.

/// Errors raised while starting or running the application shell.
///
/// Messages must never contain secrets (API keys, environment variable
/// values); they end up in logs and diagnostics.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("failed to resolve the app data directory: {0}")]
    AppDataDirUnavailable(#[source] tauri::Error),

    #[error("failed to initialize app data directories: {0}")]
    AppDataInit(#[from] std::io::Error),
}
