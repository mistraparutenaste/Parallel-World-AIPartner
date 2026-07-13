use std::cell::Cell;
use std::time::Duration;

use pw_application::recovery::{BackoffDecision, BackoffPolicy, Clock, RandomSource};

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

struct MaliciousRandom;
impl RandomSource for MaliciousRandom {
    fn uniform_inclusive(&mut self, upper: u64) -> u64 {
        upper.saturating_add(999_999)
    }
}

#[test]
fn full_jitter_is_capped_and_circuit_opens_after_eight_failures() {
    let clock = FakeClock(Cell::new(0));
    let mut policy = BackoffPolicy::new(&clock, MaxRandom);
    let expected = [250, 500, 1_000, 2_000, 4_000, 8_000, 16_000];
    for delay in expected {
        assert_eq!(
            policy.record_failure(),
            BackoffDecision::RetryAfter(Duration::from_millis(delay))
        );
    }
    assert_eq!(policy.record_failure(), BackoffDecision::CircuitOpen);
}

#[test]
fn malicious_rng_is_clamped_to_the_attempt_maximum() {
    let clock = FakeClock(Cell::new(0));
    let mut policy = BackoffPolicy::new(&clock, MaliciousRandom);
    assert_eq!(
        policy.record_failure(),
        BackoffDecision::RetryAfter(Duration::from_millis(250))
    );
}

#[test]
fn stable_health_for_sixty_seconds_resets_attempts() {
    let clock = FakeClock(Cell::new(0));
    let mut policy = BackoffPolicy::new(&clock, MaxRandom);
    assert_eq!(
        policy.record_failure(),
        BackoffDecision::RetryAfter(Duration::from_millis(250))
    );
    policy.record_healthy();
    clock.0.set(60_000);
    assert!(policy.reset_if_stable());
    assert_eq!(policy.attempts(), 0);
    assert_eq!(
        policy.record_failure(),
        BackoffDecision::RetryAfter(Duration::from_millis(250))
    );
}

#[test]
fn jitter_never_exceeds_thirty_second_cap() {
    let clock = FakeClock(Cell::new(0));
    let mut policy = BackoffPolicy::new(&clock, MaxRandom);
    for _ in 0..7 {
        let _ = policy.record_failure();
    }
    assert_eq!(policy.maximum_delay(), Duration::from_secs(16));
}

#[test]
fn failure_after_a_stable_healthy_operation_restarts_at_attempt_one() {
    let clock = FakeClock(Cell::new(0));
    let mut policy = BackoffPolicy::new(&clock, MaxRandom);
    let _ = policy.record_failure();
    let _ = policy.record_failure();
    policy.record_healthy();
    clock.0.set(60_001);

    assert_eq!(
        policy.record_failure(),
        BackoffDecision::RetryAfter(Duration::from_millis(250))
    );
    assert_eq!(policy.attempts(), 1);
}
