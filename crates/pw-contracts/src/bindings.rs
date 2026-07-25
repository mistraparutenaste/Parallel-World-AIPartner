//! Regenerates the TypeScript side of the IPC contracts.
//!
//! `crates/pw-contracts/src/dto/*.rs` is the single source of truth: every
//! `#[derive(TS)]` type and every `pub const` schema version / event name
//! lives there. This module reads those Rust definitions and writes matching
//! TypeScript into `packages/contracts/src/generated` and
//! `packages/contracts/src/index.ts`, so the two sides can never drift.
//!
//! Run `cargo run -p pw-contracts --bin export-bindings` from the repository
//! root to regenerate. `cargo test -p pw-contracts` (see
//! `tests/generated_bindings.rs`) regenerates into a scratch directory and
//! diffs it against the committed output, so a stale commit fails CI instead
//! of silently drifting.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use ts_rs::{Config, TS};

use crate::{
    ACTIVE_MODE_CHANGED_EVENT, ACTIVITY_COLLECTION_HEALTH_EVENT, ACTIVITY_SESSION_SCHEMA_VERSION,
    ActiveModeChangedEventDto, ActivityCollectionHealthEventDto, ActivitySessionPageDto,
    AppStatusDto, AudioDeviceDto, AudioDiagnosticsDto, AudioLevelEventDto,
    BEHAVIOR_SETTINGS_CHANGED_EVENT, BEHAVIOR_SETTINGS_SCHEMA_VERSION,
    BehaviorSettingsChangedEventDto, CHARACTER_MANIFEST_SCHEMA_VERSION,
    CHARACTER_SETTINGS_CHANGED_EVENT, CHARACTER_SETTINGS_SCHEMA_VERSION,
    CHARACTER_SETUP_SCHEMA_VERSION, CharacterCursorEventDto, CharacterManifestDto,
    CharacterSettingsChangedEventDto, CharacterSetupDto, ChatMessageEventDto,
    ConversationHistoryDeletedEventDto, ConversationLogPageDto, ConversationStateEventDto,
    DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION, DARK_EXPRESSION_SAFETY_CHANGED_EVENT,
    DARK_EXPRESSION_SAFETY_SCHEMA_VERSION, DarkExpressionSafetyChangedEventDto,
    DataDeletionResultDto, DataUsageDto, DeviceFallbackEventDto, DiagnosticReportDto,
    FrontendDiagnosticDto, LlmSettingsDto, MemoryCenterDto, PERSONA_SETTINGS_SCHEMA_VERSION,
    PersonaSettingsDto, RUNTIME_HEALTH_EVENT, RuntimeDiagnosticsDto, SAFEWORD_TRIGGERED_EVENT,
    SCHEMA_VERSION, SafewordTriggeredEventDto, SpeechAudioEventDto, SpeechStopEventDto,
    SttStateEventDto, TechnicalLogChunkDto, TranscriptEventDto, TtsSettingsDto, TtsStateEventDto,
    TtsVoiceDto, UiPreferencesDto, UpdateStateDto, UserDictWordDto,
};

