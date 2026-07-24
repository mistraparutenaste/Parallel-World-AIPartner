//! Conversation worker: owns the orchestrator, emits UI events and
//! maps control JSON onto the character.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use pw_application::PortError;
use pw_application::behavior::proactive::InteractionGate;
use pw_application::conversation::{
    ChatMessage, ChatRole, ConversationEvents, ConversationOrchestrator, ExistingContextRetriever,
    FixedSurfaceRealizer, LexicalResponsePlanner, LlmClient, OrchestratorConfig, PlanningBudget,
    PromptBuilder, StateAwareRetriever, response_pipeline,
};
use pw_application::history::{ConversationHistory, MessageRole, StoredTurn};
use pw_application::memory::{
    AsyncStateWrite, CandidateOperation, CandidateProvenanceRelation, ClassificationOutcome,
    ClassificationRun, DEFAULT_MEMORY_LIMIT, DiscourseFeatures, EpistemicForm, Fictionality,
    HybridConsolidator, LlmMemoryClassifier, MemoryAtom, MemoryClassifier, MemoryContext,
    MemoryStore, NewObservation, ObservationOutcome, ObservationStore, PersistCandidateOutcome,
    PersistedCandidate, Polarity, ProposedAction, ProvenanceLink, ProvisionalMemoryChangeSet,
    RollingSummaryGenerator, SourceMode, SourceSpan, SpeechAct, SubjectScope, SummaryGenerator,
    TemporalScope, VerificationStatus, VersionedMemoryAction, derive_dialogue_signals,
    is_role_preserving_summary, is_safe_persistent_content, merge_rolling_summaries,
    redact_persistent_content,
};
#[cfg(test)]
use pw_application::memory::{EvidenceSource, has_explicit_pin_intent};
use pw_application::recovery::{
    FeatureHealthSupervisor, HealthTransition, SystemClock, TimeJitter,
};
use pw_contracts::{
    ChatMessageEventDto, ChatRoleDto, ConversationStateEventDto, LlmSettingsDto,
    RUNTIME_HEALTH_EVENT, RuntimeHealthEventDto, SCHEMA_VERSION,
};
use pw_domain::conversation::ConversationState;
use pw_domain::reply::{ReplyControl, ReplyEvent, ReplyParser, TurnId, strip_emoji};
use pw_domain::runtime_health::{FailureCode, RuntimeFailure, RuntimeFeature};
use pw_llm::{LlmClientConfig, OpenAiCompatClient};
use pw_platform::paths::AppDataLayout;
use pw_storage::{
    CompanionStateWorker, DEFAULT_STATE_QUEUE_CAPACITY, Database, SqliteConversationHistory,
    SqliteMemoryStore, SqlitePlannedStateContext,
};
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime};

use crate::character::CharacterCapabilities;
use crate::commands::character::{
    CharacterSnapshot, CharacterState, EXPRESSION_EVENT, MOTION_EVENT, resolve_character_manifest,
};
use crate::diagnostics::QueueMetrics;

pub const MESSAGE_EVENT: &str = "chat-message";
pub const STATE_EVENT: &str = "conversation-state";
const CHARACTER_WEBVIEW: &str = "character";

/// Kept messages of context (user + assistant combined).
const MAX_HISTORY_MESSAGES: usize = 20;
const DEFAULT_CONVERSATION_ID: &str = "default";
const SUBMIT_QUEUE_CAPACITY: usize = 8;
const CONVERSATION_QUEUE_CAPACITY: usize = 8;
const ENRICHMENT_QUEUE_CAPACITY: usize = 1;
const ENRICHMENT_PENDING_CAPACITY: usize = 64;
const ADAPTER_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(2);
#[allow(clippy::duration_suboptimal_units)]
const MEMORY_MAINTENANCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(86_400);
const MEMORY_MAINTENANCE_FOLLOWUP: std::time::Duration = std::time::Duration::from_millis(100);

struct UnavailableMemoryClassifier;

impl MemoryClassifier for UnavailableMemoryClassifier {
    fn classify(
        &mut self,
        _: &str,
        _: &[pw_application::memory::MemoryCandidate],
    ) -> Result<ProposedAction, PortError> {
        Err(PortError("memory classifier unavailable".into()))
    }
}

fn load_memory_context<M: MemoryStore>(
    memory: &mut M,
    query: &str,
    _turn_id: u64,
    now: i64,
) -> MemoryContext {
    let candidates = memory
        .search_active_for_prompt(query, DEFAULT_MEMORY_LIMIT, now)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "memory search failed; continuing without long-term memory");
            Vec::new()
        });
    let summary = memory
        .load_summary(DEFAULT_CONVERSATION_ID)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "summary restore failed; continuing without summary");
            None
        });
    let summary = summary.and_then(|item| {
        if is_role_preserving_summary(&item.content) {
            Some(item.content)
        } else {
            tracing::warn!(
                through_message_id = item.through_message_id,
                "legacy untyped summary excluded from prompt; stored history will rebuild it"
            );
            None
        }
    });
    MemoryContext {
        user_settings: None,
        memories: candidates.iter().map(|item| item.content.clone()).collect(),
        summary,
    }
    .bounded()
}

const SUMMARY_RECENT_MESSAGES: usize = MAX_HISTORY_MESSAGES;
const SUMMARY_BATCH_MESSAGES: usize = 8;
const SUMMARY_DRAIN_BATCHES_PER_PASS: usize = 4;
const SUMMARY_MAX_CHARS: usize = 2_000;
const SUMMARY_DRAIN_TIME_BUDGET: std::time::Duration = std::time::Duration::from_millis(50);
const ENRICHMENT_IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(20);
const ENRICHMENT_FOLLOWUP_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
const ENRICHMENT_JOBS_PER_SLICE: usize = 4;
const OBSERVATION_WRITE_QUEUE_CAPACITY: usize = 64;
/// Ordinary chat must fail open quickly when a maintenance/promotion writer
/// owns `SQLite`.  The durable worker will retry its own work later.
const OBSERVATION_EVENT_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(25);

enum ObservationWrite {
    Insert {
        conversation_id: String,
        turn_id: u64,
        text: String,
    },
    Finalize {
        conversation_id: String,
        turn_id: u64,
        outcome: ObservationOutcome,
    },
}

/// Bounded, long-lived writer for the observation ledger.  Chat event
/// delivery only enqueues a command, so opening/configuring `SQLite` and WAL
/// contention can never sit in front of the ordinary user echo or LLM start.
#[derive(Clone)]
struct ObservationWriter {
    tx: SyncSender<ObservationWrite>,
}

impl ObservationWriter {
    fn insert(&self, conversation_id: String, turn_id: u64, text: String) -> Result<(), ()> {
        self.tx
            .try_send(ObservationWrite::Insert {
                conversation_id,
                turn_id,
                text,
            })
            .map_err(|_| ())
    }

    fn finalize(
        &self,
        conversation_id: String,
        turn_id: u64,
        outcome: ObservationOutcome,
    ) -> Result<(), ()> {
        self.tx
            .try_send(ObservationWrite::Finalize {
                conversation_id,
                turn_id,
                outcome,
            })
            .map_err(|_| ())
    }
}

fn run_observation_writer(
    database_path: &PathBuf,
    rx: &Receiver<ObservationWrite>,
    enrichment: Option<&EnrichmentSender>,
) {
    // Opening and migration are intentionally owned by this worker.  The
    // connection stays alive for the worker lifetime instead of being opened
    // once per user event.
    let mut memory =
        Database::open_with_busy_timeout(database_path, OBSERVATION_EVENT_BUSY_TIMEOUT)
            .map(SqliteMemoryStore::new)
            .map_err(|error| error.to_string());
    while let Ok(command) = rx.recv() {
        if memory.is_err() {
            memory =
                Database::open_with_busy_timeout(database_path, OBSERVATION_EVENT_BUSY_TIMEOUT)
                    .map(SqliteMemoryStore::new)
                    .map_err(|error| error.to_string());
        }
        let Ok(store) = memory.as_mut() else {
            tracing::warn!("memory observation writer unavailable; conversation remains available");
            continue;
        };
        match command {
            ObservationWrite::Insert {
                conversation_id,
                turn_id,
                text,
            } => {
                match store.insert_observation(NewObservation::new(
                    conversation_id,
                    turn_id,
                    text,
                    unix_timestamp(),
                )) {
                    Ok(_) => {
                        // The wake follows the committed INSERT and carries no user content.
                        if enrichment.is_some_and(|sender| {
                            sender.replace_latest(EnrichmentJob::wake(turn_id)).is_err()
                        }) {
                            tracing::warn!(
                                "memory enrichment worker unavailable; observation remains durable"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "memory observation persistence failed; conversation remains available");
                    }
                }
            }
            ObservationWrite::Finalize {
                conversation_id,
                turn_id,
                outcome,
            } => {
                if let Err(error) = store.finalize_observation_by_turn(
                    &conversation_id,
                    turn_id,
                    outcome,
                    unix_timestamp(),
                ) {
                    tracing::warn!(%error, "memory observation outcome finalization failed; conversation remains available");
                }
            }
        }
    }
}

#[cfg(test)]
fn process_enrichment_actions<C: MemoryClassifier>(
    database_path: &Path,
    _wake: &EnrichmentJob,
    consolidator: &mut HybridConsolidator<C>,
) -> Result<(), String> {
    let mut memory =
        SqliteMemoryStore::new(Database::open(database_path).map_err(|error| error.to_string())?);
    process_next_durable_observation(&mut memory, consolidator)?;
    Ok(())
}

/// `SQLite` is the sole enrichment queue.  The in-process wake only asks this
/// function to drain one eligible record; it never supplies memory content.
#[allow(clippy::too_many_lines)]
fn process_next_durable_observation<C: MemoryClassifier>(
    memory: &mut SqliteMemoryStore,
    consolidator: &mut HybridConsolidator<C>,
) -> Result<bool, String> {
    const LEASE_SECONDS: i64 = 60;
    let now = unix_timestamp();
    let Some(lease) = memory
        .claim_next_observation("desktop-enrichment", now, LEASE_SECONDS)
        .map_err(|error| error.to_string())?
    else {
        return Ok(false);
    };
    let run = ClassificationRun::new(
        lease.observation_id,
        "local-consolidator-v1",
        1,
        &lease.canonical_input_hash,
    );
    let run_id = match memory.begin_classification_run(&lease, &run, now) {
        Ok(id) => id,
        Err(error) => {
            retry_observation(memory, &lease, "classification run unavailable", now)?;
            return Err(error.to_string());
        }
    };
    let candidates =
        match memory.find_consolidation_candidates(&lease.user_text, DEFAULT_MEMORY_LIMIT, now) {
            Ok(candidates) => candidates,
            Err(error) => {
                finish_failed_run_and_retry(
                    memory,
                    &lease,
                    run_id,
                    "candidate search unavailable",
                    now,
                )?;
                return Err(error.to_string());
            }
        };
    // The classifier may normalize a statement, but promotion keeps the full
    // accepted source clause.  This makes the persisted candidate replayable
    // by the typed validator and prevents a model-only paraphrase from being
    // stored as user evidence.
    let action = match consolidator.decide(&lease.user_text, &candidates) {
        pw_application::memory::MemoryAction::Add { pinned, .. } => {
            pw_application::memory::MemoryAction::Add {
                content: lease.user_text.clone(),
                pinned,
            }
        }
        pw_application::memory::MemoryAction::Supersede {
            old_memory_id,
            pin_replacement,
            ..
        } => pw_application::memory::MemoryAction::Supersede {
            old_memory_id,
            content: lease.user_text.clone(),
            pin_replacement,
        },
        action => action,
    };
    let (operation, relation, expected_revision) = match &action {
        pw_application::memory::MemoryAction::Ignore => {
            memory
                .finish_classification_run(
                    &lease,
                    run_id,
                    ClassificationOutcome::Completed,
                    0,
                    Some("no durable candidate"),
                    now,
                )
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }
        pw_application::memory::MemoryAction::Add { .. } => (
            CandidateOperation::Add,
            CandidateProvenanceRelation::Originated,
            None,
        ),
        pw_application::memory::MemoryAction::Reinforce { memory_id, .. } => (
            CandidateOperation::Reinforce,
            CandidateProvenanceRelation::Reasserted,
            candidates
                .iter()
                .find(|candidate| candidate.id == *memory_id)
                .and_then(|candidate| candidate.revision),
        ),
        pw_application::memory::MemoryAction::Supersede { old_memory_id, .. } => (
            CandidateOperation::Supersede,
            CandidateProvenanceRelation::Corrected,
            candidates
                .iter()
                .find(|candidate| candidate.id == *old_memory_id)
                .and_then(|candidate| candidate.revision),
        ),
    };
    let target_memory_id = match &action {
        pw_application::memory::MemoryAction::Reinforce { memory_id, .. } => Some(*memory_id),
        pw_application::memory::MemoryAction::Supersede { old_memory_id, .. } => {
            Some(*old_memory_id)
        }
        _ => None,
    };
    let atom = MemoryAtom {
        id: 0,
        revision: 1,
        content: lease.user_text.clone(),
        subject_scope: SubjectScope::UserSelf,
        epistemic_form: EpistemicForm::FactClaim,
        attribution: pw_application::memory::Attribution::User,
        discourse: DiscourseFeatures {
            speech_act: SpeechAct::Asserted,
            source_mode: SourceMode::Direct,
            polarity: Polarity::Affirmed,
            conditionality: pw_application::memory::Conditionality::Actual,
            fictionality: Fictionality::RealWorld,
        },
        verification_status: VerificationStatus::UserReported,
        temporal_scope: TemporalScope::Stable,
        lifecycle_state: pw_application::memory::MemoryState::Active,
        source_spans: vec![SourceSpan {
            source_id: lease.canonical_input_hash.clone(),
            start: 0,
            end: lease.user_text.len(),
        }],
    };
    let candidate_id = match memory.persist_candidate(
        PersistedCandidate {
            classification_run_id: run_id,
            ordinal: 0,
            atom,
            target_memory_id,
            expected_target_revision: expected_revision,
            operation,
            relation,
            domain: pw_application::memory::MemoryDomain::SemanticUser,
            write_class: pw_application::memory::MemoryWriteClass::NormalExplicit,
            normalization_edits: Vec::new(),
        },
        now,
    ) {
        Ok(PersistCandidateOutcome::Persisted(id)) => id,
        Ok(PersistCandidateOutcome::DeterministicallyRejected(_)) => {
            let count = memory
                .reject_pending_candidates(
                    &lease,
                    run_id,
                    "candidate rejected by safety policy",
                    now,
                )
                .map_err(|error| error.to_string())?;
            memory
                .finish_classification_run(
                    &lease,
                    run_id,
                    ClassificationOutcome::Rejected,
                    count,
                    Some("candidate rejected by safety policy"),
                    now,
                )
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }
        Err(error) => {
            finish_failed_run_and_retry(
                memory,
                &lease,
                run_id,
                "candidate persistence unavailable",
                now,
            )?;
            return Err(error.to_string());
        }
    };
    memory
        .finish_classification_run(
            &lease,
            run_id,
            ClassificationOutcome::Completed,
            1,
            None,
            now,
        )
        .map_err(|error| error.to_string())?;
    let result = memory.promote(
        &ProvisionalMemoryChangeSet {
            request_key: run.request_key.clone(),
            lease: lease.clone(),
            classification_run_id: run_id,
            classifier_version: run.classifier_version.clone(),
            schema_version: run.schema_version,
            input_hash: run.input_hash.clone(),
            actions: vec![VersionedMemoryAction {
                action,
                expected_revision,
            }],
            provenance: vec![ProvenanceLink {
                candidate_id,
                relation: match relation {
                    CandidateProvenanceRelation::Originated => "originated",
                    CandidateProvenanceRelation::Reasserted => "reasserted",
                    CandidateProvenanceRelation::Corrected => "corrected",
                    CandidateProvenanceRelation::ChangedStance => "changed_stance",
                    CandidateProvenanceRelation::Contradicted => "contradicted",
                }
                .into(),
            }],
        },
        now,
    );
    if let Err(error) = result {
        tracing::warn!(%error, "durable observation promotion did not complete");
        finish_failed_run_and_retry(memory, &lease, run_id, "promotion unavailable", now)?;
        return Err(error.to_string());
    }
    Ok(true)
}

/// Converts `SQLite`'s durable epoch-second eligibility timestamp into the
/// worker's monotonic deadline.  The database remains the source of truth;
/// this only prevents an in-process retry wake from being dropped early.
fn observation_follow_up_deadline(database_path: &Path) -> Option<std::time::Instant> {
    let now = unix_timestamp();
    let due_at = Database::open(database_path)
        .map(SqliteMemoryStore::new)
        .map_err(|error| error.to_string())
        .and_then(|store| {
            store
                .next_observation_due_at(now)
                .map_err(|error| error.to_string())
        })
        .ok()
        .flatten()?;
    let delay_seconds = due_at.saturating_sub(now);
    Some(std::time::Instant::now() + std::time::Duration::from_secs(delay_seconds.cast_unsigned()))
}

fn retry_observation(
    memory: &mut SqliteMemoryStore,
    lease: &pw_application::memory::ObservationLease,
    reason: &str,
    now: i64,
) -> Result<(), String> {
    memory
        .retry_or_defer_observation(lease, reason, now, 3, 1)
        .map_err(|error| error.to_string())
}

fn finish_failed_run_and_retry(
    memory: &mut SqliteMemoryStore,
    lease: &pw_application::memory::ObservationLease,
    run_id: i64,
    reason: &str,
    now: i64,
) -> Result<(), String> {
    let count = memory
        .reject_pending_candidates(lease, run_id, reason, now)
        .map_err(|error| error.to_string())?;
    memory
        .finish_classification_run(
            lease,
            run_id,
            ClassificationOutcome::Failed,
            count,
            Some(reason),
            now,
        )
        .map_err(|error| error.to_string())?;
    retry_observation(memory, lease, reason, now)
}

#[cfg(test)]
fn process_enrichment_job_with_consolidator<C: MemoryClassifier>(
    database_path: &Path,
    job: &EnrichmentJob,
    consolidator: &mut HybridConsolidator<C>,
) -> Result<bool, String> {
    let history = SqliteConversationHistory::new(
        Database::open(database_path).map_err(|error| error.to_string())?,
    );
    let mut memory =
        SqliteMemoryStore::new(Database::open(database_path).map_err(|error| error.to_string())?);
    // The helper mirrors production: test payloads are first committed to the
    // durable queue and are never consumed directly by promotion.
    memory
        .insert_observation(NewObservation::new(
            DEFAULT_CONVERSATION_ID,
            job.turn_id,
            &job.user_text,
            unix_timestamp(),
        ))
        .map_err(|error| error.to_string())?;
    process_enrichment_actions(database_path, job, consolidator)?;
    drain_rolling_summary_pass(&history, &mut memory)
}

#[cfg(test)]
fn update_rolling_summary(
    history: &SqliteConversationHistory,
    memory: &mut SqliteMemoryStore,
) -> Result<bool, String> {
    update_rolling_summary_until(history, memory, None)
}

fn update_rolling_summary_until(
    history: &SqliteConversationHistory,
    memory: &mut SqliteMemoryStore,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    let Some(stable_through) = history
        .summary_stable_through_id(DEFAULT_CONVERSATION_ID, SUMMARY_RECENT_MESSAGES)
        .map_err(|error| error.to_string())?
    else {
        return Ok(false);
    };
    let existing = memory
        .load_summary(DEFAULT_CONVERSATION_ID)
        .map_err(|error| error.to_string())?;
    let existing_is_role_preserving = existing
        .as_ref()
        .is_some_and(|item| is_role_preserving_summary(&item.content));
    if existing.is_some() && !existing_is_role_preserving {
        tracing::warn!("legacy untyped summary will be rebuilt from stored role messages");
    }
    let through = existing
        .as_ref()
        .filter(|_| existing_is_role_preserving)
        .map_or(0, |item| item.through_message_id);
    let mut pending = history
        .list_messages_by_id_page(
            DEFAULT_CONVERSATION_ID,
            through,
            stable_through,
            SUMMARY_BATCH_MESSAGES.saturating_add(1),
        )
        .map_err(|error| error.to_string())?;
    let remaining = pending.len() > SUMMARY_BATCH_MESSAGES;
    pending.truncate(SUMMARY_BATCH_MESSAGES);
    if pending.is_empty() {
        return Ok(false);
    }
    let prompt_messages = pending
        .iter()
        .filter(|item| is_safe_persistent_content(&item.content))
        .map(|item| {
            ChatMessage::new(
                match item.role {
                    MessageRole::User => ChatRole::User,
                    MessageRole::Assistant => ChatRole::Assistant,
                },
                item.content.clone(),
            )
        })
        .collect::<Vec<_>>();
    let delta = RollingSummaryGenerator
        .summarize(&prompt_messages)
        .map_err(|error| error.to_string())?;
    let bounded = merge_rolling_summaries(
        existing
            .as_ref()
            .filter(|_| existing_is_role_preserving)
            .map(|item| item.content.as_str()),
        &delta,
        SUMMARY_MAX_CHARS,
    )
    .map_err(|error| error.to_string())?;
    let through = pending
        .last()
        .and_then(|item| item.id)
        .ok_or_else(|| "pending summary message lacks id".to_owned())?;
    if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
        return Ok(true);
    }
    memory
        .upsert_summary(DEFAULT_CONVERSATION_ID, &bounded, through, unix_timestamp())
        .map_err(|error| error.to_string())?;
    Ok(remaining)
}

