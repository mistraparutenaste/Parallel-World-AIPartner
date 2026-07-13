use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const RUNTIME_HEALTH_EVENT: &str = "runtime-health";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RuntimeFeatureDto {
    SpeechToText,
    LanguageModel,
    TextToSpeech,
    Live2D,
    AudioInput,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HealthStatusDto {
    Starting,
    Healthy,
    Recovering,
    Degraded,
    Stopped,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FailureClassDto {
    Transient,
    Permanent,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RuntimeHealthEventDto {
    pub schema_version: u16,
    pub feature: RuntimeFeatureDto,
    pub status: HealthStatusDto,
    pub failure_class: Option<FailureClassDto>,
    pub last_error: Option<String>,
    pub attempts: u8,
    #[ts(type = "number")]
    pub changed_at_ms: u64,
}
