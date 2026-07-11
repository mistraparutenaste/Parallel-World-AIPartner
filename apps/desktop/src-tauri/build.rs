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
        ]),
    ))
    .expect("failed to run tauri build script");
}
