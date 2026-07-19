//! Versioned IPC contracts between the Rust core and the webview windows.
//!
//! Every DTO in this crate derives `ts_rs::TS`; the TypeScript side of
//! the contract is generated into `packages/contracts/src/generated`
//! and must never be edited by hand.

pub mod dto;

pub use dto::{
    ACTIVE_MODE_CHANGED_EVENT, ACTIVITY_COLLECTION_HEALTH_EVENT, ACTIVITY_SESSION_SCHEMA_VERSION,
    ActiveModeChangedEventDto, ActiveModeDto, ActiveModeSourceDto,
    ActivityCollectionHealthEventDto, ActivityCollectionHealthStatusDto, ActivitySessionDto,
    ActivitySessionPageDto, AppActivationRuleDto, AppStatusDto, AudioDeviceDto,
    AudioDiagnosticsDto, AudioLevelEventDto, BEHAVIOR_SETTINGS_CHANGED_EVENT,
    BEHAVIOR_SETTINGS_SCHEMA_VERSION, BehaviorSettingsChangedEventDto, BehaviorSettingsDto,
    CHARACTER_MANIFEST_SCHEMA_VERSION, CHARACTER_SETTINGS_CHANGED_EVENT,
    CHARACTER_SETTINGS_SCHEMA_VERSION, CHARACTER_SETUP_SCHEMA_VERSION, CharacterCursorEventDto,
    CharacterManifestDto, CharacterRendererDto, CharacterRendererKindDto,
    CharacterSettingsChangedEventDto, CharacterSettingsDto, CharacterSetupDto,
    CharacterSourceStatusDto, ChatMessageEventDto, ChatPlacementDto, ChatRoleDto, CompanionModeDto,
    ConsentStateDto, ConversationHistoryDeletedEventDto, ConversationLogPageDto,
    ConversationMessageDto, ConversationStateDto, ConversationStateEventDto,
    DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION, DARK_EXPRESSION_SAFETY_CHANGED_EVENT,
    DARK_EXPRESSION_SAFETY_SCHEMA_VERSION, DarkExpressionSafetyChangedEventDto,
    DarkExpressionSafetySettingsDto, DataDeletionResultDto, DataUsageDto, DeviceFallbackEventDto,
    DiagnosticReportDto, ExclusionRuleDto, FailureClassDto, FrequencyPolicyDto,
    FrontendDiagnosticDto, FrontendErrorKindDto, FullscreenActivationRuleDto, HealthStatusDto,
    LlmSettingsDto, MAX_ACTIVITY_APP_ID_CHARS, ModeActivationRulesDto, ModeProfileDto,
    ModeProfilesDto, MotionGroupDto, PERSONA_SETTINGS_SCHEMA_VERSION, PersonaProfileDto,
    PersonaSettingsDto, ProcessOwnershipDto, QueueMetricsDto, QuietHoursRuleDto,
    RUNTIME_HEALTH_EVENT, RuntimeDiagnosticsDto, RuntimeFeatureDto, RuntimeHealthEventDto,
    SAFEWORD_TRIGGERED_EVENT, SCHEMA_VERSION, SafewordTriggeredEventDto, ScheduleActivationRuleDto,
    ShortcutSettingsDto, SpeechAudioEventDto, SpeechStopEventDto, StaticExpressionDto, SttPhaseDto,
    SttStateEventDto, TechnicalLogChunkDto, TechnicalLogCursorDto, ThemePreferenceDto,
    TranscriptEventDto, TriggerPolicyDto, TtsEngineKind, TtsSettingsDto, TtsStateEventDto,
    TtsVoiceDto, UiPreferencesDto, UpdateStateDto, UpdateStatusDto, UserDictWordDto,
    normalize_activity_app_id,
};
