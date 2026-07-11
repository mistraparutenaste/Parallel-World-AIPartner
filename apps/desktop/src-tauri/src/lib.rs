//! Tauri shell for Parallel World.
//!
//! This layer only wires windows, IPC commands and lifecycle; all
//! behaviour lives in the `pw-*` crates.

pub mod commands;
pub mod windows;

/// Builds and runs the Tauri application.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start.
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::app_status::get_app_status
        ])
        .setup(|app| {
            windows::create_missing_windows(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running parallel-world");
}
