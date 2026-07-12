//! TTS (`AivisSpeech`) contracts.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Persisted TTS connection and voice settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "TtsSettingsDto.ts")]
pub struct TtsSettingsDto {
    pub schema_version: u16,
    /// Master switch; when off, replies stay text-only.
    pub enabled: bool,
    /// `AivisSpeech` Engine base URL (loopback only), e.g.
    /// `http://127.0.0.1:10101`.
    pub base_url: String,
    /// Selected style id (`/speakers` の styles[].id).
    pub style_id: u32,
    /// `volumeScale`, 1.0 = unchanged.
    pub volume: f32,
    /// `speedScale`, 1.0 = unchanged.
    pub speed: f32,
}

/// One selectable voice style, flattened for the settings dropdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "TtsSpeakerDto.ts")]
pub struct TtsSpeakerDto {
    /// Speaker (voice) name.
    pub name: String,
    /// Style name within the speaker.
    pub style_name: String,
    /// Style id passed to synthesis.
    pub style_id: u32,
}

/// `speech-audio` event payload: one synthesized sentence ready for
/// playback. The `WebView` receives a file path only (基本設計 8章).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "SpeechAudioEventDto.ts")]
pub struct SpeechAudioEventDto {
    pub schema_version: u16,
    /// Turn the audio belongs to; stale turns must be dropped.
    /// (JSON payloads carry plain numbers, so the TS type is number.)
    #[ts(type = "number")]
    pub turn_id: u64,
    /// Playback order within the turn (0-based).
    pub seq: u32,
    /// Absolute path of the cached WAV file.
    pub wav_path: String,
    /// The spoken sentence (for diagnostics / accessibility).
    pub text: String,
}

/// `speech-stop` event payload: halt playback and clear the queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "SpeechStopEventDto.ts")]
pub struct SpeechStopEventDto {
    pub schema_version: u16,
}

/// `tts-state` event payload (diagnostics; degraded-state banner).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "TtsStateEventDto.ts")]
pub struct TtsStateEventDto {
    pub schema_version: u16,
    /// False while the engine is unreachable (TTS障害 → テキスト表示).
    pub available: bool,
    /// Human-readable detail for degraded states (no secrets).
    pub message: Option<String>,
}

/// One user dictionary entry (pronunciation override).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "UserDictWordDto.ts")]
pub struct UserDictWordDto {
    pub uuid: String,
    pub surface: String,
    pub pronunciation: String,
    pub accent_type: u32,
}

#[cfg(test)]
mod tests {
    use super::TtsSettingsDto;
    use crate::SCHEMA_VERSION;

    #[test]
    fn settings_round_trip() {
        let settings = TtsSettingsDto {
            schema_version: SCHEMA_VERSION,
            enabled: true,
            base_url: "http://127.0.0.1:10101".into(),
            style_id: 888_753_760,
            volume: 1.0,
            speed: 1.0,
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: TtsSettingsDto = serde_json::from_str(&json).unwrap();
        assert_eq!(back, settings);
    }
}
