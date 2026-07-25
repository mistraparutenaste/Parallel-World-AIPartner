//! Guard tests for the per-window capability files.
//!
//! These tests pin the security boundary: the character window must
//! never gain shell, filesystem or settings powers, and the chat and
//! settings windows may only call the commands listed here.

use std::collections::BTreeSet;
use std::path::Path;

fn capability_permissions(name: &str) -> (serde_json::Value, Vec<String>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("capabilities")
        .join(format!("{name}.json"));
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let json: serde_json::Value = serde_json::from_str(&raw).expect("capability must be JSON");
    let permissions = json["permissions"]
        .as_array()
        .expect("permissions must be an array")
        .iter()
        .map(|permission| {
            permission
                .as_str()
                .expect("permissions must be plain identifiers")
                .to_owned()
        })
        .collect();
    (json, permissions)
}

fn custom_permissions(permissions: &[String]) -> Vec<&String> {
    permissions
        .iter()
        .filter(|permission| !permission.starts_with("core:"))
        .collect()
}

#[test]
fn character_capability_targets_only_the_character_window() {
    let (json, _) = capability_permissions("character");
    assert_eq!(json["windows"], serde_json::json!(["character"]));
}

#[test]
fn character_capability_denies_shell_fs_and_settings_commands() {
    let (_, permissions) = capability_permissions("character");
    for permission in &permissions {
        assert!(!permission.contains("shell"), "shell leaked: {permission}");
        assert!(!permission.contains("fs:"), "fs leaked: {permission}");
        assert!(
            !permission.contains("get_app_status"),
            "settings/status command leaked: {permission}"
        );
        assert!(
            !permission.contains("set-expression-idle-timeout")
                && !permission.contains("set-expression")
                && !permission.contains("start-motion"),
            "state-changing character command leaked: {permission}"
        );
        assert!(
            !permission.contains("microphone") && !permission.contains("listening"),
            "audio command leaked into character: {permission}"
        );
    }
    assert_eq!(
        custom_permissions(&permissions),
        [
            "allow-get-character-manifest",
            "allow-get-character-settings",
            "allow-set-click-through",
            "allow-set-speech-playback",
            "allow-report-runtime-failure",
            "allow-report-runtime-success",
            "allow-retry-character-renderer",
            "allow-report-frontend-error"
        ]
    );
}

#[test]
fn chat_capability_exposes_chat_surface_commands_only() {
    let (json, permissions) = capability_permissions("chat");
    assert_eq!(json["windows"], serde_json::json!(["chat"]));
    assert_eq!(
        custom_permissions(&permissions),
        [
            "allow-get-app-status",
            "allow-get-ui-preferences",
            "allow-set-chat-placement",
            "allow-send-chat-message",
            "allow-cancel-turn",
            "allow-list-conversation-history",
            "allow-get-dark-expression-safety-settings",
            "allow-resume-dark-expression",
            "allow-report-frontend-error"
        ]
    );
    for permission in &permissions {
        assert!(
            !permission.contains("llm-settings"),
            "llm settings command leaked into chat: {permission}"
        );
        assert!(
            !permission.contains("delete-") && !permission.contains("export-user-data"),
            "data command leaked into chat: {permission}"
        );
    }
}

