//! Bounded, policy-gated companion state contracts.
//!
//! These records intentionally do not carry a transcript.  Durable text stays
//! in the observation ledger; this module only describes user-approved state
//! derived from it.

use std::sync::mpsc::{SyncSender, TrySendError};

use crate::PortError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemoryDomain {
    Working,
    Episode,
    SemanticUser,
    Relationship,
    AiSelf,
    Procedural,
    Commitment,
    Reflection,
}

impl MemoryDomain {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Episode => "episode",
            Self::SemanticUser => "semantic_user",
            Self::Relationship => "relationship",
            Self::AiSelf => "ai_self",
            Self::Procedural => "procedural",
            Self::Commitment => "commitment",
            Self::Reflection => "reflection",
        }
    }

    pub fn parse(value: &str) -> Result<Self, PortError> {
        match value {
            "working" => Ok(Self::Working),
            "episode" => Ok(Self::Episode),
            "semantic_user" => Ok(Self::SemanticUser),
            "relationship" => Ok(Self::Relationship),
            "ai_self" => Ok(Self::AiSelf),
            "procedural" => Ok(Self::Procedural),
            "commitment" => Ok(Self::Commitment),
            "reflection" => Ok(Self::Reflection),
            _ => Err(PortError("unknown memory domain".into())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainConsent {
    Allowed,
    PendingApproval,
    NeverStore,
}

impl DomainConsent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::PendingApproval => "pending_approval",
            Self::NeverStore => "never_store",
        }
    }

    pub fn parse(value: &str) -> Result<Self, PortError> {
        match value {
            "allowed" => Ok(Self::Allowed),
            "pending_approval" => Ok(Self::PendingApproval),
            "never_store" => Ok(Self::NeverStore),
            _ => Err(PortError("unknown domain consent".into())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainControl {
    pub domain: MemoryDomain,
    pub consent: DomainConsent,
    /// `None` means the lifecycle policy owns retention; zero is invalid.
    pub retention_seconds: Option<i64>,
    pub revision: i64,
}

impl DomainControl {
    pub fn validate(&self) -> Result<(), PortError> {
        if self.revision < 0 || self.retention_seconds.is_some_and(|value| value <= 0) {
            return Err(PortError("invalid domain control".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryWriteClass {
    NormalExplicit,
    Inferred,
    Personal,
    Sensitive,
    Secret,
    NeverStore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryWriteDisposition {
    AutoApproved,
    PendingApproval,
    Rejected,
}

#[must_use]
pub const fn memory_write_disposition(
    class: MemoryWriteClass,
    temporary_conversation: bool,
    control: DomainConsent,
) -> MemoryWriteDisposition {
    if temporary_conversation
        || matches!(
            class,
            MemoryWriteClass::Secret | MemoryWriteClass::NeverStore
        )
        || matches!(control, DomainConsent::NeverStore)
    {
        return MemoryWriteDisposition::Rejected;
    }
    if matches!(control, DomainConsent::PendingApproval)
        || matches!(
            class,
            MemoryWriteClass::Inferred | MemoryWriteClass::Personal | MemoryWriteClass::Sensitive
        )
    {
        return MemoryWriteDisposition::PendingApproval;
    }
    MemoryWriteDisposition::AutoApproved
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryVersion {
    pub memory_id: i64,
    pub revision: i64,
    pub content_hash: String,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryLinkRelation {
    Supports,
    Supersedes,
    Contradicts,
    DerivedFrom,
}

impl MemoryLinkRelation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supports => "supports",
            Self::Supersedes => "supersedes",
            Self::Contradicts => "contradicts",
            Self::DerivedFrom => "derived_from",
        }
    }

    pub fn parse(value: &str) -> Result<Self, PortError> {
        match value {
            "supports" => Ok(Self::Supports),
            "supersedes" => Ok(Self::Supersedes),
            "contradicts" => Ok(Self::Contradicts),
            "derived_from" => Ok(Self::DerivedFrom),
            _ => Err(PortError("unknown memory link relation".into())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryLink {
    pub from_memory_id: i64,
    pub to_memory_id: i64,
    pub relation: MemoryLinkRelation,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryTombstone {
    pub memory_id: i64,
    pub generation: i64,
    pub deleted_at: i64,
    pub final_support_removed: bool,
    pub pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitmentStatus {
    Open,
    Completed,
    Cancelled,
    Expired,
}

impl CommitmentStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    pub fn parse(value: &str) -> Result<Self, PortError> {
        match value {
            "open" => Ok(Self::Open),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "expired" => Ok(Self::Expired),
            _ => Err(PortError("unknown commitment status".into())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commitment {
    pub id: Option<i64>,
    pub conversation_id: String,
    pub content: String,
    pub status: CommitmentStatus,
    pub due_at: Option<i64>,
    pub next_check_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporaryConversationSettings {
    pub conversation_id: String,
    pub temporary: bool,
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogueState {
    pub conversation_id: String,
    pub mood: Option<String>,
    pub relationship_summary: Option<String>,
    pub relationship_score: Option<i64>,
    pub reaction: Option<String>,
    pub reflection_cursor: Option<String>,
    pub reflection_state: Option<String>,
    pub expires_at: i64,
    pub revision: i64,
}

/// Bounded signals derived from one completed turn.  This record intentionally
/// carries no user or assistant transcript; only a small deterministic delta is
/// handed to the asynchronous companion-state writer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogueSignals {
    pub conversation_id: String,
    pub mood: Option<String>,
    pub reaction: Option<String>,
    pub relationship_delta: i64,
    pub reflection_cursor: Option<String>,
    pub reflection_state: Option<String>,
    pub commitment: Option<Commitment>,
    pub observed_at: i64,
    pub expires_at: i64,
}

impl DialogueSignals {
    pub fn validate(&self) -> Result<(), PortError> {
        if self.conversation_id.trim().is_empty()
            || self.conversation_id.chars().count() > 96
            || self.conversation_id.contains(char::is_control)
            || self.observed_at < 0
            || self.expires_at <= self.observed_at
            || !(-10..=10).contains(&self.relationship_delta)
            || [
                self.mood.as_deref(),
                self.reaction.as_deref(),
                self.reflection_cursor.as_deref(),
                self.reflection_state.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|value| value.trim().is_empty() || value.chars().count() > 96)
        {
            return Err(PortError("invalid bounded dialogue signals".into()));
        }
        if let Some(commitment) = &self.commitment {
            if commitment.conversation_id != self.conversation_id
                || commitment.content.chars().count() > 160
            {
                return Err(PortError("invalid bounded commitment signal".into()));
            }
        }
        Ok(())
    }
}

impl DialogueState {
    pub fn validate(&self) -> Result<(), PortError> {
        if self.conversation_id.trim().is_empty()
            || self.expires_at <= 0
            || self.revision < 0
            || self
                .relationship_score
                .is_some_and(|score| !(-100..=100).contains(&score))
            || [
                self.mood.as_deref(),
                self.relationship_summary.as_deref(),
                self.reaction.as_deref(),
                self.reflection_cursor.as_deref(),
                self.reflection_state.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|value| value.len() > 1_024)
        {
            return Err(PortError("invalid bounded dialogue state".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CasOutcome {
    Applied(i64),
    Conflict,
    Rejected,
}

pub trait CompanionStateStore {
    fn get_domain_control(&self, domain: MemoryDomain) -> Result<DomainControl, PortError>;
    fn compare_and_set_domain_control(
        &mut self,
        control: DomainControl,
        expected_revision: i64,
        now: i64,
    ) -> Result<CasOutcome, PortError>;
    fn set_temporary_conversation(
        &mut self,
        settings: TemporaryConversationSettings,
        expected_revision: i64,
        now: i64,
    ) -> Result<CasOutcome, PortError>;
    fn is_temporary_conversation(&self, conversation_id: &str) -> Result<bool, PortError>;
    fn save_commitment(
        &mut self,
        commitment: Commitment,
        expected_revision: Option<i64>,
        now: i64,
    ) -> Result<CasOutcome, PortError>;
    /// Loads a bounded set of open commitments for planned/proactive context.
    /// Implementations may return an opaque error; callers fail closed.
    fn list_open_commitments(
        &self,
        conversation_id: &str,
        now: i64,
        limit: usize,
    ) -> Result<Vec<Commitment>, PortError> {
        let _ = (conversation_id, now, limit);
        Err(PortError("open commitment listing unavailable".into()))
    }
    fn expire_commitments(&mut self, now: i64) -> Result<usize, PortError>;
    fn compare_and_set_dialogue_state(
        &mut self,
        state: DialogueState,
        expected_revision: i64,
        now: i64,
    ) -> Result<CasOutcome, PortError>;
    fn read_dialogue_state(
        &self,
        conversation_id: &str,
        now: i64,
    ) -> Result<Option<DialogueState>, PortError>;
    fn record_memory_version(&mut self, version: MemoryVersion) -> Result<(), PortError>;
    fn link_memories(&mut self, link: MemoryLink) -> Result<(), PortError>;
    fn tombstone_memory(
        &mut self,
        tombstone: MemoryTombstone,
        expected_generation: i64,
    ) -> Result<CasOutcome, PortError>;
}

/// A bounded write request.  The UI/turn path is allowed to drop it: it must
/// never wait for SQLite or make a reply fail because background state failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncStateWrite {
    DomainControl(DomainControl),
    TemporaryConversation(TemporaryConversationSettings),
    Commitment(Commitment),
    DialogueState(DialogueState),
    DialogueSignals(DialogueSignals),
}

pub trait AsyncStateWriter {
    fn try_enqueue(&self, write: AsyncStateWrite) -> bool;
}

impl AsyncStateWriter for SyncSender<AsyncStateWrite> {
    fn try_enqueue(&self, write: AsyncStateWrite) -> bool {
        match self.try_send(write) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                // Full and disconnected are intentionally fail-open.  The next
                // turn may reconstruct bounded state from durable memory.
                true
            }
        }
    }
}

/// Centralized helper so callers do not accidentally turn a best-effort
/// companion update into a response-path error.
#[must_use]
pub fn enqueue_state_fail_open(writer: &dyn AsyncStateWriter, write: AsyncStateWrite) -> bool {
    writer.try_enqueue(write)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::sync_channel;

    use super::*;

    #[test]
    fn normal_facts_auto_approve_and_sensitive_or_secret_writes_do_not() {
        assert_eq!(
            memory_write_disposition(
                MemoryWriteClass::NormalExplicit,
                false,
                DomainConsent::Allowed
            ),
            MemoryWriteDisposition::AutoApproved
        );
        for class in [
            MemoryWriteClass::Inferred,
            MemoryWriteClass::Personal,
            MemoryWriteClass::Sensitive,
        ] {
            assert_eq!(
                memory_write_disposition(class, false, DomainConsent::Allowed),
                MemoryWriteDisposition::PendingApproval
            );
        }
        for class in [MemoryWriteClass::Secret, MemoryWriteClass::NeverStore] {
            assert_eq!(
                memory_write_disposition(class, false, DomainConsent::Allowed),
                MemoryWriteDisposition::Rejected
            );
        }
    }

    #[test]
    fn temporary_or_never_store_conversations_reject_durable_writes() {
        assert_eq!(
            memory_write_disposition(
                MemoryWriteClass::NormalExplicit,
                true,
                DomainConsent::Allowed
            ),
            MemoryWriteDisposition::Rejected
        );
        assert_eq!(
            memory_write_disposition(
                MemoryWriteClass::NormalExplicit,
                false,
                DomainConsent::NeverStore
            ),
            MemoryWriteDisposition::Rejected
        );
    }

    #[test]
    fn bounded_writer_never_propagates_queue_or_worker_failure() {
        let (sender, receiver) = sync_channel(1);
        let write = AsyncStateWrite::TemporaryConversation(TemporaryConversationSettings {
            conversation_id: "chat".into(),
            temporary: true,
            revision: 0,
        });
        assert!(enqueue_state_fail_open(&sender, write.clone()));
        assert!(enqueue_state_fail_open(&sender, write.clone()));
        drop(receiver);
        assert!(enqueue_state_fail_open(&sender, write));
    }
}
