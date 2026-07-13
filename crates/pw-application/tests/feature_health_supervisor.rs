use std::cell::Cell;

use pw_application::recovery::{
    BackoffDecision, Clock, FeatureHealthSupervisor, HealthTransition, HealthUpdate, RandomSource,
};
use pw_domain::runtime_health::{FailureCode, HealthStatus, RuntimeFailure, RuntimeFeature};

#[derive(Default)]
struct FakeClock(Cell<u64>);
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.get()
    }
}

#[test]
fn duplicate_failure_after_open_circuit_is_unchanged() {
    let clock = FakeClock::default();
    let mut supervisor = FeatureHealthSupervisor::new(RuntimeFeature::Live2D, &clock, MaxRandom);
    for _ in 0..8 {
        let _ = supervisor.record_failure(RuntimeFailure::transient(FailureCode::Internal));
    }

    assert!(matches!(
        supervisor.record_failure(RuntimeFailure::transient(FailureCode::Internal)),
        HealthUpdate::Unchanged {
            decision: BackoffDecision::CircuitOpen
        }
    ));
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
        let HealthUpdate::Changed {
            health,
            attempts,
            decision,
        } = supervisor.record_failure(RuntimeFailure::transient(FailureCode::Timeout))
        else {
            panic!()
        };
        assert_eq!(attempts, attempt);
        assert!(matches!(decision, BackoffDecision::RetryAfter(_)));
        assert_eq!(health.status(), HealthStatus::Recovering);
    }
    let HealthUpdate::Changed {
        health,
        attempts,
        decision,
    } = supervisor.record_failure(RuntimeFailure::transient(FailureCode::Timeout))
    else {
        panic!()
    };
    assert_eq!(attempts, 8);
    assert_eq!(decision, BackoffDecision::CircuitOpen);
    assert_eq!(health.status(), HealthStatus::Degraded);
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

#[test]
fn explicit_retry_can_recover_a_failed_frontend_before_the_circuit_opens() {
    let clock = FakeClock::default();
    let mut supervisor = FeatureHealthSupervisor::new(RuntimeFeature::Live2D, &clock, MaxRandom);
    supervisor.record_failure(RuntimeFailure::transient(FailureCode::Internal));

    let HealthTransition::Changed { attempts, health } = supervisor.retry_now().unwrap() else {
        panic!()
    };
    assert_eq!(attempts, 1);
    assert_eq!(health.status(), HealthStatus::Starting);
    assert!(supervisor.can_attempt());
}

#[test]
fn attempts_are_blocked_until_retry_deadline_and_forever_while_circuit_is_open() {
    let clock = FakeClock::default();
    let mut supervisor =
        FeatureHealthSupervisor::new(RuntimeFeature::LanguageModel, &clock, MaxRandom);
    supervisor.record_failure(RuntimeFailure::transient(FailureCode::Timeout));
    assert!(!supervisor.can_attempt());
    assert_eq!(supervisor.next_retry_at_ms(), Some(250));
    clock.0.set(249);
    assert!(!supervisor.can_attempt());
    clock.0.set(250);
    assert!(supervisor.can_attempt());
    for _ in 1..8 {
        supervisor.record_failure(RuntimeFailure::transient(FailureCode::Timeout));
    }
    clock.0.set(u64::MAX);
    assert!(!supervisor.can_attempt());
    assert!(supervisor.circuit_open());
    assert_eq!(supervisor.next_retry_at_ms(), None);
}
