//! Runtime feature health state and safe diagnostic metadata.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFeature {
    SpeechToText,
    LanguageModel,
    TextToSpeech,
    Live2D,
    AudioInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Starting,
    Healthy,
    Recovering,
    Degraded,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    Transient,
    Permanent,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHealth {
    feature: RuntimeFeature,
    status: HealthStatus,
    failure_class: Option<FailureClass>,
    last_error: Option<String>,
    stable_since_ms: Option<u64>,
    changed_at_ms: u64,
}

impl RuntimeHealth {
    #[must_use]
    pub const fn new(feature: RuntimeFeature) -> Self {
        Self {
            feature,
            status: HealthStatus::Starting,
            failure_class: None,
            last_error: None,
            stable_since_ms: None,
            changed_at_ms: 0,
        }
    }
    #[must_use]
    pub const fn feature(&self) -> RuntimeFeature {
        self.feature
    }
    #[must_use]
    pub const fn status(&self) -> HealthStatus {
        self.status
    }
    #[must_use]
    pub const fn failure_class(&self) -> Option<FailureClass> {
        self.failure_class
    }
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
    #[must_use]
    pub const fn stable_since_ms(&self) -> Option<u64> {
        self.stable_since_ms
    }
    #[must_use]
    pub const fn changed_at_ms(&self) -> u64 {
        self.changed_at_ms
    }
    pub fn mark_healthy(&mut self, now_ms: u64) {
        self.status = HealthStatus::Healthy;
        self.failure_class = None;
        self.last_error = None;
        self.stable_since_ms = Some(now_ms);
        self.changed_at_ms = now_ms;
    }
    pub fn mark_failed(&mut self, class: FailureClass, error: &str, now_ms: u64) {
        self.status = if class == FailureClass::Transient {
            HealthStatus::Recovering
        } else {
            HealthStatus::Degraded
        };
        self.failure_class = Some(class);
        self.last_error = Some(redact_diagnostic(error));
        self.stable_since_ms = None;
        self.changed_at_ms = now_ms;
    }
    pub fn mark_stopped(&mut self, now_ms: u64) {
        self.status = HealthStatus::Stopped;
        self.failure_class = Some(FailureClass::Cancelled);
        self.last_error = None;
        self.stable_since_ms = None;
        self.changed_at_ms = now_ms;
    }
}

/// Redacts common key/value credentials before diagnostic persistence or emission.
#[must_use]
pub fn redact_diagnostic(input: &str) -> String {
    let parts = input.split_whitespace().collect::<Vec<_>>();
    let mut redact_next_bearer_value = false;
    parts
        .iter()
        .map(|part| {
            if redact_next_bearer_value {
                redact_next_bearer_value = false;
                return "[REDACTED]".to_owned();
            }
            let lower = part.to_ascii_lowercase();
            if lower == "bearer" {
                redact_next_bearer_value = true;
                return (*part).to_owned();
            }
            if ["token=", "secret=", "password=", "api_key=", "apikey="]
                .iter()
                .any(|key| lower.starts_with(key))
            {
                let key = part.split_once('=').map_or(*part, |(key, _)| key);
                format!("{key}=[REDACTED]")
            } else {
                (*part).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
