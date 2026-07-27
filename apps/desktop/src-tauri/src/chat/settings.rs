//! LLM settings persisted as `config/llm.json`.

use std::path::PathBuf;

use pw_contracts::{LlmProviderKind, LlmSettingsDto, SCHEMA_VERSION};
use pw_platform::config_io::{JsonFormat, read_json_lenient, write_atomic_json};
use pw_platform::paths::AppDataLayout;

const FILE_NAME: &str = "llm.json";
const CREDENTIAL_SERVICE: &str = "com.parallelworld.desktop.llm";

fn credential_account(provider: LlmProviderKind) -> &'static str {
    match provider {
        LlmProviderKind::Local => "local",
        LlmProviderKind::Openai => "openai",
        LlmProviderKind::Gemini => "gemini",
        LlmProviderKind::OpencodeZen => "opencode-zen",
        LlmProviderKind::Custom => "custom",
    }
}

fn credential_entry(provider: LlmProviderKind) -> Result<keyring::Entry, String> {
    keyring::Entry::new(CREDENTIAL_SERVICE, credential_account(provider))
        .map_err(|error| format!("failed to access credential store: {error}"))
}

/// Loads the provider key without exposing it through IPC or persisted settings.
///
/// # Errors
/// Returns an error when the platform credential store cannot be accessed.
pub fn load_llm_api_key(provider: LlmProviderKind) -> Result<Option<String>, String> {
    if provider == LlmProviderKind::Local {
        return Ok(None);
    }
    match credential_entry(provider)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("failed to read API key: {error}")),
    }
}

/// Built-in defaults: llama-server on loopback and the reply format
/// contract (control JSON prelude) from 基本設計 7章.
#[must_use]
pub fn default_llm_settings() -> LlmSettingsDto {
    LlmSettingsDto {
        schema_version: SCHEMA_VERSION,
        provider: LlmProviderKind::Local,
        base_url: "http://127.0.0.1:8080/v1".to_owned(),
        model: "default".to_owned(),
        api_key: String::new(),
        api_key_configured: false,
        clear_api_key: false,
        allow_remote: false,
        system_prompt: "あなたはデスクトップに常駐するAIパートナーです。\
応答の1行目には {\"emotion\":\"表情名\",\"intensity\":0.0から1.0,\"motion\":\"モーション名\"} \
という制御JSONだけを出力し、空行を1行挟んでから本文を書いてください。\
制御JSONは1行目以外に書かず、箇条書き・コードブロック・記号で飾らないでください。\
本文は日本語の話し言葉で、最大3文・全角150文字以内に収めてください。\
話題は1つに絞り、思いついたことを次々に並べないでください。\n\
絵文字・顔文字・記号の羅列は使わないでください。"
            .to_owned(),
        character_prompt: "あなたの名前はエプシロン。明るく丁寧な口調で話す、\
好奇心旺盛なパートナーです。"
            .to_owned(),
        strip_emoji: true,
        temperature: None,
        top_p: None,
        // A spoken turn never needs more; small models otherwise keep
        // generating until the context window ends.
        max_tokens: Some(256),
        repeat_penalty: None,
    }
}

fn ensure_range(value: Option<f64>, min: f64, max: f64, name: &str) -> Result<(), String> {
    if let Some(value) = value
        && !(value.is_finite() && (min..=max).contains(&value))
    {
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(())
}

fn validate_sampling(settings: &LlmSettingsDto) -> Result<(), String> {
    ensure_range(settings.temperature, 0.0, 2.0, "temperature")?;
    ensure_range(settings.top_p, 0.0, 1.0, "top_p")?;
    ensure_range(settings.repeat_penalty, 0.5, 4.0, "repeat_penalty")?;
    if settings.max_tokens == Some(0) {
        return Err("max_tokens must be greater than 0".to_owned());
    }
    Ok(())
}

fn settings_path(layout: &AppDataLayout) -> PathBuf {
    layout.config.join(FILE_NAME)
}

/// Loads settings, falling back to defaults when missing or invalid.
#[must_use]
pub fn load_llm_settings(layout: &AppDataLayout) -> LlmSettingsDto {
    let mut settings =
        read_json_lenient(&settings_path(layout)).unwrap_or_else(default_llm_settings);
    settings.api_key.clear();
    settings.clear_api_key = false;
    if settings.provider != LlmProviderKind::Local {
        settings.api_key_configured = load_llm_api_key(settings.provider).ok().flatten().is_some();
    }
    settings
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
    validate_sampling(settings)?;
    let api_key_configured = if settings.clear_api_key {
        let entry = credential_entry(settings.provider)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => false,
            Err(error) => return Err(format!("failed to delete API key: {error}")),
        }
    } else if settings.api_key.trim().is_empty() {
        settings.api_key_configured
    } else {
        let entry = credential_entry(settings.provider)?;
        entry
            .set_password(settings.api_key.trim())
            .map_err(|error| format!("failed to store API key: {error}"))?;
        true
    };
    let mut persisted = settings.clone();
    persisted.api_key.clear();
    persisted.api_key_configured = api_key_configured;
    persisted.clear_api_key = false;
    write_atomic_json(&layout.config, FILE_NAME, &persisted, JsonFormat::Pretty)
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
    fn sampling_values_round_trip_and_out_of_range_values_are_rejected() {
        let layout = temp_layout("sampling");
        let mut settings = default_llm_settings();
        settings.temperature = Some(1.2);
        settings.top_p = Some(0.9);
        settings.max_tokens = Some(512);
        settings.repeat_penalty = Some(1.1);
        save_llm_settings(&layout, &settings).unwrap();
        let loaded = load_llm_settings(&layout);
        assert_eq!(loaded.temperature, Some(1.2));
        assert_eq!(loaded.top_p, Some(0.9));
        assert_eq!(loaded.max_tokens, Some(512));
        assert_eq!(loaded.repeat_penalty, Some(1.1));

        settings.temperature = Some(2.5);
        assert!(save_llm_settings(&layout, &settings).is_err());
        settings.temperature = None;
        settings.max_tokens = Some(0);
        assert!(save_llm_settings(&layout, &settings).is_err());
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
