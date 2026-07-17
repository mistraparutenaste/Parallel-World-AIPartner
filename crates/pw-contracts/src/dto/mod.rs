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
mod persona;
mod runtime_health;
mod safety;
mod tts;
mod ui;
mod update;

pub use activity::{ACTIVITY_SESSION_SCHEMA_VERSION, ActivitySessionDto, ActivitySessionPageDto};
pub use app_status::{AppStatusDto, ConversationStateDto, SCHEMA_VERSION};
pub use audio::{
    AudioDeviceDto, AudioDiagnosticsDto, AudioLevelEventDto, DeviceFallbackEventDto, SttPhaseDto,
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
pub use character_cursor::CharacterCursorEventDto;
pub use character_manifest::{
    CHARACTER_MANIFEST_SCHEMA_VERSION, CHARACTER_SETTINGS_CHANGED_EVENT,
    CHARACTER_SETTINGS_SCHEMA_VERSION, CHARACTER_SETUP_SCHEMA_VERSION, CharacterManifestDto,
    CharacterRendererDto, CharacterRendererKindDto, CharacterSettingsChangedEventDto,
    CharacterSettingsDto, CharacterSetupDto, CharacterSourceStatusDto, MotionGroupDto,
    StaticExpressionDto,
};
pub use chat::{
    ChatMessageEventDto, ChatRoleDto, ConversationHistoryDeletedEventDto, ConversationLogPageDto,
    ConversationMessageDto, ConversationStateEventDto, LlmSettingsDto,
};
pub use data::{DataDeletionResultDto, DataUsageDto};
pub use diagnostics::{
    DiagnosticReportDto, FrontendDiagnosticDto, FrontendErrorKindDto, TechnicalLogChunkDto,
    TechnicalLogCursorDto,
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
    SpeechAudioEventDto, SpeechStopEventDto, TtsSettingsDto, TtsSpeakerDto, TtsStateEventDto,
    UserDictWordDto,
};
pub use ui::{ChatPlacementDto, ThemePreferenceDto, UiPreferencesDto};
pub use update::{UpdateStateDto, UpdateStatusDto};
