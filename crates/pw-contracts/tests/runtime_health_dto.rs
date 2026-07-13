use pw_contracts::{
    FailureClassDto, HealthStatusDto, ProcessOwnershipDto, RUNTIME_HEALTH_EVENT, RuntimeFeatureDto,
    RuntimeHealthEventDto,
};
use pw_domain::runtime_health::{FailureCode, RuntimeFailure, RuntimeFeature, RuntimeHealth};

#[test]
fn runtime_health_event_is_versioned_and_named() {
    let dto = RuntimeHealthEventDto {
        schema_version: pw_contracts::SCHEMA_VERSION,
        feature: RuntimeFeatureDto::SpeechToText,
        status: HealthStatusDto::Recovering,
        failure_class: Some(FailureClassDto::Transient),
        last_error: Some("timeout".into()),
        attempts: 2,
        ownership: ProcessOwnershipDto::Managed,
        circuit_open: false,
        changed_at_ms: 42,
    };
    let json = serde_json::to_value(dto).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(RUNTIME_HEALTH_EVENT, "runtime-health");
}

#[test]
fn domain_health_converts_exhaustively_and_live2d_wire_name_is_stable() {
    let features = [
        RuntimeFeature::SpeechToText,
        RuntimeFeature::LanguageModel,
        RuntimeFeature::TextToSpeech,
        RuntimeFeature::Live2D,
        RuntimeFeature::AudioInput,
    ];
    for feature in features {
        let mut health = RuntimeHealth::new(feature);
        health.mark_failed(&RuntimeFailure::transient(FailureCode::Timeout), 7);
        let dto = RuntimeHealthEventDto::from((&health, 3));
        assert_eq!(dto.attempts, 3);
    }
    assert_eq!(
        serde_json::to_value(RuntimeFeatureDto::Live2D).unwrap(),
        "live2d"
    );
}
