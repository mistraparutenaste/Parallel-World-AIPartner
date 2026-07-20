fn main() {
    // Windows installer detection treats test executables containing "updater"
    // as installers unless their execution level is explicit. The manifest
    // also declares Common-Controls v6 so loader imports such as
    // `TaskDialogIndirect` resolve. These flags only reach integration
    // tests; the main binary gets its manifest from tauri-build, and the
    // lib unittest binary gets none, so tests that link the full Tauri
    // runtime (mock app) must live under `tests/`.
    #[cfg(target_os = "windows")]
    {
        let test_manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("windows-test.manifest");
        println!("cargo:rerun-if-changed={}", test_manifest.display());
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-tests=/MANIFESTUAC:level='asInvoker'");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
            test_manifest.display()
        );
    }
    // Register the full set of exposed commands with the app manifest so
    // the ACL can reject anything that is not explicitly listed here.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_app_status",
            "get_character_manifest",
            "get_character_settings",
            "get_character_setup",
            "import_character_asset",
            "set_active_character_renderer",
            "set_expression_idle_timeout",
            "set_character_size",
            "set_expression",
            "set_click_through",
            "list_microphones",
            "start_listening",
            "set_input_device",
            "stop_listening",
            "set_capture_enabled",
            "get_audio_diagnostics",
            "get_stt_state",
            "send_chat_message",
            "cancel_turn",
            "get_llm_settings",
            "set_llm_settings",
            "get_persona_profile",
            "set_persona_profile",
            "get_behavior_settings",
            "set_behavior_settings",
            "get_dark_expression_safety_settings",
            "set_safe_word",
            "resume_dark_expression",
            "set_speech_playback",
            "get_tts_settings",
            "set_tts_settings",
            // The settings window normalizes both engine APIs to this command.
            "list_tts_voices",
            "list_user_dict",
            "add_user_dict_word",
            "delete_user_dict_word",
            "get_runtime_diagnostics",
            "report_frontend_error",
            "list_diagnostic_reports",
            "export_diagnostic_reports",
            "get_update_state",
            "check_for_updates",
            "install_update",
            "get_ui_preferences",
            "set_theme_preference",
            "set_chat_placement",
            "list_conversation_history",
            "list_conversation_log",
            "get_data_usage",
            "export_user_data",
            "delete_conversation_history",
            "delete_memories",
            "clear_tts_audio_cache",
            "read_technical_log",
            "report_runtime_failure",
            "report_runtime_success",
            "retry_character_renderer",
            "rearm_runtime_feature",
        ]),
    ))
    .expect("failed to run tauri build script");
}
