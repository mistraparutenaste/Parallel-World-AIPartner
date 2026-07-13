//! Audio input and STT contracts.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One selectable microphone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "AudioDeviceDto.ts")]
pub struct AudioDeviceDto {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Snapshot of the speech pipeline counters for the diagnostics UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "AudioDiagnosticsDto.ts")]
pub struct AudioDiagnosticsDto {
    pub schema_version: u16,
    pub running: bool,
    pub capture_enabled: bool,
    pub frames_processed: u64,
    pub segments_completed: u64,
    pub transcripts_accepted: u64,
    pub transcripts_rejected: u64,
    pub dropped_samples: u64,
    pub failure_queue_depth: usize,
    pub failure_queue_dropped: u64,
}

/// Emitted when the preferred microphone disappears and capture moves to the default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "DeviceFallbackEventDto.ts")]
pub struct DeviceFallbackEventDto {
    pub schema_version: u16,
    pub preferred_device_id: Option<String>,
    pub active_device_id: Option<String>,
}

/// Lifecycle state changes of the speech pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "SttPhaseDto.ts")]
pub enum SttPhaseDto {
    Starting,
    Listening,
    Stopped,
    Unavailable,
}

/// `stt-state` event payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "SttStateEventDto.ts")]
pub struct SttStateEventDto {
    pub schema_version: u16,
    pub phase: SttPhaseDto,
    /// Human-readable detail for degraded states (no secrets).
    pub message: Option<String>,
}

/// `stt-transcript` event payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "TranscriptEventDto.ts")]
pub struct TranscriptEventDto {
    pub schema_version: u16,
    pub text: String,
}

/// `stt-level` event payload (RMS of the latest frame, 0.0..=1.0).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "AudioLevelEventDto.ts")]
pub struct AudioLevelEventDto {
    pub schema_version: u16,
    pub rms: f32,
}

#[cfg(test)]
mod tests {
    use super::{DeviceFallbackEventDto, SttPhaseDto, SttStateEventDto};
    use crate::SCHEMA_VERSION;

    #[test]
    fn serializes_stt_state_with_snake_case_phase() {
        let value = SttStateEventDto {
            schema_version: SCHEMA_VERSION,
            phase: SttPhaseDto::Unavailable,
            message: Some("model missing".into()),
        };
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["phase"], "unavailable");
        assert_eq!(json["message"], "model missing");
    }

    #[test]
    fn device_fallback_keeps_preferred_and_active_ids_distinct() {
        let value = DeviceFallbackEventDto {
            schema_version: SCHEMA_VERSION,
            preferred_device_id: Some("usb-mic".into()),
            active_device_id: Some("built-in".into()),
        };
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["preferred_device_id"], "usb-mic");
        assert_eq!(json["active_device_id"], "built-in");
    }
}
