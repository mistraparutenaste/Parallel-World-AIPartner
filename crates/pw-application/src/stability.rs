//! Deterministic resource-bound evaluation for short stress and long soak runs.

#![allow(clippy::cast_precision_loss)] // Resource counters are intentionally regressed as f64.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceSample {
    pub timestamp_ms: u64,
    pub rss_bytes: u64,
    pub private_bytes: u64,
    pub handle_count: u64,
    pub thread_count: u64,
    pub input_queue_depth: u64,
    pub output_queue_depth: u64,
    pub dropped_items: u64,
    pub cache_file_count: u64,
    pub log_bytes: u64,
    pub restart_count: u64,
    pub fault_count: u64,
    pub unexpected_exit_count: u64,
    pub panic_count: u64,
    pub orphan_process_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourceLimits {
    pub warmup: Duration,
    pub max_rss_slope_bytes_per_hour: f64,
    pub max_rss_growth_bytes: u64,
    pub max_private_slope_bytes_per_hour: f64,
    pub max_private_growth_bytes: u64,
    pub max_handle_slope_per_hour: f64,
    pub max_handle_growth: u64,
    pub max_thread_slope_per_hour: f64,
    pub max_thread_growth: u64,
    pub max_input_queue_depth: u64,
    pub max_output_queue_depth: u64,
    pub max_dropped_items: u64,
    pub max_cache_file_count: u64,
    pub max_log_bytes: u64,
    pub max_restart_count: u64,
    pub max_fault_count: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        const MIB: u64 = 1_048_576;
        Self {
            warmup: Duration::from_mins(2),
            max_rss_slope_bytes_per_hour: 67_108_864.0,
            max_rss_growth_bytes: 64 * MIB,
            max_private_slope_bytes_per_hour: 67_108_864.0,
            max_private_growth_bytes: 64 * MIB,
            max_handle_slope_per_hour: 60.0,
            max_handle_growth: 100,
            max_thread_slope_per_hour: 6.0,
            max_thread_growth: 8,
            max_input_queue_depth: 64,
            max_output_queue_depth: 64,
            max_dropped_items: 100,
            max_cache_file_count: 1_000,
            max_log_bytes: 256 * MIB,
            max_restart_count: 8,
            max_fault_count: 32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StabilityViolationKind {
    InvalidTimeline,
    RssSlope,
    RssGrowth,
    PrivateBytesSlope,
    PrivateBytesGrowth,
    HandleSlope,
    HandleGrowth,
    ThreadSlope,
    ThreadGrowth,
    InputQueueDepth,
    OutputQueueDepth,
    DroppedItems,
    CacheFileCount,
    LogBytes,
    RestartCount,
    FaultCount,
    UnexpectedExit,
    Panic,
    OrphanProcess,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StabilityViolation {
    pub kind: StabilityViolationKind,
    pub observed: f64,
    pub limit: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct StabilityEvaluator {
    limits: ResourceLimits,
}

impl StabilityEvaluator {
    #[must_use]
    pub const fn new(limits: ResourceLimits) -> Self {
        Self { limits }
    }

    /// Returns every violated bound so one run yields actionable diagnostics.
    ///
    /// # Errors
    ///
    /// Returns all detected violations, including an invalid empty or unordered timeline.
    #[allow(clippy::too_many_lines)]
    pub fn evaluate(&self, samples: &[ResourceSample]) -> Result<(), Vec<StabilityViolation>> {
        let Some(first) = samples.first() else {
            return Err(vec![violation(
                StabilityViolationKind::InvalidTimeline,
                0,
                1,
            )]);
        };
        if samples
            .windows(2)
            .any(|pair| pair[0].timestamp_ms >= pair[1].timestamp_ms)
        {
            return Err(vec![violation(
                StabilityViolationKind::InvalidTimeline,
                0,
                1,
            )]);
        }

        let warmup_ms = u64::try_from(self.limits.warmup.as_millis()).unwrap_or(u64::MAX);
        let start_ms = first.timestamp_ms.saturating_add(warmup_ms);
        let steady: Vec<_> = samples
            .iter()
            .filter(|sample| sample.timestamp_ms >= start_ms)
            .collect();
        let all: Vec<_> = samples.iter().collect();
        let measured = if steady.len() >= 2 {
            steady.as_slice()
        } else {
            all.as_slice()
        };
        let mut violations = Vec::new();
        evaluate_trend(
            measured,
            |s| s.rss_bytes,
            self.limits.max_rss_slope_bytes_per_hour,
            self.limits.max_rss_growth_bytes,
            StabilityViolationKind::RssSlope,
            StabilityViolationKind::RssGrowth,
            &mut violations,
        );
        evaluate_trend(
            measured,
            |s| s.private_bytes,
            self.limits.max_private_slope_bytes_per_hour,
            self.limits.max_private_growth_bytes,
            StabilityViolationKind::PrivateBytesSlope,
            StabilityViolationKind::PrivateBytesGrowth,
            &mut violations,
        );
        evaluate_trend(
            measured,
            |s| s.handle_count,
            self.limits.max_handle_slope_per_hour,
            self.limits.max_handle_growth,
            StabilityViolationKind::HandleSlope,
            StabilityViolationKind::HandleGrowth,
            &mut violations,
        );
        evaluate_trend(
            measured,
            |s| s.thread_count,
            self.limits.max_thread_slope_per_hour,
            self.limits.max_thread_growth,
            StabilityViolationKind::ThreadSlope,
            StabilityViolationKind::ThreadGrowth,
            &mut violations,
        );
        for (kind, observed, limit) in [
            (
                StabilityViolationKind::InputQueueDepth,
                max_of(samples, |s| s.input_queue_depth),
                self.limits.max_input_queue_depth,
            ),
            (
                StabilityViolationKind::OutputQueueDepth,
                max_of(samples, |s| s.output_queue_depth),
                self.limits.max_output_queue_depth,
            ),
            (
                StabilityViolationKind::DroppedItems,
                max_of(samples, |s| s.dropped_items),
                self.limits.max_dropped_items,
            ),
            (
                StabilityViolationKind::CacheFileCount,
                max_of(samples, |s| s.cache_file_count),
                self.limits.max_cache_file_count,
            ),
            (
                StabilityViolationKind::LogBytes,
                max_of(samples, |s| s.log_bytes),
                self.limits.max_log_bytes,
            ),
            (
                StabilityViolationKind::RestartCount,
                max_of(samples, |s| s.restart_count),
                self.limits.max_restart_count,
            ),
            (
                StabilityViolationKind::FaultCount,
                max_of(samples, |s| s.fault_count),
                self.limits.max_fault_count,
            ),
            (
                StabilityViolationKind::UnexpectedExit,
                max_of(samples, |s| s.unexpected_exit_count),
                0,
            ),
            (
                StabilityViolationKind::Panic,
                max_of(samples, |s| s.panic_count),
                0,
            ),
            (
                StabilityViolationKind::OrphanProcess,
                max_of(samples, |s| s.orphan_process_count),
                0,
            ),
        ] {
            if observed > limit {
                violations.push(violation(kind, observed, limit));
            }
        }
        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}

fn max_of(samples: &[ResourceSample], value: impl Fn(&ResourceSample) -> u64) -> u64 {
    samples.iter().map(value).max().unwrap_or(0)
}

fn evaluate_trend(
    samples: &[&ResourceSample],
    value: impl Fn(&ResourceSample) -> u64,
    max_slope: f64,
    max_growth: u64,
    slope_kind: StabilityViolationKind,
    growth_kind: StabilityViolationKind,
    violations: &mut Vec<StabilityViolation>,
) {
    if samples.len() < 2 {
        return;
    }
    let origin = samples[0].timestamp_ms;
    let points: Vec<_> = samples
        .iter()
        .map(|s| {
            (
                (s.timestamp_ms - origin) as f64 / 3_600_000.0,
                value(s) as f64,
            )
        })
        .collect();
    let mean_x = points.iter().map(|p| p.0).sum::<f64>() / points.len() as f64;
    let mean_y = points.iter().map(|p| p.1).sum::<f64>() / points.len() as f64;
    let denominator = points.iter().map(|p| (p.0 - mean_x).powi(2)).sum::<f64>();
    let slope = if denominator == 0.0 {
        0.0
    } else {
        points
            .iter()
            .map(|p| (p.0 - mean_x) * (p.1 - mean_y))
            .sum::<f64>()
            / denominator
    };
    if slope > max_slope {
        violations.push(StabilityViolation {
            kind: slope_kind,
            observed: slope,
            limit: max_slope,
        });
    }
    let growth = value(samples[samples.len() - 1]).saturating_sub(value(samples[0]));
    if growth > max_growth {
        violations.push(violation(growth_kind, growth, max_growth));
    }
}

fn violation(kind: StabilityViolationKind, observed: u64, limit: u64) -> StabilityViolation {
    StabilityViolation {
        kind,
        observed: observed as f64,
        limit: limit as f64,
    }
}
