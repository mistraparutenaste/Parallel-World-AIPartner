//! TTS settings persisted as `config/tts.json`.

use std::path::{Path, PathBuf};

use pw_contracts::{SCHEMA_VERSION, TtsEngineKind, TtsSettingsDto};
use pw_platform::config_io::{JsonFormat, write_atomic_json};
use pw_platform::paths::AppDataLayout;

const FILE_NAME: &str = "tts.json";

/// Returns the loopback endpoint used by a fresh configuration for one engine.
#[must_use]
pub const fn default_base_url(engine: TtsEngineKind) -> &'static str {
    match engine {
        TtsEngineKind::Aivis => "http://127.0.0.1:10101",
        TtsEngineKind::Irodori => "http://127.0.0.1:8088",
    }
}

/// `AivisSpeech` Engine defaults: loopback port 10101 and the bundled
/// Anneli voice (ノーマル style).
#[must_use]
pub fn default_tts_settings() -> TtsSettingsDto {
    TtsSettingsDto {
        schema_version: SCHEMA_VERSION,
        enabled: true,
        base_url: default_base_url(TtsEngineKind::Aivis).to_owned(),
        engine: TtsEngineKind::Aivis,
        voice_id: "888753760".to_owned(),
        irodori_lora_adapter: String::new(),
        style_id: 888_753_760,
        volume: 1.0,
        speed: 1.0,
    }
}

fn settings_path(layout: &AppDataLayout) -> PathBuf {
    layout.config.join(FILE_NAME)
}

/// Loads settings, falling back to defaults when missing or invalid.
#[must_use]
pub fn load_tts_settings(layout: &AppDataLayout) -> TtsSettingsDto {
    read_settings(&settings_path(layout)).unwrap_or_else(default_tts_settings)
}

fn read_settings(path: &Path) -> Option<TtsSettingsDto> {
    let content = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<TtsSettingsDto>(&content) {
        Ok(mut settings) => {
            if settings.voice_id.is_empty() {
                settings.voice_id = settings.style_id.to_string();
            }
            Some(settings)
        }
        Err(error) => {
            tracing::warn!(%error, "invalid tts.json; using defaults");
            None
        }
    }
}

/// Persists settings after validating the endpoint (loopback only).
///
/// # Errors
///
/// Returns an error message when the endpoint is invalid or the file
/// cannot be written.
pub fn save_tts_settings(layout: &AppDataLayout, settings: &TtsSettingsDto) -> Result<(), String> {
    pw_platform::net::validate_base_url(&settings.base_url, false)
        .map_err(|error| error.to_string())?;
    let mut normalized = settings.clone();
    settings
        .voice_id
        .trim()
        .clone_into(&mut normalized.voice_id);
    settings
        .irodori_lora_adapter
        .trim()
        .clone_into(&mut normalized.irodori_lora_adapter);
    if normalized.voice_id.is_empty() {
        return Err("voice_id must not be empty".to_owned());
    }
    if normalized.engine == TtsEngineKind::Aivis {
        normalized.style_id = normalized
            .voice_id
            .parse::<u32>()
            .map_err(|_| "Aivis voice_id must be a numeric style ID".to_owned())?;
    }
    write_atomic_json(&layout.config, FILE_NAME, &normalized, JsonFormat::Pretty)
        .map_err(|error| format!("failed to write tts.json: {error}"))
}

#[cfg(test)]
mod tests {
    use pw_contracts::TtsEngineKind;
    use pw_platform::paths::AppDataLayout;

    use super::{default_base_url, default_tts_settings, load_tts_settings, save_tts_settings};

    fn temp_layout(tag: &str) -> AppDataLayout {
        let root =
            std::env::temp_dir().join(format!("pw-tts-settings-{tag}-{}", std::process::id()));
        let layout = AppDataLayout::under(root);
        layout.create_all().unwrap();
        layout
    }

    #[test]
    fn missing_file_falls_back_to_defaults() {
        let layout = temp_layout("defaults");
        assert_eq!(load_tts_settings(&layout), default_tts_settings());
    }

    #[test]
    fn save_then_load_round_trips() {
        let layout = temp_layout("roundtrip");
        let settings = pw_contracts::TtsSettingsDto {
            voice_id: "42".to_owned(),
            style_id: 42,
            volume: 0.5,
            enabled: false,
            ..default_tts_settings()
        };
        save_tts_settings(&layout, &settings).unwrap();
        assert_eq!(load_tts_settings(&layout), settings);
    }

