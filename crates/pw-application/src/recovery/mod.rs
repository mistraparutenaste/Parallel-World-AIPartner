use std::time::Duration;

use pw_domain::runtime_health::{RuntimeFailure, RuntimeFeature, RuntimeHealth};

pub trait Clock {
    fn now_ms(&self) -> u64;
}

#[derive(Clone, Copy, Default)]
pub struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }
}

pub struct TimeJitter(u64);
impl Default for TimeJitter {
    fn default() -> Self {
        Self(SystemClock.now_ms().max(1))
    }
}
impl RandomSource for TimeJitter {
    fn uniform_inclusive(&mut self, upper: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        if upper == u64::MAX {
            self.0
        } else {
            self.0 % (upper + 1)
        }
    }
}
impl<T: Clock + ?Sized> Clock for &T {
    fn now_ms(&self) -> u64 {
        (*self).now_ms()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthUpdate {
    Changed {
        health: RuntimeHealth,
        attempts: u8,
        decision: BackoffDecision,
    },
    Unchanged {
        decision: BackoffDecision,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthTransition {
    Changed { health: RuntimeHealth, attempts: u8 },
    Unchanged,
}

/// Persistent, feature-local health state. Callers emit only `Changed` transitions.
pub struct FeatureHealthSupervisor<C, R> {
    policy: BackoffPolicy<C, R>,
    health: RuntimeHealth,
    circuit_open: bool,
    next_retry_at_ms: Option<u64>,
}

impl<C: Clock, R: RandomSource> FeatureHealthSupervisor<C, R> {
    pub fn new(feature: RuntimeFeature, clock: C, rng: R) -> Self {
        Self {
            policy: BackoffPolicy::new(clock, rng),
            health: RuntimeHealth::new(feature),
            circuit_open: false,
            next_retry_at_ms: None,
        }
    }

    pub fn record_failure(&mut self, failure: RuntimeFailure) -> HealthUpdate {
        if self.circuit_open {
            return HealthUpdate::Unchanged {
                decision: BackoffDecision::CircuitOpen,
            };
        }
        let decision = self.policy.record_failure();
        let now = self.policy.now_ms();
        if decision == BackoffDecision::CircuitOpen {
            self.circuit_open = true;
            self.next_retry_at_ms = None;
            self.health.mark_degraded(&failure, now);
        } else {
            let BackoffDecision::RetryAfter(delay) = decision else {
                unreachable!()
            };
            self.next_retry_at_ms =
                Some(now.saturating_add(u64::try_from(delay.as_millis()).unwrap_or(u64::MAX)));
            self.health.mark_failed(&failure, now);
        }
        HealthUpdate::Changed {
            health: self.health.clone(),
            attempts: self.policy.attempts(),
            decision,
        }
    }

    pub fn record_success(&mut self) -> HealthTransition {
        let was_healthy = self.health.status() == pw_domain::runtime_health::HealthStatus::Healthy;
        self.policy.record_healthy();
        self.next_retry_at_ms = None;
        let reset = self.policy.reset_if_stable();
        if was_healthy && !reset {
            return HealthTransition::Unchanged;
        }
        self.health.mark_healthy(self.policy.now_ms());
        HealthTransition::Changed {
            health: self.health.clone(),
            attempts: self.policy.attempts(),
        }
    }

    /// Clears this feature's open circuit without affecting another supervisor.
    ///
    /// # Errors
    /// Returns an error unless this feature's circuit is currently open.
    pub fn rearm(&mut self) -> Result<HealthTransition, &'static str> {
        if !self.circuit_open {
            return Err("feature circuit is not open");
        }
        self.circuit_open = false;
        self.next_retry_at_ms = None;
        self.policy.rearm();
        self.health.mark_starting(self.policy.now_ms());
        Ok(HealthTransition::Changed {
            health: self.health.clone(),
            attempts: 0,
        })
    }

    /// Explicitly retries a failed frontend runtime before its circuit opens.
    /// Automatic callers must continue to respect [`Self::can_attempt`].
    ///
    /// # Errors
    /// Returns an error while the feature is already healthy or starting.
    pub fn retry_now(&mut self) -> Result<HealthTransition, &'static str> {
        if self.circuit_open {
            return self.rearm();
        }
        if matches!(
            self.health.status(),
            pw_domain::runtime_health::HealthStatus::Healthy
                | pw_domain::runtime_health::HealthStatus::Starting
        ) {
            return Err("feature is not waiting for retry");
        }
        self.next_retry_at_ms = None;
        self.health.mark_starting(self.policy.now_ms());
        Ok(HealthTransition::Changed {
            health: self.health.clone(),
            attempts: self.policy.attempts(),
        })
    }

    #[must_use]
    pub const fn attempts(&self) -> u8 {
        self.policy.attempts()
    }
    #[must_use]
    pub const fn health(&self) -> &RuntimeHealth {
        &self.health
    }
    #[must_use]
    pub const fn circuit_open(&self) -> bool {
        self.circuit_open
    }
    #[must_use]
    pub fn can_attempt(&self) -> bool {
        !self.circuit_open
            && self
                .next_retry_at_ms
                .is_none_or(|deadline| self.policy.now_ms() >= deadline)
    }
    #[must_use]
    pub const fn next_retry_at_ms(&self) -> Option<u64> {
        self.next_retry_at_ms
    }
}
pub trait RandomSource {
    fn uniform_inclusive(&mut self, upper: u64) -> u64;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackoffDecision {
    RetryAfter(Duration),
    CircuitOpen,
}

pub struct BackoffPolicy<C, R> {
    clock: C,
    rng: R,
    attempts: u8,
    healthy_since_ms: Option<u64>,
}

impl<C: Clock, R: RandomSource> BackoffPolicy<C, R> {
    pub const BASE_DELAY_MS: u64 = 250;
    pub const CAP_DELAY_MS: u64 = 30_000;
    pub const STABLE_RESET_MS: u64 = 60_000;
    pub const MAX_FAILURES: u8 = 8;
    #[must_use]
    pub const fn new(clock: C, rng: R) -> Self {
        Self {
            clock,
            rng,
            attempts: 0,
            healthy_since_ms: None,
        }
    }
    pub fn record_failure(&mut self) -> BackoffDecision {
        self.reset_if_stable();
        self.healthy_since_ms = None;
        self.attempts = self.attempts.saturating_add(1);
        if self.attempts >= Self::MAX_FAILURES {
            return BackoffDecision::CircuitOpen;
        }
        let exponent = u32::from(self.attempts.saturating_sub(1));
        let maximum = Self::BASE_DELAY_MS
            .saturating_mul(2_u64.saturating_pow(exponent))
            .min(Self::CAP_DELAY_MS);
        BackoffDecision::RetryAfter(Duration::from_millis(
            self.rng.uniform_inclusive(maximum).min(maximum),
        ))
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
    pub fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }
    pub fn rearm(&mut self) {
        self.attempts = 0;
        self.healthy_since_ms = None;
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
