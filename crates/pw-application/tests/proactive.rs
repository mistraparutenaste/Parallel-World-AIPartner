use std::sync::atomic::{AtomicUsize, Ordering};

use pw_application::behavior::proactive::{
    CandidateEngine, CandidateKind, CategoryId, FrequencyHistory, FrequencyPolicy,
    FrequencySnapshot, HistoryQuery, InteractionGate, Observation, ProactiveThresholds,
    eligible_to_evaluate,
};

fn observation(session_id: u64, started_at: i64, last_seen_at: i64, category: &str) -> Observation {
    Observation::new(
        session_id,
        started_at,
        last_seen_at,
        CategoryId::new(category).unwrap(),
    )
    .unwrap()
}

fn engine() -> CandidateEngine {
    CandidateEngine::new(ProactiveThresholds {
        return_after: 600,
        long_session: 3_600,
        category_change: 600,
    })
}

#[test]
fn proactive_return_fires_at_ten_minute_boundary_but_not_before() {
    let mut just_before = engine();
    assert_eq!(just_before.observe(observation(1, 0, 100, "work")), None);
    assert_eq!(just_before.observe(observation(2, 699, 699, "work")), None);

    let mut boundary = engine();
    assert_eq!(boundary.observe(observation(1, 0, 100, "work")), None);
    let candidate = boundary
        .observe(observation(2, 700, 700, "work"))
        .expect("boundary emits return");
    assert_eq!(candidate.kind(), CandidateKind::Return);
}

#[test]
fn proactive_long_session_fires_at_sixty_minutes_and_only_once() {
    let mut engine = engine();
    assert_eq!(engine.observe(observation(1, 10, 3_609, "work")), None);
    assert_eq!(
        engine
            .observe(observation(1, 10, 3_610, "work"))
            .unwrap()
            .kind(),
        CandidateKind::LongSession
    );
    assert_eq!(engine.observe(observation(1, 10, 4_000, "work")), None);
}

#[test]
fn proactive_category_change_sustains_resets_on_bounce_and_emits_once() {
    let mut engine = engine();
    assert_eq!(engine.observe(observation(1, 0, 100, "work")), None);
    assert_eq!(engine.observe(observation(2, 101, 101, "game")), None);
    assert_eq!(engine.observe(observation(2, 101, 700, "game")), None);
    assert_eq!(engine.observe(observation(3, 701, 701, "work")), None);
    assert_eq!(engine.observe(observation(4, 702, 702, "game")), None);
    assert_eq!(
        engine
            .observe(observation(4, 702, 1_302, "game"))
            .unwrap()
            .kind(),
        CandidateKind::CategoryChange
    );
    assert_eq!(engine.observe(observation(4, 702, 1_303, "game")), None);
}

#[test]
fn proactive_category_timer_continues_across_a_new_session_of_same_category() {
    let mut engine = engine();
    engine.observe(observation(1, 0, 100, "work"));
    engine.observe(observation(2, 101, 101, "game"));
    assert_eq!(engine.observe(observation(3, 400, 400, "game")), None);
    assert_eq!(
        engine
            .observe(observation(3, 400, 701, "game"))
            .unwrap()
            .kind(),
        CandidateKind::CategoryChange
    );
}

#[test]
fn proactive_priority_delays_lower_candidate_to_later_observation() {
    let mut candidate_engine = engine();
    assert_eq!(candidate_engine.observe(observation(1, 0, 0, "work")), None);
    assert_eq!(candidate_engine.observe(observation(2, 1, 1, "game")), None);
    let long = candidate_engine
        .observe(observation(2, 1, 3_601, "game"))
        .expect("long and category are due; long wins");
    assert_eq!(long.kind(), CandidateKind::LongSession);
    let category = candidate_engine
        .observe(observation(2, 1, 3_602, "game"))
        .expect("unemitted category remains due");
    assert_eq!(category.kind(), CandidateKind::CategoryChange);

    let mut return_priority = engine();
    assert_eq!(return_priority.observe(observation(9, 0, 0, "work")), None);
    let first = return_priority
        .observe(observation(10, 600, 4_200, "work"))
        .expect("return and long are due; return wins");
    assert_eq!(first.kind(), CandidateKind::Return);
    assert_eq!(
        return_priority
            .observe(observation(10, 600, 4_201, "work"))
            .unwrap()
            .kind(),
        CandidateKind::LongSession
    );
}

#[test]
fn proactive_topic_hash_is_stable_distinct_and_matches_golden_vector() {
    let mut a = engine();
    a.observe(observation(1, 0, 100, "work"));
    let first = a.observe(observation(2, 700, 700, "work")).unwrap();
    let mut b = engine();
    b.observe(observation(1, 0, 100, "work"));
    let same = b.observe(observation(2, 700, 700, "work")).unwrap();
    assert_eq!(first.topic_hash(), same.topic_hash());
    assert_eq!(
        hex(first.topic_hash()),
        "b18ab75f5a5684aa3c1256d05d2ae2eada181c153d9624f58b69d1a2bdff6744"
    );

    let mut c = engine();
    c.observe(observation(3, 0, 0, "work"));
    let long = c.observe(observation(3, 0, 3_600, "work")).unwrap();
    assert_ne!(first.topic_hash(), long.topic_hash());

    let mut different_session = engine();
    different_session.observe(observation(8, 0, 100, "work"));
    let different = different_session
        .observe(observation(9, 700, 700, "work"))
        .unwrap();
    assert_ne!(first.topic_hash(), different.topic_hash());
}

