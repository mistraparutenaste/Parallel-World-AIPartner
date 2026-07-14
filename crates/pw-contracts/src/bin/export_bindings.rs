//! Exports the TypeScript side of the IPC contracts.
//!
//! Run from the repository root:
//! `cargo run -p pw-contracts --bin export-bindings`

use std::fs;
use std::path::Path;

use pw_contracts::{
    AppStatusDto, AudioDeviceDto, AudioDiagnosticsDto, AudioLevelEventDto, CharacterCursorEventDto,
    CharacterManifestDto, CharacterRendererDto, CharacterSettingsChangedEventDto,
    CharacterSettingsDto, ChatMessageEventDto, ChatPlacementDto, ChatRoleDto,
    ConversationHistoryDeletedEventDto, ConversationLogPageDto, ConversationMessageDto,
    ConversationStateDto, ConversationStateEventDto, DeviceFallbackEventDto, DiagnosticReportDto,
    FailureClassDto, FrontendDiagnosticDto, FrontendErrorKindDto, HealthStatusDto, LlmSettingsDto,
    MotionGroupDto, ProcessOwnershipDto, QueueMetricsDto, RuntimeDiagnosticsDto, RuntimeFeatureDto,
    RuntimeHealthEventDto, SpeechAudioEventDto, SpeechStopEventDto, StaticExpressionDto,
    SttPhaseDto, SttStateEventDto, TechnicalLogChunkDto, TechnicalLogCursorDto, ThemePreferenceDto,
    TranscriptEventDto, TtsSettingsDto, TtsSpeakerDto, TtsStateEventDto, UiPreferencesDto,
    UpdateStateDto, UpdateStatusDto, UserDictWordDto,
};
use ts_rs::{Config, TS};

fn main() {
    let out_dir = Path::new("packages/contracts/src/generated");
    fs::create_dir_all(out_dir).expect("create bindings output directory");

    let config = Config::new().with_out_dir(out_dir);
    AppStatusDto::export_all(&config).expect("export AppStatusDto bindings");
    ConversationStateDto::export_all(&config).expect("export ConversationStateDto bindings");
    CharacterManifestDto::export_all(&config).expect("export CharacterManifestDto bindings");
    CharacterRendererDto::export_all(&config).expect("export CharacterRendererDto bindings");
    StaticExpressionDto::export_all(&config).expect("export StaticExpressionDto bindings");
    CharacterSettingsDto::export_all(&config).expect("export CharacterSettingsDto bindings");
    CharacterSettingsChangedEventDto::export_all(&config)
        .expect("export CharacterSettingsChangedEventDto bindings");
    MotionGroupDto::export_all(&config).expect("export MotionGroupDto bindings");
    CharacterCursorEventDto::export_all(&config).expect("export CharacterCursorEventDto bindings");
    AudioDeviceDto::export_all(&config).expect("export AudioDeviceDto bindings");
    AudioDiagnosticsDto::export_all(&config).expect("export AudioDiagnosticsDto bindings");
    AudioLevelEventDto::export_all(&config).expect("export AudioLevelEventDto bindings");
    DeviceFallbackEventDto::export_all(&config).expect("export DeviceFallbackEventDto bindings");
    SttPhaseDto::export_all(&config).expect("export SttPhaseDto bindings");
    SttStateEventDto::export_all(&config).expect("export SttStateEventDto bindings");
    TranscriptEventDto::export_all(&config).expect("export TranscriptEventDto bindings");
    ChatMessageEventDto::export_all(&config).expect("export ChatMessageEventDto bindings");
    ChatRoleDto::export_all(&config).expect("export ChatRoleDto bindings");
    ConversationMessageDto::export_all(&config).expect("export ConversationMessageDto bindings");
    ConversationLogPageDto::export_all(&config).expect("export ConversationLogPageDto bindings");
    ConversationHistoryDeletedEventDto::export_all(&config)
        .expect("export deletion event bindings");
    ConversationStateEventDto::export_all(&config)
        .expect("export ConversationStateEventDto bindings");
    LlmSettingsDto::export_all(&config).expect("export LlmSettingsDto bindings");
    TtsSettingsDto::export_all(&config).expect("export TtsSettingsDto bindings");
    TtsSpeakerDto::export_all(&config).expect("export TtsSpeakerDto bindings");
    SpeechAudioEventDto::export_all(&config).expect("export SpeechAudioEventDto bindings");
    SpeechStopEventDto::export_all(&config).expect("export SpeechStopEventDto bindings");
    TtsStateEventDto::export_all(&config).expect("export TtsStateEventDto bindings");
    UserDictWordDto::export_all(&config).expect("export UserDictWordDto bindings");
    RuntimeFeatureDto::export_all(&config).expect("export RuntimeFeatureDto bindings");
    HealthStatusDto::export_all(&config).expect("export HealthStatusDto bindings");
    ProcessOwnershipDto::export_all(&config).expect("export ProcessOwnershipDto bindings");
    FailureClassDto::export_all(&config).expect("export FailureClassDto bindings");
    RuntimeHealthEventDto::export_all(&config).expect("export RuntimeHealthEventDto bindings");
    QueueMetricsDto::export_all(&config).expect("export QueueMetricsDto bindings");
    RuntimeDiagnosticsDto::export_all(&config).expect("export RuntimeDiagnosticsDto bindings");
    DiagnosticReportDto::export_all(&config).expect("export DiagnosticReportDto bindings");
    FrontendDiagnosticDto::export_all(&config).expect("export FrontendDiagnosticDto bindings");
    FrontendErrorKindDto::export_all(&config).expect("export FrontendErrorKindDto bindings");
    TechnicalLogCursorDto::export_all(&config).expect("export TechnicalLogCursorDto bindings");
    TechnicalLogChunkDto::export_all(&config).expect("export TechnicalLogChunkDto bindings");
    ThemePreferenceDto::export_all(&config).expect("export ThemePreferenceDto bindings");
    ChatPlacementDto::export_all(&config).expect("export ChatPlacementDto bindings");
    UiPreferencesDto::export_all(&config).expect("export UiPreferencesDto bindings");
    UpdateStatusDto::export_all(&config).expect("export UpdateStatusDto bindings");
    UpdateStateDto::export_all(&config).expect("export UpdateStateDto bindings");

    println!("TypeScript bindings exported to {}", out_dir.display());
}