#[test]
fn settings_capability_exposes_status_character_and_audio_control() {
    let (json, permissions) = capability_permissions("settings");
    assert_eq!(json["windows"], serde_json::json!(["settings"]));
    assert_eq!(
        custom_permissions(&permissions),
        [
            "allow-get-app-status",
            "allow-get-ui-preferences",
            "allow-set-theme-preference",
            "allow-set-chat-placement",
            "allow-send-chat-message",
            "allow-cancel-turn",
            "allow-list-conversation-history",
            "allow-list-conversation-log",
            "allow-get-memory-center",
            "allow-set-memory-domain-control",
            "allow-set-temporary-conversation",
            "allow-delete-memory",
            "allow-get-data-usage",
            "allow-read-technical-log",
            "allow-get-character-manifest",
            "allow-get-character-settings",
            "allow-get-character-setup",
            "allow-import-character-asset",
            "allow-set-active-character-renderer",
            "dialog:allow-open",
            "allow-set-expression-idle-timeout",
            "allow-set-character-size",
            "allow-set-expression",
            "allow-start-motion",
            "allow-list-microphones",
            "allow-start-listening",
            "allow-set-input-device",
            "allow-stop-listening",
            "allow-set-capture-enabled",
            "allow-get-audio-diagnostics",
            "allow-get-stt-state",
            "allow-get-runtime-diagnostics",
            "allow-get-llm-settings",
            "allow-set-llm-settings",
            "allow-get-persona-profile",
            "allow-set-persona-profile",
            "allow-get-behavior-settings",
            "allow-set-behavior-settings",
            "allow-get-active-mode",
            "allow-get-activity-collection-health",
            "allow-list-activity-sessions",
            "allow-get-dark-expression-safety-settings",
            "allow-set-safe-word",
            "allow-resume-dark-expression",
            "allow-get-tts-settings",
            "allow-set-tts-settings",
            "allow-list-tts-voices",
            "allow-list-user-dict",
            "allow-add-user-dict-word",
            "allow-delete-user-dict-word",
            "allow-export-user-data",
            "allow-delete-conversation-history",
            "allow-delete-memories",
            "allow-clear-tts-audio-cache",
            "allow-rearm-runtime-feature",
            "allow-report-frontend-error",
            "allow-list-diagnostic-reports",
            "allow-export-diagnostic-reports",
            "allow-get-update-state",
            "allow-check-for-updates",
            "allow-install-update"
        ]
    );
}

#[test]
fn persona_profile_commands_are_settings_only() {
    let expected = ["allow-get-persona-profile", "allow-set-persona-profile"];
    let (_, settings) = capability_permissions("settings");
    for permission in expected {
        assert!(settings.iter().any(|item| item == permission));
    }
    for capability in ["character", "chat"] {
        let (_, permissions) = capability_permissions(capability);
        assert!(
            expected
                .iter()
                .all(|permission| !permissions.iter().any(|item| item == permission)),
            "persona command leaked into {capability}"
        );
    }
}

#[test]
fn behavior_settings_commands_are_settings_only() {
    let expected = ["allow-get-behavior-settings", "allow-set-behavior-settings"];
    let (_, settings) = capability_permissions("settings");
    for permission in expected {
        assert!(settings.iter().any(|item| item == permission));
    }
    for capability in ["character", "chat"] {
        let (_, permissions) = capability_permissions(capability);
        assert!(
            expected
                .iter()
                .all(|permission| !permissions.iter().any(|item| item == permission)),
            "behavior settings command leaked into {capability}"
        );
    }
}

#[test]
fn dark_expression_safety_commands_follow_rendered_surface_boundaries() {
    let settings_only = ["allow-set-safe-word"];
    let shared_with_chat = [
        "allow-get-dark-expression-safety-settings",
        "allow-resume-dark-expression",
    ];
    let (_, settings) = capability_permissions("settings");
    for permission in settings_only.into_iter().chain(shared_with_chat) {
        assert!(settings.iter().any(|item| item == permission));
    }

    let (_, chat) = capability_permissions("chat");
    for permission in shared_with_chat {
        assert!(chat.iter().any(|item| item == permission));
    }
    assert!(!chat.iter().any(|item| item == "allow-set-safe-word"));

    let (_, character) = capability_permissions("character");
    assert!(
        settings_only
            .into_iter()
            .chain(shared_with_chat)
            .all(|permission| !character.iter().any(|item| item == permission)),
        "dark expression safety command leaked into character"
    );
}

#[test]
fn updater_commands_are_settings_only() {
    for capability in ["character", "chat"] {
        let (_, permissions) = capability_permissions(capability);
        assert!(
            permissions.iter().all(|permission| {
                !permission.contains("update-state")
                    && !permission.contains("check-for-updates")
                    && !permission.contains("install-update")
            }),
            "updater command leaked into {capability}"
        );
    }
}

#[test]
fn common_runtime_rearm_is_settings_only_and_character_gets_renderer_retry_only() {
    for capability in ["character", "chat"] {
        let (_, permissions) = capability_permissions(capability);
        assert!(
            !permissions
                .iter()
                .any(|permission| permission.contains("rearm-runtime-feature")),
            "common runtime rearm leaked into {capability}"
        );
    }
    let (_, character) = capability_permissions("character");
    assert!(
        character
            .iter()
            .any(|permission| permission == "allow-retry-character-renderer")
    );
}

