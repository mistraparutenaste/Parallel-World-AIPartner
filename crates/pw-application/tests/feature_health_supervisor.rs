use std::cell::Cell;

use pw_application::recovery::{
    BackoffDecision, Clock, FeatureHealthSupervisor, HealthTransition, RandomSource,
};
use pw_domain::runtime_health::{FailureCode, HealthStatus, RuntimeFailure, RuntimeFeature};

#[derive(Default)]
struct FakeClock(Cell<u64>);
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.get()
    }
}
struct MaxRandom;
impl RandomSource for MaxRandom {
    fn uniform_inclusive(&mut self, upper: u64) -> u64 {
        upper
    }
}

#[test]
fn failures_persist_attempts_and_open_the_eighth_circuit() {
    let clock = FakeClock::default();
    let mut supervisor =
        FeatureHealthSupervisor::new(RuntimeFeature::LanguageModel, &clock, MaxRandom);

    for attempt in 1..8 {
        let transition = supervisor.record_failure(RuntimeFailure::transient(FailureCode::Timeout));
        assert_eq!(transition.attempts, attempt);
        assert!(matches!(
            transition.decision,
            BackoffDecision::RetryAfter(_)
        ));
        assert_eq!(transition.health.status(), HealthStatus::Recovering);
    }
    let transition = supervisor.record_failure(RuntimeFailure::transient(FailureCode::Timeout));
    assert_eq!(transition.attempts, 8);
    assert_eq!(transition.decision, BackoffDecision::CircuitOpen);
    assert_eq!(transition.health.status(), HealthStatus::Degraded);
}

#[test]
fn sixty_seconds_of_actual_success_resets_only_that_feature() {
    let clock = FakeClock::default();
    let mut supervisor =
        FeatureHealthSupervisor::new(RuntimeFeature::TextToSpeech, &clock, MaxRandom);
    supervisor.record_failure(RuntimeFailure::transient(FailureCode::Unavailable));
    let first = supervisor.record_success();
    assert!(matches!(
        first,
        HealthTransition::Changed { attempts: 1, .. }
    ));

    clock.0.set(59_999);
    assert!(matches!(
        supervisor.record_success(),
        HealthTransition::Unchanged
    ));
    assert_eq!(supervisor.attempts(), 1);
    clock.0.set(60_000);
    assert!(matches!(
        supervisor.record_success(),
        HealthTransition::Changed { attempts: 0, .. }
    ));
    assert_eq!(supervisor.attempts(), 0);
}

#[test]
fn duplicate_success_does_not_emit_a_duplicate_transition() {
    let clock = FakeClock::default();
    let mut supervisor = FeatureHealthSupervisor::new(RuntimeFeature::Live2D, &clock, MaxRandom);
    assert!(matches!(
        supervisor.record_success(),
        HealthTransition::Changed { .. }
    ));
    assert_eq!(supervisor.record_success(), HealthTransition::Unchanged);
}

#[test]
fn rearm_requires_an_open_circuit_and_clears_only_current_feature() {
    let clock = FakeClock::default();
    let mut supervisor = FeatureHealthSupervisor::new(RuntimeFeature::Live2D, &clock, MaxRandom);
    assert!(supervisor.rearm().is_err());
    for _ in 0..8 {
        supervisor.record_failure(RuntimeFailure::transient(FailureCode::Internal));
    }
    assert!(supervisor.rearm().is_ok());
    assert_eq!(supervisor.attempts(), 0);
    assert_eq!(supervisor.health().status(), HealthStatus::Starting);
}
