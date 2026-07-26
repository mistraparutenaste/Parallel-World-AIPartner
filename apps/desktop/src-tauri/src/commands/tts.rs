//! TTS commands: settings, voice list, user dictionary.

use std::time::Duration;

use pw_contracts::{
    IrodoriInstallStateDto, SCHEMA_VERSION, TtsEngineKind, TtsSettingsDto, TtsVoiceDto,
    UserDictWordDto,
};
use pw_platform::paths::AppDataLayout;
use pw_tts::{AivisSpeechClient, IrodoriTtsClient, Speaker, TtsClientConfig};
use tauri::State;

use crate::tts::{load_tts_settings, save_tts_settings};

const IRODORI_ARTIFACTS: [&str; 8] = [
    "tools/uv/uv.exe",
    "tools/git",
    "server",
    "models/model.safetensors",
    "models/codec/weights.pth",
    "models/tokenizer/tokenizer.model",
    "models/tokenizer/tokenizer_config.json",
    "models/tokenizer/config.json",
];

fn irodori_install_state(layout: &AppDataLayout) -> IrodoriInstallStateDto {
    let root = layout.root.join("irodori");
    let missing_artifacts = IRODORI_ARTIFACTS
        .iter()
        .filter(|relative| !root.join(relative).exists())
        .map(|relative| (*relative).to_owned())
        .collect::<Vec<_>>();
    IrodoriInstallStateDto {
        schema_version: SCHEMA_VERSION,
        installed: missing_artifacts.is_empty(),
        install_root: root.to_string_lossy().into_owned(),
        missing_artifacts,
    }
}

/// Returns whether every pinned Irodori artifact is present.
#[tauri::command]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn get_irodori_install_state(layout: State<'_, AppDataLayout>) -> IrodoriInstallStateDto {
    irodori_install_state(&layout)
}

/// Starts the existing managed Irodori bootstrap in a separate console.
///
/// # Errors
/// Returns an error when the launcher is missing or cannot be started.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn install_irodori(layout: State<'_, AppDataLayout>) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = layout;
        Err("Irodori managed installation is available on Windows only".into())
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .ok_or_else(|| "repository root is unavailable".to_owned())?;
        let script = repository_root.join("tools/scripts/irodori-bootstrap.ps1");
        if !script.is_file() {
            return Err("Irodori bootstrap script is unavailable".into());
        }
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .arg("-DataRoot")
            .arg(layout.root.join("irodori"))
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn ensure_dictionary_supported(engine: TtsEngineKind) -> Result<(), String> {
    if engine == TtsEngineKind::Irodori {
        return Err("Irodori TTS does not support user dictionary commands".to_owned());
    }
    Ok(())
}

fn dictionary_client(engine: TtsEngineKind, base_url: String) -> Result<AivisSpeechClient, String> {
    ensure_dictionary_supported(engine)?;
    let config = TtsClientConfig {
        base_url,
        timeout: Duration::from_secs(10),
    };
    AivisSpeechClient::new(&config).map_err(|error| error.to_string())
}

fn aivis_voices(speakers: Vec<Speaker>) -> Vec<TtsVoiceDto> {
    speakers
        .into_iter()
        .flat_map(|speaker| {
            speaker.styles.into_iter().map(move |style| TtsVoiceDto {
                id: style.id.to_string(),
                label: format!("{} / {}", speaker.name, style.name),
            })
        })
        .collect()
}

fn irodori_voices(voice_ids: Vec<String>) -> Vec<TtsVoiceDto> {
    voice_ids
        .into_iter()
        .map(|id| TtsVoiceDto {
            label: id.clone(),
            id,
        })
        .collect()
}

enum VoiceClient {
    Aivis(AivisSpeechClient),
    Irodori(IrodoriTtsClient),
}

fn voice_client(engine: TtsEngineKind, base_url: String) -> Result<VoiceClient, String> {
    let config = TtsClientConfig {
        base_url,
        timeout: Duration::from_secs(10),
    };
    match engine {
        TtsEngineKind::Aivis => AivisSpeechClient::new(&config)
            .map(VoiceClient::Aivis)
            .map_err(|error| error.to_string()),
        TtsEngineKind::Irodori => IrodoriTtsClient::new(&config)
            .map(VoiceClient::Irodori)
            .map_err(|error| error.to_string()),
    }
}

/// Returns the persisted TTS settings (defaults when unset).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
#[must_use]
pub fn get_tts_settings(layout: State<'_, AppDataLayout>) -> TtsSettingsDto {
    load_tts_settings(&layout)
}

/// Validates and persists TTS settings. The synthesis worker is
/// rebuilt with the new settings on the next sentence.
///
/// # Errors
///
/// Returns an error message for invalid endpoints or write failures.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn set_tts_settings(
    layout: State<'_, AppDataLayout>,
    settings: TtsSettingsDto,
) -> Result<(), String> {
    save_tts_settings(&layout, &settings)
}

/// Lists voice styles from the engine, flattened for the dropdown.
///
/// # Errors
///
/// Returns an error message when the engine is unreachable.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn list_tts_voices(
    engine: TtsEngineKind,
    base_url: String,
) -> Result<Vec<TtsVoiceDto>, String> {
    match voice_client(engine, base_url)? {
        VoiceClient::Aivis(client) => client
            .speakers()
            .map(aivis_voices)
            .map_err(|error| error.to_string()),
        VoiceClient::Irodori(client) => client
            .voices()
            .map(irodori_voices)
            .map_err(|error| error.to_string()),
    }
}