#[test]
fn proactive_invalid_or_nonmonotonic_observation_resets_continuity() {
    let mut candidate_engine = engine();
    candidate_engine.observe(observation(1, 0, 100, "work"));
    assert_eq!(
        candidate_engine.observe(observation(1, 0, 99, "work")),
        None
    );
    assert_eq!(
        candidate_engine.observe(observation(2, 700, 700, "work")),
        None
    );
    assert!(CategoryId::new("").is_err());
    assert!(CategoryId::new("Raw Title").is_err());
    assert!(Observation::new(0, 0, 0, CategoryId::new("work").unwrap()).is_err());
    assert!(Observation::new(1, -1, 0, CategoryId::new("work").unwrap()).is_err());

    let mut changed_same_session = engine();
    changed_same_session.observe(observation(1, 0, 100, "work"));
    assert_eq!(
        changed_same_session.observe(observation(1, 0, 101, "game")),
        None
    );
    assert_eq!(
        changed_same_session.observe(observation(2, 700, 700, "game")),
        None
    );

    let mut invalid_input = engine();
    invalid_input.observe(observation(1, 0, 100, "work"));
    invalid_input.observe_checked(Observation::new(0, 0, 0, CategoryId::new("work").unwrap()));
    assert_eq!(
        invalid_input.observe(observation(2, 700, 700, "work")),
        None
    );
}

struct History {
    calls: AtomicUsize,
    result: Result<FrequencySnapshot, ()>,
    query: std::sync::Mutex<Option<HistoryQuery>>,
}

impl FrequencyHistory for History {
    type Error = ();

    fn snapshot(&self, query: HistoryQuery) -> Result<FrequencySnapshot, Self::Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.query.lock().unwrap() = Some(query);
        self.result
    }
}

fn history(result: Result<FrequencySnapshot, ()>) -> History {
    History {
        calls: AtomicUsize::new(0),
        result,
        query: std::sync::Mutex::new(None),
    }
}

#[test]
fn proactive_frequency_boundaries_use_one_snapshot_and_fail_closed() {
    let policy = FrequencyPolicy::default();
    let topic = [7; 32];
    let ok = history(Ok(FrequencySnapshot {
        topic_exists: false,
        latest_spoken_at: Some(100),
        spoken_last_hour: 2,
        spoken_last_day: 15,
    }));
    assert!(eligible_to_evaluate(&ok, topic, 1_000, policy));
    assert_eq!(ok.calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        ok.query.lock().unwrap().as_ref().unwrap(),
        &HistoryQuery {
            topic_hash: topic,
            hour_since: 0,
            day_since: 0,
        }
    );

    for snapshot in [
        FrequencySnapshot {
            topic_exists: true,
            latest_spoken_at: None,
            spoken_last_hour: 0,
            spoken_last_day: 0,
        },
        FrequencySnapshot {
            topic_exists: false,
            latest_spoken_at: Some(101),
            spoken_last_hour: 0,
            spoken_last_day: 0,
        },
        FrequencySnapshot {
            topic_exists: false,
            latest_spoken_at: Some(2_000),
            spoken_last_hour: 0,
            spoken_last_day: 0,
        },
        FrequencySnapshot {
            topic_exists: false,
            latest_spoken_at: None,
            spoken_last_hour: 3,
            spoken_last_day: 0,
        },
        FrequencySnapshot {
            topic_exists: false,
            latest_spoken_at: None,
            spoken_last_hour: 0,
            spoken_last_day: 16,
        },
    ] {
        assert!(!eligible_to_evaluate(
            &history(Ok(snapshot)),
            topic,
            1_000,
            policy
        ));
    }
    assert!(!eligible_to_evaluate(
        &history(Err(())),
        topic,
        1_000,
        policy
    ));

    let later = history(Ok(FrequencySnapshot::default()));
    assert!(eligible_to_evaluate(&later, topic, 86_400, policy));
    assert_eq!(
        later.query.lock().unwrap().as_ref().unwrap().hour_since,
        82_801
    );
    assert_eq!(later.query.lock().unwrap().as_ref().unwrap().day_since, 1);
}

#[test]
fn proactive_interaction_gate_has_epoch_cancellation_and_linearization() {
    let gate = InteractionGate::new();
    let epoch = gate.capture_idle_epoch().unwrap();
    assert!(!gate.is_cancelled(epoch));
    gate.begin_user_turn();
    assert!(gate.is_cancelled(epoch));
    assert!(gate.commit_if_idle(epoch, || 1).is_none());
    gate.end_user_turn();
    gate.end_user_turn();
    let current = gate.capture_idle_epoch().unwrap();
    assert_eq!(gate.commit_if_idle(current, || 7), Some(7));
    gate.begin_user_turn();
    gate.end_user_turn();
    assert!(gate.commit_if_idle(current, || 9).is_none());
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    bytes.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").unwrap();
        output
    })
}
