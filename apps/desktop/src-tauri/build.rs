fn main() {
    // Register the full set of exposed commands with the app manifest so
    // the ACL can reject anything that is not explicitly listed here.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_app_status",
            "get_character_manifest",
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
        ]),
    ))
    .expect("failed to run tauri build script");
}