/// Lists the engine's user dictionary.
///
/// # Errors
///
/// Returns an error message when the engine is unreachable.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn list_user_dict(
    engine: TtsEngineKind,
    base_url: String,
) -> Result<Vec<UserDictWordDto>, String> {
    let client = dictionary_client(engine, base_url)?;
    let words = client.user_dict().map_err(|error| error.to_string())?;
    Ok(words
        .into_iter()
        .map(|word| UserDictWordDto {
            uuid: word.uuid,
            surface: word.surface,
            pronunciation: word.pronunciation,
            accent_type: word.accent_type,
        })
        .collect())
}

/// Adds a pronunciation override, returning its UUID.
///
/// # Errors
///
/// Returns an error message when the input is empty or the engine is
/// unreachable.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn add_user_dict_word(
    engine: TtsEngineKind,
    base_url: String,
    surface: String,
    pronunciation: String,
    accent_type: u32,
) -> Result<String, String> {
    let surface = surface.trim();
    let pronunciation = pronunciation.trim();
    if surface.is_empty() || pronunciation.is_empty() {
        return Err("単語と読みを入力してください".to_owned());
    }
    let client = dictionary_client(engine, base_url)?;
    client
        .add_user_dict_word(surface, pronunciation, accent_type)
        .map_err(|error| error.to_string())
}

/// Deletes a pronunciation override.
///
/// # Errors
///
/// Returns an error message when the engine is unreachable.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // tauri commands take owned args
pub fn delete_user_dict_word(
    engine: TtsEngineKind,
    base_url: String,
    uuid: String,
) -> Result<(), String> {
    let client = dictionary_client(engine, base_url)?;
    client
        .delete_user_dict_word(&uuid)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use pw_contracts::{TtsEngineKind, TtsVoiceDto};
    use pw_tts::{Speaker, SpeakerStyle};

    use super::{
        IRODORI_ARTIFACTS, VoiceClient, aivis_voices, dictionary_client,
        ensure_dictionary_supported, irodori_install_state, irodori_voices, voice_client,
    };

    #[test]
    fn irodori_install_state_requires_every_manifest_artifact() {
        let root = std::env::temp_dir().join(format!("pw-irodori-state-{}", std::process::id()));
        let layout = pw_platform::paths::AppDataLayout::under(root.clone());
        let initial = irodori_install_state(&layout);
        assert!(!initial.installed);
        assert_eq!(initial.missing_artifacts.len(), IRODORI_ARTIFACTS.len());
        for relative in IRODORI_ARTIFACTS {
            let path = root.join("irodori").join(relative);
            if std::path::Path::new(relative).extension().is_some() {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, b"test").unwrap();
            } else {
                std::fs::create_dir_all(path).unwrap();
            }
        }
        assert!(irodori_install_state(&layout).installed);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn voice_preview_selects_aivis_from_transient_arguments() {
        let client =
            voice_client(TtsEngineKind::Aivis, "http://127.0.0.1:10101".to_owned()).unwrap();

        assert!(matches!(client, VoiceClient::Aivis(_)));
    }

    #[test]
    fn voice_preview_selects_irodori_from_transient_arguments() {
        let client =
            voice_client(TtsEngineKind::Irodori, "http://127.0.0.1:8088".to_owned()).unwrap();

        assert!(matches!(client, VoiceClient::Irodori(_)));
    }

    #[test]
    fn voice_preview_rejects_remote_transient_url_before_transport() {
        let Err(error) = voice_client(
            TtsEngineKind::Irodori,
            "http://tts.example.com:8088".to_owned(),
        ) else {
            panic!("remote URL must be rejected");
        };

        assert!(error.contains("loopback"));
    }

    #[test]
    fn aivis_voices_flatten_speaker_and_style_names() {
        let voices = aivis_voices(vec![Speaker {
            name: "Anneli".to_owned(),
            styles: vec![SpeakerStyle {
                name: "Normal".to_owned(),
                id: 42,
            }],
        }]);

        assert_eq!(
            voices,
            vec![TtsVoiceDto {
                id: "42".to_owned(),
                label: "Anneli / Normal".to_owned(),
            }]
        );
    }

    #[test]
    fn irodori_voices_use_ids_as_labels() {
        assert_eq!(
            irodori_voices(vec!["voice-a".to_owned()]),
            vec![TtsVoiceDto {
                id: "voice-a".to_owned(),
                label: "voice-a".to_owned(),
            }]
        );
    }

    #[test]
    fn dictionary_commands_reject_irodori_before_transport() {
        let error = ensure_dictionary_supported(TtsEngineKind::Irodori).unwrap_err();

        assert!(error.contains("Irodori"));
        assert!(error.contains("dictionary"));
        assert!(ensure_dictionary_supported(TtsEngineKind::Aivis).is_ok());
    }

    #[test]
    fn transient_irodori_dictionary_selection_is_rejected_before_transport() {
        let error = dictionary_client(TtsEngineKind::Irodori, "http://127.0.0.1:8088".to_owned())
            .unwrap_err();

        assert!(error.contains("Irodori"));
        assert!(error.contains("dictionary"));
    }

    #[test]
    fn transient_aivis_dictionary_url_must_be_loopback() {
        let error = dictionary_client(
            TtsEngineKind::Aivis,
            "http://tts.example.com:10101".to_owned(),
        )
        .unwrap_err();

        assert!(error.contains("loopback"));
    }
}
