//! IPC data transfer objects.

mod app_status;
mod audio;
mod character_cursor;
mod character_manifest;
mod chat;
mod diagnostics;
mod runtime_health;
mod tts;
mod update;

pub use app_status::{AppStatusDto, ConversationStateDto, SCHEMA_VERSION};
pub use audio::{
    AudioDeviceDto, AudioDiagnosticsDto, AudioLevelEventDto, DeviceFallbackEventDto, SttPhaseDto,
    SttStateEventDto, TranscriptEventDto,
};
pub use character_cursor::CharacterCursorEventDto;
pub use character_manifest::{CharacterManifestDto, MotionGroupDto};
pub use chat::{
    ChatMessageEventDto, ChatRoleDto, ConversationHistoryDeletedEventDto, ConversationMessageDto,
    ConversationStateEventDto, LlmSettingsDto,
};
pub use diagnostics::{DiagnosticReportDto, FrontendDiagnosticDto, FrontendErrorKindDto};
pub use runtime_health::{
    FailureClassDto, HealthStatusDto, ProcessOwnershipDto, QueueMetricsDto, RUNTIME_HEALTH_EVENT,
    RuntimeDiagnosticsDto, RuntimeFeatureDto, RuntimeHealthEventDto,
};
pub use tts::{
    SpeechAudioEventDto, SpeechStopEventDto, TtsSettingsDto, TtsSpeakerDto, TtsStateEventDto,
    UserDictWordDto,
};
pub use update::{UpdateStateDto, UpdateStatusDto};
