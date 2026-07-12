//! LLM settings persisted as `config/llm.json`.

use std::path::{Path, PathBuf};

use pw_contracts::{LlmSettingsDto, SCHEMA_VERSION};
use pw_platform::paths::AppDataLayout;

const FILE_NAME: &str = "llm.json";

/// Built-in defaults: llama-server on loopback and the reply format
/// contract (control JSON prelude) from 基本設計 7章.
#[must_use]
pub fn default_llm_settings() -> LlmSettingsDto {
    LlmSettingsDto {
        schema_version: SCHEMA_VERSION,
        base_url: "http://127.0.0.1:8080/v1".to_owned(),
        model: "default".to_owned(),
        allow_remote: false,
        system_prompt: "あなたはデスクトップに常駐するAIパートナーです。\
応答の1行目には {\"emotion\":\"表情名\",\"intensity\":0.0から1.0,\"motion\":\"モーション名\"} \
という制御JSONだけを出力し、空行を1行挟んでから本文を書いてください。\
本文は日本語の話し言葉で、短く自然な文にしてください。"
            .to_owned(),
        character_prompt: "あなたの名前はエプシロン。明るく丁寧な口調で話す、\
好奇心旺盛なパートナーです。"
            .to_owned(),
    }
}

fn settings_path(layout: &AppDataLayout) -> PathBuf {
    layout.config.join(FILE_NAME)
}

/// Loads settings, falling back to defaults when missing or invalid.
#[must_use]
pub fn load_llm_settings(layout: &AppDataLayout) -> LlmSettingsDto {
    read_settings(&settings_path(layout)).unwrap_or_else(default_llm_settings)
}

fn read_settings(path: &Path) -> Option<LlmSettingsDto> {
    let content = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&content) {
        Ok(settings) => Some(settings),
        Err(error) => {
            tracing::warn!(%error, "invalid llm.json; using defaults");
            None
        }
    }
}

/// Persists settings after validating the endpoint.
///
/// # Errors
///
/// Returns an error message when the endpoint is invalid or the file
/// cannot be written.
pub fn save_llm_settings(layout: &AppDataLayout, settings: &LlmSettingsDto) -> Result<(), String> {
    pw_llm::validate_base_url(&settings.base_url, settings.allow_remote)
        .map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("failed to serialize settings: {error}"))?;
    std::fs::create_dir_all(&layout.config)
        .map_err(|error| format!("failed to create config dir: {error}"))?;
    std::fs::write(settings_path(layout), json)
        .map_err(|error| format!("failed to write llm.json: {error}"))
}

#[cfg(test)]
mod tests {
    use pw_platform::paths::AppDataLayout;

    use super::{default_llm_settings, load_llm_settings, save_llm_settings};

    fn temp_layout(tag: &str) -> AppDataLayout {
        let root =
            std::env::temp_dir().join(format!("pw-llm-settings-{tag}-{}", std::process::id()));
        let layout = AppDataLayout::under(root);
        layout.create_all().unwrap();
        layout
    }

    #[test]
    fn missing_file_falls_back_to_defaults() {
        let layout = temp_layout("missing");
        let settings = load_llm_settings(&layout);
        assert_eq!(settings.base_url, "http://127.0.0.1:8080/v1");
        std::fs::remove_dir_all(&layout.root).unwrap();
    }

    #[test]
    fn saved_settings_round_trip() {
        let layout = temp_layout("roundtrip");
        let mut settings = default_llm_settings();
        settings.model = "qwen2.5".into();
        save_llm_settings(&layout, &settings).unwrap();
        assert_eq!(load_llm_settings(&layout).model, "qwen2.5");
        std::fs::remove_dir_all(&layout.root).unwrap();
    }

    #[test]
    fn remote_endpoints_are_rejected_without_allow_remote() {
        let layout = temp_layout("remote");
        let mut settings = default_llm_settings();
        settings.base_url = "https://api.example.com/v1".into();
        assert!(save_llm_settings(&layout, &settings).is_err());
        settings.allow_remote = true;
        assert!(save_llm_settings(&layout, &settings).is_ok());
        std::fs::remove_dir_all(&layout.root).unwrap();
    }
}
