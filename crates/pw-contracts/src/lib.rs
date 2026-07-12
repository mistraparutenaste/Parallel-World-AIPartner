//! Versioned IPC contracts between the Rust core and the webview windows.
//!
//! Every DTO in this crate derives `ts_rs::TS`; the TypeScript side of
//! the contract is generated into `packages/contracts/src/generated`
//! and must never be edited by hand.

pub mod dto;

pub use dto::{
    AppStatusDto, AudioDeviceDto, AudioDiagnosticsDto, AudioLevelEventDto, CharacterCursorEventDto,
    CharacterManifestDto, ChatMessageEventDto, ChatRoleDto, ConversationStateDto,
    ConversationStateEventDto, LlmSettingsDto, MotionGroupDto, SCHEMA_VERSION, SttPhaseDto,
    SttStateEventDto, TranscriptEventDto,
};
