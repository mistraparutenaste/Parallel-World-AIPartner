//! IPC data transfer objects.

mod app_status;
mod audio;
mod character_cursor;
mod character_manifest;
mod chat;
mod runtime_health;
mod tts;

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
pub use runtime_health::{
    FailureClassDto, HealthStatusDto, ProcessOwnershipDto, RUNTIME_HEALTH_EVENT, RuntimeFeatureDto,
    RuntimeHealthEventDto,
};
pub use tts::{
    SpeechAudioEventDto, SpeechStopEventDto, TtsSettingsDto, TtsSpeakerDto, TtsStateEventDto,
    UserDictWordDto,
};
