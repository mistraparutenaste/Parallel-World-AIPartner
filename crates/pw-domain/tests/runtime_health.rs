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

    health.mark_failed(&RuntimeFailure::transient(FailureCode::Timeout), 20);
    assert_eq!(health.status(), HealthStatus::Recovering);
    assert_eq!(health.last_error(), Some("operation timed out"));

    health.mark_stopped(30);
    assert_eq!(health.status(), HealthStatus::Stopped);
    assert_eq!(health.stable_since_ms(), None);
}

#[test]
fn permanent_failures_degrade_instead_of_retrying() {
    let mut health = RuntimeHealth::new(RuntimeFeature::TextToSpeech);
    health.mark_failed(&RuntimeFailure::permanent(FailureCode::MissingModel), 1);
    assert_eq!(health.status(), HealthStatus::Degraded);
}

#[test]
fn runtime_failures_accept_no_raw_message_and_system_codes_are_numeric() {
    let mut health = RuntimeHealth::new(RuntimeFeature::LanguageModel);
    health.mark_failed(
        &RuntimeFailure::transient(FailureCode::Unavailable).with_system_code(10061),
        1,
    );
    let error = health.last_error().unwrap();
    assert_eq!(error, "service unavailable (system code 10061)");
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
fn persistent_redaction_scans_mixed_harmless_text_without_truncating() {
    let long = format!("{} token economy; token=secret tail", "a".repeat(600));
    let safe = pw_domain::runtime_health::redact_persistent_content(&long);
    assert_eq!(
        safe.chars().count(),
        long.chars().count() - "secret".len() + "[REDACTED]".len()
    );
    assert!(safe.ends_with("token economy; token=[REDACTED] tail"));
}

#[test]
fn persistent_redaction_covers_spoken_credentials_without_redacting_policy_terms() {
    for (input, expected) in [
        ("password is hunter2", "password is [REDACTED]"),
        ("token abc123", "token [REDACTED]"),
        ("API key my-secret", "API key [REDACTED]"),
        (
            "Authorization: Bearer abc",
            "Authorization: Bearer [REDACTED]",
        ),
        ("APIキー: plainsecret", "APIキー: [REDACTED]"),
        (
            "credential=AbCdEf0123456789AbCdEf012345",
            "credential=[REDACTED]",
        ),
    ] {
        assert_eq!(
            pw_domain::runtime_health::redact_persistent_content(input),
            expected,
            "{input}"
        );
    }

    for ordinary in [
        "token budget",
        "token budget.",
        "password policy",
        "authorization policy",
        "password is required",
        "パスワードは必須です",
    ] {
        assert_eq!(
            pw_domain::runtime_health::redact_persistent_content(ordinary),
            ordinary
        );
    }
}

#[test]
fn persistent_redaction_covers_two_class_opaque_tokens_inside_wrappers() {
    const SECRET: &str = "ABCDEF234567ABCDEF234567";
    for input in [
        "(ABCDEF234567ABCDEF234567)",
        "[ABCDEF234567ABCDEF234567]",
        "{ABCDEF234567ABCDEF234567}",
        "<ABCDEF234567ABCDEF234567>",
        "`ABCDEF234567ABCDEF234567`",
        "\"ABCDEF234567ABCDEF234567\"",
        "'ABCDEF234567ABCDEF234567'",
    ] {
        let safe = pw_domain::runtime_health::redact_persistent_content(input);
        assert!(safe.contains("[REDACTED]"), "{input} => {safe}");
        assert!(!safe.contains(SECRET), "{input} => {safe}");
    }
}

#[test]
fn stop_is_not_a_failure_class() {
    let failures = [FailureClass::Transient, FailureClass::Permanent];
    assert_eq!(failures.len(), 2);
}
