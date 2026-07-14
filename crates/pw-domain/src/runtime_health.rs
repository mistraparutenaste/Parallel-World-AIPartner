//! Runtime feature health state and safe diagnostic metadata.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFeature {
    SpeechToText,
    LanguageModel,
    TextToSpeech,
    CharacterRenderer,
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
    const fn message(self) -> &'static str {
        match self {
            Self::Timeout => "operation timed out",
            Self::Unavailable => "service unavailable",
            Self::MissingModel => "required model is missing",
            Self::InvalidConfiguration => "configuration is invalid",
            Self::Internal => "internal runtime error",
        }
    }
}

/// Failure metadata intentionally contains no arbitrary text or raw cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeFailure {
    class: FailureClass,
    code: FailureCode,
    system_code: Option<i64>,
}
impl RuntimeFailure {
    #[must_use]
    pub const fn transient(code: FailureCode) -> Self {
        Self {
            class: FailureClass::Transient,
            code,
            system_code: None,
        }
    }
    #[must_use]
    pub const fn permanent(code: FailureCode) -> Self {
        Self {
            class: FailureClass::Permanent,
            code,
            system_code: None,
        }
    }
    #[must_use]
    pub const fn with_system_code(mut self, code: i64) -> Self {
        self.system_code = Some(code);
        self
    }
    #[must_use]
    pub const fn class(self) -> FailureClass {
        self.class
    }
    fn safe_message(self) -> String {
        self.system_code.map_or_else(
            || self.code.message().to_owned(),
            |code| format!("{} (system code {code})", self.code.message()),
        )
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
        self.last_error = Some(failure.safe_message());
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
    pub fn mark_degraded(&mut self, failure: &RuntimeFailure, now_ms: u64) {
        self.status = HealthStatus::Degraded;
        self.failure_class = Some(failure.class);
        self.last_error = Some(failure.safe_message());
        self.stable_since_ms = None;
        self.changed_at_ms = now_ms;
    }
    pub fn mark_starting(&mut self, now_ms: u64) {
        self.status = HealthStatus::Starting;
        self.failure_class = None;
        self.last_error = None;
        self.stable_since_ms = None;
        self.changed_at_ms = now_ms;
    }
}

/// Replaces credential values without changing unrelated text or imposing a length limit.
///
/// # Panics
///
/// Panics only if a compile-time constant regular expression is invalid.
#[must_use]
pub fn redact_credentials(input: &str) -> String {
    use std::sync::OnceLock;
    static CREDENTIAL: OnceLock<regex::Regex> = OnceLock::new();
    static JAPANESE: OnceLock<regex::Regex> = OnceLock::new();
    static AUTH: OnceLock<regex::Regex> = OnceLock::new();
    static JSON: OnceLock<regex::Regex> = OnceLock::new();
    static SPOKEN: OnceLock<regex::Regex> = OnceLock::new();
    let credential = CREDENTIAL.get_or_init(|| regex::Regex::new(r#"(?ix)(api[_ ]?key|token|password|passwd|secret(?:\s+value)?)(\s*[:=]\s*)(?:\"(?:\\.|[^\"])*\"|'(?:\\.|[^'])*'|[^\s,;&}]+)"#).expect("credential regex is constant and valid"));
    let japanese = JAPANESE.get_or_init(|| regex::Regex::new(r#"(APIキー|トークン|パスワード(?:の値)?|秘密(?:の?値)?|認証(?:情報)?)(\s*(?:[:=：]|は|が)\s*)(?:\"(?:\\.|[^\"])*\"|“[^”]*”|'(?:\\.|[^'])*'|[^\s,;&}]+)"#).expect("Japanese credential regex is constant and valid"));
    let auth = AUTH.get_or_init(|| regex::Regex::new(r#"(?ix)(authorization\s*[:=]?\s*(?:bearer|basic|digest)?\s*)(?:\"(?:\\.|[^\"])*\"|'(?:\\.|[^'])*'|[^\s,;&}]+)"#).expect("authorization regex is constant and valid"));
    let json = JSON.get_or_init(|| {
        regex::Regex::new(
            r#"(?ix)(\"(?:api[_ ]?key|token|password|passwd|secret)\"\s*:\s*\")[^\"]*"#,
        )
        .expect("json credential regex is constant and valid")
    });
    let spoken = SPOKEN.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)(secret\s+value\s+)(?:\"(?:\\.|[^\"])*\"|'(?:\\.|[^'])*'|[^\s,;&}]+)"#,
        )
        .expect("spoken credential regex is constant and valid")
    });
    let redacted = json.replace_all(input, "$1[REDACTED]");
    let redacted = auth.replace_all(&redacted, "$1[REDACTED]");
    let redacted = japanese.replace_all(&redacted, "$1$2[REDACTED]");
    let redacted = spoken.replace_all(&redacted, "$1[REDACTED]");
    credential
        .replace_all(&redacted, "$1$2[REDACTED]")
        .into_owned()
}

/// Diagnostic-only redaction additionally bounds emitted text.
#[must_use]
pub fn redact_diagnostic(input: &str) -> String {
    redact_credentials(input).chars().take(256).collect()
}

/// Persistent content retains its full length except for replaced secret values.
#[must_use]
pub fn redact_persistent_content(input: &str) -> String {
    let redacted = redact_credentials(input);
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
