//! IPC data transfer objects.

mod activity;
mod app_status;
mod audio;
mod behavior;
mod character_cursor;
mod character_manifest;
mod chat;
mod data;
mod diagnostics;
mod memory_center;
mod persona;
mod runtime_health;
mod safety;
mod tts;
mod ui;
mod update;

pub use activity::{ACTIVITY_SESSION_SCHEMA_VERSION, ActivitySessionDto, ActivitySessionPageDto};
pub use app_status::{AppStatusDto, ConversationStateDto, SCHEMA_VERSION};
pub use audio::{
    AudioDeviceDto, AudioDiagnosticsDto, AudioLevelEventDto, DeviceFallbackEventDto,
    STT_DEVICE_FALLBACK_EVENT, STT_LEVEL_EVENT, STT_STATE_EVENT, STT_TRANSCRIPT_EVENT, SttPhaseDto,
    SttStateEventDto, TranscriptEventDto,
};
pub use behavior::{
    ACTIVE_MODE_CHANGED_EVENT, ACTIVITY_COLLECTION_HEALTH_EVENT, ActiveModeChangedEventDto,
    ActiveModeDto, ActiveModeSourceDto, ActivityCollectionHealthEventDto,
    ActivityCollectionHealthStatusDto, AppActivationRuleDto, BEHAVIOR_SETTINGS_CHANGED_EVENT,
    BEHAVIOR_SETTINGS_SCHEMA_VERSION, BehaviorSettingsChangedEventDto, BehaviorSettingsDto,
    CompanionModeDto, ConsentStateDto, ExclusionRuleDto, FrequencyPolicyDto,
    FullscreenActivationRuleDto, MAX_ACTIVITY_APP_ID_CHARS, ModeActivationRulesDto, ModeProfileDto,
    ModeProfilesDto, QuietHoursRuleDto, ScheduleActivationRuleDto, ShortcutSettingsDto,
    TriggerPolicyDto, normalize_activity_app_id,
};
pub use character_cursor::{CHARACTER_CURSOR_EVENT, CharacterCursorEventDto};
pub use character_manifest::{
    CHARACTER_EXPRESSION_EVENT, CHARACTER_MANIFEST_SCHEMA_VERSION, CHARACTER_MOTION_EVENT,
    CHARACTER_MOTION_SCHEMA_VERSION, CHARACTER_SETTINGS_CHANGED_EVENT,
    CHARACTER_SETTINGS_SCHEMA_VERSION, CHARACTER_SETUP_SCHEMA_VERSION, CharacterManifestDto,
    CharacterMotionEventDto, CharacterRendererDto, CharacterRendererKindDto,
    CharacterSettingsChangedEventDto, CharacterSettingsDto, CharacterSetupDto,
    CharacterSourceStatusDto, MotionGroupDto, StaticExpressionDto,
};
pub use chat::{
    CHAT_MESSAGE_EVENT, CONVERSATION_HISTORY_DELETED_EVENT, CONVERSATION_STATE_EVENT,
    ChatMessageEventDto, ChatRoleDto, ConversationHistoryDeletedEventDto, ConversationLogPageDto,
    ConversationMessageDto, ConversationStateEventDto, LlmProviderKind, LlmSettingsDto,
};
pub use data::{DataDeletionResultDto, DataUsageDto, RetentionSettingsDto};
pub use diagnostics::{
    DiagnosticReportDto, FrontendDiagnosticDto, FrontendErrorKindDto, TechnicalLogChunkDto,
    TechnicalLogCursorDto,
};
pub use memory_center::{
    CommitmentSummaryDto, DialogueSummaryDto, MemoryCenterDto, MemoryDomainControlDto,
    MemorySummaryDto, PendingMemoryCandidateDto, SelfReviewDto,
};
pub use persona::{
    DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION, PERSONA_SETTINGS_SCHEMA_VERSION, PersonaProfileDto,
    PersonaSettingsDto,
};
pub use runtime_health::{
    FailureClassDto, HealthStatusDto, ProcessOwnershipDto, QueueMetricsDto, RUNTIME_HEALTH_EVENT,
    RuntimeDiagnosticsDto, RuntimeFeatureDto, RuntimeHealthEventDto,
};
pub use safety::{
    DARK_EXPRESSION_SAFETY_CHANGED_EVENT, DARK_EXPRESSION_SAFETY_SCHEMA_VERSION,
    DarkExpressionSafetyChangedEventDto, DarkExpressionSafetySettingsDto, SAFEWORD_TRIGGERED_EVENT,
    SafewordTriggeredEventDto,
};
pub use tts::{
    IrodoriInstallStateDto, SPEECH_AUDIO_EVENT, SPEECH_STOP_EVENT, SpeechAudioEventDto,
    SpeechStopEventDto, TTS_STATE_EVENT, TtsEngineKind, TtsSettingsDto, TtsStateEventDto,
    TtsVoiceDto, UserDictWordDto,
};
pub use ui::{
    CONTROL_CENTER_NAVIGATE_EVENT, ChatPlacementDto, ThemePreferenceDto,
    UI_PREFERENCES_CHANGED_EVENT, UiPreferencesDto,
};
pub use update::{UPDATE_PROGRESS_EVENT, UpdateStateDto, UpdateStatusDto};
