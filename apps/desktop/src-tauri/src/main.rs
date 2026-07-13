#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    pw_platform::diagnostics::install_panic_hook(pw_platform::diagnostics::DiagnosticStore::new(
        std::env::temp_dir().join("parallel-world-early-crashes"),
        pw_platform::diagnostics::RetentionPolicy::default(),
    ));
    pw_platform::diagnostics::start_diagnostic_maintenance()
        .expect("failed to start diagnostic retention worker");
    parallel_world_desktop::run();
}
