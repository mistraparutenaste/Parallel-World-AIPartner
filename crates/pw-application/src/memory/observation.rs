use super::{MemoryAtom, NormalizationEdit};
use crate::PortError;
use sha2::{Digest, Sha256};

/// The durable, user-authored input that classification is allowed to inspect.
/// Assistant output and summaries are deliberately not representable here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewObservation {
    pub conversation_id: String,
    pub turn_id: u64,
    pub user_text: String,
    pub canonical_input_hash: String,
    pub observed_at: i64,
}

impl NewObservation {
    #[must_use]
    pub fn new(
        conversation_id: impl Into<String>,
        turn_id: u64,
        user_text: impl Into<String>,
        observed_at: i64,
    ) -> Self {
        let user_text = user_text.into();
        let canonical_input_hash = input_hash(&user_text);
        Self {
            conversation_id: conversation_id.into(),
            turn_id,
            user_text,
            canonical_input_hash,
            observed_at,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationOutcome {
    Pending,
    Completed,
    Cancelled,
    LlmFailed,
    HistoryPersistFailed,
    Interrupted,
}

impl ObservationOutcome {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Pending,
                Self::Completed
                    | Self::Cancelled
                    | Self::LlmFailed
                    | Self::HistoryPersistFailed
                    | Self::Interrupted
            )
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessingState {
    Pending,
    Processing,
    Completed,
    Deferred,
}

/// The terminal result of one classifier transport attempt.  These values are
/// deliberately separate from the response outcome: an accepted user turn can
/// be classified even when generating its reply was cancelled or failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassificationOutcome {
    Completed,
    Failed,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationLease {
    pub observation_id: i64,
    pub conversation_id: String,
    pub turn_id: u64,
    pub user_text: String,
    pub canonical_input_hash: String,
    pub deletion_generation: i64,
    pub owner: String,
    pub expires_at: i64,
    pub attempt_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassificationRun {
    pub observation_id: i64,
    pub classifier_version: String,
    pub schema_version: i64,
    pub input_hash: String,
    pub request_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistedCandidate {
    pub classification_run_id: i64,
    pub ordinal: i64,
    pub atom: MemoryAtom,
    pub target_memory_id: Option<i64>,
    pub expected_target_revision: Option<i64>,
    pub operation: CandidateOperation,
    pub relation: CandidateProvenanceRelation,
    /// A deterministic normalization trace is persisted and replayed before a
    /// candidate can change durable memory.
    pub normalization_edits: Vec<NormalizationEdit>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateOperation {
    Add,
    Reinforce,
    Supersede,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateProvenanceRelation {
    Originated,
    Reasserted,
    Corrected,
    ChangedStance,
    Contradicted,
}

impl ClassificationRun {
    #[must_use]
    pub fn new(
        observation_id: i64,
        classifier_version: impl Into<String>,
        schema_version: i64,
        input_hash: impl Into<String>,
    ) -> Self {
        let classifier_version = classifier_version.into();
        let input_hash = input_hash.into();
        let request_key = request_key(
            observation_id,
            &classifier_version,
            schema_version,
            &input_hash,
        );
        Self {
            observation_id,
            classifier_version,
            schema_version,
            input_hash,
            request_key,
        }
    }
}

#[must_use]
pub fn input_hash(input: &str) -> String {
    digest([input.as_bytes()])
}

#[must_use]
pub fn request_key(
    observation_id: i64,
    classifier_version: &str,
    schema_version: i64,
    input_hash: &str,
) -> String {
    digest([
        &observation_id.to_be_bytes(),
        classifier_version.as_bytes(),
        &schema_version.to_be_bytes(),
        input_hash.as_bytes(),
    ])
}

fn digest<const N: usize>(fields: [&[u8]; N]) -> String {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    }
    format!("{:x}", hasher.finalize())
}

pub trait ObservationStore {
    fn insert_observation(&mut self, input: NewObservation) -> Result<i64, PortError>;
    fn finalize_observation_outcome(
        &mut self,
        observation_id: i64,
        outcome: ObservationOutcome,
        now: i64,
    ) -> Result<(), PortError>;
    fn claim_next_observation(
        &mut self,
        owner: &str,
        now: i64,
        lease_seconds: i64,
    ) -> Result<Option<ObservationLease>, PortError>;
    fn defer_observation(
        &mut self,
        lease: &ObservationLease,
        error: &str,
        now: i64,
    ) -> Result<(), PortError>;
    fn begin_classification_run(
        &mut self,
        lease: &ObservationLease,
        run: &ClassificationRun,
        now: i64,
    ) -> Result<i64, PortError>;
    fn persist_candidate(
        &mut self,
        candidate: PersistedCandidate,
        now: i64,
    ) -> Result<i64, PortError>;
    /// Records the terminal result of a run. `candidate_count` is the number
    /// of durable candidate rows (including deterministic rejections).
    fn finish_classification_run(
        &mut self,
        lease: &ObservationLease,
        classification_run_id: i64,
        outcome: ClassificationOutcome,
        candidate_count: i64,
        reason: Option<&str>,
        now: i64,
    ) -> Result<(), PortError>;
    /// Marks every still-pending proposal in a run terminal before a retry or
    /// deterministic rejection.  Reasons are bounded diagnostics, never user
    /// source content.
    fn reject_pending_candidates(
        &mut self,
        lease: &ObservationLease,
        classification_run_id: i64,
        reason: &str,
        now: i64,
    ) -> Result<i64, PortError>;
    /// Releases a current lease for a bounded retry, or defers it when the
    /// retry limit is reached. Errors are diagnostics, never source content.
    fn retry_or_defer_observation(
        &mut self,
        lease: &ObservationLease,
        error: &str,
        now: i64,
        retry_limit: i64,
        retry_after_seconds: i64,
    ) -> Result<(), PortError>;
    /// On worker startup pending reply outcomes are no longer live turns.
    fn recover_interrupted_observations(&mut self, now: i64) -> Result<usize, PortError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_key_is_stable_and_input_bound() {
        let first = ClassificationRun::new(7, "v1", 10, "abc");
        assert_eq!(
            first.request_key,
            ClassificationRun::new(7, "v1", 10, "abc").request_key
        );
        assert_ne!(
            first.request_key,
            ClassificationRun::new(7, "v1", 10, "changed").request_key
        );
    }
}