#[test]
fn runtime_diagnostics_is_exposed_only_to_settings() {
    for capability in ["character", "chat"] {
        let (_, permissions) = capability_permissions(capability);
        assert!(
            !permissions
                .iter()
                .any(|permission| permission.contains("get-runtime-diagnostics"))
        );
    }
}

#[test]
fn crash_report_listing_and_export_are_settings_only() {
    for capability in ["character", "chat"] {
        let (_, permissions) = capability_permissions(capability);
        assert!(
            !permissions
                .iter()
                .any(|permission| permission.contains("diagnostic-reports"))
        );
    }
}

#[test]
fn chat_capability_never_gains_microphone_control() {
    let (_, permissions) = capability_permissions("chat");
    for permission in &permissions {
        assert!(
            !permission.contains("microphone") && !permission.contains("listening"),
            "audio command leaked into chat: {permission}"
        );
    }
}

#[test]
fn control_center_commands_are_scoped_to_the_windows_that_use_them() {
    let (_, settings) = capability_permissions("settings");
    for permission in [
        "allow-get-ui-preferences",
        "allow-set-theme-preference",
        "allow-set-chat-placement",
        "allow-list-conversation-log",
        "allow-get-data-usage",
        "allow-clear-tts-audio-cache",
        "allow-read-technical-log",
        "allow-send-chat-message",
        "allow-cancel-turn",
        "allow-list-conversation-history",
    ] {
        assert!(
            settings.iter().any(|item| item == permission),
            "{permission}"
        );
    }
    let (_, chat) = capability_permissions("chat");
    for permission in ["allow-get-ui-preferences", "allow-set-chat-placement"] {
        assert!(chat.iter().any(|item| item == permission), "{permission}");
    }
    assert!(
        chat.iter().all(|permission| {
            !permission.contains("conversation-log")
                && !permission.contains("technical-log")
                && !permission.contains("data-usage")
                && !permission.contains("tts-audio-cache")
        }),
        "read-only log commands leaked into chat"
    );
    let (_, character) = capability_permissions("character");
    assert!(character.iter().all(|permission| {
        !permission.contains("ui-preferences")
            && !permission.contains("chat-placement")
            && !permission.contains("conversation-log")
            && !permission.contains("technical-log")
    }));
}

#[test]
fn data_export_and_destructive_commands_are_settings_only() {
    let settings_only = [
        "allow-export-user-data",
        "allow-delete-conversation-history",
        "allow-delete-memories",
        "allow-clear-tts-audio-cache",
    ];
    let (_, settings) = capability_permissions("settings");
    for permission in settings_only {
        assert!(
            settings.iter().any(|item| item == permission),
            "{permission} missing from settings"
        );
    }

    for capability in ["chat", "character"] {
        let (_, permissions) = capability_permissions(capability);
        for permission in settings_only {
            assert!(
                permissions.iter().all(|item| item != permission),
                "{permission} leaked into {capability}"
            );
        }
    }
}

#[test]
fn character_source_setup_and_dialog_permissions_are_settings_only() {
    let feature_permissions = [
        "allow-get-character-setup",
        "allow-import-character-asset",
        "allow-set-active-character-renderer",
        "dialog:allow-open",
    ];
    let (_, settings) = capability_permissions("settings");
    let granted: Vec<_> = settings
        .iter()
        .filter(|permission| feature_permissions.contains(&permission.as_str()))
        .map(String::as_str)
        .collect();
    assert_eq!(granted, feature_permissions);

    for capability in ["character", "chat"] {
        let (_, permissions) = capability_permissions(capability);
        assert!(
            permissions
                .iter()
                .all(|permission| !feature_permissions.contains(&permission.as_str())),
            "character setup or dialog permission leaked into {capability}"
        );
    }
}

// --- Command manifest / handler parity ---------------------------------
//
// `src/lib.rs` registers every IPC command with Tauri's `invoke_handler`
// via `generate_handler!`, and `build.rs` separately declares the same
// commands to `tauri_build::AppManifest::commands` so the ACL can reject
// anything not explicitly listed there. These two lists are maintained by
// hand in two different files, so nothing stops them from drifting apart;
// when that happens the "ACL rejects unlisted commands" security boundary
// silently stops applying to whichever commands are missing from the
// manifest. The tests below pin both lists, and the on-disk autogenerated
// permission TOMLs, to stay in lockstep.

