#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let early_store = pw_platform::diagnostics::DiagnosticStore::new(
        std::env::temp_dir().join("parallel-world-early-crashes"),
        pw_platform::diagnostics::RetentionPolicy::default(),
    );
    let _ = early_store.recover_after_unclean_shutdown();
    pw_platform::diagnostics::install_panic_hook(early_store);
    pw_platform::diagnostics::start_diagnostic_maintenance()
        .expect("failed to start diagnostic retention worker");
    parallel_world_desktop::run();
}
