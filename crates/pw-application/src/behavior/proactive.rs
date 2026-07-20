//! Candidate generation and fail-closed proactive interaction policies.
//!
//! Restart deliberately discards previous-session and pending-category continuity.
//! This can under-trigger return/category candidates. A currently long-running session
//! can be rediscovered; callers deduplicate its stable topic hash through history.

use std::sync::Mutex;

use sha2::{Digest, Sha256};

const TOPIC_PREFIX: &[u8] = b"parallel-world/proactive-topic/v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    Return,
    LongSession,
    CategoryChange,
}

impl CandidateKind {
    const fn tag(self) -> u8 {
        match self {
            Self::Return => 1,
            Self::LongSession => 2,
            Self::CategoryChange => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryId(String);

impl CategoryId {
    /// Validates a bounded, non-sensitive category identifier.
    ///
    /// # Errors
    /// Returns [`InvalidInput`] unless the value is 1..=32 bytes of lowercase
    /// ASCII letters, digits, underscores, or hyphens.
    pub fn new(value: &str) -> Result<Self, InvalidInput> {
        if !(1..=32).contains(&value.len())
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte)
            })
        {
            return Err(InvalidInput);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidInput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    session_id: u64,
    started_at: i64,
    last_seen_at: i64,
    category: CategoryId,
}

impl Observation {
    /// Creates one validated activity observation.
    /// Session ids are expected to be monotonically increasing `SQLite` row ids
    /// supplied by the activity repository; reuse or decrease resets the engine.
    ///
    /// # Errors
    /// Returns [`InvalidInput`] for a zero session id, negative start, or an
    /// observation timestamp before its start.
    pub fn new(
        session_id: u64,
        started_at: i64,
        last_seen_at: i64,
        category: CategoryId,
    ) -> Result<Self, InvalidInput> {
        if session_id == 0 || started_at < 0 || last_seen_at < started_at {
            return Err(InvalidInput);
        }
        Ok(Self {
            session_id,
            started_at,
            last_seen_at,
            category,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProactiveThresholds {
    pub return_after: i64,
    pub long_session: i64,
    pub category_change: i64,
}

impl Default for ProactiveThresholds {
    fn default() -> Self {
        Self {
            return_after: 600,
            long_session: 3_600,
            category_change: 600,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    kind: CandidateKind,
    category: CategoryId,
    duration_seconds: u64,
    topic_hash: [u8; 32],
}

impl Candidate {
    #[must_use]
    pub const fn kind(&self) -> CandidateKind {
        self.kind
    }

    #[must_use]
    pub fn category(&self) -> &CategoryId {
        &self.category
    }

    #[must_use]
    pub const fn duration_seconds(&self) -> u64 {
        self.duration_seconds
    }

    #[must_use]
    pub const fn topic_hash(&self) -> &[u8; 32] {
        &self.topic_hash
    }
}

#[derive(Debug, Clone)]
struct Previous {
    session_id: u64,
    started_at: i64,
    last_seen_at: i64,
    category: CategoryId,
}

#[derive(Debug, Clone)]
struct PendingCategory {
    category: CategoryId,
    since: i64,
    session_id: u64,
    started_at: i64,
}

pub struct CandidateEngine {
    thresholds: ProactiveThresholds,
    previous: Option<Previous>,
    stable_category: Option<CategoryId>,
    pending_category: Option<PendingCategory>,
    long_emitted: Option<(u64, i64)>,
}

impl CandidateEngine {
    #[must_use]
    pub fn new(thresholds: ProactiveThresholds) -> Self {
        Self {
            thresholds,
            previous: None,
            stable_category: None,
            pending_category: None,
            long_emitted: None,
        }
    }

    /// Consumes one validated observation and returns at most one candidate.
    pub fn observe(&mut self, observation: Observation) -> Option<Candidate> {
        let result = self.observe_ref(&observation);
        drop(observation);
        result
    }

    fn observe_ref(&mut self, observation: &Observation) -> Option<Candidate> {
        if !self.thresholds_valid() {
            self.reset();
            return None;
        }
        let Some(previous) = self.previous.clone() else {
            self.stable_category = Some(observation.category.clone());
            self.previous = Some(previous_from(observation));
            return None;
        };
        if observation.last_seen_at < previous.last_seen_at
            || observation.session_id < previous.session_id
            || (observation.session_id == previous.session_id
                && (observation.started_at != previous.started_at
                    || observation.category != previous.category))
        {
            self.reset();
            return None;
        }

        let new_session = observation.session_id > previous.session_id;
        let return_due = new_session
            && observation
                .started_at
                .checked_sub(previous.last_seen_at)
                .is_some_and(|gap| gap >= self.thresholds.return_after);

        self.update_pending(observation);

        let session_key = (observation.session_id, observation.started_at);
        let long_due = observation
            .last_seen_at
            .checked_sub(observation.started_at)
            .is_some_and(|duration| duration >= self.thresholds.long_session)
            && self.long_emitted != Some(session_key);
        let category_due = self.pending_category.as_ref().is_some_and(|pending| {
            observation
                .last_seen_at
                .checked_sub(pending.since)
                .is_some_and(|duration| duration >= self.thresholds.category_change)
        });

        self.previous = Some(previous_from(observation));

        if return_due {
            return Some(make_candidate(
                CandidateKind::Return,
                observation.session_id,
                observation.started_at,
                observation.started_at,
                &observation.category,
                observation.last_seen_at - observation.started_at,
            ));
        }
        if long_due {
            self.long_emitted = Some(session_key);
            return Some(make_candidate(
                CandidateKind::LongSession,
                observation.session_id,
                observation.started_at,
                observation.started_at,
                &observation.category,
                observation.last_seen_at - observation.started_at,
            ));
        }
        if category_due {
            let pending = self.pending_category.take()?;
            self.stable_category = Some(pending.category.clone());
            return Some(make_candidate(
                CandidateKind::CategoryChange,
                pending.session_id,
                pending.started_at,
                pending.since,
                &pending.category,
                observation.last_seen_at - pending.since,
            ));
        }
        None
    }

    /// Consumes constructor output so invalid numeric/category input also resets
    /// continuity instead of being silently discarded by an adapter.
    pub fn observe_checked(
        &mut self,
        observation: Result<Observation, InvalidInput>,
    ) -> Option<Candidate> {
        if let Ok(observation) = observation {
            self.observe(observation)
        } else {
            self.reset();
            None
        }
    }

    fn update_pending(&mut self, observation: &Observation) {
        let Some(stable) = &self.stable_category else {
            self.stable_category = Some(observation.category.clone());
            return;
        };
        if observation.category == *stable {
            self.pending_category = None;
            return;
        }
        if self
            .pending_category
            .as_ref()
            .is_none_or(|pending| pending.category != observation.category)
        {
            self.pending_category = Some(PendingCategory {
                category: observation.category.clone(),
                since: observation.last_seen_at,
                session_id: observation.session_id,
                started_at: observation.started_at,
            });
        }
    }

    fn thresholds_valid(&self) -> bool {
        self.thresholds.return_after >= 0
            && self.thresholds.long_session >= 0
            && self.thresholds.category_change >= 0
    }

    fn reset(&mut self) {
        self.previous = None;
        self.stable_category = None;
        self.pending_category = None;
        self.long_emitted = None;
    }
}

fn previous_from(observation: &Observation) -> Previous {
    Previous {
        session_id: observation.session_id,
        started_at: observation.started_at,
        last_seen_at: observation.last_seen_at,
        category: observation.category.clone(),
    }
}

fn make_candidate(
    kind: CandidateKind,
    session_id: u64,
    started_at: i64,
    event_at: i64,
    category: &CategoryId,
    duration_seconds: i64,
) -> Candidate {
    let mut hasher = Sha256::new();
    hasher.update(TOPIC_PREFIX);
    hasher.update([kind.tag()]);
    hasher.update(session_id.to_be_bytes());
    hasher.update(u64::try_from(started_at).unwrap_or_default().to_be_bytes());
    hasher.update(u64::try_from(event_at).unwrap_or_default().to_be_bytes());
    hasher.update([u8::try_from(category.0.len()).expect("category is at most 32 bytes")]);
    hasher.update(category.0.as_bytes());
    Candidate {
        kind,
        category: category.clone(),
        duration_seconds: u64::try_from(duration_seconds).unwrap_or_default(),
        topic_hash: hasher.finalize().into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrequencyPolicy {
    pub minimum_interval: i64,
    pub max_per_hour: u64,
    pub max_per_day: u64,
}

impl Default for FrequencyPolicy {
    fn default() -> Self {
        Self {
            minimum_interval: 900,
            max_per_hour: 3,
            max_per_day: 16,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrequencySnapshot {
    pub topic_exists: bool,
    pub latest_spoken_at: Option<i64>,
    pub spoken_last_hour: u64,
    pub spoken_last_day: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryQuery {
    pub topic_hash: [u8; 32],
    pub hour_since: i64,
    pub day_since: i64,
}

pub trait FrequencyHistory {
    type Error;
    /// Loads one temporally consistent snapshot for all rate-limit checks.
    ///
    /// # Errors
    /// Returns the storage implementation's opaque error. Callers fail closed.
    fn snapshot(&self, query: HistoryQuery) -> Result<FrequencySnapshot, Self::Error>;
}

/// Checks all topic and spoken-frequency constraints from one stable history snapshot.
#[must_use]
pub fn eligible_to_evaluate<H: FrequencyHistory>(
    history: &H,
    topic_hash: [u8; 32],
    now: i64,
    policy: FrequencyPolicy,
) -> bool {
    if now < 0 || policy.minimum_interval < 0 || policy.max_per_hour == 0 || policy.max_per_day == 0
    {
        return false;
    }
    let cutoff = |window: i64| if now < window { 0 } else { now - window + 1 };
    let Ok(snapshot) = history.snapshot(HistoryQuery {
        topic_hash,
        hour_since: cutoff(3_600),
        day_since: cutoff(86_400),
    }) else {
        return false;
    };
    if snapshot.topic_exists
        || snapshot.spoken_last_hour >= policy.max_per_hour
        || snapshot.spoken_last_day >= policy.max_per_day
    {
        return false;
    }
    match snapshot.latest_spoken_at {
        None => true,
        Some(latest) if latest < 0 || latest > now => false,
        Some(latest) => now - latest >= policy.minimum_interval,
    }
}

/// Inputs shared by the desktop behavior settings and the application gate.
/// Any unavailable/unsafe state is represented as `false` and fails closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProactiveGatePolicy {
    pub master_enabled: bool,
    pub profile_enabled: bool,
    pub snoozed_until: Option<i64>,
    pub temporary_conversation: bool,
    pub policy_error: bool,
    pub now: i64,
    pub frequency: FrequencyPolicy,
}

/// Lease returned inside the atomic proactive-claim closure. Callers should
/// check [`Self::is_cancelled`] immediately before enqueueing UI/TTS output.
pub struct ProactiveLease<'a> {
    gate: &'a InteractionGate,
    epoch: u64,
}

impl ProactiveLease<'_> {
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.gate.is_cancelled(self.epoch)
    }
}

/// Evaluates all proactive gates and runs the action at the claim boundary.
pub fn with_proactive_turn<H, T>(
    gate: &InteractionGate,
    history: &H,
    candidate: &Candidate,
    policy: ProactiveGatePolicy,
    action: impl FnOnce(ProactiveLease<'_>) -> T,
) -> Option<T>
where
    H: FrequencyHistory,
{
    if !policy.master_enabled
        || !policy.profile_enabled
        || policy.temporary_conversation
        || policy.policy_error
        || policy.now < 0
        || policy
            .snoozed_until
            .is_some_and(|until| until < 0 || until > policy.now)
    {
        return None;
    }
    let epoch = gate.capture_idle_epoch()?;
    if !eligible_to_evaluate(
        history,
        *candidate.topic_hash(),
        policy.now,
        policy.frequency,
    ) {
        return None;
    }
    gate.claim_if_idle(epoch, || action(ProactiveLease { gate, epoch }))
}

/// Grants one optional proactive turn only while the captured idle epoch is
/// still current and the durable frequency snapshot allows it.
#[must_use]
pub fn grant_proactive_turn<H: FrequencyHistory>(
    gate: &InteractionGate,
    history: &H,
    candidate: &Candidate,
    policy: ProactiveGatePolicy,
) -> bool {
    with_proactive_turn(gate, history, candidate, policy, |_| true).unwrap_or(false)
}

#[derive(Debug, Default)]
struct GateState {
    epoch: u64,
    outstanding_user_turns: u64,
    proactive_claimed: bool,
}

#[derive(Debug, Default)]
pub struct InteractionGate {
    state: Mutex<GateState>,
}

impl InteractionGate {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin_user_turn(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.epoch = state.epoch.wrapping_add(1);
            state.outstanding_user_turns = state.outstanding_user_turns.saturating_add(1);
            state.proactive_claimed = false;
        }
    }

    pub fn end_user_turn(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.outstanding_user_turns = state.outstanding_user_turns.saturating_sub(1);
        }
    }

    #[must_use]
    pub fn capture_idle_epoch(&self) -> Option<u64> {
        self.state
            .lock()
            .ok()
            .and_then(|state| (state.outstanding_user_turns == 0).then_some(state.epoch))
    }

    #[must_use]
    pub fn is_cancelled(&self, captured_epoch: u64) -> bool {
        self.state.lock().map_or(true, |state| {
            state.epoch != captured_epoch || state.outstanding_user_turns != 0
        })
    }

    /// Runs a constant-time, non-I/O, non-reentrant and non-panicking closure
    /// if the capture remains idle. Granting the result is the linearization point.
    pub fn commit_if_idle<T>(&self, captured_epoch: u64, grant: impl FnOnce() -> T) -> Option<T> {
        let state = self.state.lock().ok()?;
        if state.epoch != captured_epoch || state.outstanding_user_turns != 0 {
            return None;
        }
        Some(grant())
    }

    /// Atomically reserves the current idle epoch for one proactive action.
    /// A second candidate cannot claim the same idle epoch; the next user turn
    /// clears the reservation while also advancing the cancellation epoch.
    pub fn claim_if_idle<T>(&self, captured_epoch: u64, grant: impl FnOnce() -> T) -> Option<T> {
        let mut state = self.state.lock().ok()?;
        if state.epoch != captured_epoch
            || state.outstanding_user_turns != 0
            || state.proactive_claimed
        {
            return None;
        }
        state.proactive_claimed = true;
        drop(state);
        Some(grant())
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::{
        CandidateEngine, CategoryId, FrequencyHistory, FrequencyPolicy, FrequencySnapshot,
        HistoryQuery, InteractionGate, Observation, ProactiveGatePolicy, ProactiveThresholds,
        grant_proactive_turn, with_proactive_turn,
    };

    struct History(FrequencySnapshot);
    impl FrequencyHistory for History {
        type Error = ();
        fn snapshot(&self, _query: HistoryQuery) -> Result<FrequencySnapshot, Self::Error> {
            Ok(self.0)
        }
    }

    fn candidate() -> super::Candidate {
        let mut engine = CandidateEngine::new(ProactiveThresholds::default());
        let category = CategoryId::new("chat").unwrap();
        engine.observe(Observation::new(1, 0, 0, category.clone()).unwrap());
        engine
            .observe(Observation::new(2, 601, 1_201, category).unwrap())
            .expect("return candidate")
    }

    fn policy() -> ProactiveGatePolicy {
        ProactiveGatePolicy {
            master_enabled: true,
            profile_enabled: true,
            snoozed_until: None,
            temporary_conversation: false,
            policy_error: false,
            now: 1_000,
            frequency: FrequencyPolicy::default(),
        }
    }

    #[test]
    fn poisoned_gate_fails_closed() {
        let gate = InteractionGate::new();
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _lock = gate.state.lock().unwrap();
            panic!("poison gate");
        }));
        assert_eq!(gate.capture_idle_epoch(), None);
        assert!(gate.is_cancelled(0));
        assert_eq!(gate.commit_if_idle(0, || 1), None);
    }

    #[test]
    fn proactive_gate_fails_closed_for_user_turn_privacy_settings_and_rate_limits() {
        let gate = InteractionGate::new();
        let candidate = candidate();
        let history = History(FrequencySnapshot::default());
        assert!(grant_proactive_turn(&gate, &history, &candidate, policy()));
        assert!(!grant_proactive_turn(&gate, &history, &candidate, policy()));

        for mutate in [
            |value: &mut ProactiveGatePolicy| value.master_enabled = false,
            |value: &mut ProactiveGatePolicy| value.snoozed_until = Some(2_000),
            |value: &mut ProactiveGatePolicy| value.temporary_conversation = true,
            |value: &mut ProactiveGatePolicy| value.policy_error = true,
        ] {
            let mut blocked = policy();
            mutate(&mut blocked);
            assert!(!grant_proactive_turn(&gate, &history, &candidate, blocked));
        }
        let mut invalid_snooze = policy();
        invalid_snooze.snoozed_until = Some(-1);
        assert!(!grant_proactive_turn(
            &gate,
            &history,
            &candidate,
            invalid_snooze
        ));

        let limited = History(FrequencySnapshot {
            topic_exists: true,
            ..FrequencySnapshot::default()
        });
        assert!(!grant_proactive_turn(&gate, &limited, &candidate, policy()));

        gate.begin_user_turn();
        assert!(!grant_proactive_turn(&gate, &history, &candidate, policy()));
        gate.end_user_turn();
        assert!(grant_proactive_turn(&gate, &history, &candidate, policy()));
    }

    #[test]
    fn proactive_lease_cancels_output_when_user_turn_begins_after_claim() {
        let gate = InteractionGate::new();
        let history = History(FrequencySnapshot::default());
        let candidate = candidate();
        let cancelled = with_proactive_turn(&gate, &history, &candidate, policy(), |lease| {
            gate.begin_user_turn();
            lease.is_cancelled()
        });
        assert_eq!(cancelled, Some(true));
        gate.end_user_turn();
    }
}
