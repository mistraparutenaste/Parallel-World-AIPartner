use super::{MemoryAction, ObservationLease};
use crate::PortError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedMemoryAction {
    pub action: MemoryAction,
    pub expected_revision: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceLink {
    pub candidate_id: i64,
    pub relation: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvisionalMemoryChangeSet {
    pub request_key: String,
    pub lease: ObservationLease,
    /// Immutable classifier identity is part of promotion idempotency.  A
    /// request key alone is insufficient after a worker restart or model swap.
    pub classification_run_id: i64,
    pub classifier_version: String,
    pub schema_version: i64,
    pub input_hash: String,
    pub actions: Vec<VersionedMemoryAction>,
    pub provenance: Vec<ProvenanceLink>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionResult {
    pub request_key: String,
    pub promoted_memory_ids: Vec<i64>,
    pub already_applied: bool,
}

#[allow(clippy::missing_errors_doc)]
pub trait MemoryPromoter {
    fn promote(
        &mut self,
        change_set: &ProvisionalMemoryChangeSet,
        now: i64,
    ) -> Result<PromotionResult, PortError>;
}
