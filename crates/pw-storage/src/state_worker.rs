//! Bounded asynchronous companion-state writer.
//!
//! The conversation path only performs a non-blocking `try_send`.  SQLite
//! contention, malformed signals, and worker failures are deliberately
//! isolated from the reply/TTS path.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};

use pw_application::PortError;
use pw_application::conversation::{
    BoundedStateContext, PlannedStateContextProvider, ResponsePlan,
};
use pw_application::memory::{
    AsyncStateWrite, CasOutcome, CompanionStateStore, DialogueSignals, DialogueState,
    DomainConsent, MemoryDomain,
};

use crate::{Database, SqliteCompanionStateStore};

pub const DEFAULT_STATE_QUEUE_CAPACITY: usize = 32;

/// Read-only adapter used by the planned response retriever.  It exposes
/// bounded dialogue metadata and open commitments; no transcript is queried.
pub struct SqlitePlannedStateContext {
    store: SqliteCompanionStateStore,
    conversation_id: String,
}

impl SqlitePlannedStateContext {
    #[must_use]
    pub fn new(database: Database, conversation_id: impl Into<String>) -> Self {
        Self {
            store: SqliteCompanionStateStore::new(database),
            conversation_id: conversation_id.into(),
        }
    }
}

impl PlannedStateContextProvider for SqlitePlannedStateContext {
    fn retrieve_state(&mut self, _plan: &ResponsePlan) -> Result<BoundedStateContext, PortError> {
        let now = unix_timestamp();
        let relationship_allowed = matches!(
            self.store
                .get_domain_control(MemoryDomain::Relationship)?
                .consent,
            DomainConsent::Allowed
        );
        let reflection_allowed = matches!(
            self.store
                .get_domain_control(MemoryDomain::Reflection)?
                .consent,
            DomainConsent::Allowed
        );
        let commitment_allowed = matches!(
            self.store
                .get_domain_control(MemoryDomain::Commitment)?
                .consent,
            DomainConsent::Allowed
        );
        let dialogue = if relationship_allowed || reflection_allowed {
            self.store.read_dialogue_state(&self.conversation_id, now)?
        } else {
            None
        };
        let commitments = if commitment_allowed {
            self.store
                .list_open_commitments(&self.conversation_id, now, 4)?
                .into_iter()
                .map(|commitment| commitment.content)
                .collect()
        } else {
            Vec::new()
        };
        let context = BoundedStateContext {
            mood: relationship_allowed
                .then(|| dialogue.as_ref().and_then(|state| state.mood.clone()))
                .flatten(),
            reaction: relationship_allowed
                .then(|| dialogue.as_ref().and_then(|state| state.reaction.clone()))
                .flatten(),
            relationship_score: relationship_allowed
                .then(|| dialogue.as_ref().and_then(|state| state.relationship_score))
                .flatten(),
            reflection_cursor: dialogue
                .as_ref()
                .filter(|_| reflection_allowed)
                .and_then(|state| state.reflection_cursor.clone()),
            open_commitments: commitments,
        };
        context.validate()?;
        Ok(context)
    }
}

/// Owns a bounded queue and one long-lived SQLite writer.
pub struct CompanionStateWorker {
    tx: SyncSender<AsyncStateWrite>,
    thread: Option<JoinHandle<()>>,
}

impl CompanionStateWorker {
    /// Starts a worker.  The database is opened on the worker thread so a
    /// startup failure cannot block a chat turn.
    pub fn start(path: impl Into<PathBuf>, capacity: usize) -> Result<Self, String> {
        let capacity = capacity.clamp(1, 256);
        let (tx, rx) = sync_channel(capacity);
        let path = path.into();
        let thread = thread::Builder::new()
            .name("pw-companion-state".into())
            .spawn(move || run_worker(path, rx))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            tx,
            thread: Some(thread),
        })
    }

    #[must_use]
    pub fn sender(&self) -> SyncSender<AsyncStateWrite> {
        self.tx.clone()
    }

    /// Enqueues without waiting.  Full/disconnected queues are fail-open.
    pub fn try_enqueue(&self, write: AsyncStateWrite) -> bool {
        match self.tx.try_send(write) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => true,
        }
    }

    pub fn shutdown(mut self) -> Result<(), String> {
        drop(self.tx);
        self.thread
            .take()
            .expect("worker thread exists")
            .join()
            .map_err(|_| "companion state worker panicked".to_owned())
    }
}

