//! Long-term memory ports and non-blocking summary work boundaries.

mod consolidation;
mod context;
mod lifecycle;

pub use consolidation::{
    HybridConsolidator, LlmMemoryClassifier, MemoryClassifier, ProposedAction,
    has_explicit_pin_intent,
};
pub use context::{
    DEFAULT_MEMORY_LIMIT, EvidenceSource, JapanesePersistentFactGenerator, MaintenanceReport,
    MemoryContext, MemoryRecord, MemoryStore, PersistentFactGenerator, RollingSummaryGenerator,
    StoredSummary, SummaryGenerator, SummaryWorker, is_safe_persistent_content,
    redact_persistent_content,
};
pub use lifecycle::{
    DORMANT_DELETE_AFTER_SECONDS, EvidenceKind, MemoryAction, MemoryCandidate, MemoryEvidence,
    MemoryState, memory_strength, prompt_rank, should_become_dormant,
};
