use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "DiagnosticReportDto.ts")]
pub struct DiagnosticReportDto {
    pub schema_version: u16,
    pub id: String,
    pub timestamp_ms: u64,
    pub category: String,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "FrontendErrorKindDto.ts")]
pub enum FrontendErrorKindDto {
    WindowError,
    UnhandledRejection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "FrontendDiagnosticDto.ts")]
pub struct FrontendDiagnosticDto {
    pub schema_version: u16,
    pub kind: FrontendErrorKindDto,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "TechnicalLogCursorDto.ts")]
pub struct TechnicalLogCursorDto {
    pub generation: u64,
    pub offset: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "TechnicalLogChunkDto.ts")]
pub struct TechnicalLogChunkDto {
    pub schema_version: u16,
    pub lines: Vec<String>,
    pub next_cursor: TechnicalLogCursorDto,
    pub reset: bool,
    pub has_more: bool,
}