    #[test]
    fn remote_endpoints_are_rejected() {
        let layout = temp_layout("remote");
        let settings = pw_contracts::TtsSettingsDto {
            base_url: "http://tts.example.com:10101".to_owned(),
            ..default_tts_settings()
        };
        assert!(save_tts_settings(&layout, &settings).is_err());
    }

    #[test]
    fn engine_defaults_use_the_expected_loopback_ports() {
        assert_eq!(
            default_base_url(TtsEngineKind::Aivis),
            "http://127.0.0.1:10101"
        );
        assert_eq!(
            default_base_url(TtsEngineKind::Irodori),
            "http://127.0.0.1:8088"
        );
    }

    #[test]
    fn legacy_empty_voice_id_migrates_from_style_id() {
        let layout = temp_layout("legacy-voice");
        std::fs::write(
            layout.config.join("tts.json"),
            r#"{
                "schema_version": 1,
                "enabled": true,
                "base_url": "http://127.0.0.1:10101",
                "style_id": 42,
                "volume": 1.0,
                "speed": 1.0
            }"#,
        )
        .unwrap();

        let settings = load_tts_settings(&layout);

        assert_eq!(settings.engine, TtsEngineKind::Aivis);
        assert_eq!(settings.voice_id, "42");
    }

    #[test]
    fn save_rejects_non_numeric_aivis_voice_id() {
        let layout = temp_layout("aivis-voice");
        let settings = pw_contracts::TtsSettingsDto {
            voice_id: "not-a-style-id".to_owned(),
            ..default_tts_settings()
        };

        let error = save_tts_settings(&layout, &settings).unwrap_err();

        assert!(error.contains("Aivis"));
        assert!(!layout.config.join("tts.json").exists());
    }

    #[test]
    fn save_accepts_string_irodori_voice_id() {
        let layout = temp_layout("irodori-voice");
        let settings = pw_contracts::TtsSettingsDto {
            engine: TtsEngineKind::Irodori,
            base_url: default_base_url(TtsEngineKind::Irodori).to_owned(),
            voice_id: "irodori-speaker-1".to_owned(),
            ..default_tts_settings()
        };

        save_tts_settings(&layout, &settings).unwrap();

        assert_eq!(load_tts_settings(&layout), settings);
    }

    #[test]
    fn save_rejects_trimmed_empty_voice_ids_for_both_engines() {
        for engine in [TtsEngineKind::Aivis, TtsEngineKind::Irodori] {
            let layout = temp_layout(match engine {
                TtsEngineKind::Aivis => "empty-aivis",
                TtsEngineKind::Irodori => "empty-irodori",
            });
            let settings = pw_contracts::TtsSettingsDto {
                engine,
                base_url: default_base_url(engine).to_owned(),
                voice_id: " \t ".to_owned(),
                ..default_tts_settings()
            };

            assert!(save_tts_settings(&layout, &settings).is_err());
            assert!(!layout.config.join("tts.json").exists());
        }
    }

    #[test]
    fn save_trims_irodori_voice_id() {
        let layout = temp_layout("trim-irodori");
        let settings = pw_contracts::TtsSettingsDto {
            engine: TtsEngineKind::Irodori,
            base_url: default_base_url(TtsEngineKind::Irodori).to_owned(),
            voice_id: "  voice-a  ".to_owned(),
            ..default_tts_settings()
        };

        save_tts_settings(&layout, &settings).unwrap();

        assert_eq!(load_tts_settings(&layout).voice_id, "voice-a");
    }

    #[test]
    fn save_trims_irodori_lora_adapter_path() {
        let layout = temp_layout("trim-irodori-lora");
        let settings = pw_contracts::TtsSettingsDto {
            engine: TtsEngineKind::Irodori,
            base_url: default_base_url(TtsEngineKind::Irodori).to_owned(),
            voice_id: "voice-a".to_owned(),
            irodori_lora_adapter: "  C:/models/adapters/character-a  ".to_owned(),
            ..default_tts_settings()
        };

        save_tts_settings(&layout, &settings).unwrap();

        assert_eq!(
            load_tts_settings(&layout).irodori_lora_adapter,
            "C:/models/adapters/character-a"
        );
    }

    #[test]
    fn save_syncs_aivis_style_id_for_legacy_readers() {
        let layout = temp_layout("aivis-downgrade");
        let settings = pw_contracts::TtsSettingsDto {
            voice_id: " 42 ".to_owned(),
            style_id: 888_753_760,
            ..default_tts_settings()
        };

        save_tts_settings(&layout, &settings).unwrap();
        let persisted: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(layout.config.join("tts.json")).unwrap())
                .unwrap();

        assert_eq!(persisted["voice_id"], "42");
        assert_eq!(persisted["style_id"], 42);
    }
}
