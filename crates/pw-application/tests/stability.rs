use pw_application::stability::{
    ResourceLimits, ResourceSample, StabilityEvaluator, StabilityViolationKind,
};

fn sample(minute: u64, rss_mib: u64) -> ResourceSample {
    ResourceSample {
        timestamp_ms: minute * 60_000,
        rss_bytes: rss_mib * 1_048_576,
        private_bytes: rss_mib * 1_048_576,
        handle_count: 100,
        thread_count: 8,
        input_queue_depth: 0,
        output_queue_depth: 0,
        dropped_items: 0,
        cache_file_count: 4,
        log_bytes: 1_024,
        restart_count: 0,
        fault_count: 0,
        unexpected_exit_count: 0,
        panic_count: 0,
        orphan_process_count: 0,
    }
}

#[test]
fn bounded_jitter_after_warmup_passes() {
    let samples = [
        sample(0, 90),
        sample(1, 130),
        sample(2, 101),
        sample(3, 99),
        sample(4, 102),
        sample(5, 100),
    ];

    let result = StabilityEvaluator::new(ResourceLimits::default()).evaluate(&samples);

    assert!(result.is_ok(), "bounded jitter must pass: {result:?}");
}

#[test]
fn deterministic_stress_detects_intentional_memory_leak() {
    let samples: Vec<_> = (0..=12)
        .map(|minute| sample(minute, 100 + minute * 12))
        .collect();

    let violations = StabilityEvaluator::new(ResourceLimits::default())
        .evaluate(&samples)
        .expect_err("intentional leak must fail");

    assert!(violations.iter().any(|violation| {
        matches!(
            violation.kind,
            StabilityViolationKind::RssSlope | StabilityViolationKind::RssGrowth
        )
    }));
}

#[test]
fn unexpected_exit_panic_and_orphan_must_remain_zero() {
    let mut bad = sample(5, 100);
    bad.unexpected_exit_count = 1;
    bad.panic_count = 1;
    bad.orphan_process_count = 1;

    let violations = StabilityEvaluator::new(ResourceLimits::default())
        .evaluate(&[sample(0, 100), sample(3, 100), bad])
        .expect_err("process safety counters must fail");

    assert!(
        violations
            .iter()
            .any(|v| v.kind == StabilityViolationKind::UnexpectedExit)
    );
    assert!(
        violations
            .iter()
            .any(|v| v.kind == StabilityViolationKind::Panic)
    );
    assert!(
        violations
            .iter()
            .any(|v| v.kind == StabilityViolationKind::OrphanProcess)
    );
}

#[test]
fn queue_and_file_caps_are_enforced() {
    let mut bad = sample(5, 100);
    bad.input_queue_depth = 65;
    bad.output_queue_depth = 65;
    bad.dropped_items = 101;
    bad.cache_file_count = 1_001;
    bad.log_bytes = 257 * 1_048_576;
    bad.restart_count = 9;
    bad.fault_count = 33;

    let violations = StabilityEvaluator::new(ResourceLimits::default())
        .evaluate(&[sample(0, 100), sample(3, 100), bad])
        .expect_err("hard caps must fail");

    for expected in [
        StabilityViolationKind::InputQueueDepth,
        StabilityViolationKind::OutputQueueDepth,
        StabilityViolationKind::DroppedItems,
        StabilityViolationKind::CacheFileCount,
        StabilityViolationKind::LogBytes,
        StabilityViolationKind::RestartCount,
        StabilityViolationKind::FaultCount,
    ] {
        assert!(violations.iter().any(|v| v.kind == expected));
    }
}

#[test]
fn deterministic_stress_detects_handle_and_thread_slopes() {
    let mut after_warmup = sample(3, 100);
    after_warmup.handle_count = 150;
    after_warmup.thread_count = 12;
    let mut leaking = sample(5, 100);
    leaking.handle_count = 251;
    leaking.thread_count = 21;

    let violations = StabilityEvaluator::new(ResourceLimits::default())
        .evaluate(&[sample(0, 100), after_warmup, leaking])
        .expect_err("handle and thread growth must fail");

    for expected in [
        StabilityViolationKind::HandleSlope,
        StabilityViolationKind::HandleGrowth,
        StabilityViolationKind::ThreadSlope,
        StabilityViolationKind::ThreadGrowth,
    ] {
        assert!(violations.iter().any(|v| v.kind == expected));
    }
}

#[test]
fn transient_peak_is_not_hidden_by_a_low_final_sample() {
    let mut peak = sample(4, 100);
    peak.input_queue_depth = 65;
    peak.cache_file_count = 1_001;

    let violations = StabilityEvaluator::new(ResourceLimits::default())
        .evaluate(&[sample(0, 100), sample(3, 100), peak, sample(5, 100)])
        .expect_err("a transient cap violation must be retained");

    assert!(
        violations
            .iter()
            .any(|v| v.kind == StabilityViolationKind::InputQueueDepth)
    );
    assert!(
        violations
            .iter()
            .any(|v| v.kind == StabilityViolationKind::CacheFileCount)
    );
}