/// Extracts the command names registered in `src/lib.rs`'s
/// `tauri::generate_handler![...]` invocation, reduced to the final
/// `::`-segment of each `module::path::command` reference (e.g.
/// `commands::character::start_motion` becomes `start_motion`).
fn lib_rs_handler_commands() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("lib.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let marker = "generate_handler![";
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("`{marker}` not found in {}", path.display()))
        + marker.len();
    let end = source[start..]
        .find(']')
        .unwrap_or_else(|| panic!("unterminated `{marker}` block in {}", path.display()))
        + start;
    source[start..end]
        .lines()
        .filter_map(|line| {
            let line = line.split("//").next().unwrap_or("").trim();
            let line = line.trim_end_matches(',').trim();
            if line.is_empty() {
                None
            } else {
                Some(line.rsplit("::").next().unwrap_or(line).to_owned())
            }
        })
        .collect()
}

/// Extracts the quoted command names passed to
/// `tauri_build::AppManifest::new().commands(&[...])` in `build.rs`.
fn build_rs_manifest_commands() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let marker = "commands(&[";
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("`{marker}` not found in {}", path.display()))
        + marker.len();
    let end = source[start..]
        .find("])")
        .unwrap_or_else(|| panic!("unterminated `{marker}` block in {}", path.display()))
        + start;
    source[start..end]
        .lines()
        .filter_map(|line| {
            let line = line.split("//").next().unwrap_or("").trim();
            let line = line.trim_end_matches(',').trim();
            if line.is_empty() {
                None
            } else {
                Some(line.trim_matches('"').to_owned())
            }
        })
        .collect()
}

#[test]
fn build_manifest_commands_match_generated_handler_commands() {
    let lib_commands = lib_rs_handler_commands();
    let build_commands = build_rs_manifest_commands();
    // Guard against a marker rename silently turning this test into a
    // trivial pass on two empty lists.
    assert!(
        lib_commands.len() > 30,
        "parsed suspiciously few commands ({}) out of src/lib.rs; the \
         `generate_handler!` marker or parsing logic may be out of date",
        lib_commands.len()
    );

    let lib_set: BTreeSet<String> = lib_commands.into_iter().collect();
    let build_set: BTreeSet<String> = build_commands.into_iter().collect();

    let missing_from_build: Vec<_> = lib_set.difference(&build_set).collect();
    let missing_from_lib: Vec<_> = build_set.difference(&lib_set).collect();

    assert!(
        missing_from_build.is_empty() && missing_from_lib.is_empty(),
        "src/lib.rs's generate_handler! and build.rs's AppManifest::commands() must list \
         exactly the same commands, or the ACL manifest stops covering the commands missing \
         from it. Registered in lib.rs but missing from build.rs's manifest: \
         {missing_from_build:?}. Declared in build.rs's manifest but not registered in \
         lib.rs: {missing_from_lib:?}."
    );
}

#[test]
fn autogenerated_permission_tomls_match_registered_commands() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("permissions")
        .join("autogenerated");
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read dir {}: {error}", dir.display()));
    let mut toml_commands = BTreeSet::new();
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|error| panic!("read entry in {}: {error}", dir.display()));
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_else(|| panic!("non-utf8 file name: {}", path.display()));
        toml_commands.insert(stem.to_owned());
    }

    let lib_set: BTreeSet<String> = lib_rs_handler_commands().into_iter().collect();

    let missing_tomls: Vec<_> = lib_set.difference(&toml_commands).collect();
    let orphaned_tomls: Vec<_> = toml_commands.difference(&lib_set).collect();

    assert!(
        missing_tomls.is_empty() && orphaned_tomls.is_empty(),
        "permissions/autogenerated/*.toml must exactly mirror the commands registered in \
         src/lib.rs's generate_handler!. tauri-build writes one TOML per manifest command on \
         each build but never prunes TOMLs for commands that were later removed, so a stale, \
         orphaned TOML silently keeps that removed command inside the ACL surface even though \
         nothing registers it anymore. Commands with no autogenerated TOML: {missing_tomls:?}. \
         Orphaned TOMLs with no matching registered command: {orphaned_tomls:?}."
    );
}
