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
