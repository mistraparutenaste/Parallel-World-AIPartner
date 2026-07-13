//! Versioned IPC contracts between the Rust core and the webview windows.
//!
//! Every DTO in this crate derives `ts_rs::TS`; the TypeScript side of
//! the contract is generated into `packages/contracts/src/generated`
//! and must never be edited by hand.

pub mod dto;

pub use dto::{
    AppStatusDto, AudioDeviceDto, AudioDiagnosticsDto, AudioLevelEventDto, CharacterCursorEventDto,
    CharacterManifestDto, ChatMessageEventDto, ChatRoleDto, ConversationHistoryDeletedEventDto,
    ConversationMessageDto, ConversationStateDto, ConversationStateEventDto, FailureClassDto,
    HealthStatusDto, LlmSettingsDto, MotionGroupDto, ProcessOwnershipDto, RUNTIME_HEALTH_EVENT,
    RuntimeFeatureDto, RuntimeHealthEventDto, SCHEMA_VERSION, SpeechAudioEventDto,
    SpeechStopEventDto, SttPhaseDto, SttStateEventDto, TranscriptEventDto, TtsSettingsDto,
    TtsSpeakerDto, TtsStateEventDto, UserDictWordDto,
};
