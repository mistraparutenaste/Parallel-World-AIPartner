mod bootstrap;
mod commands;
mod error;
pub mod windows;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .manage(commands::character_presentation::CharacterPresentationState::default())
        .setup(|app| {
            let bootstrap_state = bootstrap::initialize(app)?;
            app.manage(bootstrap_state);
            windows::ensure_windows(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status::get_app_status,
            commands::character_presentation::get_character_presentation,
            commands::character_presentation::set_character_presentation
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Parallel World");
}
