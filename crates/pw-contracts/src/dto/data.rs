use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Counts shown before the user enters the danger zone confirmation phrase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "DataUsageDto.ts")]
pub struct DataUsageDto {
    pub schema_version: u16,
    #[ts(type = "number")]
    pub conversation_messages: u64,
    #[ts(type = "number")]
    pub conversation_summaries: u64,
    #[ts(type = "number")]
    pub long_term_memories: u64,
    #[ts(type = "number")]
    pub tts_audio_files: u64,
    #[ts(type = "number")]
    pub tts_audio_bytes: u64,
}

/// Summary returned after one destructive data-management operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "DataDeletionResultDto.ts")]
pub struct DataDeletionResultDto {
    pub schema_version: u16,
    #[ts(type = "number")]
    pub deleted_records: u64,
    #[ts(type = "number")]
    pub deleted_files: u64,
    #[ts(type = "number")]
    pub freed_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "RetentionSettingsDto.ts")]
pub struct RetentionSettingsDto {
    pub schema_version: u16,
    pub keep_messages: u32,
}