/// One `pub const` re-exported into `packages/contracts/src/index.ts`.
enum ConstValue {
    Number(u16),
    Str(&'static str),
}

impl ConstValue {
    fn render(&self) -> String {
        match self {
            Self::Number(value) => value.to_string(),
            Self::Str(value) => format!("'{value}'"),
        }
    }
}

/// Every schema-version and event-name constant, read directly from the
/// `pub const` items in `crates/pw-contracts/src/dto/*.rs`. Adding an entry
/// here is the only step needed to expose a new constant to TypeScript; the
/// rendered value always matches the Rust value because it is the Rust
/// value, not a hand copy of it.
const CONSTANTS: &[(&str, ConstValue)] = &[
    ("SCHEMA_VERSION", ConstValue::Number(SCHEMA_VERSION)),
    (
        "ACTIVITY_SESSION_SCHEMA_VERSION",
        ConstValue::Number(ACTIVITY_SESSION_SCHEMA_VERSION),
    ),
    (
        "BEHAVIOR_SETTINGS_SCHEMA_VERSION",
        ConstValue::Number(BEHAVIOR_SETTINGS_SCHEMA_VERSION),
    ),
    (
        "BEHAVIOR_SETTINGS_CHANGED_EVENT",
        ConstValue::Str(BEHAVIOR_SETTINGS_CHANGED_EVENT),
    ),
    (
        "ACTIVE_MODE_CHANGED_EVENT",
        ConstValue::Str(ACTIVE_MODE_CHANGED_EVENT),
    ),
    (
        "ACTIVITY_COLLECTION_HEALTH_EVENT",
        ConstValue::Str(ACTIVITY_COLLECTION_HEALTH_EVENT),
    ),
    (
        "CHARACTER_MANIFEST_SCHEMA_VERSION",
        ConstValue::Number(CHARACTER_MANIFEST_SCHEMA_VERSION),
    ),
    (
        "CHARACTER_SETTINGS_SCHEMA_VERSION",
        ConstValue::Number(CHARACTER_SETTINGS_SCHEMA_VERSION),
    ),
    (
        "CHARACTER_SETUP_SCHEMA_VERSION",
        ConstValue::Number(CHARACTER_SETUP_SCHEMA_VERSION),
    ),
    (
        "CHARACTER_SETTINGS_CHANGED_EVENT",
        ConstValue::Str(CHARACTER_SETTINGS_CHANGED_EVENT),
    ),
    (
        "PERSONA_SETTINGS_SCHEMA_VERSION",
        ConstValue::Number(PERSONA_SETTINGS_SCHEMA_VERSION),
    ),
    (
        "DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION",
        ConstValue::Number(DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION),
    ),
    (
        "RUNTIME_HEALTH_EVENT",
        ConstValue::Str(RUNTIME_HEALTH_EVENT),
    ),
    (
        "DARK_EXPRESSION_SAFETY_SCHEMA_VERSION",
        ConstValue::Number(DARK_EXPRESSION_SAFETY_SCHEMA_VERSION),
    ),
    (
        "DARK_EXPRESSION_SAFETY_CHANGED_EVENT",
        ConstValue::Str(DARK_EXPRESSION_SAFETY_CHANGED_EVENT),
    ),
    (
        "SAFEWORD_TRIGGERED_EVENT",
        ConstValue::Str(SAFEWORD_TRIGGERED_EVENT),
    ),
];

/// Regenerates every file this crate owns in `packages/contracts/src`:
/// the DTO bindings under `generated_dir` and the `index.ts` barrel one
/// directory up. `generated_dir` is wiped first so a renamed or removed
/// `#[derive(TS)]` type cannot leave a stale `.ts` file behind.
///
/// # Panics
///
/// Panics if `generated_dir` cannot be (re)created, if a type fails to
/// export, or if `index.ts` cannot be written. All three indicate a broken
/// build rather than a recoverable runtime condition.
pub fn export_all(generated_dir: &Path) {
    let _ = fs::remove_dir_all(generated_dir);
    fs::create_dir_all(generated_dir).expect("create bindings output directory");

    let config = Config::new().with_out_dir(generated_dir);
    export_dto_bindings(&config);
    export_index(generated_dir);
}

/// Root DTO types. `Type::export_all` recursively exports every type each
/// root depends on, so a type only needs to be listed here if it is not
/// already reachable from another root. `tests/generated_bindings.rs`
/// (`every_ts_derive_type_is_exported`) fails the build if a new
/// `#[derive(TS)]` type stops being reachable from this list.
fn export_dto_bindings(config: &Config) {
    ActivitySessionPageDto::export_all(config).expect("export ActivitySessionPageDto bindings");
    AppStatusDto::export_all(config).expect("export AppStatusDto bindings");
    AudioDeviceDto::export_all(config).expect("export AudioDeviceDto bindings");
    AudioDiagnosticsDto::export_all(config).expect("export AudioDiagnosticsDto bindings");
    AudioLevelEventDto::export_all(config).expect("export AudioLevelEventDto bindings");
    DeviceFallbackEventDto::export_all(config).expect("export DeviceFallbackEventDto bindings");
    SttStateEventDto::export_all(config).expect("export SttStateEventDto bindings");
    TranscriptEventDto::export_all(config).expect("export TranscriptEventDto bindings");
    BehaviorSettingsChangedEventDto::export_all(config)
        .expect("export BehaviorSettingsChangedEventDto bindings");
    ActiveModeChangedEventDto::export_all(config)
        .expect("export ActiveModeChangedEventDto bindings");
    ActivityCollectionHealthEventDto::export_all(config)
        .expect("export ActivityCollectionHealthEventDto bindings");
    CharacterCursorEventDto::export_all(config).expect("export CharacterCursorEventDto bindings");
    CharacterManifestDto::export_all(config).expect("export CharacterManifestDto bindings");
    CharacterSetupDto::export_all(config).expect("export CharacterSetupDto bindings");
    CharacterSettingsChangedEventDto::export_all(config)
        .expect("export CharacterSettingsChangedEventDto bindings");
    ChatMessageEventDto::export_all(config).expect("export ChatMessageEventDto bindings");
    ConversationLogPageDto::export_all(config).expect("export ConversationLogPageDto bindings");
    ConversationHistoryDeletedEventDto::export_all(config)
        .expect("export ConversationHistoryDeletedEventDto bindings");
    ConversationStateEventDto::export_all(config)
        .expect("export ConversationStateEventDto bindings");
    LlmSettingsDto::export_all(config).expect("export LlmSettingsDto bindings");
    DataUsageDto::export_all(config).expect("export DataUsageDto bindings");
    DataDeletionResultDto::export_all(config).expect("export DataDeletionResultDto bindings");
    DiagnosticReportDto::export_all(config).expect("export DiagnosticReportDto bindings");
    FrontendDiagnosticDto::export_all(config).expect("export FrontendDiagnosticDto bindings");
    TechnicalLogChunkDto::export_all(config).expect("export TechnicalLogChunkDto bindings");
    MemoryCenterDto::export_all(config).expect("export MemoryCenterDto bindings");
    PersonaSettingsDto::export_all(config).expect("export PersonaSettingsDto bindings");
    RuntimeDiagnosticsDto::export_all(config).expect("export RuntimeDiagnosticsDto bindings");
    DarkExpressionSafetyChangedEventDto::export_all(config)
        .expect("export DarkExpressionSafetyChangedEventDto bindings");
    SafewordTriggeredEventDto::export_all(config)
        .expect("export SafewordTriggeredEventDto bindings");
    TtsSettingsDto::export_all(config).expect("export TtsSettingsDto bindings");
    TtsVoiceDto::export_all(config).expect("export TtsVoiceDto bindings");
    SpeechAudioEventDto::export_all(config).expect("export SpeechAudioEventDto bindings");
    SpeechStopEventDto::export_all(config).expect("export SpeechStopEventDto bindings");
    TtsStateEventDto::export_all(config).expect("export TtsStateEventDto bindings");
    UserDictWordDto::export_all(config).expect("export UserDictWordDto bindings");
    UiPreferencesDto::export_all(config).expect("export UiPreferencesDto bindings");
    UpdateStateDto::export_all(config).expect("export UpdateStateDto bindings");
}

/// Writes `index.ts` (one directory above `generated_dir`) from whatever
/// `.ts` files `export_dto_bindings` just wrote, plus [`CONSTANTS`]. Reading
/// the directory back (instead of keeping a second hand-written type list)
/// means the barrel can never name a type that was not actually generated.
fn export_index(generated_dir: &Path) {
    let mut type_names: Vec<String> = fs::read_dir(generated_dir)
        .expect("read bindings output directory")
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|file_name| file_name.strip_suffix(".ts").map(str::to_owned))
        .collect();
    type_names.sort();

    let mut contents = String::from(
        "// This file is generated by `cargo run -p pw-contracts --bin export-bindings`.\n\
         // Do not edit this file manually; change the Rust DTOs and consts in\n\
         // `crates/pw-contracts/src/dto/` instead and regenerate.\n\n",
    );
    for type_name in &type_names {
        let _ = writeln!(
            contents,
            "export type {{ {type_name} }} from './generated/{type_name}';"
        );
    }
    contents.push('\n');
    for (name, value) in CONSTANTS {
        let _ = writeln!(contents, "export const {name} = {};", value.render());
    }

    let index_path = generated_dir
        .parent()
        .expect("generated dir has a parent directory")
        .join("index.ts");
    fs::write(index_path, contents).expect("write index.ts");
}
