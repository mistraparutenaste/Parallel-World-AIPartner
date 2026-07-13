use pw_domain::runtime_health::{FailureClass, HealthStatus, RuntimeFeature, RuntimeHealth};

#[test]
fn health_transitions_include_explicit_stop_and_recovery() {
    let mut health = RuntimeHealth::new(RuntimeFeature::SpeechToText);
    assert_eq!(health.status(), HealthStatus::Starting);

    health.mark_healthy(10);
    assert_eq!(health.status(), HealthStatus::Healthy);
    assert_eq!(health.stable_since_ms(), Some(10));

    health.mark_failed(FailureClass::Transient, "token=secret", 20);
    assert_eq!(health.status(), HealthStatus::Recovering);
    assert_eq!(health.last_error(), Some("token=[REDACTED]"));

    health.mark_stopped(30);
    assert_eq!(health.status(), HealthStatus::Stopped);
    assert_eq!(health.stable_since_ms(), None);
}

#[test]
fn permanent_failures_degrade_instead_of_retrying() {
    let mut health = RuntimeHealth::new(RuntimeFeature::TextToSpeech);
    health.mark_failed(FailureClass::Permanent, "model missing", 1);
    assert_eq!(health.status(), HealthStatus::Degraded);
}

#[test]
fn diagnostic_errors_remove_authorization_bearer_values() {
    let mut health = RuntimeHealth::new(RuntimeFeature::LanguageModel);
    health.mark_failed(
        FailureClass::Transient,
        "request failed Authorization: Bearer abc123",
        1,
    );
    let error = health.last_error().unwrap();
    assert!(!error.contains("abc123"));
    assert_eq!(error, "request failed Authorization: Bearer [REDACTED]");
}
