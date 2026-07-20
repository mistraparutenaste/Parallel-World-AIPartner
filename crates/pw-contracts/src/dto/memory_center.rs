use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Privacy-safe snapshot rendered by the Memory Center.  It deliberately
/// contains bounded derived previews, never source observations or transcripts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "MemoryCenterDto.ts")]
pub struct MemoryCenterDto {
    pub schema_version: u16,
    pub domains: Vec<MemoryDomainControlDto>,
    pub pending: Vec<PendingMemoryCandidateDto>,
    pub commitments: Vec<CommitmentSummaryDto>,
    pub dialogue: Option<DialogueSummaryDto>,
    pub temporary: bool,
    #[ts(type = "number")]
    pub temporary_revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "MemoryDomainControlDto.ts")]
pub struct MemoryDomainControlDto {
    pub domain: String,
    pub consent: String,
    #[ts(type = "number | null")]
    pub retention_seconds: Option<i64>,
    #[ts(type = "number")]
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "PendingMemoryCandidateDto.ts")]
pub struct PendingMemoryCandidateDto {
    #[ts(type = "number")]
    pub id: i64,
    pub domain: String,
    /// Redacted and character-bounded. Never use this DTO for source text.
    pub preview: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "CommitmentSummaryDto.ts")]
pub struct CommitmentSummaryDto {
    #[ts(type = "number")]
    pub id: i64,
    pub status: String,
    #[ts(type = "number | null")]
    pub due_at: Option<i64>,
    #[ts(type = "number")]
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "DialogueSummaryDto.ts")]
pub struct DialogueSummaryDto {
    pub mood: Option<String>,
    pub relationship_summary: Option<String>,
    #[ts(type = "number | null")]
    pub relationship_score: Option<i64>,
    #[ts(type = "number")]
    pub expires_at: i64,
    #[ts(type = "number")]
    pub revision: i64,
}
