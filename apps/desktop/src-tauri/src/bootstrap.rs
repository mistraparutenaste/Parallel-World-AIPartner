//! Startup wiring: app data directories and logging.

use pw_platform::paths::AppDataLayout;
use tauri::{AppHandle, Manager, Runtime};
use tracing_appender::non_blocking::WorkerGuard;

use crate::error::AppError;

/// Keeps the non-blocking log writer alive for the app lifetime.
struct LogWriterGuard(#[allow(dead_code)] WorkerGuard);

/// Resolves the app data layout, creates all runtime directories and
/// installs a daily-rotating file logger under `logs/`.
///
/// Log output must never include API keys or environment variable
/// values.
///
/// # Errors
///
/// Returns [`AppError`] when the app data directory cannot be resolved
/// or created.
pub fn initialize<R: Runtime>(app: &AppHandle<R>) -> Result<AppDataLayout, AppError> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(AppError::AppDataDirUnavailable)?;
    let layout = AppDataLayout::under(root);
    layout.create_all()?;

    let file_appender = tracing_appender::rolling::daily(&layout.logs, "parallel-world.log");
    let (writer, guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(writer)
        .with_ansi(false)
        .init();
    app.manage(LogWriterGuard(guard));

    tracing::info!(root = %layout.root.display(), "app data initialized");
    Ok(layout)
}