#[cfg(test)]
fn drain_rolling_summary_pass(
    history: &SqliteConversationHistory,
    memory: &mut SqliteMemoryStore,
) -> Result<bool, String> {
    drain_rolling_summary_pass_until(history, memory, None)
}

fn drain_rolling_summary_pass_until(
    history: &SqliteConversationHistory,
    memory: &mut SqliteMemoryStore,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    let started = std::time::Instant::now();
    for batch in 0..SUMMARY_DRAIN_BATCHES_PER_PASS {
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
            return Ok(true);
        }
        if batch > 0 && started.elapsed() >= SUMMARY_DRAIN_TIME_BUDGET {
            return Ok(true);
        }
        if !update_rolling_summary_until(history, memory, cancel)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn drain_rolling_summary_at_path(
    database_path: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    let history = SqliteConversationHistory::new(
        Database::open(database_path).map_err(|error| error.to_string())?,
    );
    let mut memory =
        SqliteMemoryStore::new(Database::open(database_path).map_err(|error| error.to_string())?);
    drain_rolling_summary_pass_until(&history, &mut memory, cancel)
}

#[allow(clippy::needless_pass_by_value)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct EnrichmentJob {
    turn_id: u64,
    // Production wake markers are deliberately content-free.  The durable
    // SQLite observation is the only source a worker may classify.
    #[cfg(test)]
    user_text: String,
}

