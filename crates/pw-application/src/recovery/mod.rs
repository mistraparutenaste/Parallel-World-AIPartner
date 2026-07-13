use std::time::Duration;

pub trait Clock {
    fn now_ms(&self) -> u64;
}
pub trait RandomSource {
    fn uniform_inclusive(&mut self, upper: u64) -> u64;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackoffDecision {
    RetryAfter(Duration),
    CircuitOpen,
}

pub struct BackoffPolicy<'a, C, R> {
    clock: &'a C,
    rng: R,
    attempts: u8,
    healthy_since_ms: Option<u64>,
}

impl<'a, C: Clock, R: RandomSource> BackoffPolicy<'a, C, R> {
    pub const BASE_DELAY_MS: u64 = 250;
    pub const CAP_DELAY_MS: u64 = 30_000;
    pub const STABLE_RESET_MS: u64 = 60_000;
    pub const MAX_FAILURES: u8 = 8;
    #[must_use]
    pub const fn new(clock: &'a C, rng: R) -> Self {
        Self {
            clock,
            rng,
            attempts: 0,
            healthy_since_ms: None,
        }
    }
    pub fn record_failure(&mut self) -> BackoffDecision {
        self.healthy_since_ms = None;
        self.attempts = self.attempts.saturating_add(1);
        if self.attempts >= Self::MAX_FAILURES {
            return BackoffDecision::CircuitOpen;
        }
        let exponent = u32::from(self.attempts.saturating_sub(1));
        let maximum = Self::BASE_DELAY_MS
            .saturating_mul(2_u64.saturating_pow(exponent))
            .min(Self::CAP_DELAY_MS);
        BackoffDecision::RetryAfter(Duration::from_millis(self.rng.uniform_inclusive(maximum)))
    }
    pub fn record_healthy(&mut self) {
        self.healthy_since_ms
            .get_or_insert_with(|| self.clock.now_ms());
    }
    pub fn reset_if_stable(&mut self) -> bool {
        let stable = self.healthy_since_ms.is_some_and(|since| {
            self.clock.now_ms().saturating_sub(since) >= Self::STABLE_RESET_MS
        });
        if stable {
            self.attempts = 0;
            self.healthy_since_ms = None;
        }
        stable
    }
    #[must_use]
    pub const fn attempts(&self) -> u8 {
        self.attempts
    }
    #[must_use]
    pub fn maximum_delay(&self) -> Duration {
        let exponent = u32::from(self.attempts.saturating_sub(1));
        Duration::from_millis(
            Self::BASE_DELAY_MS
                .saturating_mul(2_u64.saturating_pow(exponent))
                .min(Self::CAP_DELAY_MS),
        )
    }
}
