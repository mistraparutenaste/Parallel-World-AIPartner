//! Exports the TypeScript side of the IPC contracts.
//!
//! Run from the repository root:
//! `cargo run -p pw-contracts --bin export-bindings`

use std::fs;
use std::path::Path;

use pw_contracts::{
    AppStatusDto, AudioDeviceDto, AudioDiagnosticsDto, AudioLevelEventDto, CharacterCursorEventDto,
    CharacterManifestDto, ChatMessageEventDto, ChatRoleDto, ConversationMessageDto,
    ConversationStateDto, ConversationStateEventDto, LlmSettingsDto, MotionGroupDto,
    SpeechAudioEventDto, SpeechStopEventDto, SttPhaseDto, SttStateEventDto, TranscriptEventDto,
    TtsSettingsDto, TtsSpeakerDto, TtsStateEventDto, UserDictWordDto,
};
use ts_rs::{Config, TS};

fn main() {
    let out_dir = Path::new("packages/contracts/src/generated");
    fs::create_dir_all(out_dir).expect("create bindings output directory");

    let config = Config::new().with_out_dir(out_dir);
    AppStatusDto::export_all(&config).expect("export AppStatusDto bindings");
    ConversationStateDto::export_all(&config).expect("export ConversationStateDto bindings");
    CharacterManifestDto::export_all(&config).expect("export CharacterManifestDto bindings");
    MotionGroupDto::export_all(&config).expect("export MotionGroupDto bindings");
    CharacterCursorEventDto::export_all(&config).expect("export CharacterCursorEventDto bindings");
    AudioDeviceDto::export_all(&config).expect("export AudioDeviceDto bindings");
    AudioDiagnosticsDto::export_all(&config).expect("export AudioDiagnosticsDto bindings");
    AudioLevelEventDto::export_all(&config).expect("export AudioLevelEventDto bindings");
    SttPhaseDto::export_all(&config).expect("export SttPhaseDto bindings");
    SttStateEventDto::export_all(&config).expect("export SttStateEventDto bindings");
    TranscriptEventDto::export_all(&config).expect("export TranscriptEventDto bindings");
    ChatMessageEventDto::export_all(&config).expect("export ChatMessageEventDto bindings");
    ChatRoleDto::export_all(&config).expect("export ChatRoleDto bindings");
    ConversationMessageDto::export_all(&config).expect("export ConversationMessageDto bindings");
    ConversationStateEventDto::export_all(&config)
        .expect("export ConversationStateEventDto bindings");
    LlmSettingsDto::export_all(&config).expect("export LlmSettingsDto bindings");
    TtsSettingsDto::export_all(&config).expect("export TtsSettingsDto bindings");
    TtsSpeakerDto::export_all(&config).expect("export TtsSpeakerDto bindings");
    SpeechAudioEventDto::export_all(&config).expect("export SpeechAudioEventDto bindings");
    SpeechStopEventDto::export_all(&config).expect("export SpeechStopEventDto bindings");
    TtsStateEventDto::export_all(&config).expect("export TtsStateEventDto bindings");
    UserDictWordDto::export_all(&config).expect("export UserDictWordDto bindings");

    println!("TypeScript bindings exported to {}", out_dir.display());
}
