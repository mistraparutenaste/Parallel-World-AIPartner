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
    pub actions: Vec<VersionedMemoryAction>,
    pub provenance: Vec<ProvenanceLink>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionResult {
    pub request_key: String,
    pub promoted_memory_ids: Vec<i64>,
    pub already_applied: bool,
}

pub trait MemoryPromoter {
    fn promote(
        &mut self,
        change_set: &ProvisionalMemoryChangeSet,
        now: i64,
    ) -> Result<PromotionResult, PortError>;
}
