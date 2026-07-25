//! Versioned IPC contracts between the Rust core and the webview windows.
//!
//! Every DTO in this crate derives `ts_rs::TS`; the TypeScript side of
//! the contract is generated into `packages/contracts/src/generated`
//! and must never be edited by hand.

pub mod bindings;
pub mod dto;

pub use dto::{
    ACTIVE_MODE_CHANGED_EVENT, ACTIVITY_COLLECTION_HEALTH_EVENT, ACTIVITY_SESSION_SCHEMA_VERSION,
    ActiveModeChangedEventDto, ActiveModeDto, ActiveModeSourceDto,
    ActivityCollectionHealthEventDto, ActivityCollectionHealthStatusDto, ActivitySessionDto,
    ActivitySessionPageDto, AppActivationRuleDto, AppStatusDto, AudioDeviceDto,
    AudioDiagnosticsDto, AudioLevelEventDto, BEHAVIOR_SETTINGS_CHANGED_EVENT,
    BEHAVIOR_SETTINGS_SCHEMA_VERSION, BehaviorSettingsChangedEventDto, BehaviorSettingsDto,
    CHARACTER_CURSOR_EVENT, CHARACTER_EXPRESSION_EVENT, CHARACTER_MANIFEST_SCHEMA_VERSION,
    CHARACTER_MOTION_EVENT, CHARACTER_SETTINGS_CHANGED_EVENT, CHARACTER_SETTINGS_SCHEMA_VERSION,
    CHARACTER_SETUP_SCHEMA_VERSION, CHAT_MESSAGE_EVENT, CONTROL_CENTER_NAVIGATE_EVENT,
    CONVERSATION_HISTORY_DELETED_EVENT, CONVERSATION_STATE_EVENT, CharacterCursorEventDto,
    CharacterManifestDto, CharacterRendererDto, CharacterRendererKindDto,
    CharacterSettingsChangedEventDto, CharacterSettingsDto, CharacterSetupDto,
    CharacterSourceStatusDto, ChatMessageEventDto, ChatPlacementDto, ChatRoleDto,
    CommitmentSummaryDto, CompanionModeDto, ConsentStateDto, ConversationHistoryDeletedEventDto,
    ConversationLogPageDto, ConversationMessageDto, ConversationStateDto,
    ConversationStateEventDto, DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION,
    DARK_EXPRESSION_SAFETY_CHANGED_EVENT, DARK_EXPRESSION_SAFETY_SCHEMA_VERSION,
    DarkExpressionSafetyChangedEventDto, DarkExpressionSafetySettingsDto, DataDeletionResultDto,
    DataUsageDto, DeviceFallbackEventDto, DiagnosticReportDto, DialogueSummaryDto,
    ExclusionRuleDto, FailureClassDto, FrequencyPolicyDto, FrontendDiagnosticDto,
    FrontendErrorKindDto, FullscreenActivationRuleDto, HealthStatusDto, LlmProviderKind,
    LlmSettingsDto, MAX_ACTIVITY_APP_ID_CHARS, MemoryCenterDto, MemoryDomainControlDto,
    MemorySummaryDto, ModeActivationRulesDto, ModeProfileDto, ModeProfilesDto, MotionGroupDto,
    PERSONA_SETTINGS_SCHEMA_VERSION, PendingMemoryCandidateDto, PersonaProfileDto,
    PersonaSettingsDto, ProcessOwnershipDto, QueueMetricsDto, QuietHoursRuleDto,
    RUNTIME_HEALTH_EVENT, RuntimeDiagnosticsDto, RuntimeFeatureDto, RuntimeHealthEventDto,
    SAFEWORD_TRIGGERED_EVENT, SCHEMA_VERSION, SPEECH_AUDIO_EVENT, SPEECH_STOP_EVENT,
    STT_DEVICE_FALLBACK_EVENT, STT_LEVEL_EVENT, STT_STATE_EVENT, STT_TRANSCRIPT_EVENT,
    SafewordTriggeredEventDto, ScheduleActivationRuleDto, ShortcutSettingsDto, SpeechAudioEventDto,
    SpeechStopEventDto, StaticExpressionDto, SttPhaseDto, SttStateEventDto, TTS_STATE_EVENT,
    TechnicalLogChunkDto, TechnicalLogCursorDto, ThemePreferenceDto, TranscriptEventDto,
    TriggerPolicyDto, TtsEngineKind, TtsSettingsDto, TtsStateEventDto, TtsVoiceDto,
    UI_PREFERENCES_CHANGED_EVENT, UPDATE_PROGRESS_EVENT, UiPreferencesDto, UpdateStateDto,
    UpdateStatusDto, UserDictWordDto, normalize_activity_app_id,
};
