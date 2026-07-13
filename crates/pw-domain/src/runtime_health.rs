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
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCode {
    Timeout,
    Unavailable,
    MissingModel,
    InvalidConfiguration,
    Internal,
}

impl FailureCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
            Self::MissingModel => "missing_model",
            Self::InvalidConfiguration => "invalid_configuration",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFailure {
    class: FailureClass,
    code: FailureCode,
    detail: String,
}
impl RuntimeFailure {
    #[must_use]
    pub fn transient(code: FailureCode, untrusted_detail: &str) -> Self {
        Self::new(FailureClass::Transient, code, untrusted_detail)
    }
    #[must_use]
    pub fn permanent(code: FailureCode, untrusted_detail: &str) -> Self {
        Self::new(FailureClass::Permanent, code, untrusted_detail)
    }
    fn new(class: FailureClass, code: FailureCode, detail: &str) -> Self {
        Self {
            class,
            code,
            detail: redact_diagnostic(detail),
        }
    }
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
        if self.status == HealthStatus::Healthy {
            return;
        }
        self.status = HealthStatus::Healthy;
        self.failure_class = None;
        self.last_error = None;
        self.stable_since_ms = Some(now_ms);
        self.changed_at_ms = now_ms;
    }
    pub fn mark_failed(&mut self, failure: &RuntimeFailure, now_ms: u64) {
        self.status = if failure.class == FailureClass::Transient {
            HealthStatus::Recovering
        } else {
            HealthStatus::Degraded
        };
        self.failure_class = Some(failure.class);
        self.last_error = Some(format!("{}: {}", failure.code.as_str(), failure.detail));
        self.stable_since_ms = None;
        self.changed_at_ms = now_ms;
    }
    pub fn mark_stopped(&mut self, now_ms: u64) {
        self.status = HealthStatus::Stopped;
        self.failure_class = None;
        self.last_error = None;
        self.stable_since_ms = None;
        self.changed_at_ms = now_ms;
    }
}

/// Redacts common credential shapes and bounds diagnostic text. Raw causes are never retained.
///
/// # Panics
///
/// Panics only if a compile-time constant regular expression is invalid.
#[must_use]
pub fn redact_diagnostic(input: &str) -> String {
    use std::sync::OnceLock;
    static CREDENTIAL: OnceLock<regex::Regex> = OnceLock::new();
    static AUTH: OnceLock<regex::Regex> = OnceLock::new();
    static JSON: OnceLock<regex::Regex> = OnceLock::new();
    let credential = CREDENTIAL.get_or_init(|| regex::Regex::new(r#"(?ix)(api[_ ]?key|token|password|passwd|secret(?:\s+value)?|APIキー|トークン|パスワード(?:の値)?|秘密(?:の?値)?|認証(?:情報)?)(\s*(?:[:=：]|は|が)?\s*)(?:\"(?:\\.|[^\"])*\"|“[^”]*”|'(?:\\.|[^'])*'|[^\s,;&}]+)"#).unwrap());
    let auth = AUTH.get_or_init(|| regex::Regex::new(r#"(?ix)(authorization\s*[:=]?\s*(?:bearer|basic|digest)?\s*)(?:\\?\"[^\"]*\\?\"|'[^']*'|[^\s,;&}]+)"#).unwrap());
    let json = JSON.get_or_init(|| {
        regex::Regex::new(
            r#"(?ix)(\"(?:api[_ ]?key|token|password|passwd|secret)\"\s*:\s*\")[^\"]*"#,
        )
        .unwrap()
    });
    let redacted = json.replace_all(input, "$1[REDACTED]");
    let redacted = auth.replace_all(&redacted, "$1[REDACTED]");
    let redacted = credential.replace_all(&redacted, "$1$2[REDACTED]");
    redacted.chars().take(256).collect()
}

/// Shared persistence redaction preserving harmless discussion of credential concepts.
#[must_use]
pub fn redact_persistent_content(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let harmless = [
        "token economy",
        "password management",
        "パスワード管理方法",
        "APIキー管理方法",
    ];
    if harmless
        .iter()
        .any(|phrase| lower.contains(&phrase.to_ascii_lowercase()))
    {
        return input.to_owned();
    }
    let redacted = redact_diagnostic(input);
    redacted
        .split_whitespace()
        .map(|part| {
            let candidate = part.trim_matches(|c: char| matches!(c, ',' | ';' | '"' | '\''));
            if candidate.chars().count() >= 24
                && candidate
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
                && candidate.chars().any(|c| c.is_ascii_lowercase())
                && candidate
                    .chars()
                    .any(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            {
                "[REDACTED]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
