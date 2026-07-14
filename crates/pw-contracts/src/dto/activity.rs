//! Decrypted activity-session IPC contracts.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const ACTIVITY_SESSION_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ActivitySessionDto.ts")]
pub struct ActivitySessionDto {
    pub schema_version: u16,
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub started_at: i64,
    #[ts(type = "number | null")]
    pub ended_at: Option<i64>,
    #[ts(type = "number")]
    pub duration_seconds: u64,
    pub category: String,
    pub display_app: String,
    pub display_title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ActivitySessionPageDto.ts")]
pub struct ActivitySessionPageDto {
    pub schema_version: u16,
    pub sessions: Vec<ActivitySessionDto>,
    #[ts(type = "number | null")]
    pub next_before_id: Option<i64>,
}
