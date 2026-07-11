use serde_json::Value;
use std::{collections::BTreeSet, fs, path::PathBuf};

const EXPECTED_CSP: &str = "default-src 'self'; img-src 'self' asset: http://asset.localhost; style-src 'self' 'unsafe-inline'; script-src 'self'";

fn tauri_config() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let contents = fs::read_to_string(path).expect("tauri config must exist");
    serde_json::from_str(&contents).expect("tauri config must be valid JSON")
}

fn capability(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("capabilities")
        .join(format!("{name}.json"));
    let contents = fs::read_to_string(path).expect("capability file must exist");
    serde_json::from_str(&contents).expect("capability must be valid JSON")
}

fn permissions(value: &Value) -> Vec<&str> {
    value["permissions"]
        .as_array()
        .expect("permissions must be an array")
        .iter()
        .map(|permission| permission.as_str().expect("permission must be a string"))
        .collect()
}

fn assert_capability_metadata(value: &Value, expected_identifier: &str) {
    assert_eq!(value["identifier"], expected_identifier);
    assert_eq!(value["local"], true);
    assert_eq!(value["$schema"], "../gen/schemas/desktop-schema.json");
    assert!(value.get("remote").is_none());
}

#[test]
fn config_explicitly_enables_exactly_the_three_local_capabilities() {
    let config = tauri_config();
    assert_eq!(
        config["app"]["security"]["capabilities"],
        serde_json::json!(["character", "chat", "settings"])
    );
    assert_eq!(config["app"]["security"]["csp"], EXPECTED_CSP);
    assert_eq!(config["app"]["macOSPrivateApi"], true);

    let capability_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let files: BTreeSet<_> = fs::read_dir(capability_dir)
        .expect("capability directory must exist")
        .map(|entry| {
            entry
                .expect("capability entry must be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        files,
        BTreeSet::from([
            "character.json".to_owned(),
            "chat.json".to_owned(),
            "settings.json".to_owned(),
        ])
    );
}

#[test]
fn character_has_only_minimal_window_permissions() {
    let value = capability("character");
    assert_capability_metadata(&value, "character");
    assert_eq!(value["windows"], serde_json::json!(["character"]));
    assert_eq!(
        permissions(&value),
        ["core:window:allow-start-dragging", "core:window:allow-show"]
    );
    let serialized = value.to_string();
    assert!(!serialized.contains("shell"));
    assert!(!serialized.contains("fs:"));
    assert!(!serialized.contains("allow-get-app-status"));
}

#[test]
fn chat_exposes_only_get_app_status() {
    let value = capability("chat");
    assert_capability_metadata(&value, "chat");
    assert_eq!(value["windows"], serde_json::json!(["chat"]));
    assert_eq!(permissions(&value), ["allow-get-app-status"]);
}

#[test]
fn settings_exposes_get_app_status() {
    let value = capability("settings");
    assert_capability_metadata(&value, "settings");
    assert_eq!(value["windows"], serde_json::json!(["settings"]));
    assert_eq!(permissions(&value), ["allow-get-app-status"]);
}
