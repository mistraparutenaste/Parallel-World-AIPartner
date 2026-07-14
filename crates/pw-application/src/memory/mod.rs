//! Long-term memory ports and non-blocking summary work boundaries.

mod context;
mod lifecycle;

pub use context::{
    DEFAULT_MEMORY_LIMIT, JapanesePersistentFactGenerator, MemoryContext, MemoryRecord,
    MemoryStore, PersistentFactGenerator, RollingSummaryGenerator, StoredSummary, SummaryGenerator,
    SummaryWorker, is_safe_persistent_content, redact_persistent_content,
};
pub use lifecycle::{
    DORMANT_DELETE_AFTER_SECONDS, EvidenceKind, MemoryAction, MemoryCandidate, MemoryEvidence,
    MemoryState, memory_strength, prompt_rank, should_become_dormant,
};
