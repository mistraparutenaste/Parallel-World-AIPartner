//! Versioned IPC contracts between the Rust core and the webview windows.
//!
//! Every DTO in this crate derives `ts_rs::TS`; the TypeScript side of
//! the contract is generated into `packages/contracts/src/generated`
//! and must never be edited by hand.

pub mod dto;

pub use dto::{
    AppStatusDto, AudioDeviceDto, AudioDiagnosticsDto, AudioLevelEventDto,
    CHARACTER_MANIFEST_SCHEMA_VERSION, CHARACTER_SETTINGS_CHANGED_EVENT,
    CHARACTER_SETTINGS_SCHEMA_VERSION, CharacterCursorEventDto, CharacterManifestDto,
    CharacterRendererDto, CharacterSettingsChangedEventDto, CharacterSettingsDto,
    ChatMessageEventDto, ChatPlacementDto, ChatRoleDto, ConversationHistoryDeletedEventDto,
    ConversationLogPageDto, ConversationMessageDto, ConversationStateDto,
    ConversationStateEventDto, DeviceFallbackEventDto, DiagnosticReportDto, FailureClassDto,
    FrontendDiagnosticDto, FrontendErrorKindDto, HealthStatusDto, LlmSettingsDto, MotionGroupDto,
    ProcessOwnershipDto, QueueMetricsDto, RUNTIME_HEALTH_EVENT, RuntimeDiagnosticsDto,
    RuntimeFeatureDto, RuntimeHealthEventDto, SCHEMA_VERSION, SpeechAudioEventDto,
    SpeechStopEventDto, StaticExpressionDto, SttPhaseDto, SttStateEventDto, TechnicalLogChunkDto,
    TechnicalLogCursorDto, ThemePreferenceDto, TranscriptEventDto, TtsSettingsDto, TtsSpeakerDto,
    TtsStateEventDto, UiPreferencesDto, UpdateStateDto, UpdateStatusDto, UserDictWordDto,
};
