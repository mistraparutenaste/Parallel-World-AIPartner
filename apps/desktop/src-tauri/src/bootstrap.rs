use pw_platform::paths::AppDataLayout;
use tauri::Manager;
use tracing_subscriber::EnvFilter;

use crate::error::BootstrapError;

pub struct BootstrapState {
    _layout: AppDataLayout,
    _log_guard: tracing_appender::non_blocking::WorkerGuard,
}

pub fn initialize<R: tauri::Runtime>(
    app: &tauri::App<R>,
) -> Result<BootstrapState, BootstrapError> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(BootstrapError::AppDataDirectory)?;
    let layout = AppDataLayout::under(root);
    layout
        .create_all()
        .map_err(BootstrapError::CreateDirectories)?;

    let file_appender = tracing_appender::rolling::daily(&layout.logs, "parallel-world.log");
    let (non_blocking, log_guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(non_blocking)
        .try_init()
        .map_err(BootstrapError::InitializeLogging)?;

    tracing::info!("application bootstrap initialized");
    Ok(BootstrapState {
        _layout: layout,
        _log_guard: log_guard,
    })
}