fn run_worker(path: PathBuf, rx: Receiver<AsyncStateWrite>) {
    let Ok(database) = Database::open(&path) else {
        tracing::warn!(path = %path.display(), "companion state worker database unavailable");
        return;
    };
    let mut store = SqliteCompanionStateStore::new(database);
    while let Ok(write) = rx.recv() {
        if let Err(error) = apply_async_state_write(&mut store, write, unix_timestamp()) {
            tracing::warn!(%error, "companion state update dropped; conversation remains available");
        }
    }
}

/// Applies one queue item.  Kept public for deterministic focused tests.
pub fn apply_async_state_write(
    store: &mut impl CompanionStateStore,
    write: AsyncStateWrite,
    now: i64,
) -> Result<(), PortError> {
    match write {
        AsyncStateWrite::DialogueSignals(signals) => apply_signals(store, signals, now),
        AsyncStateWrite::DomainControl(control) => {
            control.validate()?;
            match store.compare_and_set_domain_control(control, 0, now)? {
                CasOutcome::Applied(_) | CasOutcome::Conflict | CasOutcome::Rejected => Ok(()),
            }
        }
        AsyncStateWrite::TemporaryConversation(settings) => {
            match store.set_temporary_conversation(settings, 0, now)? {
                CasOutcome::Applied(_) | CasOutcome::Conflict | CasOutcome::Rejected => Ok(()),
            }
        }
        AsyncStateWrite::Commitment(commitment) => {
            if store.is_temporary_conversation(&commitment.conversation_id)? {
                return Ok(());
            }
            let consent = store.get_domain_control(MemoryDomain::Commitment)?.consent;
            if !matches!(consent, DomainConsent::Allowed) {
                return Ok(());
            }
            let _ = store.save_commitment(commitment, None, now)?;
            store.expire_commitments(now)?;
            Ok(())
        }
        AsyncStateWrite::DialogueState(state) => {
            if store.is_temporary_conversation(&state.conversation_id)? {
                return Ok(());
            }
            let relationship = store
                .get_domain_control(MemoryDomain::Relationship)?
                .consent;
            if !matches!(relationship, DomainConsent::Allowed) {
                return Ok(());
            }
            let expected = state.revision;
            let _ = store.compare_and_set_dialogue_state(state, expected, now)?;
            Ok(())
        }
    }
}

fn apply_signals(
    store: &mut impl CompanionStateStore,
    signals: DialogueSignals,
    now: i64,
) -> Result<(), PortError> {
    signals.validate()?;
    if store.is_temporary_conversation(&signals.conversation_id)? {
        return Ok(());
    }
    // Inferred relationship/reflection updates are only written after the
    // user enables their domains.  Commitment candidates are explicit and
    // follow their own control.
    let relationship_allowed = matches!(
        store
            .get_domain_control(MemoryDomain::Relationship)?
            .consent,
        DomainConsent::Allowed
    );
    let reflection_allowed = matches!(
        store.get_domain_control(MemoryDomain::Reflection)?.consent,
        DomainConsent::Allowed
    );
    if relationship_allowed || reflection_allowed {
        let existing = store
            .read_dialogue_state(&signals.conversation_id, now)?
            .unwrap_or_else(|| DialogueState {
                conversation_id: signals.conversation_id.clone(),
                mood: None,
                relationship_summary: None,
                relationship_score: None,
                reaction: None,
                reflection_cursor: None,
                reflection_state: None,
                expires_at: signals.expires_at,
                revision: 0,
            });
        let state = DialogueState {
            conversation_id: signals.conversation_id.clone(),
            mood: if relationship_allowed {
                signals.mood.clone().or(existing.mood)
            } else {
                existing.mood
            },
            relationship_summary: existing.relationship_summary,
            relationship_score: if relationship_allowed {
                Some(
                    existing
                        .relationship_score
                        .unwrap_or(0)
                        .saturating_add(signals.relationship_delta)
                        .clamp(-100, 100),
                )
            } else {
                existing.relationship_score
            },
            reaction: if relationship_allowed {
                signals.reaction.clone().or(existing.reaction)
            } else {
                existing.reaction
            },
            reflection_cursor: if reflection_allowed {
                signals
                    .reflection_cursor
                    .clone()
                    .or(existing.reflection_cursor)
            } else {
                existing.reflection_cursor
            },
            reflection_state: if reflection_allowed {
                signals
                    .reflection_state
                    .clone()
                    .or(existing.reflection_state)
            } else {
                existing.reflection_state
            },
            expires_at: signals.expires_at.max(existing.expires_at),
            revision: existing.revision,
        };
        let _ = store.compare_and_set_dialogue_state(state, existing.revision, now)?;
    }
    if let Some(commitment) = signals.commitment {
        if matches!(
            store.get_domain_control(MemoryDomain::Commitment)?.consent,
            DomainConsent::Allowed
        ) {
            let _ = store.save_commitment(commitment, None, now)?;
        }
    }
    store.expire_commitments(now)?;
    Ok(())
}

fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use pw_application::memory::{
        AsyncStateWrite, CasOutcome, CompanionStateStore, DomainConsent, DomainControl,
        MemoryDomain, TemporaryConversationSettings,
    };

    use super::*;

    fn store() -> SqliteCompanionStateStore {
        SqliteCompanionStateStore::new(Database::open_in_memory().unwrap())
    }

    #[test]
    fn signals_write_only_after_explicit_domain_consent() {
        let mut store = store();
        let signals = pw_application::memory::derive_dialogue_signals(
            "chat",
            "ありがとう、次は確認します",
            "了解",
            10,
        )
        .unwrap();
        apply_async_state_write(&mut store, AsyncStateWrite::DialogueSignals(signals), 10).unwrap();
        assert!(store.read_dialogue_state("chat", 10).unwrap().is_none());
        for domain in [
            MemoryDomain::Relationship,
            MemoryDomain::Reflection,
            MemoryDomain::Commitment,
        ] {
            let current = store.get_domain_control(domain).unwrap();
            assert_eq!(
                store
                    .compare_and_set_domain_control(
                        DomainControl {
                            consent: DomainConsent::Allowed,
                            ..current
                        },
                        0,
                        11
                    )
                    .unwrap(),
                CasOutcome::Applied(1)
            );
        }
        let mut signals = pw_application::memory::derive_dialogue_signals(
            "chat",
            "ありがとう、次は確認します",
            "了解",
            20,
        )
        .unwrap();
        signals.commitment = Some(pw_application::memory::Commitment {
            id: None,
            conversation_id: "chat".into(),
            content: "promise: verify document".into(),
            status: pw_application::memory::CommitmentStatus::Open,
            due_at: None,
            next_check_at: None,
            expires_at: Some(86_420),
            revision: 0,
        });
        apply_async_state_write(&mut store, AsyncStateWrite::DialogueSignals(signals), 20).unwrap();
        assert!(store.read_dialogue_state("chat", 20).unwrap().is_some());
        assert_eq!(store.list_open_commitments("chat", 20, 4).unwrap().len(), 1);
    }

    #[test]
    fn temporary_mode_rejects_signals_without_error() {
        let mut store = store();
        assert_eq!(
            store
                .set_temporary_conversation(
                    TemporaryConversationSettings {
                        conversation_id: "chat".into(),
                        temporary: true,
                        revision: 0
                    },
                    0,
                    1
                )
                .unwrap(),
            CasOutcome::Applied(1)
        );
        let signals =
            pw_application::memory::derive_dialogue_signals("chat", "約束します", "ok", 10)
                .unwrap();
        apply_async_state_write(&mut store, AsyncStateWrite::DialogueSignals(signals), 10).unwrap();
        assert!(store.read_dialogue_state("chat", 10).unwrap().is_none());
        assert!(
            store
                .list_open_commitments("chat", 10, 4)
                .unwrap()
                .is_empty()
        );
    }
}