impl EnrichmentJob {
    fn wake(turn_id: u64) -> Self {
        Self {
            turn_id,
            #[cfg(test)]
            user_text: String::new(),
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
#[derive(Clone)]
struct EnrichmentSender {
    wake: Arc<SyncSender<()>>,
    pending: Arc<Mutex<Option<Vec<EnrichmentJob>>>>,
    metrics: Arc<QueueMetrics>,
}

impl EnrichmentSender {
    fn replace_latest(&self, job: EnrichmentJob) -> Result<(), ()> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let batch = pending.get_or_insert_with(Vec::new);
        if batch.iter().any(|existing| existing.turn_id == job.turn_id) {
            self.metrics.coalesced();
            return Ok(());
        } else if batch.len() < ENRICHMENT_PENDING_CAPACITY {
            batch.push(job);
            self.metrics.enqueued();
        } else {
            self.metrics.dropped();
            return Err(());
        }
        drop(pending);
        match self.wake.try_send(()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(())) => {
                self.metrics.busy();
                Ok(())
            }
            Err(TrySendError::Disconnected(())) => {
                let discarded = self
                    .pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .map_or(0, |batch| batch.len());
                for _ in 0..discarded {
                    self.metrics.dequeued();
                    self.metrics.dropped();
                }
                Err(())
            }
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
#[cfg(test)]
fn run_enrichment(
    database_path: &Path,
    rx: Receiver<()>,
    wake: Arc<SyncSender<()>>,
    pending: Arc<Mutex<Option<Vec<EnrichmentJob>>>>,
    metrics: Arc<QueueMetrics>,
    consolidator: HybridConsolidator<Box<dyn MemoryClassifier>>,
) {
    run_enrichment_until_cancelled(
        database_path,
        rx,
        wake,
        pending,
        metrics,
        consolidator,
        Arc::new(AtomicBool::new(false)),
    );
}

#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
fn run_enrichment_until_cancelled(
    database_path: &Path,
    rx: Receiver<()>,
    wake: Arc<SyncSender<()>>,
    pending: Arc<Mutex<Option<Vec<EnrichmentJob>>>>,
    metrics: Arc<QueueMetrics>,
    mut consolidator: HybridConsolidator<Box<dyn MemoryClassifier>>,
    cancel: Arc<AtomicBool>,
) {
    // A process restart must not leave a pending response outcome looking
    // live.  Expired leases are reclaimed by the claim query; active leases
    // remain fenced until they expire.
    match Database::open(database_path).map(SqliteMemoryStore::new) {
        Ok(mut store) => {
            if let Err(error) = store.recover_interrupted_observations(unix_timestamp()) {
                tracing::warn!(%error, "observation startup recovery failed; worker remains available");
            }
        }
        Err(error) => {
            tracing::warn!(%error, "observation startup database unavailable; worker remains available");
        }
    }
    // Bootstrap a drain after restart.  This marker contains no user data and
    // is only a coalescible request to read SQLite.
    {
        let mut pending = pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.is_none() {
            *pending = Some(vec![EnrichmentJob::wake(0)]);
        }
    }
    let mut follow_up_due = Some(std::time::Instant::now());
    loop {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let timeout = follow_up_due.map_or(ENRICHMENT_IDLE_POLL, |deadline: std::time::Instant| {
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(ENRICHMENT_IDLE_POLL)
        });
        match rx.recv_timeout(timeout) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => {
                let follow_up_ready =
                    follow_up_due.is_some_and(|deadline| std::time::Instant::now() >= deadline);
                if !follow_up_ready {
                    if Arc::strong_count(&wake) == 1 && follow_up_due.is_none() {
                        break;
                    }
                    continue;
                }
                // A retry lives only in SQLite.  Reinsert a content-free
                // marker at its deadline so a transient failure cannot leave
                // the row pending forever when no later user turn arrives.
                let mut pending = pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if pending.is_none() {
                    *pending = Some(vec![EnrichmentJob::wake(0)]);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let (jobs, jobs_remaining) = {
            let mut pending = pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut jobs = pending.take().unwrap_or_default();
            let remaining = (jobs.len() > ENRICHMENT_JOBS_PER_SLICE)
                .then(|| jobs.split_off(ENRICHMENT_JOBS_PER_SLICE));
            let jobs_remaining = remaining.as_ref().is_some_and(|jobs| !jobs.is_empty());
            *pending = remaining;
            (jobs, jobs_remaining)
        };
        let mut durable_processed = false;
        let mut retry_scheduled = false;
        #[cfg_attr(not(test), allow(unused_variables))]
        for job in jobs {
            metrics.dequeued();
            #[cfg(test)]
            if !job.user_text.is_empty() {
                // Test-only compatibility shim: production wake markers never
                // carry text; fixtures can still model a committed ledger.
                if let Ok(mut store) = Database::open(database_path).map(SqliteMemoryStore::new) {
                    let _ = store.insert_observation(NewObservation::new(
                        DEFAULT_CONVERSATION_ID,
                        job.turn_id,
                        &job.user_text,
                        unix_timestamp(),
                    ));
                }
            }
            match Database::open(database_path)
                .map(SqliteMemoryStore::new)
                .map_err(|error| error.to_string())
                .and_then(|mut store| {
                    process_next_durable_observation(&mut store, &mut consolidator)
                }) {
                Ok(processed) => durable_processed |= processed,
                Err(error) => {
                    retry_scheduled = true;
                    tracing::warn!(%error, "memory enrichment job failed; worker remains available");
                }
            }
        }
        let summary_remaining = if cancel.load(Ordering::Acquire) {
            false
        } else {
            match drain_rolling_summary_at_path(database_path, Some(cancel.as_ref())) {
                Ok(remaining) => remaining,
                Err(error) => {
                    tracing::warn!(%error, "rolling summary follow-up failed; worker remains available");
                    false
                }
            }
        };
        if durable_processed {
            // Keep draining the durable queue in bounded slices.  This is a
            // marker only; the next iteration re-claims SQLite and cannot
            // resurrect any in-memory user payload.
            let mut pending = pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if pending.is_none() {
                *pending = Some(vec![EnrichmentJob::wake(0)]);
            }
        }
        let durable_due = observation_follow_up_deadline(database_path);
        if (jobs_remaining || summary_remaining || durable_processed)
            && !cancel.load(Ordering::Acquire)
        {
            follow_up_due = Some(std::time::Instant::now() + ENRICHMENT_FOLLOWUP_DELAY);
        } else if let Some(deadline) = durable_due {
            // `Ok(false)` only means that no row is eligible *yet*.  Keep the
            // worker alive until the durable retry/lease deadline instead of
            // depending on a later user turn to wake it again.
            follow_up_due = Some(deadline);
        } else if retry_scheduled && !cancel.load(Ordering::Acquire) {
            // When SQLite itself is temporarily unavailable there is no
            // readable deadline.  Preserve the previous bounded retry rather
            // than abandoning the worker; a recovered database will provide
            // its durable deadline on the next pass.
            follow_up_due = Some(std::time::Instant::now() + ENRICHMENT_FOLLOWUP_DELAY);
        } else if Arc::strong_count(&wake) == 1 {
            break;
        } else {
            follow_up_due = None;
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn run_context_worker<M: MemoryStore>(
    memory: M,
    rx: Receiver<Command>,
    conversation_tx: SyncSender<Command>,
    submit_metrics: Arc<QueueMetrics>,
    context_metrics: Arc<QueueMetrics>,
    conversation_metrics: Arc<QueueMetrics>,
) {
    run_context_worker_loop(
        memory,
        &rx,
        &conversation_tx,
        &submit_metrics,
        &context_metrics,
        &conversation_metrics,
        MEMORY_MAINTENANCE_INTERVAL,
    );
}

#[cfg(test)]
#[allow(clippy::needless_pass_by_value)]
fn run_context_worker_with_interval<M: MemoryStore>(
    memory: M,
    rx: Receiver<Command>,
    conversation_tx: SyncSender<Command>,
    submit_metrics: Arc<QueueMetrics>,
    context_metrics: Arc<QueueMetrics>,
    conversation_metrics: Arc<QueueMetrics>,
    maintenance_interval: std::time::Duration,
) {
    run_context_worker_loop(
        memory,
        &rx,
        &conversation_tx,
        &submit_metrics,
        &context_metrics,
        &conversation_metrics,
        maintenance_interval,
    );
}

fn run_context_worker_loop<M: MemoryStore>(
    mut memory: M,
    rx: &Receiver<Command>,
    conversation_tx: &SyncSender<Command>,
    submit_metrics: &Arc<QueueMetrics>,
    context_metrics: &Arc<QueueMetrics>,
    conversation_metrics: &Arc<QueueMetrics>,
    maintenance_interval: std::time::Duration,
) {
    let mut next_maintenance = maintenance_deadline(&mut memory, maintenance_interval);
    loop {
        let now = std::time::Instant::now();
        if now >= next_maintenance {
            next_maintenance = maintenance_deadline(&mut memory, maintenance_interval);
        }
        let timeout = next_maintenance.saturating_duration_since(std::time::Instant::now());
        let command = match rx.recv_timeout(timeout) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => {
                next_maintenance = maintenance_deadline(&mut memory, maintenance_interval);
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        submit_metrics.dequeued();
        match command {
            Command::Submit(text, turn_id, lease) => {
                context_metrics.enqueued();
                let context_started = std::time::Instant::now();
                tracing::info!(turn_id, "chat memory context load started");
                let context = load_memory_context(&mut memory, &text, turn_id, unix_timestamp());
                tracing::info!(
                    turn_id,
                    elapsed_ms = context_started.elapsed().as_millis(),
                    memory_count = context.memories.len(),
                    has_summary = context.summary.is_some(),
                    "chat memory context ready"
                );
                context_metrics.dequeued();
                // Account for the distinct prepared-conversation queue.
                // Increment before send so a fast consumer cannot underflow depth.
                conversation_metrics.enqueued();
                if conversation_tx
                    .send(Command::Prepared(text, turn_id, context, lease))
                    .is_err()
                {
                    conversation_metrics.dequeued();
                    conversation_metrics.dropped();
                    break;
                }
            }
            Command::Prepared(..) => {}
        }
    }
}

fn maintenance_deadline<M: MemoryStore>(
    memory: &mut M,
    maintenance_interval: std::time::Duration,
) -> std::time::Instant {
    let delay = match memory.run_maintenance(unix_timestamp(), 100) {
        Ok(report) if report.remaining => MEMORY_MAINTENANCE_FOLLOWUP.min(maintenance_interval),
        Ok(_) => maintenance_interval,
        Err(error) => {
            tracing::warn!(%error, "memory maintenance failed; context worker remains available");
            maintenance_interval
        }
    };
    std::time::Instant::now() + delay
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

fn persist_completed_turn<H: ConversationHistory>(
    history: &mut H,
    conversation_id: &str,
    turn: TurnId,
    user_text: &str,
    assistant_text: &str,
) -> Result<(), pw_application::PortError> {
    history.store_completed_turn(&StoredTurn {
        conversation_id: conversation_id.to_owned(),
        turn_id: turn.value(),
        user_content: redact_persistent_content(user_text),
        assistant_content: redact_persistent_content(assistant_text),
        created_at: unix_timestamp(),
    })
}

fn load_recent_history<H: ConversationHistory>(
    history: &H,
    conversation_id: &str,
    limit: usize,
) -> Result<Vec<ChatMessage>, pw_application::PortError> {
    let messages = history.list_recent_messages_by_id(conversation_id, limit)?;
    Ok(messages
        .into_iter()
        .map(|message| {
            ChatMessage::new(
                match message.role {
                    MessageRole::User => ChatRole::User,
                    MessageRole::Assistant => ChatRole::Assistant,
                },
                message.content,
            )
        })
        .collect())
}

struct PersistentConversationEvents<E, H> {
    inner: E,
    history: Mutex<H>,
    conversation_id: String,
    pending_users: Mutex<HashMap<TurnId, (String, Option<String>)>>,
    enrichment: Option<EnrichmentSender>,
    observation_writer: Option<ObservationWriter>,
    companion_state_writer: Option<SyncSender<AsyncStateWrite>>,
}

impl<E, H> PersistentConversationEvents<E, H> {
    #[cfg(test)]
    fn new(inner: E, history: H, conversation_id: impl Into<String>) -> Self {
        Self::new_with_enrichment(inner, history, conversation_id, None, None)
    }
    #[allow(dead_code)]
    fn new_with_enrichment(
        inner: E,
        history: H,
        conversation_id: impl Into<String>,
        enrichment: Option<EnrichmentSender>,
        observation_writer: Option<ObservationWriter>,
    ) -> Self {
        Self::new_with_enrichment_and_state(
            inner,
            history,
            conversation_id,
            enrichment,
            observation_writer,
            None,
        )
    }
    fn new_with_enrichment_and_state(
        inner: E,
        history: H,
        conversation_id: impl Into<String>,
        enrichment: Option<EnrichmentSender>,
        observation_writer: Option<ObservationWriter>,
        companion_state_writer: Option<SyncSender<AsyncStateWrite>>,
    ) -> Self {
        Self {
            inner,
            history: Mutex::new(history),
            conversation_id: conversation_id.into(),
            pending_users: Mutex::new(HashMap::new()),
            enrichment,
            observation_writer,
            companion_state_writer,
        }
    }

    fn enqueue_companion_signals(&self, user_text: &str, assistant_text: &str) {
        let Some(writer) = &self.companion_state_writer else {
            return;
        };
        let Some(signals) = derive_dialogue_signals(
            &self.conversation_id,
            user_text,
            assistant_text,
            unix_timestamp(),
        ) else {
            return;
        };
        // The queue is bounded and deliberately lossy.  A full/disconnected
        // companion worker must never delay the ordinary reply or TTS.
        if writer
            .try_send(AsyncStateWrite::DialogueSignals(signals))
            .is_err()
        {
            tracing::debug!("companion state queue unavailable; continuing without state update");
        }
    }

    /// Returns true only after the durable queue record committed.  The caller
    /// may then send a lossy wake; it must never wake a worker for data that
    /// failed to reach `SQLite`.
    fn record_observation(&self, turn: TurnId, text: &str) -> bool {
        let Some(writer) = &self.observation_writer else {
            return false;
        };
        if writer
            .insert(self.conversation_id.clone(), turn.value(), text.to_owned())
            .is_err()
        {
            tracing::warn!(
                "memory observation writer queue unavailable; conversation remains available"
            );
            false
        } else {
            true
        }
    }

    fn finalize_observation(&self, turn: TurnId, outcome: ObservationOutcome) {
        let Some(writer) = &self.observation_writer else {
            return;
        };
        if writer
            .finalize(self.conversation_id.clone(), turn.value(), outcome)
            .is_err()
        {
            tracing::warn!(
                "memory observation writer queue unavailable; conversation remains available"
            );
        }
    }

    #[cfg(test)]
    fn history(&self) -> MutexGuard<'_, H> {
        self.history.lock().unwrap()
    }
}

impl<E: ConversationEvents, H: ConversationHistory> ConversationEvents
    for PersistentConversationEvents<E, H>
{
    fn on_state(&self, state: ConversationState) {
        self.inner.on_state(state);
    }
    fn on_user_message(&self, turn: TurnId, text: &str) {
        let mut pending = self.pending_users.lock().unwrap();
        let retries: Vec<_> = pending
            .iter()
            .filter_map(|(id, (user, assistant))| {
                assistant
                    .as_ref()
                    .map(|assistant| (*id, user.clone(), assistant.clone()))
            })
            .collect();
        for (retry_turn, user, assistant) in retries {
            if persist_completed_turn(
                &mut *self.history.lock().unwrap(),
                &self.conversation_id,
                retry_turn,
                &user,
                &assistant,
            )
            .is_ok()
            {
                if let Some(enrichment) = &self.enrichment
                    && enrichment
                        .replace_latest(EnrichmentJob {
                            turn_id: retry_turn.value(),
                            #[cfg(test)]
                            user_text: user.clone(),
                        })
                        .is_err()
                {
                    tracing::warn!(
                        "memory enrichment worker unavailable; conversation remains available"
                    );
                }
                pending.remove(&retry_turn);
                self.enqueue_companion_signals(&user, &assistant);
            }
        }
        pending.insert(turn, (text.to_owned(), None));
        let _observation_queued = self.record_observation(turn, text);
        self.inner.on_user_message(turn, text);
    }
    fn on_control(&self, turn: TurnId, control: &ReplyControl) {
        self.inner.on_control(turn, control);
    }
    fn on_sentence(&self, turn: TurnId, sentence: &str) {
        self.inner.on_sentence(turn, sentence);
    }
    fn on_reply_complete(&self, turn: TurnId, speech_text: &str) {
        self.inner.on_reply_complete(turn, speech_text);
        let mut pending = self.pending_users.lock().unwrap();
        let Some((user_text, assistant)) = pending.get_mut(&turn) else {
            return;
        };
        *assistant = Some(speech_text.to_owned());
        if let Err(error) = persist_completed_turn(
            &mut *self.history.lock().unwrap(),
            &self.conversation_id,
            turn,
            user_text,
            speech_text,
        ) {
            tracing::warn!(%error, "conversation history persistence degraded to memory");
            self.finalize_observation(turn, ObservationOutcome::HistoryPersistFailed);
        } else {
            self.finalize_observation(turn, ObservationOutcome::Completed);
            self.enqueue_companion_signals(user_text, speech_text);
            if let Some(enrichment) = &self.enrichment
                && enrichment
                    .replace_latest(EnrichmentJob {
                        turn_id: turn.value(),
                        #[cfg(test)]
                        user_text: user_text.clone(),
                    })
                    .is_err()
            {
                tracing::warn!(
                    "memory enrichment worker unavailable; conversation remains available"
                );
            }
            pending.remove(&turn);
        }
    }
    fn on_cancelled(&self, turn: TurnId) {
        self.pending_users.lock().unwrap().remove(&turn);
        self.finalize_observation(turn, ObservationOutcome::Cancelled);
        self.inner.on_cancelled(turn);
    }
    fn on_error(&self, turn: TurnId, message: &str) {
        self.pending_users.lock().unwrap().remove(&turn);
        self.finalize_observation(turn, ObservationOutcome::LlmFailed);
        self.inner.on_error(turn, message);
    }
}

#[derive(Debug)]
struct UserTurnLease {
    gate: Arc<InteractionGate>,
}

impl UserTurnLease {
    fn new(gate: Arc<InteractionGate>) -> Self {
        gate.begin_user_turn();
        Self { gate }
    }
}

impl Drop for UserTurnLease {
    fn drop(&mut self) {
        self.gate.end_user_turn();
    }
}

enum Command {
    Submit(String, u64, UserTurnLease),
    Prepared(String, u64, MemoryContext, UserTurnLease),
}

fn run_prepared_command<T>(
    command: Command,
    submit: impl FnOnce(&str, u64, &MemoryContext) -> T,
) -> Option<T> {
    match command {
        Command::Prepared(text, turn_id, context, _lease) => Some(submit(&text, turn_id, &context)),
        Command::Submit(..) => None,
    }
}

fn enqueue_submit(
    tx: &SyncSender<Command>,
    metrics: &QueueMetrics,
    text: String,
    turn_id: u64,
    lease: UserTurnLease,
) -> Result<(), String> {
    metrics.enqueued();
    tx.try_send(Command::Submit(text, turn_id, lease))
        .map_err(|error| match error {
            TrySendError::Full(_) => {
                metrics.dequeued();
                metrics.busy();
                metrics.dropped();
                "conversation is busy; please retry".to_owned()
            }
            TrySendError::Disconnected(_) => {
                metrics.dequeued();
                metrics.dropped();
                "conversation worker is not available".to_owned()
            }
        })
}

#[allow(clippy::struct_field_names)]
struct Worker {
    tx: SyncSender<Command>,
    settings_fingerprint: String,
    thread: Option<std::thread::JoinHandle<()>>,
    context_thread: Option<std::thread::JoinHandle<()>>,
    enrichment_thread: Option<std::thread::JoinHandle<()>>,
    observation_writer_thread: Option<std::thread::JoinHandle<()>>,
    companion_state_worker: Option<CompanionStateWorker>,
    enrichment_cancel: Arc<AtomicBool>,
}

impl Worker {
    fn shutdown(mut self) -> Result<(), String> {
        // Dropping the sole submit sender disconnects the bounded queue even when full.
        // The context worker then drops its conversation sender, cascading shutdown.
        self.enrichment_cancel.store(true, Ordering::SeqCst);
        drop(self.tx);
        let result = join_worker(self.context_thread.take(), "context");
        let conversation = join_worker(self.thread.take(), "conversation");
        let observation_writer =
            join_worker(self.observation_writer_thread.take(), "observation writer");
        let enrichment = join_worker(self.enrichment_thread.take(), "enrichment");
        let companion = self
            .companion_state_worker
            .take()
            .map_or(Ok(()), CompanionStateWorker::shutdown);
        result
            .and(conversation)
            .and(observation_writer)
            .and(enrichment)
            .and(companion)
    }
}

fn join_worker(thread: Option<std::thread::JoinHandle<()>>, name: &str) -> Result<(), String> {
    let Some(thread) = thread else {
        return Ok(());
    };
    // Adapter I/O has a finite total timeout. Always join so a reset cannot
    // leave stale workers mutating history after the replacement starts.
    thread
        .join()
        .map_err(|_| format!("{name} worker panicked during shutdown"))
}

/// Managed state: at most one conversation worker.
pub struct ChatService {
    operation: Mutex<()>,
    worker: Mutex<Option<Worker>>,
    cancel: Arc<AtomicBool>,
    fallback_turn_id: AtomicU64,
    health: Arc<Mutex<FeatureHealthSupervisor<SystemClock, TimeJitter>>>,
    submit_metrics: Arc<QueueMetrics>,
    context_metrics: Arc<QueueMetrics>,
    conversation_metrics: Arc<QueueMetrics>,
    enrichment_metrics: Arc<QueueMetrics>,
    interaction_gate: Arc<InteractionGate>,
}

impl Default for ChatService {
    fn default() -> Self {
        Self {
            operation: Mutex::new(()),
            worker: Mutex::new(None),
            cancel: Arc::new(AtomicBool::new(false)),
            fallback_turn_id: AtomicU64::new(1_u64 << 63),
            health: Arc::new(Mutex::new(FeatureHealthSupervisor::new(
                RuntimeFeature::LanguageModel,
                SystemClock,
                TimeJitter::default(),
            ))),
            submit_metrics: Arc::new(QueueMetrics::new("chat_submit", SUBMIT_QUEUE_CAPACITY)),
            context_metrics: Arc::new(QueueMetrics::new("chat_context", SUBMIT_QUEUE_CAPACITY)),
            conversation_metrics: Arc::new(QueueMetrics::new(
                "chat_conversation",
                CONVERSATION_QUEUE_CAPACITY,
            )),
            enrichment_metrics: Arc::new(QueueMetrics::new(
                "chat_enrichment",
                ENRICHMENT_PENDING_CAPACITY,
            )),
            interaction_gate: Arc::new(InteractionGate::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatWorkerContext {
    character_prompt: String,
    persona_fingerprint: String,
    character: Option<CharacterSnapshot>,
}

impl ChatWorkerContext {
    fn character_control_context(
        &self,
    ) -> Option<crate::commands::character::CharacterControlContext> {
        self.character.as_ref().map(|character| {
            crate::commands::character::CharacterControlContext {
                renderer: character.renderer,
                capabilities: character.capabilities.clone(),
            }
        })
    }
}

fn prepare_worker_context(
    persona: crate::behavior::ResolvedPersonaPrompt,
    character: Option<CharacterSnapshot>,
) -> ChatWorkerContext {
    let matching_character =
        character.filter(|snapshot| persona.character_id.as_deref() == Some(snapshot.id.as_str()));
    let character_prompt = matching_character.as_ref().map_or_else(
        || persona.character_prompt.clone(),
        |snapshot| character_instruction(&persona.character_prompt, &snapshot.capabilities),
    );
    ChatWorkerContext {
        character_prompt,
        persona_fingerprint: persona.fingerprint,
        character: matching_character,
    }
}

fn resolve_worker_context(
    layout: &AppDataLayout,
    state: &CharacterState,
    settings: &LlmSettingsDto,
    dark_expression_paused: bool,
) -> ChatWorkerContext {
    let character = resolve_character_manifest(layout, state)
        .ok()
        .map(|manifest| CharacterSnapshot::from_manifest(&manifest));
    let persona = crate::behavior::resolve_persona_prompt_with_pause(
        layout,
        character.as_ref().map(|snapshot| snapshot.id.as_str()),
        settings,
        dark_expression_paused,
    );
    prepare_worker_context(persona, character)
}

fn worker_fingerprint(settings: &LlmSettingsDto, context: &ChatWorkerContext) -> String {
    serde_json::json!({
        "version": 1,
        "base_url": settings.base_url,
        "model": settings.model,
        "allow_remote": settings.allow_remote,
        "system_prompt": settings.system_prompt,
        "strip_emoji": settings.strip_emoji,
        "persona": context.persona_fingerprint,
        "character_prompt": context.character_prompt,
        "character": context.character.as_ref().map(|character| serde_json::json!({
            "id": character.id,
            "renderer": character.renderer,
            "expressions": character.capabilities.expressions,
            "motions": character.capabilities.motions,
        })),
    })
    .to_string()
}

impl ChatService {
    #[must_use]
    pub(crate) fn interaction_gate(&self) -> Arc<InteractionGate> {
        Arc::clone(&self.interaction_gate)
    }

    /// Generates a bounded proactive reply without creating a synthetic user
    /// turn or emitting any UI/TTS events. The behavior runtime owns the final
    /// privacy, frequency, persistence, and delivery gates.
    #[allow(clippy::unused_self)]
    pub(crate) fn generate_proactive_reply<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        cue: &str,
    ) -> Result<String, String> {
        let layout = app.state::<AppDataLayout>();
        let settings = super::settings::load_llm_settings(&layout);
        let context = resolve_worker_context(
            &layout,
            &app.state::<CharacterState>(),
            &settings,
            app.state::<crate::behavior::DarkExpressionSafetyState>()
                .is_paused(),
        );
        let config = LlmClientConfig {
            base_url: settings.base_url.clone(),
            model: settings.model.clone(),
            api_key: super::settings::load_llm_api_key(settings.provider)?,
            allow_remote: settings.allow_remote,
            timeout: ADAPTER_TIMEOUT,
        };
        let mut llm = OpenAiCompatClient::new(config).map_err(|error| error.to_string())?;
        let database_path = layout.data.join("parallel-world.sqlite3");
        let history = Database::open(&database_path)
            .map(SqliteConversationHistory::new)
            .ok()
            .and_then(|history| {
                load_recent_history(&history, DEFAULT_CONVERSATION_ID, MAX_HISTORY_MESSAGES).ok()
            })
            .unwrap_or_default();
        let prompt = PromptBuilder {
            system_rules: settings.system_prompt,
            character_prompt: context.character_prompt,
        };
        let request = format!(
            "This is a proactive check-in, not a user message. {cue} Respond with one brief, natural utterance. Do not claim to know hidden window titles or private content."
        );
        let messages = prompt.build(&history, &request);
        let cancel = AtomicBool::new(false);
        let mut parser = ReplyParser::new();
        let mut speech = String::new();
        llm.stream_chat(&messages, &cancel, &mut |delta| {
            for event in parser.push(delta) {
                if let ReplyEvent::Speech(chunk) = event {
                    speech.push_str(&chunk);
                    if speech.chars().count() > 2_000 {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
            }
        })
        .map_err(|error| error.to_string())?;
        for event in parser.finish() {
            if let ReplyEvent::Speech(chunk) = event {
                speech.push_str(&chunk);
            }
        }
        let speech = if settings.strip_emoji {
            strip_emoji(&speech)
        } else {
            speech
        };
        let speech = speech.trim();
        if speech.is_empty() || speech.chars().count() > 500 {
            return Err("proactive response was empty or too long".to_owned());
        }
        Ok(speech.to_owned())
    }

    #[allow(clippy::unused_self)]
    pub(crate) fn evaluate_proactive_reply<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        cue: &str,
        generated: &str,
        endpoint: &str,
        model: &str,
    ) -> Result<bool, String> {
        let layout = app.state::<AppDataLayout>();
        let settings = super::settings::load_llm_settings(&layout);
        let mut llm = OpenAiCompatClient::new(LlmClientConfig {
            base_url: endpoint.to_owned(),
            model: model.to_owned(),
            api_key: super::settings::load_llm_api_key(settings.provider)?,
            allow_remote: settings.allow_remote,
            timeout: ADAPTER_TIMEOUT,
        })
        .map_err(|error| error.to_string())?;
        let messages = vec![
            ChatMessage::new(
                ChatRole::System,
                "Approve only a safe, relevant, low-pressure proactive check-in. Reply with exactly APPROVE or SKIP.",
            ),
            ChatMessage::new(
                ChatRole::User,
                format!("Context: {cue}\nCandidate: {generated}"),
            ),
        ];
        let cancel = AtomicBool::new(false);
        let mut result = String::new();
        llm.stream_chat(&messages, &cancel, &mut |delta| {
            result.push_str(delta);
            if result.len() > 32 {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .map_err(|error| error.to_string())?;
        Ok(result.trim() == "APPROVE")
    }
    fn require_healthy(&self, lease: UserTurnLease) -> Result<UserTurnLease, String> {
        if self
            .health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .can_attempt()
        {
            Ok(lease)
        } else {
            Err("language model is recovering; retry after the backoff period".to_owned())
        }
    }
    fn with_user_turn<T>(
        &self,
        submit: impl FnOnce(UserTurnLease) -> Result<T, String>,
    ) -> Result<T, String> {
        let lease = self.require_healthy(UserTurnLease::new(self.interaction_gate()))?;
        submit(lease)
    }
    /// Clears the application-owned LLM circuit.
    ///
    /// # Errors
    /// Returns an error when the circuit is not open.
    pub fn rearm(&self) -> Result<HealthTransition, &'static str> {
        self.health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .rearm()
    }
    #[must_use]
    pub fn health_snapshot(&self) -> RuntimeHealthEventDto {
        let health = self
            .health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut event = RuntimeHealthEventDto::from((health.health(), health.attempts()));
        event.circuit_open = health.circuit_open();
        event
    }
    pub fn queue_metrics(&self) -> Vec<pw_contracts::QueueMetricsDto> {
        [
            &self.submit_metrics,
            &self.context_metrics,
            &self.conversation_metrics,
            &self.enrichment_metrics,
        ]
        .into_iter()
        .map(|metrics| metrics.snapshot())
        .collect()
    }
    /// Stops the active workers and clears their in-memory prompt state.
    ///
    /// # Errors
    /// Returns an error if a worker thread panicked while shutting down.
    pub fn reset(&self) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.reset_locked()
    }

    fn reset_locked(&self) -> Result<(), String> {
        self.cancel.store(true, Ordering::SeqCst);
        let result = self.lock().take().map_or(Ok(()), Worker::shutdown);
        self.cancel.store(false, Ordering::SeqCst);
        result
    }

    /// Runs a destructive operation while submissions are excluded.
    ///
    /// # Errors
    /// Returns a worker shutdown error or the operation's error.
    pub fn with_exclusive_reset<T>(
        &self,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = self
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.reset_locked()?;
        operation()
    }
    fn reserve_turn_id(&self, database_path: &Path) -> u64 {
        let durable = Database::open(database_path)
            .map_err(|error| error.to_string())
            .and_then(|database| {
                SqliteConversationHistory::new(database)
                    .reserve_turn_id(DEFAULT_CONVERSATION_ID, unix_timestamp())
                    .map_err(|error| error.to_string())
            });
        match durable {
            Ok(id) => id,
            Err(error) => {
                let id = self.fallback_turn_id.fetch_add(1, Ordering::SeqCst);
                tracing::warn!(%error, turn_id = id, "durable turn allocator unavailable; using process fallback range");
                id
            }
        }
    }
    fn lock(&self) -> MutexGuard<'_, Option<Worker>> {
        self.worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Queues a user utterance, starting (or restarting) the worker
    /// with the current settings when needed.
    ///
    /// # Errors
    ///
    /// Returns an error message when the worker cannot be started.
    pub fn submit<R: Runtime>(&self, app: &AppHandle<R>, text: String) -> Result<(), String> {
        self.with_user_turn(|lease| self.submit_with_lease(app, text, lease))
    }

    fn submit_with_lease<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        text: String,
        lease: UserTurnLease,
    ) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let layout = app.state::<AppDataLayout>();
        let settings = super::settings::load_llm_settings(&layout);
        let context = resolve_worker_context(
            &layout,
            &app.state::<CharacterState>(),
            &settings,
            app.state::<crate::behavior::DarkExpressionSafetyState>()
                .is_paused(),
        );
        let wanted = worker_fingerprint(&settings, &context);

        let mut guard = self.lock();
        let restart = match guard.as_ref() {
            Some(worker) => worker.settings_fingerprint != wanted,
            None => true,
        };
        if restart {
            if let Some(worker) = guard.take() {
                worker.shutdown()?;
            }
            *guard = Some(self.start_worker(app.clone(), &settings, &context)?);
        }
        let Some(worker) = guard.as_ref() else {
            return Err("conversation worker is not available".to_owned());
        };
        let database_path = layout.data.join("parallel-world.sqlite3");
        let turn_id = self.reserve_turn_id(&database_path);
        tracing::info!(turn_id, "chat turn accepted");
        enqueue_submit(&worker.tx, &self.submit_metrics, text, turn_id, lease)
    }

    /// Cancels the in-flight turn (生成途中で停止).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    #[allow(clippy::too_many_lines)]
    fn start_worker<R: Runtime>(
        &self,
        app: AppHandle<R>,
        settings: &LlmSettingsDto,
        context: &ChatWorkerContext,
    ) -> Result<Worker, String> {
        let llm_config = LlmClientConfig {
            base_url: settings.base_url.clone(),
            model: settings.model.clone(),
            api_key: super::settings::load_llm_api_key(settings.provider)?,
            allow_remote: settings.allow_remote,
            timeout: ADAPTER_TIMEOUT,
        };
        let llm = OpenAiCompatClient::new(llm_config.clone()).map_err(|error| error.to_string())?;

        let config = OrchestratorConfig {
            prompt: PromptBuilder {
                system_rules: settings.system_prompt.clone(),
                character_prompt: context.character_prompt.clone(),
            },
            max_history_messages: MAX_HISTORY_MESSAGES,
            strip_emoji: settings.strip_emoji,
        };
        let cancel = Arc::clone(&self.cancel);
        let (tx, context_rx) = sync_channel::<Command>(SUBMIT_QUEUE_CAPACITY);
        let (conversation_tx, rx) = sync_channel::<Command>(CONVERSATION_QUEUE_CAPACITY);
        let (enrichment_wake, enrichment_rx) = sync_channel::<()>(ENRICHMENT_QUEUE_CAPACITY);
        let enrichment_wake = Arc::new(enrichment_wake);
        let enrichment_pending = Arc::new(Mutex::new(None));
        let enrichment_cancel = Arc::new(AtomicBool::new(false));
        let enrichment_tx = EnrichmentSender {
            wake: Arc::clone(&enrichment_wake),
            pending: Arc::clone(&enrichment_pending),
            metrics: Arc::clone(&self.enrichment_metrics),
        };
        let (observation_tx, observation_rx) = sync_channel(OBSERVATION_WRITE_QUEUE_CAPACITY);
        let observation_writer = ObservationWriter { tx: observation_tx };
        let database_path = app
            .state::<AppDataLayout>()
            .data
            .join("parallel-world.sqlite3");
        let companion_state_worker =
            CompanionStateWorker::start(database_path.clone(), DEFAULT_STATE_QUEUE_CAPACITY)
                .map_err(|error| format!("failed to spawn companion state worker: {error}"))?;
        let database = Database::open(&database_path).or_else(|error| {
            tracing::warn!(%error, path = %database_path.display(), "conversation history unavailable; using temporary history");
            Database::open_in_memory()
        }).map_err(|error| format!("failed to initialize conversation history: {error}"))?;
        let history = SqliteConversationHistory::new(database);
        let memory = Database::open(&database_path).or_else(|error| {
            tracing::warn!(%error, "memory database unavailable; using empty temporary context");
            Database::open_in_memory()
        }).map(SqliteMemoryStore::new).map_err(|error| format!("failed to initialize temporary memory context: {error}"))?;
        let state_context_database = Database::open(&database_path).or_else(|error| {
            tracing::warn!(%error, "companion state context unavailable; using temporary state context");
            Database::open_in_memory()
        }).map_err(|error| format!("failed to initialize companion state context: {error}"))?;
        let state_context =
            SqlitePlannedStateContext::new(state_context_database, DEFAULT_CONVERSATION_ID);
        let seed = load_recent_history(&history, DEFAULT_CONVERSATION_ID, MAX_HISTORY_MESSAGES)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "conversation history restore failed; continuing without restored history");
                Vec::new()
            });
        let last_turn_id = history.max_turn_id(DEFAULT_CONVERSATION_ID).unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to restore turn sequence; starting from temporary sequence");
            None
        }).unwrap_or(0);
        let observation_writer_path = database_path.clone();
        let observation_writer_thread = std::thread::Builder::new()
            .name("pw-observation-writer".into())
            .spawn({
                let writer_enrichment = enrichment_tx.clone();
                move || {
                    run_observation_writer(
                        &observation_writer_path,
                        &observation_rx,
                        Some(&writer_enrichment),
                    );
                }
            })
            .map_err(|error| format!("failed to spawn observation writer: {error}"))?;
        let events = PersistentConversationEvents::new_with_enrichment_and_state(
            TauriConversationEvents {
                runtime: AppConversationEventRuntime {
                    app,
                    character_context: context.character_control_context(),
                },
                health: Arc::clone(&self.health),
            },
            history,
            DEFAULT_CONVERSATION_ID,
            Some(enrichment_tx),
            Some(observation_writer),
            Some(companion_state_worker.sender()),
        );

        let context_thread = std::thread::Builder::new()
            .name("pw-memory-context".into())
            .spawn({
                let submit_metrics = Arc::clone(&self.submit_metrics);
                let context_metrics = Arc::clone(&self.context_metrics);
                let conversation_metrics = Arc::clone(&self.conversation_metrics);
                move || {
                    run_context_worker(
                        memory,
                        context_rx,
                        conversation_tx,
                        submit_metrics,
                        context_metrics,
                        conversation_metrics,
                    );
                }
            })
            .map_err(|error| format!("failed to spawn memory context worker: {error}"))?;

        let enrichment_path = database_path.clone();
        let enrichment_worker_wake = Arc::clone(&enrichment_wake);
        let enrichment_thread = std::thread::Builder::new()
            .name("pw-memory-enrichment".into())
            .spawn({
                let metrics = Arc::clone(&self.enrichment_metrics);
                let classifier_cancel = Arc::clone(&enrichment_cancel);
                let worker_cancel = Arc::clone(&enrichment_cancel);
                move || {
                    let enrichment_classifier: Box<dyn MemoryClassifier> =
                        match OpenAiCompatClient::new(llm_config) {
                            Ok(client) => Box::new(LlmMemoryClassifier::new_with_cancel(
                                client,
                                classifier_cancel,
                            )),
                            Err(error) => {
                                tracing::warn!(%error, "memory classifier unavailable; using exact-match fallback");
                                Box::new(UnavailableMemoryClassifier)
                            }
                        };
                    run_enrichment_until_cancelled(
                        &enrichment_path,
                        enrichment_rx,
                        enrichment_worker_wake,
                        enrichment_pending,
                        metrics,
                        HybridConsolidator::new(enrichment_classifier),
                        worker_cancel,
                    );
                }
            })
            .map_err(|error| format!("failed to spawn memory enrichment worker: {error}"))?;

        let conversation_metrics_for_worker = Arc::clone(&self.conversation_metrics);
        let thread = std::thread::Builder::new()
            .name("pw-conversation".into())
            .spawn(move || {
                let response_pipeline = response_pipeline(
                    LexicalResponsePlanner,
                    StateAwareRetriever::new(ExistingContextRetriever, state_context),
                    FixedSurfaceRealizer,
                    PlanningBudget::default(),
                );
                let mut orchestrator =
                    ConversationOrchestrator::new_with_history_after_and_response_pipeline(
                        config,
                        llm,
                        events,
                        cancel,
                        seed,
                        last_turn_id,
                        response_pipeline,
                    );
                while let Ok(command) = rx.recv() {
                    conversation_metrics_for_worker.dequeued();
                    let _ = run_prepared_command(command, |text, turn_id, context| {
                        tracing::info!(turn_id, "chat generation started");
                        orchestrator.recover();
                        orchestrator.submit_user_text_with_context(text, turn_id, context);
                    });
                }
            })
            .map_err(|error| format!("failed to spawn conversation worker: {error}"))?;

        Ok(Worker {
            tx,
            settings_fingerprint: worker_fingerprint(settings, context),
            thread: Some(thread),
            context_thread: Some(context_thread),
            enrichment_thread: Some(enrichment_thread),
            observation_writer_thread: Some(observation_writer_thread),
            companion_state_worker: Some(companion_state_worker),
            enrichment_cancel,
        })
    }
}

/// Appends the loaded character's expression / motion names so the
/// model can emit control JSON the renderer understands.
fn character_instruction(base: &str, capabilities: &CharacterCapabilities) -> String {
    let mut lines = vec![base.to_owned()];
    if !capabilities.expressions.is_empty() {
        lines.push(format!(
            "利用できる表情(emotion): {}",
            capabilities.expressions.join(", ")
        ));
    }
    if !capabilities.motions.is_empty() {
        lines.push(format!(
            "利用できるモーション(motion): {}",
            capabilities.motions.join(", ")
        ));
    }
    lines.join("\n")
}

fn dispatch_character_control(
    capabilities: &CharacterCapabilities,
    renderer: &str,
    control: &ReplyControl,
    mut emit: impl FnMut(&'static str, &str),
) {
    if let Some(emotion) = &control.emotion {
        if capabilities
            .expressions
            .iter()
            .any(|known| known == emotion)
        {
            emit(EXPRESSION_EVENT, emotion);
        } else {
            tracing::warn!(
                renderer = %renderer,
                control = "emotion",
                name = %emotion,
                "ignoring unsupported character control"
            );
        }
    }
    if let Some(motion) = &control.motion {
        if capabilities.motions.iter().any(|known| known == motion) {
            emit(MOTION_EVENT, motion);
        } else {
            tracing::warn!(
                renderer = %renderer,
                control = "motion",
                name = %motion,
                "ignoring unsupported character control"
            );
        }
    }
}

trait ConversationEventRuntime: Send {
    fn character_context(&self) -> Option<crate::commands::character::CharacterControlContext>;
    fn emit_to_webview(
        &self,
        target: &'static str,
        event: &'static str,
        name: &str,
    ) -> Result<(), String>;
    fn emit_chat_message(&self, payload: ChatMessageEventDto);
    fn emit_conversation_state(&self, payload: ConversationStateEventDto);
    fn emit_runtime_health(&self, payload: RuntimeHealthEventDto);
    fn enqueue_speech(&self, turn: TurnId, sentence: &str);
}

struct AppConversationEventRuntime<R: Runtime> {
    app: AppHandle<R>,
    character_context: Option<crate::commands::character::CharacterControlContext>,
}

impl<R: Runtime> ConversationEventRuntime for AppConversationEventRuntime<R> {
    fn character_context(&self) -> Option<crate::commands::character::CharacterControlContext> {
        self.character_context.clone()
    }

    fn emit_to_webview(
        &self,
        target: &'static str,
        event: &'static str,
        name: &str,
    ) -> Result<(), String> {
        self.app
            .emit_to(EventTarget::webview_window(target), event, name)
            .map_err(|error| error.to_string())
    }

    fn emit_chat_message(&self, payload: ChatMessageEventDto) {
        let _ = self.app.emit(MESSAGE_EVENT, payload);
    }

    fn emit_conversation_state(&self, payload: ConversationStateEventDto) {
        let _ = self.app.emit(STATE_EVENT, payload);
    }

    fn emit_runtime_health(&self, payload: RuntimeHealthEventDto) {
        let _ = self.app.emit(RUNTIME_HEALTH_EVENT, payload);
    }

    fn enqueue_speech(&self, turn: TurnId, sentence: &str) {
        self.app
            .state::<crate::tts::TtsService>()
            .enqueue(&self.app, turn, sentence);
    }
}

struct TauriConversationEvents<A: ConversationEventRuntime> {
    runtime: A,
    health: Arc<Mutex<FeatureHealthSupervisor<SystemClock, TimeJitter>>>,
}

impl<A: ConversationEventRuntime> TauriConversationEvents<A> {
    fn emit_health(&self, healthy: bool) {
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transition = if healthy {
            health.record_success()
        } else {
            match health.record_failure(RuntimeFailure::transient(FailureCode::Unavailable)) {
                pw_application::recovery::HealthUpdate::Changed {
                    health, attempts, ..
                } => HealthTransition::Changed { health, attempts },
                pw_application::recovery::HealthUpdate::Unchanged { .. } => {
                    HealthTransition::Unchanged
                }
            }
        };
        if let HealthTransition::Changed { health, attempts } = transition {
            self.runtime
                .emit_runtime_health(RuntimeHealthEventDto::from((&health, attempts)));
        }
    }
    fn emit_message(&self, turn: TurnId, role: ChatRoleDto, text: &str) {
        let payload = ChatMessageEventDto {
            schema_version: SCHEMA_VERSION,
            turn_id: turn.value(),
            message_id: None,
            role,
            text: text.to_owned(),
        };
        // Single broadcast: see the event conventions note.
        self.runtime.emit_chat_message(payload);
    }

    fn emit_state(&self, state: ConversationState, message: Option<String>) {
        let dto = match state {
            ConversationState::Starting => pw_contracts::ConversationStateDto::Starting,
            ConversationState::Idle => pw_contracts::ConversationStateDto::Idle,
            ConversationState::Listening => pw_contracts::ConversationStateDto::Listening,
            ConversationState::Transcribing => pw_contracts::ConversationStateDto::Transcribing,
            ConversationState::Thinking => pw_contracts::ConversationStateDto::Thinking,
            ConversationState::Speaking => pw_contracts::ConversationStateDto::Speaking,
            ConversationState::Muted => pw_contracts::ConversationStateDto::Muted,
            ConversationState::Interrupting => pw_contracts::ConversationStateDto::Interrupting,
            ConversationState::Cancelled => pw_contracts::ConversationStateDto::Cancelled,
            ConversationState::Recovering => pw_contracts::ConversationStateDto::Recovering,
            ConversationState::SttUnavailable => pw_contracts::ConversationStateDto::SttUnavailable,
            ConversationState::LlmUnavailable => pw_contracts::ConversationStateDto::LlmUnavailable,
            ConversationState::TtsUnavailable => pw_contracts::ConversationStateDto::TtsUnavailable,
            ConversationState::RendererUnavailable => {
                pw_contracts::ConversationStateDto::RendererUnavailable
            }
        };
        let payload = ConversationStateEventDto {
            schema_version: SCHEMA_VERSION,
            state: dto,
            message,
        };
        self.runtime.emit_conversation_state(payload);
    }
}

impl<A: ConversationEventRuntime> ConversationEvents for TauriConversationEvents<A> {
    fn on_state(&self, state: ConversationState) {
        self.emit_state(state, None);
    }

    fn on_user_message(&self, turn: TurnId, text: &str) {
        tracing::info!(
            turn_id = turn.value(),
            user_chars = text.chars().count(),
            "chat user message emitted"
        );
        self.emit_message(turn, ChatRoleDto::User, text);
    }

    fn on_control(&self, _turn: TurnId, control: &ReplyControl) {
        let Some(context) = self.runtime.character_context() else {
            if let Some(emotion) = &control.emotion {
                tracing::warn!(
                    renderer = "unavailable",
                    control = "emotion",
                    name = %emotion,
                    "ignoring character control before capabilities are loaded"
                );
            }
            if let Some(motion) = &control.motion {
                tracing::warn!(
                    renderer = "unavailable",
                    control = "motion",
                    name = %motion,
                    "ignoring character control before capabilities are loaded"
                );
            }
            return;
        };
        dispatch_character_control(
            &context.capabilities,
            context.renderer,
            control,
            |event, name| {
                if let Err(error) = self.runtime.emit_to_webview(CHARACTER_WEBVIEW, event, name) {
                    tracing::warn!(
                        %error,
                        renderer = %context.renderer,
                        control = %event,
                        name = %name,
                        "failed to emit character control"
                    );
                }
            },
        );
    }

    fn on_sentence(&self, turn: TurnId, sentence: &str) {
        let sentence = redact_persistent_content(sentence);
        self.emit_message(turn, ChatRoleDto::Assistant, &sentence);
        // Sentence-level read-ahead: synthesis of this sentence runs
        // while earlier ones are still playing (基本設計 8章).
        self.runtime.enqueue_speech(turn, &sentence);
    }

    fn on_reply_complete(&self, turn: TurnId, speech_text: &str) {
        tracing::info!(
            turn_id = turn.value(),
            assistant_chars = speech_text.chars().count(),
            "chat reply completed"
        );
        self.emit_health(true);
    }

    fn on_cancelled(&self, _turn: TurnId) {}

    fn on_error(&self, _turn: TurnId, message: &str) {
        self.emit_health(false);
        self.emit_state(ConversationState::LlmUnavailable, Some(message.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    use super::*;
    use pw_application::PortError;
    use pw_application::conversation::LlmClient;
    use pw_application::history::{
        ConversationHistory, StoredConversation, StoredMessage, StoredTurn,
    };
    use pw_application::memory::{
        HybridConsolidator, MemoryAction, MemoryCandidate, MemoryClassifier, MemoryRecord,
        MemoryState, ProposedAction, StoredSummary,
    };
    use pw_contracts::MotionGroupDto;
    use pw_domain::reply::TurnTracker;
    use pw_storage::{Database, SqliteConversationHistory};

    fn test_lease() -> UserTurnLease {
        UserTurnLease::new(Arc::new(InteractionGate::new()))
    }

    #[test]
    fn production_llm_timeout_allows_local_streaming_inference() {
        assert!(ADAPTER_TIMEOUT >= std::time::Duration::from_secs(30));
    }

    fn static_capabilities() -> crate::character::CharacterCapabilities {
        crate::character::CharacterCapabilities {
            expressions: vec!["neutral".into(), "happy".into()],
            motions: Vec::new(),
        }
    }

    fn live2d_capabilities() -> crate::character::CharacterCapabilities {
        crate::character::CharacterCapabilities {
            expressions: vec!["neutral".into(), "happy".into()],
            motions: vec!["Idle".into(), "Tap".into()],
        }
    }

    #[test]
    fn static_character_instruction_lists_expressions_without_motion_line() {
        let instruction = character_instruction("base", &static_capabilities());

        assert!(instruction.contains("利用できる表情(emotion): neutral, happy"));
        assert!(!instruction.contains("利用できるモーション(motion):"));
    }

    #[test]
    fn live2d_character_instruction_lists_expressions_and_motions() {
        let instruction = character_instruction("base", &live2d_capabilities());

        assert!(instruction.contains("利用できる表情(emotion): neutral, happy"));
        assert!(instruction.contains("利用できるモーション(motion): Idle, Tap"));
    }

    fn resolved_persona_for_test(id: &str, prompt: &str) -> crate::behavior::ResolvedPersonaPrompt {
        crate::behavior::ResolvedPersonaPrompt {
            character_id: Some(id.into()),
            character_prompt: prompt.into(),
            source: crate::behavior::PersonaPromptSource::Persona,
            fingerprint: serde_json::to_string(&(Some(id), prompt)).unwrap(),
        }
    }

    fn character_snapshot_for_test(
        id: &str,
        renderer: &'static str,
        capabilities: CharacterCapabilities,
    ) -> crate::commands::character::CharacterSnapshot {
        crate::commands::character::CharacterSnapshot {
            id: id.into(),
            renderer,
            capabilities,
        }
    }

    #[test]
    fn chat_worker_fingerprint_changes_for_persona_character_or_capabilities() {
        let settings = crate::chat::default_llm_settings();
        let base = prepare_worker_context(
            resolved_persona_for_test("epsilon", "persona"),
            Some(character_snapshot_for_test(
                "epsilon",
                "live2d",
                live2d_capabilities(),
            )),
        );
        let same = base.clone();
        let edited = prepare_worker_context(
            resolved_persona_for_test("epsilon", "persona edited"),
            Some(character_snapshot_for_test(
                "epsilon",
                "live2d",
                live2d_capabilities(),
            )),
        );
        let changed_character = prepare_worker_context(
            resolved_persona_for_test("zeta", "persona"),
            Some(character_snapshot_for_test(
                "zeta",
                "static_image",
                static_capabilities(),
            )),
        );
        let changed_capabilities = prepare_worker_context(
            resolved_persona_for_test("epsilon", "persona"),
            Some(character_snapshot_for_test(
                "epsilon",
                "live2d",
                CharacterCapabilities {
                    expressions: vec!["neutral".into()],
                    motions: vec!["Idle".into()],
                },
            )),
        );

        assert_eq!(
            worker_fingerprint(&settings, &base),
            worker_fingerprint(&settings, &same)
        );
        assert_ne!(
            worker_fingerprint(&settings, &base),
            worker_fingerprint(&settings, &edited)
        );
        assert_ne!(
            worker_fingerprint(&settings, &base),
            worker_fingerprint(&settings, &changed_character)
        );
        assert_ne!(
            worker_fingerprint(&settings, &base),
            worker_fingerprint(&settings, &changed_capabilities)
        );
    }

    #[test]
    fn chat_worker_prompt_uses_persona_and_capabilities_from_matching_snapshot() {
        let context = prepare_worker_context(
            resolved_persona_for_test("epsilon", "persona data"),
            Some(character_snapshot_for_test(
                "epsilon",
                "live2d",
                live2d_capabilities(),
            )),
        );

        assert!(context.character_prompt.starts_with("persona data\n"));
        assert!(context.character_prompt.contains("neutral, happy"));
        assert!(context.character_prompt.contains("Idle, Tap"));
    }

    #[test]
    fn chat_worker_omits_capabilities_when_snapshot_identity_does_not_match_persona() {
        let context = prepare_worker_context(
            resolved_persona_for_test("epsilon", "legacy rollback"),
            Some(character_snapshot_for_test(
                "stale-character",
                "live2d",
                live2d_capabilities(),
            )),
        );

        assert_eq!(context.character_prompt, "legacy rollback");
        assert_eq!(context.character, None);
    }

    #[test]
    fn chat_character_resolution_failure_uses_legacy_without_persona_write() {
        let root =
            std::env::temp_dir().join(format!("pw-chat-character-failure-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let layout = AppDataLayout::under(root.clone());
        layout.create_all().unwrap();
        let state = CharacterState::default();
        state.cache_manifest(live2d_character()).unwrap();
        let mut settings = crate::chat::default_llm_settings();
        settings.character_prompt = "legacy fallback".into();

        let context = resolve_worker_context(&layout, &state, &settings, false);

        assert_eq!(context.character_prompt, "legacy fallback");
        assert_eq!(context.character, None);
        assert!(!layout.config.join("personas.json").exists());
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let events = recording_events_with_context(
            context.character_control_context(),
            Arc::clone(&attempts),
            false,
            Arc::new(Mutex::new(Vec::new())),
        );
        events.on_control(
            TurnTracker::new().begin_turn(),
            &ReplyControl {
                emotion: Some("happy".into()),
                intensity: None,
                motion: Some("Tap".into()),
            },
        );
        assert!(attempts.lock().unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedCharacterEvent {
        target: String,
        event: String,
        name: String,
    }

    struct RecordingConversationRuntime {
        character_context: Option<crate::commands::character::CharacterControlContext>,
        attempts: Arc<Mutex<Vec<RecordedCharacterEvent>>>,
        fail_emit: bool,
        messages: Arc<Mutex<Vec<ChatMessageEventDto>>>,
        speech: Arc<Mutex<Vec<(TurnId, String)>>>,
    }

    impl ConversationEventRuntime for RecordingConversationRuntime {
        fn character_context(&self) -> Option<crate::commands::character::CharacterControlContext> {
            self.character_context.clone()
        }

        fn emit_to_webview(
            &self,
            target: &'static str,
            event: &'static str,
            name: &str,
        ) -> Result<(), String> {
            self.attempts.lock().unwrap().push(RecordedCharacterEvent {
                target: target.into(),
                event: event.into(),
                name: name.into(),
            });
            if self.fail_emit {
                Err("injected character event failure".into())
            } else {
                Ok(())
            }
        }

        fn emit_chat_message(&self, payload: ChatMessageEventDto) {
            self.messages.lock().unwrap().push(payload);
        }

        fn emit_conversation_state(&self, _payload: ConversationStateEventDto) {}

        fn emit_runtime_health(&self, _payload: RuntimeHealthEventDto) {}

        fn enqueue_speech(&self, turn: TurnId, sentence: &str) {
            self.speech.lock().unwrap().push((turn, sentence.into()));
        }
    }

    fn live2d_character() -> crate::character::ResolvedCharacter {
        crate::character::ResolvedCharacter {
            id: "live2d-test".into(),
            display_name: "Live2D Test".into(),
            profile_root: PathBuf::from("C:/characters/live2d-test"),
            renderer: crate::character::ResolvedRenderer::Live2d {
                model_path: PathBuf::from("C:/characters/live2d-test/model.model3.json"),
                default_expression: Some("neutral".into()),
                expressions: vec!["neutral".into(), "happy".into()],
                motion_groups: vec![MotionGroupDto {
                    name: "Tap".into(),
                    motion_count: 1,
                }],
            },
        }
    }

    fn static_character() -> crate::character::ResolvedCharacter {
        crate::character::ResolvedCharacter {
            id: "static-test".into(),
            display_name: "Static Test".into(),
            profile_root: PathBuf::from("C:/characters/static-test"),
            renderer: crate::character::ResolvedRenderer::StaticImage {
                default_expression: "neutral".into(),
                expressions: vec![
                    crate::character::ResolvedStaticExpression {
                        name: "neutral".into(),
                        image_path: PathBuf::from("C:/characters/static-test/neutral.png"),
                    },
                    crate::character::ResolvedStaticExpression {
                        name: "happy".into(),
                        image_path: PathBuf::from("C:/characters/static-test/happy.png"),
                    },
                ],
                width: 512,
                height: 1024,
            },
        }
    }

    fn test_health() -> Arc<Mutex<FeatureHealthSupervisor<SystemClock, TimeJitter>>> {
        Arc::new(Mutex::new(FeatureHealthSupervisor::new(
            RuntimeFeature::LanguageModel,
            SystemClock,
            TimeJitter::default(),
        )))
    }

    fn recording_events(
        state: &CharacterState,
        attempts: Arc<Mutex<Vec<RecordedCharacterEvent>>>,
        fail_emit: bool,
        speech: Arc<Mutex<Vec<(TurnId, String)>>>,
    ) -> TauriConversationEvents<RecordingConversationRuntime> {
        recording_events_with_context(state.control_context(), attempts, fail_emit, speech)
    }

    fn recording_events_with_context(
        character_context: Option<crate::commands::character::CharacterControlContext>,
        attempts: Arc<Mutex<Vec<RecordedCharacterEvent>>>,
        fail_emit: bool,
        speech: Arc<Mutex<Vec<(TurnId, String)>>>,
    ) -> TauriConversationEvents<RecordingConversationRuntime> {
        TauriConversationEvents {
            runtime: RecordingConversationRuntime {
                character_context,
                attempts,
                fail_emit,
                messages: Arc::new(Mutex::new(Vec::new())),
                speech,
            },
            health: test_health(),
        }
    }

    #[test]
    fn assistant_sentence_uses_one_redacted_value_for_ui_and_tts_without_changing_user_echo() {
        let messages = Arc::new(Mutex::new(Vec::new()));
        let speech = Arc::new(Mutex::new(Vec::new()));
        let events = TauriConversationEvents {
            runtime: RecordingConversationRuntime {
                character_context: None,
                attempts: Arc::new(Mutex::new(Vec::new())),
                fail_emit: false,
                messages: Arc::clone(&messages),
                speech: Arc::clone(&speech),
            },
            health: test_health(),
        };
        let turn = TurnTracker::new().begin_turn();
        let user = "password is hunter2";
        let assistant = "password is hunter2; opaque=`ABCDEF234567ABCDEF234567`";

        events.on_user_message(turn, user);
        events.on_sentence(turn, assistant);

        let messages = messages.lock().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, ChatRoleDto::User);
        assert_eq!(messages[0].text, user);
        assert_eq!(messages[1].role, ChatRoleDto::Assistant);
        assert_eq!(
            messages[1].text,
            "password is [REDACTED]; opaque=`[REDACTED]`"
        );
        let speech = speech.lock().unwrap();
        assert_eq!(speech.as_slice(), [(turn, messages[1].text.clone())]);
    }

    #[test]
    fn unknown_emotion_uses_cached_state_and_sends_no_webview_event() {
        let state = Arc::new(CharacterState::default());
        state.cache_manifest(static_character()).unwrap();
        let before = state.control_context();
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let events = recording_events(
            &state,
            Arc::clone(&attempts),
            false,
            Arc::new(Mutex::new(Vec::new())),
        );
        let turn = TurnTracker::new().begin_turn();

        events.on_control(
            turn,
            &ReplyControl {
                emotion: Some("surprised".into()),
                intensity: None,
                motion: None,
            },
        );

        assert!(attempts.lock().unwrap().is_empty());
        assert_eq!(state.control_context(), before);
    }

    #[test]
    fn static_motion_uses_cached_state_and_sends_no_webview_event() {
        let state = Arc::new(CharacterState::default());
        state.cache_manifest(static_character()).unwrap();
        let before = state.control_context();
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let events = recording_events(
            &state,
            Arc::clone(&attempts),
            false,
            Arc::new(Mutex::new(Vec::new())),
        );
        let turn = TurnTracker::new().begin_turn();

        events.on_control(
            turn,
            &ReplyControl {
                emotion: None,
                intensity: None,
                motion: Some("Tap".into()),
            },
        );

        assert!(attempts.lock().unwrap().is_empty());
        assert_eq!(state.control_context(), before);
    }

    #[test]
    fn valid_live2d_controls_target_only_the_character_webview() {
        let state = Arc::new(CharacterState::default());
        state.cache_manifest(live2d_character()).unwrap();
        let before = state.control_context();
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let events = recording_events(
            &state,
            Arc::clone(&attempts),
            false,
            Arc::new(Mutex::new(Vec::new())),
        );
        let turn = TurnTracker::new().begin_turn();

        events.on_control(
            turn,
            &ReplyControl {
                emotion: Some("happy".into()),
                intensity: Some(0.75),
                motion: Some("Tap".into()),
            },
        );

        assert_eq!(
            *attempts.lock().unwrap(),
            [
                RecordedCharacterEvent {
                    target: "character".into(),
                    event: EXPRESSION_EVENT.into(),
                    name: "happy".into(),
                },
                RecordedCharacterEvent {
                    target: "character".into(),
                    event: MOTION_EVENT.into(),
                    name: "Tap".into(),
                },
            ]
        );
        assert_eq!(state.control_context(), before);
    }

    #[test]
    fn worker_control_keeps_character_generation_captured_with_its_prompt() {
        let state = Arc::new(CharacterState::default());
        state.cache_manifest(live2d_character()).unwrap();
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let events = recording_events(
            &state,
            Arc::clone(&attempts),
            false,
            Arc::new(Mutex::new(Vec::new())),
        );
        state.cache_manifest(static_character()).unwrap();
        let turn = TurnTracker::new().begin_turn();

        events.on_control(
            turn,
            &ReplyControl {
                emotion: Some("happy".into()),
                intensity: None,
                motion: Some("Tap".into()),
            },
        );

        assert_eq!(attempts.lock().unwrap().len(), 2);
        assert_eq!(attempts.lock().unwrap()[0].event, EXPRESSION_EVENT);
        assert_eq!(attempts.lock().unwrap()[1].event, MOTION_EVENT);
    }

    struct ScriptedLlm(&'static str);

    impl LlmClient for ScriptedLlm {
        fn stream_chat(
            &mut self,
            _messages: &[ChatMessage],
            _cancel: &AtomicBool,
            on_delta: &mut dyn FnMut(&str),
        ) -> Result<(), PortError> {
            on_delta(self.0);
            Ok(())
        }
    }

    #[test]
    fn character_emit_failure_does_not_stop_sentence_or_tts_enqueue() {
        let state = Arc::new(CharacterState::default());
        state.cache_manifest(live2d_character()).unwrap();
        let before = state.control_context();
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let speech = Arc::new(Mutex::new(Vec::new()));
        let events = recording_events(&state, Arc::clone(&attempts), true, Arc::clone(&speech));
        let mut orchestrator = ConversationOrchestrator::new(
            OrchestratorConfig {
                prompt: PromptBuilder {
                    system_rules: "rules".into(),
                    character_prompt: "character".into(),
                },
                max_history_messages: 4,
                strip_emoji: true,
            },
            ScriptedLlm("{\"emotion\":\"happy\"}\n続きます。"),
            events,
            Arc::new(AtomicBool::new(false)),
        );

        orchestrator.submit_user_text("話して");

        assert_eq!(attempts.lock().unwrap().len(), 1);
        assert_eq!(attempts.lock().unwrap()[0].target, "character");
        assert_eq!(speech.lock().unwrap().len(), 1);
        assert_eq!(speech.lock().unwrap()[0].1, "続きます。");
        assert_eq!(state.control_context(), before);
    }

    #[test]
    fn destructive_operation_excludes_competing_operation_until_commit_boundary() {
        let service = Arc::new(ChatService::default());
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first = Arc::clone(&service);
        let thread = std::thread::spawn(move || {
            first
                .with_exclusive_reset(|| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap();
        });
        entered_rx.recv().unwrap();
        let second = Arc::clone(&service);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            second.with_exclusive_reset(|| Ok(())).unwrap();
            done_tx.send(()).unwrap();
        });
        assert!(
            done_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err()
        );
        release_tx.send(()).unwrap();
        done_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        thread.join().unwrap();
        waiter.join().unwrap();
    }

    #[test]
    fn interaction_gate_lease_invalidates_epoch_and_releases_exactly_once() {
        let gate = Arc::new(pw_application::behavior::proactive::InteractionGate::new());
        let captured = gate.capture_idle_epoch().unwrap();

        let lease = UserTurnLease::new(Arc::clone(&gate));

        assert!(gate.is_cancelled(captured));
        assert_eq!(gate.capture_idle_epoch(), None);
        drop(lease);
        let after = gate.capture_idle_epoch().expect("lease released");
        assert_ne!(after, captured);
    }

    #[test]
    fn interaction_gate_multiple_queued_leases_are_counted_independently() {
        let gate = Arc::new(pw_application::behavior::proactive::InteractionGate::new());
        let first = UserTurnLease::new(Arc::clone(&gate));
        let second = UserTurnLease::new(Arc::clone(&gate));

        drop(first);
        assert_eq!(gate.capture_idle_epoch(), None);
        drop(second);
        assert!(gate.capture_idle_epoch().is_some());
    }

    #[test]
    fn interaction_gate_prepared_command_holds_lease_until_command_drop() {
        let gate = Arc::new(pw_application::behavior::proactive::InteractionGate::new());
        let command = Command::Prepared(
            "text".into(),
            1,
            MemoryContext::default(),
            UserTurnLease::new(Arc::clone(&gate)),
        );

        assert_eq!(gate.capture_idle_epoch(), None);
        drop(command);
        assert!(gate.capture_idle_epoch().is_some());
    }

    #[test]
    fn interaction_gate_thread_unwind_drops_prepared_lease() {
        let gate = Arc::new(pw_application::behavior::proactive::InteractionGate::new());
        let worker_gate = Arc::clone(&gate);
        let thread = std::thread::spawn(move || {
            let _command = Command::Prepared(
                "text".into(),
                1,
                MemoryContext::default(),
                UserTurnLease::new(worker_gate),
            );
            panic!("injected worker unwind");
        });

        assert!(thread.join().is_err());
        assert!(gate.capture_idle_epoch().is_some());
    }

    #[test]
    fn interaction_gate_prepared_callback_holds_until_success_or_error_returns() {
        for succeeds in [true, false] {
            let gate = Arc::new(InteractionGate::new());
            let command = Command::Prepared(
                "text".into(),
                1,
                MemoryContext::default(),
                UserTurnLease::new(Arc::clone(&gate)),
            );
            let result = run_prepared_command(command, |_, _, _| {
                assert_eq!(gate.capture_idle_epoch(), None);
                if succeeds { Ok(()) } else { Err("cancelled") }
            })
            .expect("prepared command");

            assert_eq!(result.is_ok(), succeeds);
            assert!(gate.capture_idle_epoch().is_some());
        }
    }

    #[test]
    fn interaction_gate_context_failure_paths_release_lease() {
        for conversation_connected in [true, false] {
            let gate = Arc::new(InteractionGate::new());
            let (tx, rx) = sync_channel(1);
            let (conversation_tx, conversation_rx) = sync_channel(1);
            let conversation_rx = conversation_connected.then_some(conversation_rx);
            let worker = std::thread::spawn(move || {
                run_context_worker(
                    FailingMemory::new(),
                    rx,
                    conversation_tx,
                    Arc::new(QueueMetrics::new("test_submit", 1)),
                    Arc::new(QueueMetrics::new("test_context", 1)),
                    Arc::new(QueueMetrics::new("test_conversation", 1)),
                );
            });
            tx.send(Command::Submit(
                "query".into(),
                1,
                UserTurnLease::new(Arc::clone(&gate)),
            ))
            .unwrap();
            drop(tx);
            if let Some(conversation_rx) = conversation_rx {
                let prepared = conversation_rx.recv().unwrap();
                assert_eq!(gate.capture_idle_epoch(), None);
                drop(prepared);
            }
            worker.join().unwrap();
            assert!(gate.capture_idle_epoch().is_some());
        }
    }

    #[test]
    fn interaction_gate_disconnected_submit_queue_releases_rejected_lease() {
        let gate = Arc::new(InteractionGate::new());
        let captured = gate.capture_idle_epoch().unwrap();
        let (tx, rx) = sync_channel(1);
        drop(rx);

        let error = enqueue_submit(
            &tx,
            &QueueMetrics::new("test_submit", 1),
            "rejected".into(),
            1,
            UserTurnLease::new(Arc::clone(&gate)),
        )
        .unwrap_err();

        assert_eq!(error, "conversation worker is not available");
        assert!(gate.is_cancelled(captured));
        assert!(gate.capture_idle_epoch().is_some());
    }

    #[test]
    fn interaction_gate_health_rejection_still_advances_epoch_and_releases() {
        let service = ChatService::default();
        service
            .health
            .lock()
            .unwrap()
            .record_failure(RuntimeFailure::permanent(FailureCode::InvalidConfiguration));
        let gate = service.interaction_gate();
        let captured = gate.capture_idle_epoch().unwrap();

        let error = service
            .require_healthy(UserTurnLease::new(Arc::clone(&gate)))
            .expect_err("health gate must reject");

        assert!(error.contains("recovering"));
        assert!(gate.is_cancelled(captured));
        assert!(gate.capture_idle_epoch().is_some());
    }

    #[test]
    fn interaction_gate_worker_start_failure_advances_epoch_and_releases() {
        let service = ChatService::default();
        let gate = service.interaction_gate();
        let captured = gate.capture_idle_epoch().unwrap();

        let error = service
            .with_user_turn(|_lease| Err::<(), _>("injected worker start failure".to_owned()))
            .unwrap_err();

        assert_eq!(error, "injected worker start failure");
        assert!(gate.is_cancelled(captured));
        assert!(gate.capture_idle_epoch().is_some());
    }

    #[test]
    fn interaction_gate_reset_releases_queued_worker_lease() {
        let service = ChatService::default();
        let gate = service.interaction_gate();
        let (tx, rx) = sync_channel(1);
        tx.send(Command::Submit(
            "queued".into(),
            1,
            UserTurnLease::new(Arc::clone(&gate)),
        ))
        .unwrap();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            release_rx.recv().unwrap();
            while rx.recv().is_ok() {}
        });
        *service.lock() = Some(Worker {
            tx,
            settings_fingerprint: "test".into(),
            thread: Some(thread),
            context_thread: None,
            enrichment_thread: None,
            observation_writer_thread: None,
            companion_state_worker: None,
            enrichment_cancel: Arc::new(AtomicBool::new(false)),
        });
        assert_eq!(gate.capture_idle_epoch(), None);

        release_tx.send(()).unwrap();
        service.reset().unwrap();

        assert!(gate.capture_idle_epoch().is_some());
    }

    #[test]
    fn shutdown_failure_always_restores_cancel_flag() {
        let service = ChatService::default();
        let (tx, _rx) = sync_channel(8);
        *service.lock() = Some(Worker {
            tx,
            settings_fingerprint: String::new(),
            thread: Some(std::thread::spawn(|| panic!("injected"))),
            context_thread: None,
            enrichment_thread: None,
            observation_writer_thread: None,
            companion_state_worker: None,
            enrichment_cancel: Arc::new(AtomicBool::new(false)),
        });
        assert!(service.reset().is_err());
        assert!(!service.cancel.load(Ordering::SeqCst));
    }

    #[derive(Default)]
    struct NoopEvents {
        errors: Mutex<Vec<String>>,
    }

    impl ConversationEvents for NoopEvents {
        fn on_state(&self, _: ConversationState) {}
        fn on_user_message(&self, _: TurnId, _: &str) {}
        fn on_control(&self, _: TurnId, _: &ReplyControl) {}
        fn on_sentence(&self, _: TurnId, _: &str) {}
        fn on_reply_complete(&self, _: TurnId, _: &str) {}
        fn on_cancelled(&self, _: TurnId) {}
        fn on_error(&self, _: TurnId, message: &str) {
            self.errors.lock().unwrap().push(message.to_owned());
        }
    }

    struct ExactFakeClassifier;

    impl MemoryClassifier for ExactFakeClassifier {
        fn classify(
            &mut self,
            statement: &str,
            candidates: &[MemoryCandidate],
        ) -> Result<ProposedAction, pw_application::PortError> {
            Ok(candidates
                .iter()
                .find(|candidate| candidate.content == statement)
                .map_or_else(
                    || ProposedAction::Add {
                        content: statement.to_owned(),
                    },
                    |candidate| ProposedAction::Reinforce {
                        memory_id: candidate.id,
                    },
                ))
        }
    }

    struct PinFakeClassifier;

    impl MemoryClassifier for PinFakeClassifier {
        fn classify(
            &mut self,
            statement: &str,
            candidates: &[MemoryCandidate],
        ) -> Result<ProposedAction, pw_application::PortError> {
            if has_explicit_pin_intent(statement) {
                return Ok(ProposedAction::Pin {
                    memory_id: candidates.first().map(|candidate| candidate.id),
                    content: candidates.is_empty().then(|| "私は猫が好きです".to_owned()),
                });
            }
            Ok(ProposedAction::Add {
                content: statement.to_owned(),
            })
        }
    }

    struct ContradictionFakeClassifier;

    impl MemoryClassifier for ContradictionFakeClassifier {
        fn classify(
            &mut self,
            statement: &str,
            candidates: &[MemoryCandidate],
        ) -> Result<ProposedAction, pw_application::PortError> {
            Ok(candidates.first().map_or_else(
                || ProposedAction::Add {
                    content: statement.to_owned(),
                },
                |candidate| ProposedAction::Supersede {
                    old_memory_id: candidate.id,
                    content: statement.to_owned(),
                },
            ))
        }
    }

    struct SummaryCursorRecordingClassifier {
        database_path: PathBuf,
        seen: Arc<Mutex<Vec<Option<i64>>>>,
    }

    impl MemoryClassifier for SummaryCursorRecordingClassifier {
        fn classify(
            &mut self,
            _: &str,
            _: &[MemoryCandidate],
        ) -> Result<ProposedAction, pw_application::PortError> {
            let store = SqliteMemoryStore::new(Database::open(&self.database_path).unwrap());
            let through = store
                .load_summary(DEFAULT_CONVERSATION_ID)
                .unwrap()
                .map(|summary| summary.through_message_id);
            self.seen.lock().unwrap().push(through);
            Ok(ProposedAction::Ignore)
        }
    }

    #[test]
    fn unavailable_classifier_uses_only_exact_match_fallback() {
        let candidate = MemoryCandidate {
            id: 7,
            revision: Some(1),
            content: "私は猫が好きです".into(),
            state: MemoryState::Active,
            pinned: false,
            mention_count: 1,
            last_seen_at: 1,
            lexical_relevance: 1.0,
            strength: 1.0,
        };
        let mut consolidator = HybridConsolidator::new(UnavailableMemoryClassifier);
        assert_eq!(
            consolidator.decide("私は猫が好きです。", std::slice::from_ref(&candidate)),
            MemoryAction::Reinforce {
                memory_id: 7,
                pin: false,
            }
        );
        assert_eq!(
            consolidator.decide("私は犬が好きです", &[candidate]),
            MemoryAction::Ignore
        );
    }

    type MaintenanceCalls = Arc<Mutex<Vec<(i64, usize)>>>;

    struct FailingMemory {
        maintenance_calls: Option<MaintenanceCalls>,
        active_candidates: Option<Vec<MemoryCandidate>>,
        recalled_ids: Option<Arc<Mutex<Vec<i64>>>>,
        recall_fails: bool,
    }

    impl FailingMemory {
        fn new() -> Self {
            Self {
                maintenance_calls: None,
                active_candidates: None,
                recalled_ids: None,
                recall_fails: false,
            }
        }

        fn tracking_maintenance(calls: MaintenanceCalls) -> Self {
            Self {
                maintenance_calls: Some(calls),
                active_candidates: None,
                recalled_ids: None,
                recall_fails: false,
            }
        }

        fn recording_recall(
            candidates: Vec<MemoryCandidate>,
            recalled_ids: Arc<Mutex<Vec<i64>>>,
            recall_fails: bool,
        ) -> Self {
            Self {
                maintenance_calls: None,
                active_candidates: Some(candidates),
                recalled_ids: Some(recalled_ids),
                recall_fails,
            }
        }
    }

    impl MemoryStore for FailingMemory {
        fn load_summary(
            &self,
            _: &str,
        ) -> Result<Option<StoredSummary>, pw_application::PortError> {
            Err(pw_application::PortError("failed".into()))
        }
        fn upsert_summary(
            &mut self,
            _: &str,
            _: &str,
            _: i64,
            _: i64,
        ) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn upsert_memory(
            &mut self,
            _: Option<&str>,
            _: &str,
            _: i64,
        ) -> Result<i64, pw_application::PortError> {
            unreachable!()
        }
        fn update_memory(
            &mut self,
            _: i64,
            _: &str,
            _: i64,
        ) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn delete_memory(&mut self, _: i64) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn delete_summary(&mut self, _: &str) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn search(
            &self,
            _: &str,
            _: usize,
        ) -> Result<Vec<MemoryRecord>, pw_application::PortError> {
            Err(pw_application::PortError("failed".into()))
        }
        fn search_active_for_prompt(
            &self,
            _: &str,
            _: usize,
            _: i64,
        ) -> Result<Vec<MemoryCandidate>, pw_application::PortError> {
            self.active_candidates
                .clone()
                .ok_or_else(|| pw_application::PortError("failed".into()))
        }
        fn record_recalled(
            &mut self,
            ids: &[i64],
            _: &EvidenceSource,
            _: i64,
        ) -> Result<(), pw_application::PortError> {
            if let Some(recalled_ids) = &self.recalled_ids {
                recalled_ids.lock().unwrap().extend_from_slice(ids);
            }
            if self.recall_fails {
                Err(pw_application::PortError("failed".into()))
            } else {
                Ok(())
            }
        }
        fn run_maintenance(
            &mut self,
            now: i64,
            limit: usize,
        ) -> Result<pw_application::memory::MaintenanceReport, pw_application::PortError> {
            if let Some(calls) = &self.maintenance_calls {
                calls.lock().unwrap().push((now, limit));
            }
            Err(pw_application::PortError("failed".into()))
        }
    }

    struct SlowMemory;
    impl MemoryStore for SlowMemory {
        fn load_summary(
            &self,
            _: &str,
        ) -> Result<Option<StoredSummary>, pw_application::PortError> {
            Ok(None)
        }
        fn upsert_summary(
            &mut self,
            _: &str,
            _: &str,
            _: i64,
            _: i64,
        ) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn upsert_memory(
            &mut self,
            _: Option<&str>,
            _: &str,
            _: i64,
        ) -> Result<i64, pw_application::PortError> {
            unreachable!()
        }
        fn update_memory(
            &mut self,
            _: i64,
            _: &str,
            _: i64,
        ) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn delete_memory(&mut self, _: i64) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn delete_summary(&mut self, _: &str) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn search(
            &self,
            _: &str,
            _: usize,
        ) -> Result<Vec<MemoryRecord>, pw_application::PortError> {
            std::thread::sleep(std::time::Duration::from_millis(200));
            Ok(Vec::new())
        }
    }

    struct RemainingMaintenance {
        calls: Arc<AtomicU64>,
    }

    impl MemoryStore for RemainingMaintenance {
        fn load_summary(&self, _: &str) -> Result<Option<StoredSummary>, PortError> {
            Ok(None)
        }
        fn upsert_summary(&mut self, _: &str, _: &str, _: i64, _: i64) -> Result<(), PortError> {
            unreachable!()
        }
        fn upsert_memory(&mut self, _: Option<&str>, _: &str, _: i64) -> Result<i64, PortError> {
            unreachable!()
        }
        fn update_memory(&mut self, _: i64, _: &str, _: i64) -> Result<(), PortError> {
            unreachable!()
        }
        fn delete_memory(&mut self, _: i64) -> Result<(), PortError> {
            unreachable!()
        }
        fn delete_summary(&mut self, _: &str) -> Result<(), PortError> {
            unreachable!()
        }
        fn search(&self, _: &str, _: usize) -> Result<Vec<MemoryRecord>, PortError> {
            Ok(Vec::new())
        }
        fn run_maintenance(
            &mut self,
            _: i64,
            _: usize,
        ) -> Result<pw_application::memory::MaintenanceReport, PortError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(pw_application::memory::MaintenanceReport {
                dormant: 0,
                deleted: 0,
                remaining: call == 0,
            })
        }
    }

    #[test]
    fn memory_lookup_failure_keeps_the_conversation_path_available() {
        let mut memory = FailingMemory::new();
        assert_eq!(
            load_memory_context(&mut memory, "hello", 1, 100),
            MemoryContext::default()
        );
    }

    #[test]
    fn legacy_summary_is_retained_in_storage_but_excluded_from_prompt_context() {
        let path = std::env::temp_dir().join(format!(
            "pw-legacy-summary-context-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        persist_completed_turn(
            &mut history,
            DEFAULT_CONVERSATION_ID,
            TurnTracker::new().begin_turn(),
            "stored user message",
            "stored assistant message",
        )
        .unwrap();
        drop(history);
        let mut memory = SqliteMemoryStore::new(Database::open(&path).unwrap());
        memory
            .upsert_summary(DEFAULT_CONVERSATION_ID, "legacy / flat", 2, 1)
            .unwrap();

        let context = load_memory_context(&mut memory, "query", 1, 100);

        assert!(context.summary.is_none());
        let stored = memory
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        assert_eq!(stored.content, "legacy / flat");
        assert_eq!(stored.through_message_id, 2);
        drop(memory);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn oversized_summary_entry_keeps_its_role_and_content_when_cursor_advances() {
        let path = std::env::temp_dir().join(format!(
            "pw-oversized-summary-entry-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        let mut tracker = TurnTracker::new();
        persist_completed_turn(
            &mut history,
            DEFAULT_CONVERSATION_ID,
            tracker.begin_turn(),
            "short user message",
            &"long assistant response ".repeat(200),
        )
        .unwrap();
        for index in 0..10 {
            persist_completed_turn(
                &mut history,
                DEFAULT_CONVERSATION_ID,
                tracker.begin_turn(),
                &format!("recent-user-{index}"),
                &format!("recent-assistant-{index}"),
            )
            .unwrap();
        }
        let mut memory = SqliteMemoryStore::new(Database::open(&path).unwrap());

        update_rolling_summary(&history, &mut memory).unwrap();

        let summary = memory
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        assert_eq!(summary.through_message_id, 2);
        assert!(summary.content.chars().count() <= SUMMARY_MAX_CHARS);
        let document: serde_json::Value = serde_json::from_str(&summary.content).unwrap();
        let entries = document["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["role"], "assistant");
        assert!(
            entries[0]["content"]
                .as_str()
                .is_some_and(|content| !content.is_empty())
        );

        drop(memory);
        drop(history);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_live_sized_summary_rebuilds_by_role_without_overlapping_recent_history() {
        let path = std::env::temp_dir().join(format!(
            "pw-legacy-summary-rebuild-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        let mut tracker = TurnTracker::new();
        for index in 0..33 {
            persist_completed_turn(
                &mut history,
                DEFAULT_CONVERSATION_ID,
                tracker.begin_turn(),
                &format!("user-{index}"),
                &format!("assistant-{index}"),
            )
            .unwrap();
        }
        let mut memory = SqliteMemoryStore::new(Database::open(&path).unwrap());
        memory
            .upsert_summary(
                DEFAULT_CONVERSATION_ID,
                "legacy flat summary with wrong answer 10, 4",
                62,
                1,
            )
            .unwrap();
        assert!(
            load_memory_context(&mut memory, "query", 34, 100)
                .summary
                .is_none()
        );
        drop(memory);
        drop(history);

        let (wake, rx) = sync_channel(ENRICHMENT_QUEUE_CAPACITY);
        let wake = Arc::new(wake);
        let pending = Arc::new(Mutex::new(None));
        let sender = EnrichmentSender {
            wake: Arc::clone(&wake),
            pending: Arc::clone(&pending),
            metrics: Arc::new(QueueMetrics::new("test_enrichment", 8)),
        };
        let worker_wake = wake;
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            run_enrichment(
                &worker_path,
                rx,
                worker_wake,
                pending,
                Arc::new(QueueMetrics::new("test_enrichment", 8)),
                HybridConsolidator::new(
                    Box::new(UnavailableMemoryClassifier) as Box<dyn MemoryClassifier>
                ),
            );
        });
        sender
            .replace_latest(EnrichmentJob {
                user_text: "ordinary query".into(),
                turn_id: 34,
            })
            .unwrap();
        drop(sender);
        worker.join().unwrap();

        let history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        let memory = SqliteMemoryStore::new(Database::open(&path).unwrap());
        let rebuilt = memory
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        assert_eq!(rebuilt.through_message_id, 46);
        assert!(is_role_preserving_summary(&rebuilt.content));
        assert!(!rebuilt.content.contains("legacy flat summary"));
        let document: serde_json::Value = serde_json::from_str(&rebuilt.content).unwrap();
        assert!(
            document["entries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| { matches!(entry["role"].as_str(), Some("user" | "assistant")) })
        );
        let messages = history.list_messages(DEFAULT_CONVERSATION_ID).unwrap();
        let first_recent_id = messages[messages.len() - MAX_HISTORY_MESSAGES].id.unwrap();
        assert_eq!(first_recent_id, 47);
        assert_eq!(rebuilt.through_message_id, first_recent_id - 1);
        drop(memory);
        drop(history);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn summary_cursor_uses_id_order_when_created_at_moves_backwards() {
        let path = std::env::temp_dir().join(format!(
            "pw-summary-id-order-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        history
            .upsert_conversation(&StoredConversation {
                id: DEFAULT_CONVERSATION_ID.into(),
                created_at: 1,
                updated_at: 200,
            })
            .unwrap();
        for (prefix, count, created_at) in [
            ("older", 2, 100),
            ("clock-rollback", 8, 50),
            ("recent", SUMMARY_RECENT_MESSAGES, 200),
        ] {
            for index in 0..count {
                history
                    .append_message(&StoredMessage {
                        id: None,
                        conversation_id: DEFAULT_CONVERSATION_ID.into(),
                        turn_id: None,
                        role: MessageRole::User,
                        content: format!("{prefix}-{index}"),
                        created_at,
                    })
                    .unwrap();
            }
        }
        let mut memory = SqliteMemoryStore::new(Database::open(&path).unwrap());

        assert!(!drain_rolling_summary_pass(&history, &mut memory).unwrap());

        let summary = memory
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        assert_eq!(summary.through_message_id, 10);
        assert!(summary.content.contains("older-0"));
        assert!(summary.content.contains("clock-rollback-0"));
        assert!(!summary.content.contains("recent-0"));
        let recent =
            load_recent_history(&history, DEFAULT_CONVERSATION_ID, SUMMARY_RECENT_MESSAGES)
                .unwrap();
        assert_eq!(recent.len(), SUMMARY_RECENT_MESSAGES);
        assert!(
            recent
                .iter()
                .all(|message| message.content.starts_with("recent-"))
        );
        drop(memory);
        drop(history);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn one_summary_pass_processes_at_most_the_configured_batch_budget() {
        let path = std::env::temp_dir().join(format!(
            "pw-summary-pass-budget-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        history
            .upsert_conversation(&StoredConversation {
                id: DEFAULT_CONVERSATION_ID.into(),
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        let total =
            SUMMARY_BATCH_MESSAGES * (SUMMARY_DRAIN_BATCHES_PER_PASS + 8) + SUMMARY_RECENT_MESSAGES;
        for index in 0..total {
            history
                .append_message(&StoredMessage {
                    id: None,
                    conversation_id: DEFAULT_CONVERSATION_ID.into(),
                    turn_id: None,
                    role: MessageRole::User,
                    content: format!("message-{index}"),
                    created_at: i64::try_from(index).unwrap(),
                })
                .unwrap();
        }
        let mut memory = SqliteMemoryStore::new(Database::open(&path).unwrap());

        assert!(drain_rolling_summary_pass(&history, &mut memory).unwrap());

        let summary = memory
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        assert_eq!(
            summary.through_message_id,
            i64::try_from(SUMMARY_BATCH_MESSAGES * SUMMARY_DRAIN_BATCHES_PER_PASS).unwrap()
        );
        drop(memory);
        drop(history);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cancellation_before_summary_upsert_leaves_the_cursor_unchanged() {
        let path = std::env::temp_dir().join(format!(
            "pw-summary-cancel-before-upsert-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        let mut tracker = TurnTracker::new();
        for index in 0..11 {
            persist_completed_turn(
                &mut history,
                DEFAULT_CONVERSATION_ID,
                tracker.begin_turn(),
                &format!("user-{index}"),
                &format!("assistant-{index}"),
            )
            .unwrap();
        }
        let mut memory = SqliteMemoryStore::new(Database::open(&path).unwrap());
        let cancel = AtomicBool::new(true);

        assert!(update_rolling_summary_until(&history, &mut memory, Some(&cancel)).unwrap());
        assert!(
            memory
                .load_summary(DEFAULT_CONVERSATION_ID)
                .unwrap()
                .is_none()
        );

        drop(memory);
        drop(history);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn one_external_wake_finishes_a_large_finite_summary_backlog() {
        let path = std::env::temp_dir().join(format!(
            "pw-summary-finite-backlog-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        history
            .upsert_conversation(&StoredConversation {
                id: DEFAULT_CONVERSATION_ID.into(),
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        let stable_messages = 100;
        for index in 0..(stable_messages + SUMMARY_RECENT_MESSAGES) {
            history
                .append_message(&StoredMessage {
                    id: None,
                    conversation_id: DEFAULT_CONVERSATION_ID.into(),
                    turn_id: None,
                    role: MessageRole::User,
                    content: format!("backlog-{index}"),
                    created_at: i64::try_from(index).unwrap(),
                })
                .unwrap();
        }
        drop(history);
        let (wake, rx) = sync_channel(ENRICHMENT_QUEUE_CAPACITY);
        let wake = Arc::new(wake);
        let pending = Arc::new(Mutex::new(None));
        let sender = EnrichmentSender {
            wake: Arc::clone(&wake),
            pending: Arc::clone(&pending),
            metrics: Arc::new(QueueMetrics::new("test_enrichment", 8)),
        };
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            run_enrichment(
                &worker_path,
                rx,
                wake,
                pending,
                Arc::new(QueueMetrics::new("test_enrichment", 8)),
                HybridConsolidator::new(
                    Box::new(UnavailableMemoryClassifier) as Box<dyn MemoryClassifier>
                ),
            );
        });

        sender
            .replace_latest(EnrichmentJob {
                user_text: "ordinary query".into(),
                turn_id: 1,
            })
            .unwrap();
        drop(sender);
        worker.join().unwrap();

        let memory = SqliteMemoryStore::new(Database::open(&path).unwrap());
        assert_eq!(
            memory
                .load_summary(DEFAULT_CONVERSATION_ID)
                .unwrap()
                .unwrap()
                .through_message_id,
            i64::try_from(stable_messages).unwrap()
        );
        drop(memory);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pending_jobs_run_before_each_delayed_summary_follow_up() {
        let path = std::env::temp_dir().join(format!(
            "pw-summary-job-priority-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        history
            .upsert_conversation(&StoredConversation {
                id: DEFAULT_CONVERSATION_ID.into(),
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        let stable_messages = 100;
        for index in 0..(stable_messages + SUMMARY_RECENT_MESSAGES) {
            history
                .append_message(&StoredMessage {
                    id: None,
                    conversation_id: DEFAULT_CONVERSATION_ID.into(),
                    turn_id: None,
                    role: MessageRole::User,
                    content: format!("history-{index}"),
                    created_at: i64::try_from(index).unwrap(),
                })
                .unwrap();
        }
        drop(history);
        let (wake, rx) = sync_channel(ENRICHMENT_QUEUE_CAPACITY);
        let wake = Arc::new(wake);
        let pending = Arc::new(Mutex::new(None));
        let sender = EnrichmentSender {
            wake: Arc::clone(&wake),
            pending: Arc::clone(&pending),
            metrics: Arc::new(QueueMetrics::new("test_enrichment", 8)),
        };
        for index in 0..5 {
            sender
                .replace_latest(EnrichmentJob {
                    user_text: format!("私は猫{index}が好きです"),
                    turn_id: u64::try_from(index + 1).unwrap(),
                })
                .unwrap();
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let worker_seen = Arc::clone(&seen);
        let worker_path = path.clone();
        let classifier_path = path.clone();
        let worker = std::thread::spawn(move || {
            run_enrichment(
                &worker_path,
                rx,
                wake,
                pending,
                Arc::new(QueueMetrics::new("test_enrichment", 8)),
                HybridConsolidator::new(Box::new(SummaryCursorRecordingClassifier {
                    database_path: classifier_path,
                    seen: worker_seen,
                }) as Box<dyn MemoryClassifier>),
            );
        });
        drop(sender);
        worker.join().unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 5);
        assert!(
            seen[..ENRICHMENT_JOBS_PER_SLICE]
                .iter()
                .all(Option::is_none)
        );
        assert!(seen[ENRICHMENT_JOBS_PER_SLICE].is_some_and(
            |through| through > 0 && through < i64::try_from(stable_messages).unwrap()
        ));
        drop(seen);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn memory_context_prompt_context_excludes_dormant_without_recording_active_memory() {
        let path = std::env::temp_dir().join(format!(
            "pw-context-lifecycle-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut memory = SqliteMemoryStore::new(Database::open(&path).unwrap());
        memory
            .apply_action(
                &MemoryAction::Add {
                    content: "cats were an old preference".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 1),
                0,
            )
            .unwrap();
        memory.run_maintenance(31 * 86_400, 100).unwrap();
        let active = memory
            .apply_action(
                &MemoryAction::Add {
                    content: "cats are an active preference".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 2),
                31 * 86_400,
            )
            .unwrap()
            .unwrap();

        let context = load_memory_context(&mut memory, "cats", 21, 31 * 86_400);
        let repeated = load_memory_context(&mut memory, "cats", 21, 31 * 86_400);

        assert_eq!(context.memories, ["cats are an active preference"]);
        assert_eq!(repeated, context);
        drop(memory);
        let database = Database::open(&path).unwrap();
        let recalled = {
            let mut statement = database
                .connection()
                .prepare(
                    "SELECT source_turn_id FROM memory_evidence WHERE memory_id=?1 AND kind='recalled' ORDER BY id",
                )
                .unwrap();
            statement
                .query_map([active], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(recalled.is_empty());
        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn maintenance_failure_does_not_disconnect_context_worker() {
        let mut memory = FailingMemory::new();
        assert!(memory.run_maintenance(100, 100).is_err());
        let context = load_memory_context(&mut memory, "query", 1, 100);
        assert!(context.memories.is_empty());
    }

    fn candidate(id: i64, content: impl Into<String>) -> MemoryCandidate {
        MemoryCandidate {
            id,
            revision: Some(1),
            content: content.into(),
            state: MemoryState::Active,
            pinned: false,
            mention_count: 1,
            last_seen_at: 1,
            lexical_relevance: 1.0,
            strength: 1.0,
        }
    }

    #[test]
    fn memory_context_never_persists_prompt_recalls() {
        let recalled_ids = Arc::new(Mutex::new(Vec::new()));
        let mut memory = FailingMemory::recording_recall(
            vec![
                candidate(1, ""),
                candidate(2, "a".repeat(1_999)),
                candidate(3, "bc"),
                candidate(4, "excluded by character bound"),
                candidate(5, "also excluded"),
                candidate(6, "excluded by count bound"),
            ],
            Arc::clone(&recalled_ids),
            false,
        );

        let context = load_memory_context(&mut memory, "query", 7, 100);

        assert_eq!(context.memories.len(), 2);
        assert_eq!(context.memories[0].chars().count(), 1_999);
        assert_eq!(context.memories[1], "b");
        assert!(recalled_ids.lock().unwrap().is_empty());
    }

    #[test]
    fn memory_context_does_not_depend_on_recall_persistence() {
        let recalled_ids = Arc::new(Mutex::new(Vec::new()));
        let mut memory = FailingMemory::recording_recall(
            vec![candidate(9, "kept memory")],
            Arc::clone(&recalled_ids),
            true,
        );

        let context = load_memory_context(&mut memory, "query", 8, 200);

        assert_eq!(context.memories, ["kept memory"]);
        assert!(recalled_ids.lock().unwrap().is_empty());
    }

    #[test]
    fn completed_turn_enrichment_survives_restart_and_is_loaded_into_context() {
        let path =
            std::env::temp_dir().join(format!("pw-enrichment-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let (wake, rx) = sync_channel(ENRICHMENT_QUEUE_CAPACITY);
        let wake = Arc::new(wake);
        let pending = Arc::new(Mutex::new(None));
        let sender = EnrichmentSender {
            wake: Arc::clone(&wake),
            pending: Arc::clone(&pending),
            metrics: Arc::new(QueueMetrics::new("test_enrichment", 8)),
        };
        let (observation_tx, observation_rx) = sync_channel(OBSERVATION_WRITE_QUEUE_CAPACITY);
        let observation_path = path.clone();
        let observation_thread = std::thread::spawn({
            let writer_sender = sender.clone();
            move || run_observation_writer(&observation_path, &observation_rx, Some(&writer_sender))
        });
        let worker_wake = wake;
        let history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        let events = PersistentConversationEvents::new_with_enrichment(
            NoopEvents::default(),
            history,
            DEFAULT_CONVERSATION_ID,
            Some(sender),
            Some(ObservationWriter { tx: observation_tx }),
        );
        let mut tracker = TurnTracker::new();
        for assistant in ["覚えました", "もう一度覚えました"] {
            let turn = tracker.begin_turn();
            events.on_user_message(turn, "私は猫が好きです");
            events.on_reply_complete(turn, assistant);
        }
        drop(events);
        observation_thread.join().unwrap();
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            run_enrichment(
                &worker_path,
                rx,
                worker_wake,
                pending,
                Arc::new(QueueMetrics::new("test_enrichment", 8)),
                HybridConsolidator::new(Box::new(ExactFakeClassifier) as Box<dyn MemoryClassifier>),
            );
        });
        worker.join().unwrap();

        let mut store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        let context = load_memory_context(&mut store, "猫", 3, unix_timestamp());
        assert_eq!(context.memories, ["私は猫が好きです"]);
        let candidates = store
            .find_consolidation_candidates("私は猫が好きです", 5, unix_timestamp())
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].mention_count, 2);
        drop(store);
        let database = Database::open(&path).unwrap();
        let evidence_count: i64 = database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM memory_evidence WHERE kind='user_mention'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evidence_count, 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn durable_retry_reaches_the_bounded_limit_without_an_external_wake() {
        let path = std::env::temp_dir().join(format!(
            "pw-enrichment-retry-without-wake-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        store
            .insert_observation(NewObservation::new(
                DEFAULT_CONVERSATION_ID,
                1,
                "retryable durable observation",
                unix_timestamp(),
            ))
            .unwrap();
        // Fail only candidate INSERTs.  The retry finalization itself remains
        // writable, modelling a transient classifier/persistence failure that
        // must be resumed by the durable deadline.
        Database::open(&path)
            .unwrap()
            .connection()
            .execute_batch(
                "CREATE TRIGGER fail_candidate_insert BEFORE INSERT ON memory_candidates BEGIN SELECT RAISE(ABORT, 'injected transient candidate write failure'); END;",
            )
            .unwrap();
        drop(store);

        // No sender retains or writes to this channel.  After the bootstrap
        // drain, each attempt must be resumed exclusively from SQLite's
        // retry_after_at deadline.
        let (wake, rx) = sync_channel(ENRICHMENT_QUEUE_CAPACITY);
        let worker = std::thread::spawn({
            let worker_path = path.clone();
            move || {
                run_enrichment(
                    &worker_path,
                    rx,
                    Arc::new(wake),
                    Arc::new(Mutex::new(None)),
                    Arc::new(QueueMetrics::new("test_enrichment", 8)),
                    HybridConsolidator::new(
                        Box::new(ExactFakeClassifier) as Box<dyn MemoryClassifier>
                    ),
                );
            }
        });
        worker.join().unwrap();

        let database = Database::open(&path).unwrap();
        let (attempt_count, state): (i64, String) = database
            .connection()
            .query_row(
                "SELECT attempt_count,processing_state FROM memory_observations",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(attempt_count, 3);
        assert_eq!(state, "deferred");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn failed_or_cancelled_reply_keeps_the_accepted_user_observation() {
        let path = std::env::temp_dir().join(format!(
            "pw-observation-terminal-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        let (observation_tx, observation_rx) = sync_channel(OBSERVATION_WRITE_QUEUE_CAPACITY);
        let observation_path = path.clone();
        let observation_thread = std::thread::spawn(move || {
            run_observation_writer(&observation_path, &observation_rx, None);
        });
        let events = PersistentConversationEvents::new_with_enrichment(
            NoopEvents::default(),
            history,
            DEFAULT_CONVERSATION_ID,
            None,
            Some(ObservationWriter { tx: observation_tx }),
        );
        let mut tracker = TurnTracker::new();
        let failed = tracker.begin_turn();
        events.on_user_message(failed, "first durable user observation");
        events.on_error(failed, "offline");
        let cancelled = tracker.begin_turn();
        events.on_user_message(cancelled, "second durable user observation");
        events.on_cancelled(cancelled);
        drop(events);
        observation_thread.join().unwrap();

        let database = Database::open(&path).unwrap();
        let outcomes = database
            .connection()
            .prepare("SELECT response_outcome FROM memory_observations ORDER BY turn_id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(outcomes, ["llm_failed", "cancelled"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn saturated_observation_queue_never_delays_the_ordinary_user_turn() {
        let (observation_tx, _observation_rx) = sync_channel(1);
        // Keep the only writer slot occupied.  `on_user_message` must use
        // try_send and remain entirely outside SQLite in this condition.
        observation_tx
            .send(ObservationWrite::Insert {
                conversation_id: DEFAULT_CONVERSATION_ID.into(),
                turn_id: 0,
                text: "queued before measurement".into(),
            })
            .unwrap();
        let events = PersistentConversationEvents::new_with_enrichment(
            NoopEvents::default(),
            SqliteConversationHistory::new(Database::open_in_memory().unwrap()),
            DEFAULT_CONVERSATION_ID,
            None,
            Some(ObservationWriter { tx: observation_tx }),
        );
        let mut tracker = TurnTracker::new();
        let mut samples = Vec::new();
        for _ in 0..100 {
            let started = std::time::Instant::now();
            events.on_user_message(tracker.begin_turn(), "ordinary user turn");
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        let p95 = samples[94];
        assert!(
            p95 < std::time::Duration::from_millis(25),
            "saturated observation queue p95 was {p95:?}"
        );
    }

    struct FailOnceHistory {
        inner: SqliteConversationHistory,
        failed: bool,
    }

    impl ConversationHistory for FailOnceHistory {
        fn store_completed_turn(&mut self, turn: &StoredTurn) -> Result<(), PortError> {
            if !self.failed {
                self.failed = true;
                return Err(PortError("injected persistence failure".into()));
            }
            self.inner.store_completed_turn(turn)
        }

        fn max_turn_id(&self, conversation_id: &str) -> Result<Option<u64>, PortError> {
            self.inner.max_turn_id(conversation_id)
        }

        fn reserve_turn_id(
            &mut self,
            conversation_id: &str,
            created_at: i64,
        ) -> Result<u64, PortError> {
            self.inner.reserve_turn_id(conversation_id, created_at)
        }

        fn upsert_conversation(
            &mut self,
            conversation: &StoredConversation,
        ) -> Result<(), PortError> {
            self.inner.upsert_conversation(conversation)
        }

        fn append_message(&mut self, message: &StoredMessage) -> Result<i64, PortError> {
            self.inner.append_message(message)
        }

        fn list_conversations(&self) -> Result<Vec<StoredConversation>, PortError> {
            self.inner.list_conversations()
        }

        fn list_messages(&self, conversation_id: &str) -> Result<Vec<StoredMessage>, PortError> {
            self.inner.list_messages(conversation_id)
        }

        fn delete_conversation(&mut self, conversation_id: &str) -> Result<bool, PortError> {
            self.inner.delete_conversation(conversation_id)
        }
    }

    #[test]
    fn explicit_pin_enrichment_preserves_the_original_intent() {
        let path =
            std::env::temp_dir().join(format!("pw-enrichment-pin-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut consolidator = HybridConsolidator::new(PinFakeClassifier);
        process_enrichment_job_with_consolidator(
            &path,
            &EnrichmentJob {
                user_text: "私は猫が好きです。覚えておいて".into(),
                turn_id: 1,
            },
            &mut consolidator,
        )
        .unwrap();

        let store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        let candidates = store
            .find_consolidation_candidates("私は猫が好きです", 5, unix_timestamp())
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].pinned);
        assert_eq!(candidates[0].mention_count, 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn enrichment_supersedes_a_lexically_related_contradiction() {
        let path = std::env::temp_dir().join(format!(
            "pw-enrichment-supersede-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut consolidator = HybridConsolidator::new(ContradictionFakeClassifier);
        for (turn_id, user_text) in [(1, "私は猫が好きです"), (2, "私は犬が好きです")]
        {
            process_enrichment_job_with_consolidator(
                &path,
                &EnrichmentJob {
                    user_text: user_text.into(),
                    turn_id,
                },
                &mut consolidator,
            )
            .unwrap();
        }

        let store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        assert!(
            store
                .search_active_for_prompt("私は猫が好きです", 5, unix_timestamp())
                .unwrap()
                .iter()
                .all(|candidate| candidate.content != "私は猫が好きです")
        );
        let active = store
            .search_active_for_prompt("私は犬が好きです", 5, unix_timestamp())
            .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].content, "私は犬が好きです");
        let old = store
            .find_consolidation_candidates("私は猫が好きです", 5, unix_timestamp())
            .unwrap();
        assert!(old.iter().any(|candidate| {
            candidate.content == "私は猫が好きです" && candidate.state == MemoryState::Superseded
        }));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn phase5_restart_delete_and_export_acceptance() {
        let root =
            std::env::temp_dir().join(format!("pw-phase5-acceptance-{}", std::process::id()));
        let database_path = root.join("parallel-world.sqlite3");
        let snapshot_path = root.join("export.sqlite3");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let mut history = SqliteConversationHistory::new(Database::open(&database_path).unwrap());
        let mut tracker = TurnTracker::new();
        persist_completed_turn(
            &mut history,
            DEFAULT_CONVERSATION_ID,
            tracker.begin_turn(),
            "猫の話を覚えて",
            "覚えました",
        )
        .unwrap();
        drop(history);
        process_enrichment_job_with_consolidator(
            &database_path,
            &EnrichmentJob {
                user_text: "私は猫が好き".into(),
                turn_id: 1,
            },
            &mut HybridConsolidator::new(ExactFakeClassifier),
        )
        .unwrap();

        let reopened_history =
            SqliteConversationHistory::new(Database::open(&database_path).unwrap());
        let restored = load_recent_history(
            &reopened_history,
            DEFAULT_CONVERSATION_ID,
            MAX_HISTORY_MESSAGES,
        )
        .unwrap();
        assert_eq!(restored.len(), 2);
        let mut store = SqliteMemoryStore::new(Database::open(&database_path).unwrap());
        let context = load_memory_context(&mut store, "猫", 2, unix_timestamp());
        let prompt = PromptBuilder {
            system_rules: "rules".into(),
            character_prompt: "character".into(),
        }
        .build_with_context(&restored, "次の質問", &context);
        assert!(prompt.iter().any(|message| message.content.contains("猫")));

        Database::open(&database_path)
            .unwrap()
            .backup_to(&snapshot_path)
            .unwrap();
        let snapshot = SqliteConversationHistory::new(Database::open(&snapshot_path).unwrap());
        assert_eq!(
            snapshot
                .list_messages(DEFAULT_CONVERSATION_ID)
                .unwrap()
                .len(),
            2
        );

        let mut history = SqliteConversationHistory::new(Database::open(&database_path).unwrap());
        history
            .delete_conversation(DEFAULT_CONVERSATION_ID)
            .unwrap();
        drop(history);
        let mut memory = SqliteMemoryStore::new(Database::open(&database_path).unwrap());
        memory.delete_summary(DEFAULT_CONVERSATION_ID).unwrap();
        for record in memory.search("猫", DEFAULT_MEMORY_LIMIT).unwrap() {
            memory.delete_memory(record.id).unwrap();
        }
        drop(memory);
        let reopened = SqliteConversationHistory::new(Database::open(&database_path).unwrap());
        assert!(
            reopened
                .list_messages(DEFAULT_CONVERSATION_ID)
                .unwrap()
                .is_empty()
        );
        let mut empty_memory = SqliteMemoryStore::new(Database::open(&database_path).unwrap());
        let empty_context = load_memory_context(&mut empty_memory, "猫", 3, unix_timestamp());
        assert!(empty_context.summary.is_none());
        assert!(empty_context.memories.is_empty());
        let next_prompt = PromptBuilder {
            system_rules: "rules".into(),
            character_prompt: "character".into(),
        }
        .build_with_context(&[], "新しい会話", &empty_context);
        assert!(
            !next_prompt
                .iter()
                .any(|message| message.content.contains("覚えて"))
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn enrichment_recovers_after_open_failure_and_rolls_summary_forward_by_message_id() {
        let root =
            std::env::temp_dir().join(format!("pw-enrichment-recovery-{}", std::process::id()));
        let path = root.join("db.sqlite3");
        let _ = std::fs::remove_dir_all(&root);
        let mut consolidator = HybridConsolidator::new(ExactFakeClassifier);
        assert!(
            process_enrichment_job_with_consolidator(
                &path,
                &EnrichmentJob {
                    user_text: "私は猫が好きです".into(),
                    turn_id: 8,
                },
                &mut consolidator,
            )
            .is_err()
        );
        std::fs::create_dir_all(&root).unwrap();
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        let mut tracker = TurnTracker::new();
        for i in 0..12 {
            persist_completed_turn(
                &mut history,
                DEFAULT_CONVERSATION_ID,
                tracker.begin_turn(),
                &format!("私は項目{i}が好きです"),
                "了解",
            )
            .unwrap();
        }
        drop(history);
        process_enrichment_job_with_consolidator(
            &path,
            &EnrichmentJob {
                user_text: "私は猫が好きです".into(),
                turn_id: 8,
            },
            &mut consolidator,
        )
        .unwrap();
        let first = SqliteMemoryStore::new(Database::open(&path).unwrap())
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        process_enrichment_job_with_consolidator(
            &path,
            &EnrichmentJob {
                user_text: "私は犬が好きです".into(),
                turn_id: 9,
            },
            &mut consolidator,
        )
        .unwrap();
        let second = SqliteMemoryStore::new(Database::open(&path).unwrap())
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        assert!(second.through_message_id >= first.through_message_id);
        assert!(second.content.chars().count() <= SUMMARY_MAX_CHARS);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unsafe_only_summary_batch_advances_cursor_then_later_safe_batch_is_processed() {
        let path =
            std::env::temp_dir().join(format!("pw-summary-cursor-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        let mut tracker = TurnTracker::new();
        for (user, assistant) in [("APIキー=abc", "token=xyz"), ("安全な質問", "安全な回答")]
        {
            persist_completed_turn(
                &mut history,
                DEFAULT_CONVERSATION_ID,
                tracker.begin_turn(),
                user,
                assistant,
            )
            .unwrap();
        }
        for index in 0..9 {
            persist_completed_turn(
                &mut history,
                DEFAULT_CONVERSATION_ID,
                tracker.begin_turn(),
                &format!("最近{index}"),
                &format!("最近{index}回答"),
            )
            .unwrap();
        }
        drop(history);
        let (wake, rx) = sync_channel(ENRICHMENT_QUEUE_CAPACITY);
        let wake = Arc::new(wake);
        let pending = Arc::new(Mutex::new(None));
        let sender = EnrichmentSender {
            wake: Arc::clone(&wake),
            pending: Arc::clone(&pending),
            metrics: Arc::new(QueueMetrics::new("test_enrichment", 8)),
        };
        let worker_wake = wake;
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            run_enrichment(
                &worker_path,
                rx,
                worker_wake,
                pending,
                Arc::new(QueueMetrics::new("test_enrichment", 8)),
                HybridConsolidator::new(
                    Box::new(UnavailableMemoryClassifier) as Box<dyn MemoryClassifier>
                ),
            );
        });
        sender
            .replace_latest(EnrichmentJob {
                user_text: "私は猫が好きです".into(),
                turn_id: 4,
            })
            .unwrap();
        drop(sender);
        worker.join().unwrap();
        let store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        let first = store
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        assert!(store.search("猫", DEFAULT_MEMORY_LIMIT).unwrap().is_empty());
        drop(store);
        assert!(first.through_message_id > 0);
        assert!(!first.content.is_empty());
        assert!(first.content.contains("[REDACTED]"));
        assert!(!first.content.contains("abc"));
        assert!(!first.content.contains("xyz"));
        assert!(is_role_preserving_summary(&first.content));
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        for (user, assistant) in [("追加1", "追加1回答"), ("追加2", "追加2回答")] {
            persist_completed_turn(
                &mut history,
                DEFAULT_CONVERSATION_ID,
                tracker.begin_turn(),
                user,
                assistant,
            )
            .unwrap();
        }
        drop(history);
        let mut consolidator = HybridConsolidator::new(UnavailableMemoryClassifier);
        process_enrichment_job_with_consolidator(
            &path,
            &EnrichmentJob {
                user_text: "通常発話".into(),
                turn_id: 7,
            },
            &mut consolidator,
        )
        .unwrap();
        let second = SqliteMemoryStore::new(Database::open(&path).unwrap())
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        assert!(second.through_message_id > first.through_message_id);
        assert!(second.content.contains("安全な質問"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn slow_memory_search_does_not_block_submit_sender() {
        let (tx, rx) = sync_channel(8);
        let (conversation_tx, conversation_rx) = sync_channel(8);
        let worker = std::thread::spawn(move || {
            run_context_worker(
                SlowMemory,
                rx,
                conversation_tx,
                Arc::new(QueueMetrics::new("test_submit", 8)),
                Arc::new(QueueMetrics::new("test_context", 8)),
                Arc::new(QueueMetrics::new("test_conversation", 8)),
            );
        });
        let started = std::time::Instant::now();
        tx.send(Command::Submit("query".into(), 1, test_lease()))
            .unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
        assert!(matches!(
            conversation_rx.recv().unwrap(),
            Command::Prepared(_, 1, _, _)
        ));
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn context_worker_recovers_from_startup_and_periodic_maintenance_failures() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = sync_channel(8);
        let (conversation_tx, conversation_rx) = sync_channel(8);
        let worker_calls = Arc::clone(&calls);
        let worker = std::thread::spawn(move || {
            run_context_worker_with_interval(
                FailingMemory::tracking_maintenance(worker_calls),
                rx,
                conversation_tx,
                Arc::new(QueueMetrics::new("test_submit", 8)),
                Arc::new(QueueMetrics::new("test_context", 8)),
                Arc::new(QueueMetrics::new("test_conversation", 8)),
                std::time::Duration::from_millis(10),
            );
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while calls.lock().unwrap().len() < 2 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        let recorded = calls.lock().unwrap().clone();
        assert!(recorded.len() >= 2);
        assert!(recorded.iter().all(|(_, limit)| *limit == 100));
        assert!(recorded.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        tx.send(Command::Submit("query".into(), 1, test_lease()))
            .unwrap();
        assert!(matches!(
            conversation_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap(),
            Command::Prepared(_, 1, _, _)
        ));
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn context_worker_promptly_follows_up_when_maintenance_has_more_rows() {
        let calls = Arc::new(AtomicU64::new(0));
        let (tx, rx) = sync_channel(1);
        let (conversation_tx, _conversation_rx) = sync_channel(1);
        let worker_calls = Arc::clone(&calls);
        let worker = std::thread::spawn(move || {
            run_context_worker_with_interval(
                RemainingMaintenance {
                    calls: worker_calls,
                },
                rx,
                conversation_tx,
                Arc::new(QueueMetrics::new("test_submit", 1)),
                Arc::new(QueueMetrics::new("test_context", 1)),
                Arc::new(QueueMetrics::new("test_conversation", 1)),
                std::time::Duration::from_mins(1),
            );
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while calls.load(Ordering::SeqCst) < 2 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn context_worker_runs_maintenance_while_submit_queue_stays_nonempty() {
        const QUEUED_COMMANDS: usize = 100_000;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = sync_channel(QUEUED_COMMANDS);
        let (conversation_tx, conversation_rx) = sync_channel(QUEUED_COMMANDS);
        for turn_id in 0..QUEUED_COMMANDS {
            tx.send(Command::Submit(
                "query".into(),
                u64::try_from(turn_id).unwrap(),
                test_lease(),
            ))
            .unwrap();
        }
        let worker_calls = Arc::clone(&calls);
        let worker = std::thread::spawn(move || {
            run_context_worker_with_interval(
                FailingMemory::tracking_maintenance(worker_calls),
                rx,
                conversation_tx,
                Arc::new(QueueMetrics::new("test_submit", QUEUED_COMMANDS)),
                Arc::new(QueueMetrics::new("test_context", QUEUED_COMMANDS)),
                Arc::new(QueueMetrics::new("test_conversation", QUEUED_COMMANDS)),
                std::time::Duration::from_millis(1),
            );
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while calls.lock().unwrap().len() < 2 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(calls.lock().unwrap().len() >= 2);

        drop(conversation_rx);
        drop(tx);
        worker.join().unwrap();
    }

    #[test]
    fn persists_only_completed_user_and_assistant_messages() {
        let history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
        let events = PersistentConversationEvents::new(NoopEvents::default(), history, "chat");
        let mut tracker = TurnTracker::new();
        let completed = tracker.begin_turn();
        events.on_user_message(completed, "kept user");
        events.on_reply_complete(completed, "kept assistant");
        let cancelled = tracker.begin_turn();
        events.on_user_message(cancelled, "cancelled user");
        events.on_sentence(cancelled, "partial");
        events.on_cancelled(cancelled);

        let messages = events.history().list_messages("chat").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "kept user");
        assert_eq!(messages[1].content, "kept assistant");
        assert!(messages.iter().all(|message| message.id.is_some()));
    }

    #[test]
    fn persistence_retry_enqueues_enrichment_only_after_success() {
        let (wake, rx) = sync_channel(1);
        let pending = Arc::new(Mutex::new(None));
        let sender = EnrichmentSender {
            wake: Arc::new(wake),
            pending: Arc::clone(&pending),
            metrics: Arc::new(QueueMetrics::new("test_enrichment", 1)),
        };
        let history = FailOnceHistory {
            inner: SqliteConversationHistory::new(Database::open_in_memory().unwrap()),
            failed: false,
        };
        let events = PersistentConversationEvents::new_with_enrichment(
            NoopEvents::default(),
            history,
            "chat",
            Some(sender),
            None,
        );
        let mut tracker = TurnTracker::new();
        let first = tracker.begin_turn();
        events.on_user_message(first, "私は猫が好きです");
        events.on_reply_complete(first, "覚えました");
        assert!(rx.try_recv().is_err());

        events.on_user_message(tracker.begin_turn(), "次の発話");

        rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
        assert_eq!(
            pending.lock().unwrap().as_deref(),
            Some(
                [EnrichmentJob {
                    user_text: "私は猫が好きです".into(),
                    turn_id: first.value(),
                }]
                .as_slice()
            )
        );
    }

    #[test]
    fn persisted_history_and_export_never_contain_raw_credentials() {
        let root = std::env::temp_dir().join(format!("pw-redacted-export-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source.sqlite3");
        let export = root.join("export.sqlite3");
        let mut history = SqliteConversationHistory::new(Database::open(&source).unwrap());
        let mut tracker = TurnTracker::new();
        persist_completed_turn(
            &mut history,
            DEFAULT_CONVERSATION_ID,
            tracker.begin_turn(),
            "keep token=\"raw Secret123\", secret value rawValue789; APIキーは japaneseRaw123, password is hunter2, token abc123, credential=AbCdEf0123456789AbCdEf012345, opaque=[ABCDEF234567ABCDEF234567], next",
            "Authorization: Basic rawAssistant456; API key my-secret; wrapped=`ZYXWVU765432ZYXWVU765432`",
        )
        .unwrap();
        drop(history);
        Database::open(&source).unwrap().backup_to(&export).unwrap();
        for path in [&source, &export] {
            let history = SqliteConversationHistory::new(Database::open(path).unwrap());
            let joined = history
                .list_messages(DEFAULT_CONVERSATION_ID)
                .unwrap()
                .into_iter()
                .map(|m| m.content)
                .collect::<Vec<_>>()
                .join(" ");
            assert!(joined.contains("[REDACTED]"));
            assert!(!joined.contains("rawSecret123"));
            assert!(!joined.contains("raw Secret123"));
            assert!(!joined.contains("rawValue789"));
            assert!(!joined.contains("japaneseRaw123"));
            assert!(!joined.contains("rawAssistant456"));
            assert!(!joined.contains("hunter2"));
            assert!(!joined.contains("abc123"));
            assert!(!joined.contains("my-secret"));
            assert!(!joined.contains("AbCdEf0123456789AbCdEf012345"));
            assert!(!joined.contains("ABCDEF234567ABCDEF234567"));
            assert!(!joined.contains("ZYXWVU765432ZYXWVU765432"));
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn restored_messages_are_limited_to_recent_prompt_history() {
        let mut history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
        let mut tracker = TurnTracker::new();
        persist_completed_turn(
            &mut history,
            "chat",
            tracker.begin_turn(),
            "old",
            "old reply",
        )
        .unwrap();
        persist_completed_turn(
            &mut history,
            "chat",
            tracker.begin_turn(),
            "new",
            "new reply",
        )
        .unwrap();

        let restored = load_recent_history(&history, "chat", 2).unwrap();

        assert_eq!(
            restored
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            ["new", "new reply"]
        );
    }

    #[test]
    fn worker_restart_waits_for_queued_turn_before_history_is_read() {
        let (tx, rx) = sync_channel(8);
        let persisted = Arc::new(AtomicBool::new(false));
        let worker_persisted = Arc::clone(&persisted);
        let thread = std::thread::spawn(move || {
            while let Ok(command) = rx.recv() {
                match command {
                    Command::Submit(..) => worker_persisted.store(true, Ordering::SeqCst),
                    Command::Prepared(..) => {}
                }
            }
        });
        let worker = Worker {
            tx,
            settings_fingerprint: "old".into(),
            thread: Some(thread),
            context_thread: None,
            enrichment_thread: None,
            observation_writer_thread: None,
            companion_state_worker: None,
            enrichment_cancel: Arc::new(AtomicBool::new(false)),
        };
        worker
            .tx
            .send(Command::Submit("turn".into(), 1, test_lease()))
            .unwrap();

        worker.shutdown().unwrap();

        assert!(
            persisted.load(Ordering::SeqCst),
            "new worker would have seeded too early"
        );
    }

    #[test]
    fn allocator_falls_back_on_database_failure_and_never_collides_after_recovery() {
        let service = ChatService::default();
        let invalid = std::env::temp_dir()
            .join("missing-parent")
            .join("db.sqlite3");
        let first = service.reserve_turn_id(&invalid);
        let second = service.reserve_turn_id(&invalid);
        assert!(first >= (1_u64 << 63));
        assert_eq!(second, first + 1);

        let path = std::env::temp_dir().join(format!(
            "pw-allocator-recovery-{}.sqlite3",
            std::process::id()
        ));
        let recovered = service.reserve_turn_id(&path);
        assert!(recovered < (1_u64 << 63));
        assert_ne!(recovered, first);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn bounded_submit_queue_returns_busy_without_silently_dropping_user_text() {
        let (tx, rx) = sync_channel(1);
        let metrics = QueueMetrics::new("test_submit", 1);
        let gate = Arc::new(pw_application::behavior::proactive::InteractionGate::new());
        enqueue_submit(
            &tx,
            &metrics,
            "first".to_owned(),
            1,
            UserTurnLease::new(Arc::clone(&gate)),
        )
        .unwrap();

        let error = enqueue_submit(
            &tx,
            &metrics,
            "second".to_owned(),
            2,
            UserTurnLease::new(Arc::clone(&gate)),
        )
        .unwrap_err();

        assert_eq!(error, "conversation is busy; please retry");
        assert_eq!(gate.capture_idle_epoch(), None);
        let Command::Submit(text, turn_id, lease) = rx.recv().unwrap() else {
            panic!("expected queued user submission");
        };
        assert_eq!((text.as_str(), turn_id), ("first", 1));
        drop(lease);
        assert!(gate.capture_idle_epoch().is_some());
    }

    #[test]
    fn enrichment_coalescing_keeps_all_distinct_pending_facts() {
        let (wake, rx) = sync_channel(1);
        let pending = Arc::new(Mutex::new(None));
        let sender = EnrichmentSender {
            wake: Arc::new(wake),
            pending: Arc::clone(&pending),
            metrics: Arc::new(QueueMetrics::new("test_enrichment", 1)),
        };
        sender
            .replace_latest(EnrichmentJob {
                user_text: "old".into(),
                turn_id: 1,
            })
            .unwrap();
        sender
            .replace_latest(EnrichmentJob {
                user_text: "latest".into(),
                turn_id: 2,
            })
            .unwrap();
        rx.recv().unwrap();
        assert_eq!(
            pending.lock().unwrap().take(),
            Some(vec![
                EnrichmentJob {
                    user_text: "old".into(),
                    turn_id: 1,
                },
                EnrichmentJob {
                    user_text: "latest".into(),
                    turn_id: 2,
                },
            ])
        );
        assert!(
            rx.try_recv().is_err(),
            "one wake coalesces multiple updates"
        );
    }

    #[test]
    fn enrichment_job_preserves_turn_identity_and_is_idempotent() {
        let (wake, rx) = sync_channel(1);
        let pending = Arc::new(Mutex::new(None));
        let sender = EnrichmentSender {
            wake: Arc::new(wake),
            pending: Arc::clone(&pending),
            metrics: Arc::new(QueueMetrics::new("test_enrichment", 1)),
        };
        sender
            .replace_latest(EnrichmentJob {
                user_text: "私は猫が好きです".into(),
                turn_id: 12,
            })
            .unwrap();
        sender
            .replace_latest(EnrichmentJob {
                user_text: "本文が変わっても同じターン".into(),
                turn_id: 12,
            })
            .unwrap();
        rx.recv().unwrap();
        assert_eq!(pending.lock().unwrap().as_ref().unwrap().len(), 1);
    }

    #[test]
    fn enrichment_metrics_track_the_bounded_pending_work_not_the_wake_token() {
        let (wake, _rx) = sync_channel(1);
        let metrics = Arc::new(QueueMetrics::new(
            "test_enrichment",
            ENRICHMENT_PENDING_CAPACITY,
        ));
        let sender = EnrichmentSender {
            wake: Arc::new(wake),
            pending: Arc::new(Mutex::new(None)),
            metrics: Arc::clone(&metrics),
        };
        for index in 0..ENRICHMENT_PENDING_CAPACITY {
            sender
                .replace_latest(EnrichmentJob {
                    user_text: format!("fact-{index}"),
                    turn_id: u64::try_from(index).unwrap(),
                })
                .unwrap();
        }
        assert!(
            sender
                .replace_latest(EnrichmentJob {
                    user_text: "overflow".into(),
                    turn_id: u64::try_from(ENRICHMENT_PENDING_CAPACITY).unwrap(),
                })
                .is_err()
        );
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.depth, ENRICHMENT_PENDING_CAPACITY);
        assert_eq!(snapshot.capacity, ENRICHMENT_PENDING_CAPACITY);
        assert_eq!(snapshot.dropped, 1);
    }

    #[test]
    fn shutdown_does_not_block_sending_control_into_a_full_queue() {
        let (tx, rx) = sync_channel(1);
        tx.send(Command::Submit("queued".into(), 1, test_lease()))
            .unwrap();
        let context = std::thread::spawn(move || while rx.recv().is_ok() {});
        let worker = Worker {
            tx,
            settings_fingerprint: String::new(),
            thread: None,
            context_thread: Some(context),
            enrichment_thread: None,
            observation_writer_thread: None,
            companion_state_worker: None,
            enrichment_cancel: Arc::new(AtomicBool::new(false)),
        };
        let started = std::time::Instant::now();
        worker.shutdown().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    struct WaitForCancelLlm {
        started: Arc<AtomicBool>,
    }

    impl pw_application::conversation::LlmClient for WaitForCancelLlm {
        fn stream_chat(
            &mut self,
            _: &[ChatMessage],
            cancel: &AtomicBool,
            _: &mut dyn FnMut(&str),
        ) -> Result<(), PortError> {
            self.started.store(true, Ordering::SeqCst);
            while !cancel.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
            Ok(())
        }
    }

    #[test]
    fn worker_shutdown_cancels_an_in_flight_enrichment_classifier() {
        let cancel = Arc::new(AtomicBool::new(false));
        let classifier_cancel = Arc::clone(&cancel);
        let started = Arc::new(AtomicBool::new(false));
        let classifier_started = Arc::clone(&started);
        let enrichment_thread = std::thread::spawn(move || {
            let mut classifier = LlmMemoryClassifier::new_with_cancel(
                WaitForCancelLlm {
                    started: classifier_started,
                },
                classifier_cancel,
            );
            let _ = classifier.classify("私は猫が好き", &[]);
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !started.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(started.load(Ordering::SeqCst));
        let (tx, _rx) = sync_channel(1);
        let worker = Worker {
            tx,
            settings_fingerprint: String::new(),
            thread: None,
            context_thread: None,
            enrichment_thread: Some(enrichment_thread),
            observation_writer_thread: None,
            companion_state_worker: None,
            enrichment_cancel: cancel,
        };

        let shutdown_started = std::time::Instant::now();
        worker.shutdown().unwrap();
        assert!(shutdown_started.elapsed() < std::time::Duration::from_secs(1));
    }
}
