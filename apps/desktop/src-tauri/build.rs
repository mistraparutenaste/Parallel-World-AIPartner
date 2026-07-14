fn main() {
    // Windows installer detection treats test executables containing "updater"
    // as installers unless their execution level is explicit.
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-tests=/MANIFESTUAC:level='asInvoker'");
    }
    // Register the full set of exposed commands with the app manifest so
    // the ACL can reject anything that is not explicitly listed here.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_app_status",
            "get_character_manifest",
            "get_character_settings",
            "set_expression_idle_timeout",
            "set_expression",
            "start_motion",
            "set_click_through",
            "list_microphones",
            "start_listening",
            "stop_listening",
            "set_capture_enabled",
            "get_audio_diagnostics",
            "send_chat_message",
            "cancel_turn",
            "get_llm_settings",
            "set_llm_settings",
            "set_speech_playback",
            "get_tts_settings",
            "set_tts_settings",
            "list_tts_speakers",
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
            "list_conversation_log",
            "read_technical_log",
        ]),
    ))
    .expect("failed to run tauri build script");
}
