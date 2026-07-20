//! Long-term memory ports and non-blocking summary work boundaries.

mod consolidation;
mod context;
pub mod epistemic;
mod lifecycle;
mod observation;
mod promotion;
mod validator;

pub use consolidation::{
    HybridConsolidator, LlmMemoryClassifier, MemoryClassifier, ProposedAction,
    has_explicit_pin_intent,
};
pub use context::{
    DEFAULT_MEMORY_LIMIT, EvidenceSource, JapanesePersistentFactGenerator, MaintenanceReport,
    MemoryContext, MemoryRecord, MemoryStore, PersistentFactGenerator, RollingSummaryGenerator,
    StoredSummary, SummaryEntry, SummaryGenerator, SummaryWorker, is_role_preserving_summary,
    is_safe_persistent_content, merge_rolling_summaries, redact_persistent_content,
};
pub use epistemic::{
    Attribution, Conditionality, DiscourseFeatures, EpistemicForm, Fictionality, MemoryAtom,
    Polarity, SourceMode, SourceSpan, SpeechAct, SubjectScope, TemporalScope, VerificationStatus,
};
pub use lifecycle::{
    DORMANT_DELETE_AFTER_SECONDS, EvidenceKind, MemoryAction, MemoryCandidate, MemoryEvidence,
    MemoryState, memory_strength, prompt_rank, should_become_dormant,
};
pub use observation::{
    CandidateOperation, CandidateProvenanceRelation, ClassificationOutcome, ClassificationRun,
    NewObservation, ObservationLease, ObservationOutcome, ObservationStore,
    PersistCandidateOutcome, PersistedCandidate, ProcessingState, input_hash, request_key,
};
pub use promotion::{
    MemoryPromoter, PromotionResult, ProvenanceLink, ProvisionalMemoryChangeSet,
    VersionedMemoryAction,
};
pub use validator::{
    CandidateRelation, NormalizationEdit, TypedCandidate, ValidationError, validate_candidate,
    validate_candidate_for_source,
};
