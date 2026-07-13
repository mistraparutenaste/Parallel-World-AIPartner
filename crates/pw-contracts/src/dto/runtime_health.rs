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
    #[serde(rename = "live2d")]
    #[ts(rename = "live2d")]
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
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProcessOwnershipDto {
    Managed,
    External,
    NotApplicable,
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
    pub ownership: ProcessOwnershipDto,
    pub circuit_open: bool,
    #[ts(type = "number")]
    pub changed_at_ms: u64,
}

impl From<pw_domain::runtime_health::RuntimeFeature> for RuntimeFeatureDto {
    fn from(value: pw_domain::runtime_health::RuntimeFeature) -> Self {
        use pw_domain::runtime_health::RuntimeFeature as Domain;
        match value {
            Domain::SpeechToText => Self::SpeechToText,
            Domain::LanguageModel => Self::LanguageModel,
            Domain::TextToSpeech => Self::TextToSpeech,
            Domain::Live2D => Self::Live2D,
            Domain::AudioInput => Self::AudioInput,
        }
    }
}
impl From<pw_domain::runtime_health::HealthStatus> for HealthStatusDto {
    fn from(value: pw_domain::runtime_health::HealthStatus) -> Self {
        use pw_domain::runtime_health::HealthStatus as Domain;
        match value {
            Domain::Starting => Self::Starting,
            Domain::Healthy => Self::Healthy,
            Domain::Recovering => Self::Recovering,
            Domain::Degraded => Self::Degraded,
            Domain::Stopped => Self::Stopped,
        }
    }
}
impl From<pw_domain::runtime_health::FailureClass> for FailureClassDto {
    fn from(value: pw_domain::runtime_health::FailureClass) -> Self {
        match value {
            pw_domain::runtime_health::FailureClass::Transient => Self::Transient,
            pw_domain::runtime_health::FailureClass::Permanent => Self::Permanent,
        }
    }
}
impl From<(&pw_domain::runtime_health::RuntimeHealth, u8)> for RuntimeHealthEventDto {
    fn from((health, attempts): (&pw_domain::runtime_health::RuntimeHealth, u8)) -> Self {
        Self {
            schema_version: super::SCHEMA_VERSION,
            feature: health.feature().into(),
            status: health.status().into(),
            failure_class: health.failure_class().map(Into::into),
            last_error: health.last_error().map(str::to_owned),
            attempts,
            ownership: ProcessOwnershipDto::NotApplicable,
            circuit_open: attempts >= 8,
            changed_at_ms: health.changed_at_ms(),
        }
    }
}
