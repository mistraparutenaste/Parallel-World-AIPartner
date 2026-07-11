fn main() {
    // Register the full set of exposed commands with the app manifest so
    // the ACL can reject anything that is not explicitly listed here.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["get_app_status"])),
    )
    .expect("failed to run tauri build script");
}
