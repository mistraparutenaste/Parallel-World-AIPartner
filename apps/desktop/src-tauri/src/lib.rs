mod bootstrap;
mod commands;
mod error;
pub mod windows;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let bootstrap_state = bootstrap::initialize(app)?;
            app.manage(bootstrap_state);
            windows::ensure_windows(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status::get_app_status
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Parallel World");
}
