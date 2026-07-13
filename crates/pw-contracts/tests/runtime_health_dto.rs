use pw_contracts::{
    FailureClassDto, HealthStatusDto, RUNTIME_HEALTH_EVENT, RuntimeFeatureDto,
    RuntimeHealthEventDto,
};

#[test]
fn runtime_health_event_is_versioned_and_named() {
    let dto = RuntimeHealthEventDto {
        schema_version: pw_contracts::SCHEMA_VERSION,
        feature: RuntimeFeatureDto::SpeechToText,
        status: HealthStatusDto::Recovering,
        failure_class: Some(FailureClassDto::Transient),
        last_error: Some("timeout".into()),
        attempts: 2,
        changed_at_ms: 42,
    };
    let json = serde_json::to_value(dto).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(RUNTIME_HEALTH_EVENT, "runtime-health");
}
