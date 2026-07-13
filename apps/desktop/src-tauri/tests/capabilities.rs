//! Guard tests for the per-window capability files.
//!
//! These tests pin the security boundary: the character window must
//! never gain shell, filesystem or settings powers, and the chat and
//! settings windows may only call the commands listed here.

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
            !permission.contains("get_app_status") && !permission.contains("settings"),
            "settings/status command leaked: {permission}"
        );
        assert!(
            !permission.contains("set-expression") && !permission.contains("start-motion"),
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
            "allow-set-click-through",
            "allow-set-speech-playback",
            "allow-report-runtime-failure",
            "allow-report-runtime-success",
            "allow-retry-live2d-runtime",
            "allow-report-frontend-error"
        ]
    );
}

#[test]
fn chat_capability_exposes_status_and_chat_commands_only() {
    let (json, permissions) = capability_permissions("chat");
    assert_eq!(json["windows"], serde_json::json!(["chat"]));
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
            "allow-read-technical-log",
            "allow-get-character-manifest",
            "allow-set-expression",
            "allow-start-motion",
            "allow-list-microphones",
            "allow-start-listening",
            "allow-stop-listening",
            "allow-set-capture-enabled",
            "allow-get-audio-diagnostics",
            "allow-get-runtime-diagnostics",
            "allow-get-llm-settings",
            "allow-set-llm-settings",
            "allow-get-tts-settings",
            "allow-set-tts-settings",
            "allow-list-tts-speakers",
            "allow-list-user-dict",
            "allow-add-user-dict-word",
            "allow-delete-user-dict-word",
            "allow-export-user-data",
            "allow-delete-conversation-history",
            "allow-delete-memories",
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
fn common_runtime_rearm_is_settings_only_and_character_gets_live2d_retry_only() {
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
            .any(|permission| permission == "allow-retry-live2d-runtime")
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
    for permission in [
        "allow-get-ui-preferences",
        "allow-set-theme-preference",
        "allow-set-chat-placement",
    ] {
        assert!(chat.iter().any(|item| item == permission), "{permission}");
    }
    assert!(
        chat.iter().all(|permission| {
            !permission.contains("conversation-log") && !permission.contains("technical-log")
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
