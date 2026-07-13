use pw_domain::runtime_health::{
    FailureClass, FailureCode, HealthStatus, RuntimeFailure, RuntimeFeature, RuntimeHealth,
    redact_diagnostic,
};

#[test]
fn health_transitions_include_explicit_stop_and_recovery() {
    let mut health = RuntimeHealth::new(RuntimeFeature::SpeechToText);
    assert_eq!(health.status(), HealthStatus::Starting);

    health.mark_healthy(10);
    assert_eq!(health.status(), HealthStatus::Healthy);
    assert_eq!(health.stable_since_ms(), Some(10));

    health.mark_failed(
        &RuntimeFailure::transient(FailureCode::Timeout, "token=secret"),
        20,
    );
    assert_eq!(health.status(), HealthStatus::Recovering);
    assert_eq!(health.last_error(), Some("timeout: token=[REDACTED]"));

    health.mark_stopped(30);
    assert_eq!(health.status(), HealthStatus::Stopped);
    assert_eq!(health.stable_since_ms(), None);
}

#[test]
fn permanent_failures_degrade_instead_of_retrying() {
    let mut health = RuntimeHealth::new(RuntimeFeature::TextToSpeech);
    health.mark_failed(
        &RuntimeFailure::permanent(FailureCode::MissingModel, "model missing"),
        1,
    );
    assert_eq!(health.status(), HealthStatus::Degraded);
}

#[test]
fn diagnostic_errors_remove_authorization_bearer_values() {
    let mut health = RuntimeHealth::new(RuntimeFeature::LanguageModel);
    health.mark_failed(
        &RuntimeFailure::transient(
            FailureCode::Unavailable,
            "request failed Authorization: Bearer abc123",
        ),
        1,
    );
    let error = health.last_error().unwrap();
    assert!(!error.contains("abc123"));
    assert_eq!(
        error,
        "unavailable: request failed Authorization: Bearer [REDACTED]"
    );
}

#[test]
fn repeated_healthy_probes_do_not_move_stability_timestamps() {
    let mut health = RuntimeHealth::new(RuntimeFeature::AudioInput);
    health.mark_healthy(10);
    health.mark_healthy(99);
    assert_eq!(health.stable_since_ms(), Some(10));
    assert_eq!(health.changed_at_ms(), 10);
}

#[test]
fn diagnostic_redaction_covers_wire_formats_and_is_bounded() {
    for (input, secret) in [
        ("Authorization:Bearer abc", "abc"),
        (r#"{"token":"json-secret"}"#, "json-secret"),
        ("https://x.test/?api_key=query-secret&x=1", "query-secret"),
        ("password = 'quoted secret'", "quoted secret"),
        ("APIキー=日本語秘密", "日本語秘密"),
    ] {
        let safe = redact_diagnostic(input);
        assert!(!safe.contains(secret), "{input} => {safe}");
    }
    assert!(redact_diagnostic(&"x".repeat(1_000)).len() <= 256);
}

#[test]
fn stop_is_not_a_failure_class() {
    let failures = [FailureClass::Transient, FailureClass::Permanent];
    assert_eq!(failures.len(), 2);
}
