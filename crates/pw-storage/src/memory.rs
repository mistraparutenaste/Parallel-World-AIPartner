use crate::Database;
use pw_application::PortError;
use pw_application::memory::{
    Attribution, CandidateOperation, CandidateProvenanceRelation, CandidateRelation,
    ClassificationOutcome, ClassificationRun, Conditionality, DORMANT_DELETE_AFTER_SECONDS,
    DiscourseFeatures, DomainConsent, EpistemicForm, EvidenceKind, EvidenceSource, Fictionality,
    MaintenanceReport, MemoryAction, MemoryAtom, MemoryCandidate, MemoryDomain, MemoryEvidence,
    MemoryPromoter, MemoryRecord, MemoryState, MemoryStore, MemoryWriteClass,
    MemoryWriteDisposition, NewObservation, ObservationLease, ObservationOutcome, ObservationStore,
    PersistCandidateOutcome, PersistedCandidate, Polarity, PromotionResult,
    ProvisionalMemoryChangeSet, SourceMode, SourceSpan, SpeechAct, StoredSummary, SubjectScope,
    TemporalScope, TypedCandidate, VerificationStatus, VersionedMemoryAction, input_hash,
    is_safe_persistent_content, memory_strength, memory_write_disposition, prompt_rank,
    redact_persistent_content, should_become_dormant, validate_candidate_for_source,
};
use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

const SEARCH_POOL_MULTIPLIER: usize = 4;
const MAX_SEARCH_POOL: usize = 100;
const MAX_FTS_PHRASES: usize = 16;

pub struct SqliteMemoryStore {
    database: Database,
    maintenance_active_after: Option<(i64, i64)>,
    maintenance_expired_after: Option<(i64, i64)>,
    maintenance_active_complete: bool,
    maintenance_expired_complete: bool,
}
impl SqliteMemoryStore {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self {
            database,
            maintenance_active_after: None,
            maintenance_expired_after: None,
            maintenance_active_complete: false,
            maintenance_expired_complete: false,
        }
    }

    /// # Errors
    /// Returns an error when the observation cannot be inserted.
    pub fn insert_observation(&mut self, input: NewObservation) -> Result<i64, PortError> {
        <Self as ObservationStore>::insert_observation(self, input)
    }

    /// # Errors
    /// Returns an error when the next eligible observation cannot be claimed.
    pub fn claim_next_observation(
        &mut self,
        owner: &str,
        now: i64,
        lease_seconds: i64,
    ) -> Result<Option<ObservationLease>, PortError> {
        <Self as ObservationStore>::claim_next_observation(self, owner, now, lease_seconds)
    }

    /// Returns the earliest durable observation eligibility timestamp.  The
    /// enrichment worker uses this to keep a content-free wake alive across a
    /// retry backoff; it must not rely on a later chat event to resume work.
    ///
    /// # Errors
    /// Returns an error for a failed `SQLite` query.
    pub fn next_observation_due_at(&self, now: i64) -> Result<Option<i64>, PortError> {
        self.database
            .connection()
            .query_row(
                "SELECT MIN(CASE processing_state WHEN 'pending' THEN COALESCE(retry_after_at, ?1) WHEN 'processing' THEN lease_expires_at END) FROM memory_observations WHERE deleted_at IS NULL AND processing_state IN ('pending','processing')",
                [now],
                |row| row.get(0),
            )
            .map_err(memory_error)
    }

    /// # Errors
    /// Returns an error when the turn's observation cannot be found or
    /// finalized.
    pub fn finalize_observation_by_turn(
        &mut self,
        conversation_id: &str,
        turn_id: u64,
        outcome: ObservationOutcome,
        now: i64,
    ) -> Result<(), PortError> {
        let observation_id: i64 = self
            .database
            .connection()
            .query_row(
                "SELECT id FROM memory_observations WHERE conversation_id=?1 AND turn_id=?2",
                params![
                    conversation_id,
                    i64::try_from(turn_id).map_err(|error| PortError(error.to_string()))?
                ],
                |row| row.get(0),
            )
            .map_err(|error| PortError(error.to_string()))?;
        <Self as ObservationStore>::finalize_observation_outcome(self, observation_id, outcome, now)
    }

    /// # Errors
    /// Returns an error when the change set cannot be promoted.
    pub fn promote(
        &mut self,
        change_set: &ProvisionalMemoryChangeSet,
        now: i64,
    ) -> Result<PromotionResult, PortError> {
        <Self as MemoryPromoter>::promote(self, change_set, now)
    }

    /// Deletes one user-visible memory with a durable generation fence, so a
    /// worker holding an older observation lease cannot restore it later.
    ///
    /// # Errors
    /// Returns an error for a failed `SQLite` transaction.
    pub fn delete_memory_fenced(&mut self, id: i64) -> Result<(), PortError> {
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(memory_error)?;
        tombstone_and_delete_memory(&transaction, id, epoch_seconds())?;
        transaction.commit().map_err(memory_error)
    }

    fn lifecycle_search(
        &self,
        query: &str,
        limit: usize,
        now: i64,
        active_only: bool,
    ) -> Result<Vec<MemoryCandidate>, PortError> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let requested_limit = limit;
        let pool_limit = requested_limit
            .saturating_mul(SEARCH_POOL_MULTIPLIER)
            .min(MAX_SEARCH_POOL);
        let limit = i64::try_from(pool_limit).unwrap_or(i64::MAX);
        let mut rows = if query.trim().chars().count() < 3 {
            let escaped = query
                .trim()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{escaped}%");
            let sql = if active_only {
                "SELECT id,revision,content,state,pinned,mention_count,last_seen_at,0.0 FROM memories WHERE state='active' AND content LIKE ?1 ESCAPE '\\' ORDER BY updated_at DESC,id DESC LIMIT ?2"
            } else {
                "SELECT id,revision,content,state,pinned,mention_count,last_seen_at,0.0 FROM memories WHERE content LIKE ?1 ESCAPE '\\' ORDER BY updated_at DESC,id DESC LIMIT ?2"
            };
            query_candidate_rows(self.database.connection(), sql, &pattern, limit)?
        } else {
            let Some(phrase) = fts_disjunction(query) else {
                return Ok(Vec::new());
            };
            let sql = if active_only {
                "SELECT m.id,m.revision,m.content,m.state,m.pinned,m.mention_count,m.last_seen_at,bm25(memories_fts) FROM memories_fts f JOIN memories m ON m.id=f.rowid WHERE memories_fts MATCH ?1 AND m.state='active' ORDER BY bm25(memories_fts),m.updated_at DESC LIMIT ?2"
            } else {
                "SELECT m.id,m.revision,m.content,m.state,m.pinned,m.mention_count,m.last_seen_at,bm25(memories_fts) FROM memories_fts f JOIN memories m ON m.id=f.rowid WHERE memories_fts MATCH ?1 ORDER BY bm25(memories_fts),m.updated_at DESC LIMIT ?2"
            };
            query_candidate_rows(self.database.connection(), sql, &phrase, limit)?
        };
        let best = rows
            .iter()
            .map(|row| row.bm25)
            .fold(f64::INFINITY, f64::min);
        let worst = rows
            .iter()
            .map(|row| row.bm25)
            .fold(f64::NEG_INFINITY, f64::max);
        let has_bm25_range = rows.len() > 1 && (worst - best).abs() > f64::EPSILON;
        let candidate_ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
        let mut evidence_by_memory =
            load_evidence_for_memories(self.database.connection(), &candidate_ids)?;
        let mut candidates = Vec::with_capacity(rows.len());
        for row in rows.drain(..) {
            let evidence = evidence_by_memory.remove(&row.id).unwrap_or_default();
            let lexical_relevance = if has_bm25_range {
                (worst - row.bm25) / (worst - best)
            } else {
                1.0
            };
            candidates.push(MemoryCandidate {
                id: row.id,
                revision: Some(row.revision),
                content: row.content,
                state: parse_state(&row.state)?,
                pinned: row.pinned,
                mention_count: u64::try_from(row.mention_count)
                    .map_err(|error| PortError(error.to_string()))?,
                last_seen_at: row.last_seen_at,
                lexical_relevance,
                strength: memory_strength(&evidence, now),
            });
        }
        let weakest = candidates
            .iter()
            .map(|candidate| candidate.strength)
            .fold(f64::INFINITY, f64::min);
        let strongest = candidates
            .iter()
            .map(|candidate| candidate.strength)
            .fold(f64::NEG_INFINITY, f64::max);
        let has_strength_range = candidates.len() > 1 && (strongest - weakest).abs() > f64::EPSILON;
        candidates.sort_by(|left, right| {
            let normalized_strength = |strength: f64| {
                if has_strength_range {
                    (strength - weakest) / (strongest - weakest)
                } else {
                    1.0
                }
            };
            prompt_rank(right.lexical_relevance, normalized_strength(right.strength))
                .total_cmp(&prompt_rank(
                    left.lexical_relevance,
                    normalized_strength(left.strength),
                ))
                .then_with(|| right.last_seen_at.cmp(&left.last_seen_at))
                .then_with(|| right.id.cmp(&left.id))
        });
        candidates.truncate(requested_limit);
        Ok(candidates)
    }
}

impl ObservationStore for SqliteMemoryStore {
    fn insert_observation(&mut self, input: NewObservation) -> Result<i64, PortError> {
        if input.conversation_id.trim().is_empty() || input.user_text.trim().is_empty() {
            return Err(PortError(
                "memory observation requires conversation and user text".into(),
            ));
        }
        let user_text = redact_persistent_content(&input.user_text);
        let hash = input_hash(&user_text);
        let connection = self.database.connection();
        let inserted = connection.execute(
            "INSERT INTO memory_observations(conversation_id,turn_id,user_text,input_hash,observed_at,created_at,updated_at) SELECT ?1,?2,?3,?4,?5,?5,?5 WHERE NOT EXISTS(SELECT 1 FROM temporary_conversations WHERE conversation_id=?1 AND temporary=1) ON CONFLICT(conversation_id,turn_id) DO NOTHING",
            params![input.conversation_id, i64::try_from(input.turn_id).map_err(|error| PortError(error.to_string()))?, user_text, hash, input.observed_at],
        ).map_err(|error| PortError(error.to_string()))?;
        if inserted == 0 {
            let temporary: bool = connection
                .query_row(
                    "SELECT temporary FROM temporary_conversations WHERE conversation_id=?1",
                    [&input.conversation_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(memory_error)?
                .unwrap_or(false);
            if temporary {
                return Err(PortError(
                    "temporary conversation cannot persist memory".into(),
                ));
            }
        }
        connection
            .query_row(
                "SELECT id FROM memory_observations WHERE conversation_id=?1 AND turn_id=?2",
                params![
                    input.conversation_id,
                    i64::try_from(input.turn_id).map_err(|error| PortError(error.to_string()))?
                ],
                |row| row.get(0),
            )
            .map_err(|error| PortError(error.to_string()))
    }

    fn finalize_observation_outcome(
        &mut self,
        observation_id: i64,
        outcome: ObservationOutcome,
        now: i64,
    ) -> Result<(), PortError> {
        if !outcome.is_terminal() {
            return Err(PortError("observation outcome must be terminal".into()));
        }
        let changed = self.database.connection().execute(
            "UPDATE memory_observations SET response_outcome=?1,updated_at=?2 WHERE id=?3 AND response_outcome='pending'",
            params![encode_outcome(outcome), now, observation_id],
        ).map_err(|error| PortError(error.to_string()))?;
        if changed == 1 {
            return Ok(());
        }
        let existing: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT response_outcome FROM memory_observations WHERE id=?1",
                [observation_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| PortError(error.to_string()))?;
        match existing.as_deref() {
            Some(value) if value == encode_outcome(outcome) => Ok(()),
            Some(_) => Err(PortError(
                "observation outcome was already terminalized".into(),
            )),
            None => Err(PortError("memory observation does not exist".into())),
        }
    }

    fn claim_next_observation(
        &mut self,
        owner: &str,
        now: i64,
        lease_seconds: i64,
    ) -> Result<Option<ObservationLease>, PortError> {
        if owner.trim().is_empty() || lease_seconds <= 0 {
            return Err(PortError("invalid observation lease".into()));
        }
        let expires_at = now.saturating_add(lease_seconds);
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(|error| PortError(error.to_string()))?;
        let row: Option<(i64, String, i64, String, String, i64, i64)> = transaction.query_row(
            "SELECT id,conversation_id,turn_id,user_text,input_hash,deletion_generation,attempt_count FROM memory_observations WHERE deleted_at IS NULL AND ((processing_state='pending' AND (retry_after_at IS NULL OR retry_after_at <= ?1)) OR (processing_state='processing' AND lease_expires_at <= ?1)) ORDER BY observed_at,id LIMIT 1",
            [now],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
        ).optional().map_err(|error| PortError(error.to_string()))?;
        let Some((
            id,
            conversation_id,
            turn_id,
            user_text,
            canonical_input_hash,
            deletion_generation,
            attempt_count,
        )) = row
        else {
            transaction
                .commit()
                .map_err(|error| PortError(error.to_string()))?;
            return Ok(None);
        };
        let attempt = attempt_count.saturating_add(1);
        let attempt_token = format!("{owner}:{id}:{attempt}:{now}");
        let changed = transaction.execute(
            "UPDATE memory_observations SET processing_state='processing',lease_owner=?1,lease_expires_at=?2,attempt_token=?3,attempt_count=?4,retry_after_at=NULL,updated_at=?5 WHERE id=?6 AND deleted_at IS NULL AND ((processing_state='pending' AND (retry_after_at IS NULL OR retry_after_at <= ?5)) OR (processing_state='processing' AND lease_expires_at <= ?5))",
            params![owner, expires_at, attempt_token, attempt, now, id],
        ).map_err(|error| PortError(error.to_string()))?;
        if changed != 1 {
            return Err(PortError("observation lease claim raced; retry".into()));
        }
        transaction
            .commit()
            .map_err(|error| PortError(error.to_string()))?;
        Ok(Some(ObservationLease {
            observation_id: id,
            conversation_id,
            turn_id: u64::try_from(turn_id).map_err(|error| PortError(error.to_string()))?,
            user_text,
            canonical_input_hash,
            deletion_generation,
            owner: owner.into(),
            expires_at,
            attempt_token,
        }))
    }

    fn defer_observation(
        &mut self,
        lease: &ObservationLease,
        error: &str,
        now: i64,
    ) -> Result<(), PortError> {
        let reason = safe_diagnostic_reason(error);
        let changed = self.database.connection().execute(
            "UPDATE memory_observations SET processing_state='deferred',last_error=?1,lease_owner=NULL,lease_expires_at=NULL,attempt_token=NULL,updated_at=?2 WHERE id=?3 AND lease_owner=?4 AND attempt_token=?5 AND deletion_generation=?6 AND lease_expires_at>?2 AND deleted_at IS NULL",
            params![reason, now, lease.observation_id, lease.owner, lease.attempt_token, lease.deletion_generation],
        ).map_err(|error| PortError(error.to_string()))?;
        (changed == 1)
            .then_some(())
            .ok_or_else(|| PortError("observation lease was lost".into()))
    }

    fn begin_classification_run(
        &mut self,
        lease: &ObservationLease,
        run: &ClassificationRun,
        now: i64,
    ) -> Result<i64, PortError> {
        if lease.observation_id != run.observation_id
            || lease.canonical_input_hash != run.input_hash
        {
            return Err(PortError(
                "classification run does not match its user observation".into(),
            ));
        }
        let valid: bool = self.database.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_observations WHERE id=?1 AND processing_state='processing' AND lease_owner=?2 AND attempt_token=?3 AND deletion_generation=?4 AND lease_expires_at>?5 AND deleted_at IS NULL)",
            params![lease.observation_id, lease.owner, lease.attempt_token, lease.deletion_generation, now], |row| row.get(0),
        ).map_err(|error| PortError(error.to_string()))?;
        if !valid {
            return Err(PortError("observation lease was lost".into()));
        }
        self.database.connection().execute(
            "INSERT INTO memory_classification_runs(observation_id,classifier_version,schema_version,input_hash,lease_attempt_token,transport_outcome,created_at) VALUES(?1,?2,?3,?4,?5,'pending',?6) ON CONFLICT(observation_id,classifier_version,schema_version,input_hash,lease_attempt_token) DO NOTHING",
            params![run.observation_id, run.classifier_version, run.schema_version, run.input_hash, lease.attempt_token, now],
        ).map_err(|error| PortError(error.to_string()))?;
        self.database.connection().query_row(
            "SELECT id FROM memory_classification_runs WHERE observation_id=?1 AND classifier_version=?2 AND schema_version=?3 AND input_hash=?4 AND lease_attempt_token=?5",
            params![run.observation_id, run.classifier_version, run.schema_version, run.input_hash, lease.attempt_token], |row| row.get(0),
        ).map_err(|error| PortError(error.to_string()))
    }

    #[allow(clippy::type_complexity)]
    fn persist_candidate(
        &mut self,
        candidate: PersistedCandidate,
        now: i64,
    ) -> Result<PersistCandidateOutcome, PortError> {
        let (source_hash, source_text, _observation_id, token): (String, String, i64, String) = self.database.connection().query_row(
            "SELECT o.input_hash,o.user_text,o.id,r.lease_attempt_token FROM memory_classification_runs r JOIN memory_observations o ON o.id=r.observation_id WHERE r.id=?1 AND o.processing_state='processing' AND o.attempt_token=r.lease_attempt_token AND o.lease_expires_at>?2 AND o.deleted_at IS NULL",
            params![candidate.classification_run_id, now], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).map_err(|error| PortError(error.to_string()))?;
        let targets = validator_targets(self.database.connection(), candidate.target_memory_id)?;
        let typed = TypedCandidate {
            atom: candidate.atom.clone(),
            relation: candidate_relation_for(candidate.relation),
            target_memory_id: candidate.target_memory_id,
            expected_target_revision: candidate.expected_target_revision,
            normalization_edits: candidate.normalization_edits.clone(),
            proposed_action: proposed_action_for_persisted_candidate(&candidate)?,
        };
        let validation =
            validate_candidate_for_source(&typed, &source_hash, &source_text, &targets);
        let [span] = candidate.atom.source_spans.as_slice() else {
            return Err(PortError("candidate requires one source span".into()));
        };
        let normalization_json =
            serde_json::to_string(&candidate.normalization_edits).map_err(memory_error)?;
        let rejection_reason = validation
            .as_ref()
            .err()
            .map(|_| "typed safety validator rejected candidate");
        self.database.connection().execute(
            "INSERT INTO memory_candidates(observation_id,classification_run_id,candidate_ordinal,content,subject_scope,epistemic_form,attribution,speech_act,source_mode,polarity,conditionality,fictionality,verification_status,temporal_scope,target_memory_id,expected_target_revision,proposed_operation,proposed_relation,source_start,source_end,normalization_json,memory_domain,write_class,candidate_state,policy_state,rejection_reason,created_at,updated_at) SELECT r.observation_id,?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,CASE WHEN ?23 IS NULL THEN 'pending' ELSE 'rejected' END,CASE WHEN ?23 IS NULL THEN 'unreviewed' ELSE 'rejected' END,?23,?24,?24 FROM memory_classification_runs r JOIN memory_observations o ON o.id=r.observation_id WHERE r.id=?1 AND r.lease_attempt_token=?25 AND NOT EXISTS(SELECT 1 FROM temporary_conversations t WHERE t.conversation_id=o.conversation_id AND t.temporary=1) ON CONFLICT(observation_id,classification_run_id,candidate_ordinal) DO NOTHING",
            params![candidate.classification_run_id, candidate.ordinal, candidate.atom.content, encode_subject_scope(candidate.atom.subject_scope), encode_epistemic_form(candidate.atom.epistemic_form), encode_attribution(candidate.atom.attribution), encode_speech_act(candidate.atom.discourse.speech_act), encode_source_mode(candidate.atom.discourse.source_mode), encode_polarity(candidate.atom.discourse.polarity), encode_conditionality(candidate.atom.discourse.conditionality), encode_fictionality(candidate.atom.discourse.fictionality), encode_verification_status(candidate.atom.verification_status), encode_temporal_scope(candidate.atom.temporal_scope), candidate.target_memory_id, candidate.expected_target_revision, encode_candidate_operation(candidate.operation), encode_candidate_relation(candidate.relation), i64::try_from(span.start).map_err(|error| PortError(error.to_string()))?, i64::try_from(span.end).map_err(|error| PortError(error.to_string()))?, normalization_json, encode_memory_domain(candidate.domain), encode_write_class(candidate.write_class), rejection_reason, now, token],
        ).map_err(memory_error)?;
        let stored: (i64, String, String, String, String, String, String, String, String, String, String, String, String, String, Option<i64>, Option<i64>, i64, i64, String, String, String, String) = self.database.connection().query_row(
            "SELECT id,content,subject_scope,epistemic_form,attribution,speech_act,source_mode,polarity,conditionality,fictionality,verification_status,temporal_scope,proposed_operation,proposed_relation,target_memory_id,expected_target_revision,source_start,source_end,normalization_json,memory_domain,write_class,candidate_state FROM memory_candidates WHERE classification_run_id=?1 AND candidate_ordinal=?2",
            params![candidate.classification_run_id, candidate.ordinal], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?, row.get(15)?, row.get(16)?, row.get(17)?, row.get(18)?, row.get(19)?, row.get(20)?, row.get(21)?)),
        ).map_err(memory_error)?;
        if stored.1 != candidate.atom.content
            || stored.2 != encode_subject_scope(candidate.atom.subject_scope)
            || stored.3 != encode_epistemic_form(candidate.atom.epistemic_form)
            || stored.4 != encode_attribution(candidate.atom.attribution)
            || stored.5 != encode_speech_act(candidate.atom.discourse.speech_act)
            || stored.6 != encode_source_mode(candidate.atom.discourse.source_mode)
            || stored.7 != encode_polarity(candidate.atom.discourse.polarity)
            || stored.8 != encode_conditionality(candidate.atom.discourse.conditionality)
            || stored.9 != encode_fictionality(candidate.atom.discourse.fictionality)
            || stored.10 != encode_verification_status(candidate.atom.verification_status)
            || stored.11 != encode_temporal_scope(candidate.atom.temporal_scope)
            || stored.12 != encode_candidate_operation(candidate.operation)
            || stored.13 != encode_candidate_relation(candidate.relation)
            || stored.14 != candidate.target_memory_id
            || stored.15 != candidate.expected_target_revision
            || stored.16 != i64::try_from(span.start).map_err(memory_error)?
            || stored.17 != i64::try_from(span.end).map_err(memory_error)?
            || stored.18
                != serde_json::to_string(&candidate.normalization_edits).map_err(memory_error)?
            || stored.19 != encode_memory_domain(candidate.domain)
            || stored.20 != encode_write_class(candidate.write_class)
        {
            return Err(PortError("candidate ordinal metadata collision".into()));
        }
        if validation.is_err() || stored.21 == "rejected" {
            return Ok(PersistCandidateOutcome::DeterministicallyRejected(stored.0));
        }
        Ok(PersistCandidateOutcome::Persisted(stored.0))
    }

    fn finish_classification_run(
        &mut self,
        lease: &ObservationLease,
        classification_run_id: i64,
        outcome: ClassificationOutcome,
        candidate_count: i64,
        reason: Option<&str>,
        now: i64,
    ) -> Result<(), PortError> {
        if candidate_count < 0 {
            return Err(PortError("invalid classification candidate count".into()));
        }
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(memory_error)?;
        let valid: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_observations o JOIN memory_classification_runs r ON r.observation_id=o.id WHERE r.id=?1 AND o.id=?2 AND o.processing_state='processing' AND o.lease_owner=?3 AND o.attempt_token=?4 AND r.lease_attempt_token=?4 AND o.deletion_generation=?5 AND o.lease_expires_at>?6 AND o.deleted_at IS NULL)",
            params![classification_run_id, lease.observation_id, lease.owner, lease.attempt_token, lease.deletion_generation, now], |row| row.get(0),
        ).map_err(memory_error)?;
        if !valid {
            return Err(PortError(
                "classification completion lost its current lease".into(),
            ));
        }
        let actual_count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM memory_candidates WHERE classification_run_id=?1",
                [classification_run_id],
                |row| row.get(0),
            )
            .map_err(memory_error)?;
        if actual_count != candidate_count {
            return Err(PortError(
                "classification candidate count does not match durable rows".into(),
            ));
        }
        let unresolved: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_candidates WHERE classification_run_id=?1 AND candidate_state='pending')",
            [classification_run_id], |row| row.get(0),
        ).map_err(memory_error)?;
        let outcome = encode_classification_outcome(outcome);
        if outcome == "completed" && unresolved {
            // A classifier completed, but its proposals have not reached a
            // terminal promotion decision.  Keep the run retryable/fenced.
            transaction.execute(
                "UPDATE memory_classification_runs SET candidate_count=?1,error_reason=?2 WHERE id=?3 AND transport_outcome='pending'",
                params![candidate_count, reason.map(safe_diagnostic_reason), classification_run_id],
            ).map_err(memory_error)?;
        } else {
            transaction.execute(
                "UPDATE memory_classification_runs SET transport_outcome=?1,candidate_count=?2,error_reason=?3,completed_at=?4 WHERE id=?5 AND transport_outcome='pending'",
                params![outcome, candidate_count, reason.map(safe_diagnostic_reason), now, classification_run_id],
            ).map_err(memory_error)?;
        }
        if !unresolved && matches!(outcome, "completed" | "rejected") {
            transaction.execute(
                "UPDATE memory_observations SET processing_state='completed',lease_owner=NULL,lease_expires_at=NULL,attempt_token=NULL,updated_at=?1 WHERE id=?2 AND attempt_token=?3 AND deletion_generation=?4 AND deleted_at IS NULL",
                params![now, lease.observation_id, lease.attempt_token, lease.deletion_generation],
            ).map_err(memory_error)?;
        }
        transaction.commit().map_err(memory_error)?;
        Ok(())
    }

    fn reject_pending_candidates(
        &mut self,
        lease: &ObservationLease,
        classification_run_id: i64,
        reason: &str,
        now: i64,
    ) -> Result<i64, PortError> {
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(memory_error)?;
        let valid: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_classification_runs r JOIN memory_observations o ON o.id=r.observation_id WHERE r.id=?1 AND o.id=?2 AND o.processing_state='processing' AND o.lease_owner=?3 AND o.attempt_token=?4 AND r.lease_attempt_token=?4 AND o.deletion_generation=?5 AND o.lease_expires_at>?6 AND o.deleted_at IS NULL)",
            params![classification_run_id, lease.observation_id, lease.owner, lease.attempt_token, lease.deletion_generation, now],
            |row| row.get(0),
        ).map_err(memory_error)?;
        if !valid {
            return Err(PortError(
                "classification rejection lost its current lease".into(),
            ));
        }
        transaction.execute(
            "UPDATE memory_candidates SET candidate_state='rejected',rejection_reason=?1,updated_at=?2 WHERE classification_run_id=?3 AND candidate_state='pending'",
            params![safe_diagnostic_reason(reason), now, classification_run_id],
        ).map_err(memory_error)?;
        let count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM memory_candidates WHERE classification_run_id=?1",
                [classification_run_id],
                |row| row.get(0),
            )
            .map_err(memory_error)?;
        transaction.commit().map_err(memory_error)?;
        Ok(count)
    }

    fn retry_or_defer_observation(
        &mut self,
        lease: &ObservationLease,
        error: &str,
        now: i64,
        retry_limit: i64,
        retry_after_seconds: i64,
    ) -> Result<(), PortError> {
        if retry_limit <= 0 || retry_after_seconds < 0 {
            return Err(PortError("invalid retry policy".into()));
        }
        let state = if lease_attempt_count(self.database.connection(), lease.observation_id)?
            >= retry_limit
        {
            "deferred"
        } else {
            "pending"
        };
        let retry_after = (state == "pending").then(|| now.saturating_add(retry_after_seconds));
        let changed = self.database.connection().execute(
            "UPDATE memory_observations SET processing_state=?1,last_error=?2,retry_after_at=?3,lease_owner=NULL,lease_expires_at=NULL,attempt_token=NULL,updated_at=?4 WHERE id=?5 AND lease_owner=?6 AND attempt_token=?7 AND deletion_generation=?8 AND lease_expires_at>?4 AND deleted_at IS NULL",
            params![state, safe_diagnostic_reason(error), retry_after, now, lease.observation_id, lease.owner, lease.attempt_token, lease.deletion_generation],
        ).map_err(memory_error)?;
        (changed == 1)
            .then_some(())
            .ok_or_else(|| PortError("observation lease was lost".into()))
    }

    fn recover_interrupted_observations(&mut self, now: i64) -> Result<usize, PortError> {
        self.database.connection().execute(
            "UPDATE memory_observations SET response_outcome='interrupted',updated_at=?1 WHERE response_outcome='pending' AND deleted_at IS NULL",
            [now],
        ).map_err(memory_error)
    }
}

fn safe_diagnostic_reason(value: &str) -> String {
    if !is_safe_persistent_content(value) {
        return "redacted diagnostic".into();
    }
    value.chars().take(128).collect()
}

/// Deletes every long-lived memory using the same tombstone protocol as a
/// single user delete.  Desktop data-erasure uses this rather than a raw SQL
/// DELETE so queued promotions lose their generation fence atomically.
///
/// # Errors
/// Returns an error for a failed `SQLite` transaction.
pub fn delete_all_memories_fenced(database: &mut Database) -> Result<usize, PortError> {
    let transaction = database
        .connection_mut()
        .transaction()
        .map_err(memory_error)?;
    let now = epoch_seconds();
    let count = delete_all_memories_in_transaction(&transaction, now)?;
    transaction.commit().map_err(memory_error)?;
    Ok(count)
}

/// Transaction form used by compound user-data erasure commands.
///
/// # Errors
/// Returns an error for a failed `SQLite` statement.
pub fn delete_all_memories_in_transaction(
    transaction: &Transaction<'_>,
    now: i64,
) -> Result<usize, PortError> {
    // A global erasure has no stable target id for an in-flight Add.  Fence
    // every outstanding observation before deleting rows so a leased worker
    // cannot recreate any fact after the user cleared memories.
    transaction.execute(
        "UPDATE memory_candidates SET candidate_state='rejected',policy_state='rejected',rejection_reason='all memories deleted',updated_at=?1 WHERE candidate_state='pending'",
        [now],
    ).map_err(memory_error)?;
    transaction.execute(
        "UPDATE memory_observations SET deletion_generation=deletion_generation+1,user_text='[deleted]',input_hash='tombstone:' || id,processing_state='deferred',lease_owner=NULL,lease_expires_at=NULL,attempt_token=NULL,retry_after_at=NULL,last_error='all memories deleted',deleted_at=?1,updated_at=?1 WHERE deleted_at IS NULL",
        [now],
    ).map_err(memory_error)?;
    let mut statement = transaction
        .prepare("SELECT id FROM memories ORDER BY id")
        .map_err(memory_error)?;
    let ids = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(memory_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(memory_error)?;
    drop(statement);
    for id in &ids {
        tombstone_and_delete_memory(transaction, *id, now)?;
    }
    Ok(ids.len())
}

/// Applies the content-free deletion fence for rows whose history was already
/// erased.  Kept crate-visible so both history deletion entry points preserve
/// their existing retention semantics while also preventing late promotion.
///
/// # Errors
/// Returns an error for a failed `SQLite` statement.
pub fn tombstone_memories_for_deleted_observations(
    transaction: &Transaction<'_>,
    conversation_id: Option<&str>,
    now: i64,
) -> Result<usize, PortError> {
    let filter = if conversation_id.is_some() {
        " AND o.conversation_id=?1"
    } else {
        ""
    };
    let sql = format!(
        "SELECT DISTINCT p.memory_id FROM memory_provenance p JOIN memory_observations o ON o.id=p.observation_id JOIN memories m ON m.id=p.memory_id WHERE o.deleted_at IS NOT NULL AND m.pinned=0 AND NOT EXISTS (SELECT 1 FROM memory_provenance live_p JOIN memory_observations live_o ON live_o.id=live_p.observation_id WHERE live_p.memory_id=p.memory_id AND live_o.deleted_at IS NULL){filter}"
    );
    let mut statement = transaction.prepare(&sql).map_err(memory_error)?;
    let ids = if let Some(conversation_id) = conversation_id {
        statement
            .query_map([conversation_id], |row| row.get::<_, i64>(0))
            .map_err(memory_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(memory_error)?
    } else {
        statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(memory_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(memory_error)?
    };
    drop(statement);
    for id in &ids {
        tombstone_and_delete_memory(transaction, *id, now)?;
    }
    Ok(ids.len())
}

fn tombstone_and_delete_memory(
    transaction: &Transaction<'_>,
    memory_id: i64,
    now: i64,
) -> Result<bool, PortError> {
    let exists: Option<bool> = transaction
        .query_row(
            "SELECT pinned FROM memories WHERE id=?1",
            [memory_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(memory_error)?;
    let Some(pinned) = exists else {
        return Ok(false);
    };
    tombstone_memory_work(transaction, memory_id, now, pinned)?;
    transaction
        .execute("DELETE FROM memories WHERE id=?1", [memory_id])
        .map_err(memory_error)?;
    Ok(true)
}

fn tombstone_memory_work(
    transaction: &Transaction<'_>,
    memory_id: i64,
    now: i64,
    pinned: bool,
) -> Result<(), PortError> {
    let generation: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(generation),0)+1 FROM memory_tombstones WHERE memory_id=?1",
            [memory_id],
            |row| row.get(0),
        )
        .map_err(memory_error)?;
    transaction.execute(
        "INSERT INTO memory_tombstones(memory_id,generation,deleted_at,final_support_removed,pinned) VALUES(?1,?2,?3,1,?4)",
        params![memory_id, generation, now, pinned],
    ).map_err(memory_error)?;
    transaction.execute(
        "UPDATE memory_candidates SET candidate_state='rejected',policy_state='rejected',rejection_reason='target memory deleted',updated_at=?1 WHERE target_memory_id=?2 AND candidate_state='pending'",
        params![now, memory_id],
    ).map_err(memory_error)?;
    // Any observation that supplied, or is attempting to alter, this memory
    // loses its lease generation.  The existing privacy convention retains a
    // content-free tombstone so historical deletion/provenance audits work.
    transaction.execute(
        "UPDATE memory_observations SET deletion_generation=deletion_generation+1,user_text='[deleted]',input_hash='tombstone:' || id,processing_state='deferred',lease_owner=NULL,lease_expires_at=NULL,attempt_token=NULL,retry_after_at=NULL,last_error='memory deleted',deleted_at=?1,updated_at=?1 WHERE deleted_at IS NULL AND (id IN (SELECT observation_id FROM memory_provenance WHERE memory_id=?2) OR id IN (SELECT observation_id FROM memory_candidates WHERE target_memory_id=?2))",
        params![now, memory_id],
    ).map_err(memory_error)?;
    Ok(())
}

fn epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |value| {
            i64::try_from(value.as_secs()).unwrap_or(i64::MAX)
        })
}
fn memory_error(error: impl std::fmt::Display) -> PortError {
    PortError(error.to_string())
}

fn lease_attempt_count(connection: &Connection, observation_id: i64) -> Result<i64, PortError> {
    connection
        .query_row(
            "SELECT attempt_count FROM memory_observations WHERE id=?1",
            [observation_id],
            |row| row.get(0),
        )
        .map_err(memory_error)
}

fn encode_classification_outcome(value: ClassificationOutcome) -> &'static str {
    match value {
        ClassificationOutcome::Completed => "completed",
        ClassificationOutcome::Failed => "failed",
        ClassificationOutcome::Rejected => "rejected",
    }
}

fn candidate_relation_for(value: CandidateProvenanceRelation) -> CandidateRelation {
    match value {
        CandidateProvenanceRelation::Originated => CandidateRelation::Unrelated,
        CandidateProvenanceRelation::Reasserted => CandidateRelation::Same,
        CandidateProvenanceRelation::Corrected
        | CandidateProvenanceRelation::ChangedStance
        | CandidateProvenanceRelation::Contradicted => CandidateRelation::Contradicts,
    }
}

fn proposed_action_for_persisted_candidate(
    candidate: &PersistedCandidate,
) -> Result<pw_application::memory::ProposedAction, PortError> {
    match candidate.operation {
        CandidateOperation::Add => Ok(pw_application::memory::ProposedAction::Add {
            content: candidate.atom.content.clone(),
        }),
        CandidateOperation::Reinforce => Ok(pw_application::memory::ProposedAction::Reinforce {
            memory_id: candidate
                .target_memory_id
                .ok_or_else(|| PortError("candidate is missing reinforce target".into()))?,
        }),
        CandidateOperation::Supersede => Ok(pw_application::memory::ProposedAction::Supersede {
            old_memory_id: candidate
                .target_memory_id
                .ok_or_else(|| PortError("candidate is missing supersede target".into()))?,
            content: candidate.atom.content.clone(),
        }),
    }
}

fn validator_targets(
    connection: &Connection,
    target: Option<i64>,
) -> Result<Vec<MemoryCandidate>, PortError> {
    let Some(target) = target else {
        return Ok(Vec::new());
    };
    let row: Option<(i64, i64, String, String, bool, i64, i64)> = connection.query_row(
        "SELECT id,revision,content,state,pinned,mention_count,last_seen_at FROM memories WHERE id=?1",
        [target],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
    ).optional().map_err(memory_error)?;
    row.map(
        |(id, revision, content, state, pinned, mention_count, last_seen_at)| {
            Ok(MemoryCandidate {
                id,
                revision: Some(revision),
                content,
                state: parse_state(&state)?,
                pinned,
                mention_count: u64::try_from(mention_count)
                    .map_err(|error| PortError(error.to_string()))?,
                last_seen_at,
                lexical_relevance: 1.0,
                strength: 1.0,
            })
        },
    )
    .transpose()
    .map(|value| value.into_iter().collect())
}

impl MemoryPromoter for SqliteMemoryStore {
    #[allow(clippy::too_many_lines, clippy::type_complexity)]
    fn promote(
        &mut self,
        change_set: &ProvisionalMemoryChangeSet,
        now: i64,
    ) -> Result<PromotionResult, PortError> {
        if change_set.actions.len() != change_set.provenance.len() || change_set.actions.is_empty()
        {
            return Err(PortError(
                "promotion must map every action to exactly one candidate provenance".into(),
            ));
        }
        let unique_candidate_ids = change_set
            .provenance
            .iter()
            .map(|link| link.candidate_id)
            .collect::<HashSet<_>>();
        if unique_candidate_ids.len() != change_set.provenance.len() {
            return Err(PortError(
                "each promotion action requires a distinct candidate".into(),
            ));
        }
        let lease = &change_set.lease;
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(|error| PortError(error.to_string()))?;
        let fingerprint = promotion_fingerprint(change_set);
        if let Some((ids, observation_id, classifier_version, schema_version, input_hash, run_id, stored_fingerprint)) = transaction
            .query_row(
                "SELECT result_memory_ids,observation_id,classifier_version,schema_version,input_hash,classification_run_id,change_set_fingerprint FROM memory_promotions WHERE request_key=?1",
                [&change_set.request_key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?, row.get::<_, String>(4)?, row.get::<_, i64>(5)?, row.get::<_, String>(6)?)),
            )
            .optional()
            .map_err(|error| PortError(error.to_string()))?
        {
            if observation_id != lease.observation_id
                || classifier_version != change_set.classifier_version
                || schema_version != change_set.schema_version
                || input_hash != change_set.input_hash
                || run_id != change_set.classification_run_id
                || stored_fingerprint != fingerprint
            {
                return Err(PortError("promotion request key metadata collision".into()));
            }
            let promoted_memory_ids = serde_json::from_str(&ids)
                .map_err(|error| PortError(format!("invalid stored promotion result: {error}")))?;
            transaction
                .commit()
                .map_err(|error| PortError(error.to_string()))?;
            return Ok(PromotionResult {
                request_key: change_set.request_key.clone(),
                promoted_memory_ids,
                already_applied: true,
            });
        }
        let lease_valid: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_observations o JOIN memory_classification_runs r ON r.observation_id=o.id WHERE o.id=?1 AND o.processing_state='processing' AND o.lease_owner=?2 AND o.attempt_token=?3 AND o.deletion_generation=?4 AND o.lease_expires_at>?5 AND o.deleted_at IS NULL AND r.id=?6 AND r.lease_attempt_token=?3 AND r.classifier_version=?7 AND r.schema_version=?8 AND r.input_hash=?9)",
            params![lease.observation_id, lease.owner, lease.attempt_token, lease.deletion_generation, now, change_set.classification_run_id, change_set.classifier_version, change_set.schema_version, change_set.input_hash], |row| row.get(0),
        ).map_err(|error| PortError(error.to_string()))?;
        if !lease_valid {
            return Err(PortError(
                "promotion rejected because observation was deleted or lease was lost".into(),
            ));
        }
        // Policy is deliberately evaluated after the lease check and inside
        // this same write transaction.  A domain-control change or temporary
        // chat transition therefore cannot race a previously classified row.
        let mut policy_results = Vec::with_capacity(change_set.provenance.len());
        for provenance in &change_set.provenance {
            let policy: Option<(String, String, String, bool)> = transaction.query_row(
                "SELECT c.memory_domain,c.write_class,COALESCE(d.consent,'never_store'),EXISTS(SELECT 1 FROM temporary_conversations t WHERE t.conversation_id=o.conversation_id AND t.temporary=1) FROM memory_candidates c JOIN memory_observations o ON o.id=c.observation_id LEFT JOIN memory_domain_controls d ON d.domain=c.memory_domain WHERE c.id=?1 AND c.observation_id=?2 AND c.classification_run_id=?3 AND c.candidate_state='pending'",
                params![provenance.candidate_id, lease.observation_id, change_set.classification_run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).optional().map_err(memory_error)?;
            let Some((domain, class, consent, temporary)) = policy else {
                return Err(PortError(
                    "promotion candidate is not pending user-observation evidence".into(),
                ));
            };
            let disposition = memory_write_disposition(
                parse_write_class(&class)?,
                temporary,
                DomainConsent::parse(&consent)?,
            );
            // Parse the persisted domain as an integrity check even though the
            // consent join uses its encoded primary key.
            let _ = MemoryDomain::parse(&domain)?;
            policy_results.push((provenance.candidate_id, disposition));
        }
        if policy_results
            .iter()
            .any(|(_, value)| *value != MemoryWriteDisposition::AutoApproved)
        {
            for (candidate_id, disposition) in &policy_results {
                let (state, policy_state, reason) = match disposition {
                    // Keep this proposal in the approval queue.  It is no
                    // longer leased because the observation is complete, but
                    // it is not a safety rejection and remains reviewable.
                    MemoryWriteDisposition::PendingApproval => {
                        ("pending", "pending_approval", "pending approval")
                    }
                    MemoryWriteDisposition::Rejected => {
                        ("rejected", "rejected", "memory policy rejected candidate")
                    }
                    MemoryWriteDisposition::AutoApproved => {
                        ("pending", "unreviewed", "policy batch held for approval")
                    }
                };
                transaction.execute(
                    "UPDATE memory_candidates SET candidate_state=?1,policy_state=?2,rejection_reason=?3,updated_at=?4 WHERE id=?5 AND candidate_state='pending'",
                    params![state, policy_state, reason, now, candidate_id],
                ).map_err(memory_error)?;
            }
            transaction.execute(
                "UPDATE memory_classification_runs SET transport_outcome='completed',candidate_count=(SELECT COUNT(*) FROM memory_candidates WHERE classification_run_id=?1),error_reason='memory policy decision',completed_at=?2 WHERE id=?1 AND transport_outcome='pending'",
                params![change_set.classification_run_id, now],
            ).map_err(memory_error)?;
            transaction.execute(
                "UPDATE memory_observations SET processing_state='completed',lease_owner=NULL,lease_expires_at=NULL,attempt_token=NULL,updated_at=?1 WHERE id=?2 AND lease_owner=?3 AND attempt_token=?4 AND deletion_generation=?5",
                params![now, lease.observation_id, lease.owner, lease.attempt_token, lease.deletion_generation],
            ).map_err(memory_error)?;
            let ids = "[]";
            transaction.execute(
                "INSERT INTO memory_promotions(request_key,observation_id,classifier_version,schema_version,input_hash,classification_run_id,change_set_fingerprint,status,result_memory_ids,created_at,committed_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'committed',?8,?9,?9)",
                params![change_set.request_key, lease.observation_id, change_set.classifier_version, change_set.schema_version, change_set.input_hash, change_set.classification_run_id, fingerprint, ids, now],
            ).map_err(memory_error)?;
            transaction.commit().map_err(memory_error)?;
            return Ok(PromotionResult {
                request_key: change_set.request_key.clone(),
                promoted_memory_ids: Vec::new(),
                already_applied: false,
            });
        }
        let source = EvidenceSource::new(lease.conversation_id.clone(), lease.turn_id);
        let mut memory_ids = Vec::with_capacity(change_set.actions.len());
        for (
            VersionedMemoryAction {
                action,
                expected_revision,
            },
            provenance,
        ) in change_set.actions.iter().zip(&change_set.provenance)
        {
            let has_target = matches!(
                action,
                MemoryAction::Reinforce { .. } | MemoryAction::Supersede { .. }
            );
            if has_target != expected_revision.is_some() {
                return Err(PortError(
                    "promotion action revision guard is incomplete".into(),
                ));
            }
            if let Some(target) = action_target(action) {
                let tombstoned: bool = transaction
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM memory_tombstones WHERE memory_id=?1)",
                        [target],
                        |row| row.get(0),
                    )
                    .map_err(memory_error)?;
                if tombstoned {
                    return Err(PortError("promotion target was deleted".into()));
                }
            }
            let candidate: Option<(String, String, String, Option<i64>, Option<i64>, String, String, String, String, String, String, String, String, String, String, i64, i64, String, String, String)> = transaction.query_row(
                "SELECT c.content,c.proposed_operation,c.proposed_relation,c.target_memory_id,c.expected_target_revision,c.subject_scope,c.epistemic_form,c.attribution,c.speech_act,c.source_mode,c.polarity,c.conditionality,c.fictionality,c.verification_status,c.temporal_scope,c.source_start,c.source_end,c.normalization_json,o.input_hash,o.user_text FROM memory_candidates c JOIN memory_observations o ON o.id=c.observation_id JOIN memory_classification_runs r ON r.id=c.classification_run_id WHERE c.id=?1 AND c.observation_id=?2 AND c.classification_run_id=?3 AND c.candidate_state='pending' AND c.attribution!='assistant' AND o.deletion_generation=?4 AND r.lease_attempt_token=?5 AND r.classifier_version=?6 AND r.schema_version=?7 AND r.input_hash=?8",
                params![provenance.candidate_id, lease.observation_id, change_set.classification_run_id, lease.deletion_generation, lease.attempt_token, change_set.classifier_version, change_set.schema_version, change_set.input_hash],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?, row.get(15)?, row.get(16)?, row.get(17)?, row.get(18)?, row.get(19)?)),
            ).optional().map_err(|error| PortError(error.to_string()))?;
            let Some((
                candidate_content,
                candidate_operation,
                candidate_relation,
                candidate_target,
                candidate_revision,
                subject_scope,
                epistemic_form,
                attribution,
                speech_act,
                source_mode,
                polarity,
                conditionality,
                fictionality,
                verification_status,
                temporal_scope,
                source_start,
                source_end,
                normalization_json,
                source_hash,
                source_text,
            )) = candidate
            else {
                return Err(PortError(
                    "promotion candidate is not pending user-observation evidence".into(),
                ));
            };
            if candidate_operation != action_operation(action)
                || candidate_relation != provenance.relation
                || candidate_target != action_target(action)
                || candidate_revision != *expected_revision
                || !action_matches_candidate_content(action, &candidate_content)
            {
                return Err(PortError(
                    "promotion action does not match its validated candidate".into(),
                ));
            }
            let typed = typed_candidate_from_storage(
                &candidate_content,
                &subject_scope,
                &epistemic_form,
                &attribution,
                &speech_act,
                &source_mode,
                &polarity,
                &conditionality,
                &fictionality,
                &verification_status,
                &temporal_scope,
                source_start,
                source_end,
                &normalization_json,
                &source_hash,
                &candidate_operation,
                &candidate_relation,
                candidate_target,
                candidate_revision,
            )?;
            let targets = validator_targets(&transaction, candidate_target)?;
            validate_candidate_for_source(&typed, &source_hash, &source_text, &targets).map_err(
                |_| PortError("promotion candidate no longer passes typed validation".into()),
            )?;
            if let MemoryAction::Add { content, .. } | MemoryAction::Supersede { content, .. } =
                action
                && !is_safe_persistent_content(content)
            {
                return Err(PortError(
                    "refusing to promote secret-shaped memory content".into(),
                ));
            }
            let id = match action {
                MemoryAction::Add { content, pinned } => {
                    apply_add(&transaction, content, *pinned, &source, now)?
                }
                MemoryAction::Reinforce { memory_id, pin } => apply_reinforce_versioned(
                    &transaction,
                    *memory_id,
                    expected_revision.expect("guard checked"),
                    *pin,
                    &source,
                    now,
                )?,
                MemoryAction::Supersede {
                    old_memory_id,
                    content,
                    pin_replacement,
                } => apply_supersede_versioned(
                    &transaction,
                    *old_memory_id,
                    expected_revision.expect("guard checked"),
                    content,
                    *pin_replacement,
                    &source,
                    now,
                )?,
                MemoryAction::Ignore => return Err(PortError("ignore is not promotable".into())),
            };
            memory_ids.push(id);
        }
        for (memory_id, provenance) in memory_ids.iter().zip(&change_set.provenance) {
            transaction.execute(
                "INSERT INTO memory_provenance(memory_id,observation_id,candidate_id,relation,created_at) VALUES(?1,?2,?3,?4,?5)",
                params![memory_id, lease.observation_id, provenance.candidate_id, provenance.relation, now],
            ).map_err(|error| PortError(error.to_string()))?;
            transaction.execute("UPDATE memory_candidates SET candidate_state='promoted',policy_state='auto_approved',updated_at=?1 WHERE id=?2", params![now, provenance.candidate_id]).map_err(|error| PortError(error.to_string()))?;
        }
        transaction.execute(
            "UPDATE memory_classification_runs SET transport_outcome='completed',candidate_count=(SELECT COUNT(*) FROM memory_candidates WHERE classification_run_id=?1),error_reason=NULL,completed_at=?2 WHERE id=?1 AND transport_outcome='pending'",
            params![change_set.classification_run_id, now],
        ).map_err(memory_error)?;
        let result_memory_ids =
            serde_json::to_string(&memory_ids).map_err(|error| PortError(error.to_string()))?;
        transaction.execute(
            "INSERT INTO memory_promotions(request_key,observation_id,classifier_version,schema_version,input_hash,classification_run_id,change_set_fingerprint,status,result_memory_ids,created_at,committed_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'committed',?8,?9,?9)",
            params![change_set.request_key, lease.observation_id, change_set.classifier_version, change_set.schema_version, change_set.input_hash, change_set.classification_run_id, fingerprint, result_memory_ids, now],
        ).map_err(|error| PortError(error.to_string()))?;
        transaction.execute(
            "UPDATE memory_observations SET processing_state='completed',lease_owner=NULL,lease_expires_at=NULL,attempt_token=NULL,updated_at=?1 WHERE id=?2 AND lease_owner=?3 AND attempt_token=?4 AND deletion_generation=?5",
            params![now, lease.observation_id, lease.owner, lease.attempt_token, lease.deletion_generation],
        ).map_err(|error| PortError(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| PortError(error.to_string()))?;
        Ok(PromotionResult {
            request_key: change_set.request_key.clone(),
            promoted_memory_ids: memory_ids,
            already_applied: false,
        })
    }
}

fn encode_outcome(outcome: ObservationOutcome) -> &'static str {
    match outcome {
        ObservationOutcome::Pending => "pending",
        ObservationOutcome::Completed => "completed",
        ObservationOutcome::Cancelled => "cancelled",
        ObservationOutcome::LlmFailed => "llm_failed",
        ObservationOutcome::HistoryPersistFailed => "history_persist_failed",
        ObservationOutcome::Interrupted => "interrupted",
    }
}

fn encode_candidate_operation(value: CandidateOperation) -> &'static str {
    match value {
        CandidateOperation::Add => "add",
        CandidateOperation::Reinforce => "reinforce",
        CandidateOperation::Supersede => "supersede",
    }
}
fn encode_candidate_relation(value: CandidateProvenanceRelation) -> &'static str {
    match value {
        CandidateProvenanceRelation::Originated => "originated",
        CandidateProvenanceRelation::Reasserted => "reasserted",
        CandidateProvenanceRelation::Corrected => "corrected",
        CandidateProvenanceRelation::ChangedStance => "changed_stance",
        CandidateProvenanceRelation::Contradicted => "contradicted",
    }
}
fn encode_memory_domain(value: MemoryDomain) -> &'static str {
    value.as_str()
}
fn encode_write_class(value: MemoryWriteClass) -> &'static str {
    match value {
        MemoryWriteClass::NormalExplicit => "normal_explicit",
        MemoryWriteClass::Inferred => "inferred",
        MemoryWriteClass::Personal => "personal",
        MemoryWriteClass::Sensitive => "sensitive",
        MemoryWriteClass::Secret => "secret",
        MemoryWriteClass::NeverStore => "never_store",
    }
}
fn parse_write_class(value: &str) -> Result<MemoryWriteClass, PortError> {
    match value {
        "normal_explicit" => Ok(MemoryWriteClass::NormalExplicit),
        "inferred" => Ok(MemoryWriteClass::Inferred),
        "personal" => Ok(MemoryWriteClass::Personal),
        "sensitive" => Ok(MemoryWriteClass::Sensitive),
        "secret" => Ok(MemoryWriteClass::Secret),
        "never_store" => Ok(MemoryWriteClass::NeverStore),
        _ => Err(PortError("unknown memory write class".into())),
    }
}
fn action_operation(action: &MemoryAction) -> &'static str {
    match action {
        MemoryAction::Add { .. } => "add",
        MemoryAction::Reinforce { .. } => "reinforce",
        MemoryAction::Supersede { .. } => "supersede",
        MemoryAction::Ignore => "ignore",
    }
}
fn action_target(action: &MemoryAction) -> Option<i64> {
    match action {
        MemoryAction::Reinforce { memory_id, .. } => Some(*memory_id),
        MemoryAction::Supersede { old_memory_id, .. } => Some(*old_memory_id),
        MemoryAction::Add { .. } | MemoryAction::Ignore => None,
    }
}

fn promotion_fingerprint(change_set: &ProvisionalMemoryChangeSet) -> String {
    use std::fmt::Write;

    let mut fields = format!(
        "run:{}|observation:{}|classifier:{}|schema:{}|input:{}",
        change_set.classification_run_id,
        change_set.lease.observation_id,
        change_set.classifier_version,
        change_set.schema_version,
        change_set.input_hash
    );
    for (action, provenance) in change_set.actions.iter().zip(&change_set.provenance) {
        let _ = write!(
            fields,
            "|candidate:{}:{}:{:?}:{:?}",
            provenance.candidate_id, provenance.relation, action.action, action.expected_revision
        );
    }
    input_hash(&fields)
}

#[allow(clippy::too_many_arguments)]
fn typed_candidate_from_storage(
    content: &str,
    subject_scope: &str,
    epistemic_form: &str,
    attribution: &str,
    speech_act: &str,
    source_mode: &str,
    polarity: &str,
    conditionality: &str,
    fictionality: &str,
    verification_status: &str,
    temporal_scope: &str,
    source_start: i64,
    source_end: i64,
    normalization_json: &str,
    source_hash: &str,
    operation: &str,
    relation: &str,
    target_memory_id: Option<i64>,
    expected_target_revision: Option<i64>,
) -> Result<TypedCandidate, PortError> {
    let start = usize::try_from(source_start).map_err(memory_error)?;
    let end = usize::try_from(source_end).map_err(memory_error)?;
    let normalization_edits = serde_json::from_str(normalization_json)
        .map_err(|_| PortError("stored candidate normalization trace is invalid".into()))?;
    let operation = parse_candidate_operation(operation)?;
    let relation = parse_candidate_relation(relation)?;
    let atom = MemoryAtom {
        id: 0,
        revision: 1,
        content: content.into(),
        subject_scope: parse_subject_scope(subject_scope)?,
        epistemic_form: parse_epistemic_form(epistemic_form)?,
        attribution: parse_attribution(attribution)?,
        discourse: DiscourseFeatures {
            speech_act: parse_speech_act(speech_act)?,
            source_mode: parse_source_mode(source_mode)?,
            polarity: parse_polarity(polarity)?,
            conditionality: parse_conditionality(conditionality)?,
            fictionality: parse_fictionality(fictionality)?,
        },
        verification_status: parse_verification_status(verification_status)?,
        temporal_scope: parse_temporal_scope(temporal_scope)?,
        lifecycle_state: MemoryState::Active,
        source_spans: vec![SourceSpan {
            source_id: source_hash.into(),
            start,
            end,
        }],
    };
    let persisted = PersistedCandidate {
        classification_run_id: 0,
        ordinal: 0,
        atom,
        target_memory_id,
        expected_target_revision,
        operation,
        relation,
        domain: MemoryDomain::SemanticUser,
        write_class: MemoryWriteClass::NormalExplicit,
        normalization_edits,
    };
    let proposed_action = proposed_action_for_persisted_candidate(&persisted)?;
    Ok(TypedCandidate {
        atom: persisted.atom.clone(),
        relation: candidate_relation_for(persisted.relation),
        target_memory_id: persisted.target_memory_id,
        expected_target_revision: persisted.expected_target_revision,
        normalization_edits: persisted.normalization_edits,
        proposed_action,
    })
}

fn parse_candidate_operation(value: &str) -> Result<CandidateOperation, PortError> {
    match value {
        "add" => Ok(CandidateOperation::Add),
        "reinforce" => Ok(CandidateOperation::Reinforce),
        "supersede" => Ok(CandidateOperation::Supersede),
        _ => Err(PortError("unknown candidate operation".into())),
    }
}

fn parse_candidate_relation(value: &str) -> Result<CandidateProvenanceRelation, PortError> {
    match value {
        "originated" => Ok(CandidateProvenanceRelation::Originated),
        "reasserted" => Ok(CandidateProvenanceRelation::Reasserted),
        "corrected" => Ok(CandidateProvenanceRelation::Corrected),
        "changed_stance" => Ok(CandidateProvenanceRelation::ChangedStance),
        "contradicted" => Ok(CandidateProvenanceRelation::Contradicted),
        _ => Err(PortError("unknown candidate relation".into())),
    }
}
fn action_matches_candidate_content(action: &MemoryAction, candidate_content: &str) -> bool {
    match action {
        MemoryAction::Add { content, .. } | MemoryAction::Supersede { content, .. } => {
            content == candidate_content
        }
        MemoryAction::Reinforce { .. } => true,
        MemoryAction::Ignore => false,
    }
}

fn fts_disjunction(query: &str) -> Option<String> {
    let chars = query.trim().chars().collect::<Vec<_>>();
    if chars.len() < 3 {
        return None;
    }
    let mut seen = HashSet::new();
    let phrases = chars
        .windows(3)
        .map(|window| window.iter().collect::<String>())
        .filter(|phrase| phrase.chars().any(|character| !character.is_whitespace()))
        .filter(|phrase| seen.insert(phrase.clone()))
        .collect::<Vec<_>>();
    if phrases.is_empty() {
        return None;
    }
    let selected = if phrases.len() <= MAX_FTS_PHRASES {
        phrases
    } else {
        (0..MAX_FTS_PHRASES)
            .map(|index| {
                let position = index * (phrases.len() - 1) / (MAX_FTS_PHRASES - 1);
                phrases[position].clone()
            })
            .collect()
    };
    Some(
        selected
            .into_iter()
            .map(|phrase| format!("\"{}\"", phrase.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

struct CandidateRow {
    id: i64,
    revision: i64,
    content: String,
    state: String,
    pinned: bool,
    mention_count: i64,
    last_seen_at: i64,
    bm25: f64,
}

fn query_candidate_rows(
    connection: &Connection,
    sql: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<CandidateRow>, PortError> {
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| PortError(error.to_string()))?;
    statement
        .query_map(params![query, limit], |row| {
            Ok(CandidateRow {
                id: row.get(0)?,
                revision: row.get(1)?,
                content: row.get(2)?,
                state: row.get(3)?,
                pinned: row.get(4)?,
                mention_count: row.get(5)?,
                last_seen_at: row.get(6)?,
                bm25: row.get(7)?,
            })
        })
        .map_err(|error| PortError(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| PortError(error.to_string()))
}

fn parse_state(state: &str) -> Result<MemoryState, PortError> {
    match state {
        "active" => Ok(MemoryState::Active),
        "dormant" => Ok(MemoryState::Dormant),
        "superseded" => Ok(MemoryState::Superseded),
        value => Err(PortError(format!("unknown memory state: {value}"))),
    }
}

fn parse_subject_scope(value: &str) -> Result<SubjectScope, PortError> {
    match value {
        "user_self" => Ok(SubjectScope::UserSelf),
        "external_world" => Ok(SubjectScope::ExternalWorld),
        "other_person" => Ok(SubjectScope::OtherPerson),
        "fictional_subject" => Ok(SubjectScope::FictionalSubject),
        "legacy_unknown" => Ok(SubjectScope::LegacyUnknown),
        _ => Err(PortError(format!("unknown subject scope: {value}"))),
    }
}
fn parse_epistemic_form(value: &str) -> Result<EpistemicForm, PortError> {
    match value {
        "fact_claim" => Ok(EpistemicForm::FactClaim),
        "belief" => Ok(EpistemicForm::Belief),
        "impression" => Ok(EpistemicForm::Impression),
        "prediction_or_hunch" => Ok(EpistemicForm::PredictionOrHunch),
        "metaphor" => Ok(EpistemicForm::Metaphor),
        "emotion" => Ok(EpistemicForm::Emotion),
        "legacy_untyped" => Ok(EpistemicForm::LegacyUntyped),
        _ => Err(PortError(format!("unknown epistemic form: {value}"))),
    }
}
fn parse_attribution(value: &str) -> Result<Attribution, PortError> {
    match value {
        "user" => Ok(Attribution::User),
        "assistant" => Ok(Attribution::Assistant),
        "named_third_party" => Ok(Attribution::NamedThirdParty),
        "external_source" => Ok(Attribution::ExternalSource),
        "unknown" => Ok(Attribution::Unknown),
        _ => Err(PortError(format!("unknown attribution: {value}"))),
    }
}
fn parse_speech_act(value: &str) -> Result<SpeechAct, PortError> {
    match value {
        "asserted" => Ok(SpeechAct::Asserted),
        "questioned" => Ok(SpeechAct::Questioned),
        "unknown" => Ok(SpeechAct::Unknown),
        _ => Err(PortError(format!("unknown speech act: {value}"))),
    }
}
fn parse_source_mode(value: &str) -> Result<SourceMode, PortError> {
    match value {
        "direct" => Ok(SourceMode::Direct),
        "reported" => Ok(SourceMode::Reported),
        "quoted" => Ok(SourceMode::Quoted),
        _ => Err(PortError(format!("unknown source mode: {value}"))),
    }
}
fn parse_polarity(value: &str) -> Result<Polarity, PortError> {
    match value {
        "affirmed" => Ok(Polarity::Affirmed),
        "negated" => Ok(Polarity::Negated),
        "unknown" => Ok(Polarity::Unknown),
        _ => Err(PortError(format!("unknown polarity: {value}"))),
    }
}
fn parse_conditionality(value: &str) -> Result<Conditionality, PortError> {
    match value {
        "actual" => Ok(Conditionality::Actual),
        "hypothetical" => Ok(Conditionality::Hypothetical),
        "unknown" => Ok(Conditionality::Unknown),
        _ => Err(PortError(format!("unknown conditionality: {value}"))),
    }
}
fn parse_fictionality(value: &str) -> Result<Fictionality, PortError> {
    match value {
        "real_world" => Ok(Fictionality::RealWorld),
        "fictional" => Ok(Fictionality::Fictional),
        "unknown" => Ok(Fictionality::Unknown),
        _ => Err(PortError(format!("unknown fictionality: {value}"))),
    }
}
fn parse_verification_status(value: &str) -> Result<VerificationStatus, PortError> {
    match value {
        "not_applicable" => Ok(VerificationStatus::NotApplicable),
        "user_reported" => Ok(VerificationStatus::UserReported),
        "unverified_external_claim" => Ok(VerificationStatus::UnverifiedExternalClaim),
        "externally_corroborated" => Ok(VerificationStatus::ExternallyCorroborated),
        "externally_contradicted" => Ok(VerificationStatus::ExternallyContradicted),
        "disputed" => Ok(VerificationStatus::Disputed),
        "unknown" => Ok(VerificationStatus::Unknown),
        _ => Err(PortError(format!("unknown verification status: {value}"))),
    }
}
fn parse_temporal_scope(value: &str) -> Result<TemporalScope, PortError> {
    match value {
        "stable" => Ok(TemporalScope::Stable),
        "current" => Ok(TemporalScope::Current),
        "past" => Ok(TemporalScope::Past),
        "future" => Ok(TemporalScope::Future),
        "unknown" => Ok(TemporalScope::Unknown),
        _ => Err(PortError(format!("unknown temporal scope: {value}"))),
    }
}

fn encode_subject_scope(value: SubjectScope) -> &'static str {
    match value {
        SubjectScope::UserSelf => "user_self",
        SubjectScope::ExternalWorld => "external_world",
        SubjectScope::OtherPerson => "other_person",
        SubjectScope::FictionalSubject => "fictional_subject",
        SubjectScope::LegacyUnknown => "legacy_unknown",
    }
}
fn encode_epistemic_form(value: EpistemicForm) -> &'static str {
    match value {
        EpistemicForm::FactClaim => "fact_claim",
        EpistemicForm::Belief => "belief",
        EpistemicForm::Impression => "impression",
        EpistemicForm::PredictionOrHunch => "prediction_or_hunch",
        EpistemicForm::Metaphor => "metaphor",
        EpistemicForm::Emotion => "emotion",
        EpistemicForm::LegacyUntyped => "legacy_untyped",
    }
}
fn encode_attribution(value: Attribution) -> &'static str {
    match value {
        Attribution::User => "user",
        Attribution::Assistant => "assistant",
        Attribution::NamedThirdParty => "named_third_party",
        Attribution::ExternalSource => "external_source",
        Attribution::Unknown => "unknown",
    }
}
fn encode_speech_act(value: SpeechAct) -> &'static str {
    match value {
        SpeechAct::Asserted => "asserted",
        SpeechAct::Questioned => "questioned",
        SpeechAct::Unknown => "unknown",
    }
}
fn encode_source_mode(value: SourceMode) -> &'static str {
    match value {
        SourceMode::Direct => "direct",
        SourceMode::Reported => "reported",
        SourceMode::Quoted => "quoted",
    }
}
fn encode_polarity(value: Polarity) -> &'static str {
    match value {
        Polarity::Affirmed => "affirmed",
        Polarity::Negated => "negated",
        Polarity::Unknown => "unknown",
    }
}
fn encode_conditionality(value: Conditionality) -> &'static str {
    match value {
        Conditionality::Actual => "actual",
        Conditionality::Hypothetical => "hypothetical",
        Conditionality::Unknown => "unknown",
    }
}
fn encode_fictionality(value: Fictionality) -> &'static str {
    match value {
        Fictionality::RealWorld => "real_world",
        Fictionality::Fictional => "fictional",
        Fictionality::Unknown => "unknown",
    }
}
fn encode_verification_status(value: VerificationStatus) -> &'static str {
    match value {
        VerificationStatus::NotApplicable => "not_applicable",
        VerificationStatus::UserReported => "user_reported",
        VerificationStatus::UnverifiedExternalClaim => "unverified_external_claim",
        VerificationStatus::ExternallyCorroborated => "externally_corroborated",
        VerificationStatus::ExternallyContradicted => "externally_contradicted",
        VerificationStatus::Disputed => "disputed",
        VerificationStatus::Unknown => "unknown",
    }
}
fn encode_temporal_scope(value: TemporalScope) -> &'static str {
    match value {
        TemporalScope::Stable => "stable",
        TemporalScope::Current => "current",
        TemporalScope::Past => "past",
        TemporalScope::Future => "future",
        TemporalScope::Unknown => "unknown",
    }
}

fn parse_evidence_kind(kind: &str) -> Result<EvidenceKind, PortError> {
    match kind {
        "user_mention" => Ok(EvidenceKind::UserMention),
        "recalled" => Ok(EvidenceKind::Recalled),
        "pinned" => Ok(EvidenceKind::Pinned),
        "imported" => Ok(EvidenceKind::Imported),
        value => Err(PortError(format!("unknown memory evidence kind: {value}"))),
    }
}

fn load_evidence_for_memories(
    connection: &Connection,
    memory_ids: &[i64],
) -> Result<HashMap<i64, Vec<MemoryEvidence>>, PortError> {
    if memory_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = std::iter::repeat_n("?", memory_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let mut statement = connection
        .prepare(&format!(
            "SELECT memory_id,id,kind,occurred_at,weight FROM memory_evidence \
             WHERE memory_id IN ({placeholders}) ORDER BY memory_id,id"
        ))
        .map_err(|error| PortError(error.to_string()))?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(memory_ids), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, f64>(4)?,
            ))
        })
        .map_err(|error| PortError(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| PortError(error.to_string()))?;
    let mut evidence_by_memory = HashMap::<i64, Vec<MemoryEvidence>>::new();
    for (memory_id, id, kind, occurred_at, weight) in rows {
        evidence_by_memory
            .entry(memory_id)
            .or_default()
            .push(MemoryEvidence {
                id,
                kind: parse_evidence_kind(&kind)?,
                occurred_at,
                weight,
            });
    }
    Ok(evidence_by_memory)
}

fn load_evidence(
    connection: &Connection,
    memory_id: i64,
) -> Result<Vec<MemoryEvidence>, PortError> {
    Ok(load_evidence_for_memories(connection, &[memory_id])?
        .remove(&memory_id)
        .unwrap_or_default())
}

fn load_active_maintenance_rows(
    transaction: &Transaction<'_>,
    after: Option<(i64, i64)>,
    sql_limit: i64,
) -> Result<Vec<(i64, i64)>, PortError> {
    let mut statement = transaction
        .prepare(
            "SELECT id,last_seen_at FROM memories WHERE state='active' AND pinned=0 AND (?1 IS NULL OR last_seen_at>?1 OR (last_seen_at=?1 AND id>?2)) ORDER BY last_seen_at,id LIMIT ?3",
        )
        .map_err(|error| PortError(error.to_string()))?;
    statement
        .query_map(
            params![
                after.map(|cursor| cursor.0),
                after.map(|cursor| cursor.1),
                sql_limit
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| PortError(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| PortError(error.to_string()))
}

fn load_expired_maintenance_rows(
    transaction: &Transaction<'_>,
    cutoff: i64,
    after: Option<(i64, i64)>,
    sql_limit: i64,
) -> Result<Vec<(i64, i64)>, PortError> {
    let mut statement = transaction
        .prepare(
            "SELECT id,state_changed_at FROM memories WHERE state IN ('dormant','superseded') AND pinned=0 AND state_changed_at<=?1 AND (?2 IS NULL OR state_changed_at>?2 OR (state_changed_at=?2 AND id>?3)) ORDER BY state_changed_at,id LIMIT ?4",
        )
        .map_err(|error| PortError(error.to_string()))?;
    statement
        .query_map(
            params![
                cutoff,
                after.map(|cursor| cursor.0),
                after.map(|cursor| cursor.1),
                sql_limit
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| PortError(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| PortError(error.to_string()))
}

fn source_turn_id(source: &EvidenceSource) -> Result<i64, PortError> {
    i64::try_from(source.turn_id).map_err(|error| PortError(error.to_string()))
}

fn insert_evidence(
    transaction: &Transaction<'_>,
    memory_id: i64,
    kind: &str,
    weight: f64,
    source: &EvidenceSource,
    now: i64,
) -> Result<usize, PortError> {
    transaction
        .execute(
            "INSERT OR IGNORE INTO memory_evidence(memory_id,kind,occurred_at,source_conversation_id,source_turn_id,weight) VALUES(?1,?2,?3,?4,?5,?6)",
            params![memory_id, kind, now, source.conversation_id, source_turn_id(source)?, weight],
        )
        .map_err(|error| PortError(error.to_string()))
}

fn find_source_memory(
    transaction: &Transaction<'_>,
    content: &str,
    source: &EvidenceSource,
) -> Result<Option<i64>, PortError> {
    transaction
        .query_row(
            "SELECT m.id FROM memories m JOIN memory_evidence e ON e.memory_id=m.id WHERE m.content=?1 AND e.kind='user_mention' AND e.source_conversation_id=?2 AND e.source_turn_id=?3 ORDER BY m.id LIMIT 1",
            params![content, source.conversation_id, source_turn_id(source)?],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| PortError(error.to_string()))
}

fn find_source_memory_excluding(
    transaction: &Transaction<'_>,
    content: &str,
    source: &EvidenceSource,
    excluded_memory_id: i64,
) -> Result<Option<i64>, PortError> {
    transaction
        .query_row(
            "SELECT m.id FROM memories m JOIN memory_evidence e ON e.memory_id=m.id WHERE m.content=?1 AND m.id!=?2 AND e.kind='user_mention' AND e.source_conversation_id=?3 AND e.source_turn_id=?4 ORDER BY m.id LIMIT 1",
            params![content, excluded_memory_id, source.conversation_id, source_turn_id(source)?],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| PortError(error.to_string()))
}

fn create_memory(
    transaction: &Transaction<'_>,
    content: &str,
    pinned: bool,
    source: &EvidenceSource,
    now: i64,
) -> Result<i64, PortError> {
    transaction
        .execute(
            "INSERT INTO memories(content,created_at,updated_at,state,pinned,mention_count,last_seen_at) VALUES(?1,?2,?2,'active',?3,1,?2)",
            params![content, now, pinned],
        )
        .map_err(|error| PortError(error.to_string()))?;
    let id = transaction.last_insert_rowid();
    insert_evidence(transaction, id, "user_mention", 1.0, source, now)?;
    Ok(id)
}

fn ensure_memory_exists(transaction: &Transaction<'_>, id: i64) -> Result<(), PortError> {
    let exists = transaction
        .query_row("SELECT 1 FROM memories WHERE id=?1", [id], |_| Ok(()))
        .optional()
        .map_err(|error| PortError(error.to_string()))?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(PortError(format!("memory {id} does not exist")))
    }
}

fn apply_add(
    transaction: &Transaction<'_>,
    content: &str,
    pinned: bool,
    source: &EvidenceSource,
    now: i64,
) -> Result<i64, PortError> {
    if let Some(id) = find_source_memory(transaction, content, source)? {
        Ok(id)
    } else {
        create_memory(transaction, content, pinned, source, now)
    }
}

fn apply_reinforce(
    transaction: &Transaction<'_>,
    memory_id: i64,
    pin: bool,
    source: &EvidenceSource,
    now: i64,
) -> Result<i64, PortError> {
    ensure_memory_exists(transaction, memory_id)?;
    let inserted = insert_evidence(transaction, memory_id, "user_mention", 1.0, source, now)?;
    if inserted == 0 {
        return Ok(memory_id);
    }
    transaction
        .execute(
            "UPDATE memories SET revision=revision+1,mention_count=mention_count+?1,last_seen_at=MAX(last_seen_at,?2),updated_at=MAX(updated_at,?2),pinned=CASE WHEN state!='superseded' AND ?3 THEN 1 ELSE pinned END,state_changed_at=CASE WHEN state='dormant' THEN NULL ELSE state_changed_at END,state=CASE WHEN state='dormant' THEN 'active' ELSE state END WHERE id=?4",
            params![1, now, pin, memory_id],
        )
        .map_err(|error| PortError(error.to_string()))?;
    Ok(memory_id)
}

fn load_versioned_lifecycle_target(
    transaction: &Transaction<'_>,
    memory_id: i64,
    expected_revision: i64,
) -> Result<(MemoryState, bool), PortError> {
    let target = transaction
        .query_row(
            "SELECT revision,state,pinned FROM memories WHERE id=?1",
            [memory_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? != 0,
                ))
            },
        )
        .optional()
        .map_err(|error| PortError(error.to_string()))?
        .ok_or_else(|| PortError(format!("memory {memory_id} does not exist")))?;
    if target.0 != expected_revision {
        return Err(PortError(format!(
            "stale memory target {memory_id} at revision {expected_revision}"
        )));
    }
    Ok((parse_state(&target.1)?, target.2))
}

fn apply_reinforce_versioned(
    transaction: &Transaction<'_>,
    memory_id: i64,
    expected_revision: i64,
    pin: bool,
    source: &EvidenceSource,
    now: i64,
) -> Result<i64, PortError> {
    let (state, pinned) =
        load_versioned_lifecycle_target(transaction, memory_id, expected_revision)?;
    if state == MemoryState::Superseded {
        return Err(PortError(format!(
            "cannot reinforce superseded memory {memory_id}"
        )));
    }
    let inserted = insert_evidence(transaction, memory_id, "user_mention", 1.0, source, now)?;
    let must_revive = state == MemoryState::Dormant;
    let must_pin = pin && !pinned;
    if inserted == 0 && !must_revive && !must_pin {
        return Ok(memory_id);
    }
    let changed = transaction
        .execute(
            "UPDATE memories SET revision=revision+1,mention_count=mention_count+?1,last_seen_at=CASE WHEN ?1 THEN MAX(last_seen_at,?2) ELSE last_seen_at END,updated_at=MAX(updated_at,?2),pinned=CASE WHEN ?3 THEN 1 ELSE pinned END,state_changed_at=CASE WHEN state='dormant' THEN NULL ELSE state_changed_at END,state=CASE WHEN state='dormant' THEN 'active' ELSE state END WHERE id=?4 AND revision=?5 AND state!='superseded'",
            params![i64::from(inserted != 0), now, pin, memory_id, expected_revision],
        )
        .map_err(|error| PortError(error.to_string()))?;
    if changed != 1 {
        return Err(PortError(format!(
            "stale memory target {memory_id} at revision {expected_revision}"
        )));
    }
    Ok(memory_id)
}

fn apply_supersede(
    transaction: &Transaction<'_>,
    old_memory_id: i64,
    content: &str,
    pin_replacement: bool,
    source: &EvidenceSource,
    now: i64,
) -> Result<i64, PortError> {
    ensure_memory_exists(transaction, old_memory_id)?;
    if let Some(id) = find_source_memory(transaction, content, source)? {
        return Ok(id);
    }
    let replacement_id = create_memory(transaction, content, pin_replacement, source, now)?;
    transaction
        .execute(
            "UPDATE memories SET revision=revision+1,state='superseded',pinned=0,state_changed_at=MAX(COALESCE(state_changed_at,updated_at,?1),updated_at,?1),superseded_by=?2,updated_at=MAX(updated_at,?1) WHERE id=?3",
            params![now, replacement_id, old_memory_id],
        )
        .map_err(|error| PortError(error.to_string()))?;
    Ok(replacement_id)
}

fn apply_supersede_versioned(
    transaction: &Transaction<'_>,
    old_memory_id: i64,
    expected_revision: i64,
    content: &str,
    pin_replacement: bool,
    source: &EvidenceSource,
    now: i64,
) -> Result<i64, PortError> {
    if load_versioned_lifecycle_target(transaction, old_memory_id, expected_revision)?.0
        == MemoryState::Superseded
    {
        return Err(PortError(format!(
            "cannot supersede already superseded memory {old_memory_id}"
        )));
    }
    let replacement_id = if let Some(id) =
        find_source_memory_excluding(transaction, content, source, old_memory_id)?
    {
        transaction
            .execute(
                "UPDATE memories SET revision=revision+1,state='active',pinned=CASE WHEN ?1 THEN 1 ELSE pinned END,state_changed_at=NULL,superseded_by=NULL,updated_at=MAX(updated_at,?2) WHERE id=?3 AND (state!='active' OR superseded_by IS NOT NULL OR (?1 AND pinned=0))",
                params![pin_replacement, now, id],
            )
            .map_err(|error| PortError(error.to_string()))?;
        id
    } else {
        let self_replacement = transaction
            .query_row(
                "SELECT 1 FROM memories WHERE id=?1 AND content=?2",
                params![old_memory_id, content],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| PortError(error.to_string()))?
            .is_some();
        if self_replacement {
            return Err(PortError(format!(
                "cannot supersede memory {old_memory_id} with itself as the replacement"
            )));
        }
        create_memory(transaction, content, pin_replacement, source, now)?
    };
    let changed = transaction
        .execute(
            "UPDATE memories SET revision=revision+1,state='superseded',pinned=0,state_changed_at=MAX(COALESCE(state_changed_at,updated_at,?1),updated_at,?1),superseded_by=?2,updated_at=MAX(updated_at,?1) WHERE id=?3 AND revision=?4 AND state!='superseded'",
            params![now, replacement_id, old_memory_id, expected_revision],
        )
        .map_err(|error| PortError(error.to_string()))?;
    if changed != 1 {
        return Err(PortError(format!(
            "stale memory target {old_memory_id} at revision {expected_revision}"
        )));
    }
    Ok(replacement_id)
}

impl MemoryStore for SqliteMemoryStore {
    fn load_summary(&self, conversation_id: &str) -> Result<Option<StoredSummary>, PortError> {
        self.database.connection().query_row("SELECT content, through_message_id FROM conversation_summaries WHERE conversation_id=?1", [conversation_id], |row| Ok(StoredSummary { content: row.get(0)?, through_message_id: row.get(1)? })).optional().map_err(|e| PortError(e.to_string()))
    }
    fn upsert_summary(
        &mut self,
        conversation_id: &str,
        content: &str,
        through_message_id: i64,
        updated_at: i64,
    ) -> Result<(), PortError> {
        if !is_safe_persistent_content(content) {
            return Err(PortError(
                "refusing to persist secret-shaped summary content".into(),
            ));
        }
        self.database.connection().execute("INSERT INTO conversation_summaries(conversation_id,content,through_message_id,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(conversation_id) DO UPDATE SET content=excluded.content,through_message_id=excluded.through_message_id,updated_at=excluded.updated_at", params![conversation_id,content,through_message_id,updated_at]).map(|_| ()).map_err(|e| PortError(e.to_string()))
    }
    fn upsert_memory(
        &mut self,
        source: Option<&str>,
        content: &str,
        updated_at: i64,
    ) -> Result<i64, PortError> {
        if !is_safe_persistent_content(content) {
            return Err(PortError(
                "refusing to persist secret-shaped memory content".into(),
            ));
        }
        if let Some(id) = self.database.connection().query_row("SELECT id FROM memories WHERE content=?1 AND source_conversation_id IS ?2 ORDER BY id LIMIT 1", params![content, source], |row| row.get(0)).optional().map_err(|e| PortError(e.to_string()))? {
            self.database.connection().execute("UPDATE memories SET updated_at=?1 WHERE id=?2", params![updated_at,id]).map_err(|e| PortError(e.to_string()))?;
            return Ok(id);
        }
        self.database.connection().execute("INSERT INTO memories(content,source_conversation_id,created_at,updated_at) VALUES(?1,?2,?3,?3)", params![content,source,updated_at]).map_err(|e| PortError(e.to_string()))?;
        Ok(self.database.connection().last_insert_rowid())
    }
    fn update_memory(&mut self, id: i64, content: &str, updated_at: i64) -> Result<(), PortError> {
        if !is_safe_persistent_content(content) {
            return Err(PortError(
                "refusing to persist secret-shaped memory content".into(),
            ));
        }
        self.database
            .connection()
            .execute(
                "UPDATE memories SET revision=revision+1,content=?1,updated_at=?2 WHERE id=?3",
                params![content, updated_at, id],
            )
            .map(|_| ())
            .map_err(|e| PortError(e.to_string()))
    }
    fn load_memory_atom(&self, id: i64) -> Result<Option<MemoryAtom>, PortError> {
        let values = self.database.connection().query_row(
            "SELECT id,revision,content,subject_scope,epistemic_form,attribution,speech_act,source_mode,polarity,conditionality,fictionality,verification_status,temporal_scope,state FROM memories WHERE id=?1",
            [id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, String>(8)?, row.get::<_, String>(9)?, row.get::<_, String>(10)?, row.get::<_, String>(11)?, row.get::<_, String>(12)?, row.get::<_, String>(13)?)),
        ).optional().map_err(|error| PortError(error.to_string()))?;
        values
            .map(
                |(
                    id,
                    revision,
                    content,
                    subject_scope,
                    epistemic_form,
                    attribution,
                    speech_act,
                    source_mode,
                    polarity,
                    conditionality,
                    fictionality,
                    verification_status,
                    temporal_scope,
                    state,
                )| {
                    Ok(MemoryAtom {
                        id,
                        revision,
                        content,
                        subject_scope: parse_subject_scope(&subject_scope)?,
                        epistemic_form: parse_epistemic_form(&epistemic_form)?,
                        attribution: parse_attribution(&attribution)?,
                        discourse: DiscourseFeatures {
                            speech_act: parse_speech_act(&speech_act)?,
                            source_mode: parse_source_mode(&source_mode)?,
                            polarity: parse_polarity(&polarity)?,
                            conditionality: parse_conditionality(&conditionality)?,
                            fictionality: parse_fictionality(&fictionality)?,
                        },
                        verification_status: parse_verification_status(&verification_status)?,
                        temporal_scope: parse_temporal_scope(&temporal_scope)?,
                        lifecycle_state: parse_state(&state)?,
                        source_spans: Vec::new(),
                    })
                },
            )
            .transpose()
    }
    fn update_memory_atom_cas(
        &mut self,
        atom: &MemoryAtom,
        expected_revision: i64,
        updated_at: i64,
    ) -> Result<MemoryAtom, PortError> {
        if !is_safe_persistent_content(&atom.content) {
            return Err(PortError(
                "refusing to persist secret-shaped memory content".into(),
            ));
        }
        if atom.attribution == Attribution::Assistant {
            return Err(PortError(
                "assistant-attributed memory cannot enter the long-term projection".into(),
            ));
        }
        if matches!(
            atom.verification_status,
            VerificationStatus::ExternallyCorroborated | VerificationStatus::ExternallyContradicted
        ) {
            return Err(PortError(
                "external verification states require a trusted verifier".into(),
            ));
        }
        let current = self
            .load_memory_atom(atom.id)?
            .ok_or_else(|| PortError(format!("memory {} does not exist", atom.id)))?;
        if current.lifecycle_state != atom.lifecycle_state {
            return Err(PortError(
                "typed semantic CAS cannot change lifecycle state; use a versioned lifecycle action"
                    .into(),
            ));
        }
        let changed = self.database.connection().execute(
            "UPDATE memories SET revision=revision+1,content=?1,updated_at=?2,subject_scope=?3,epistemic_form=?4,attribution=?5,speech_act=?6,source_mode=?7,polarity=?8,conditionality=?9,fictionality=?10,verification_status=?11,temporal_scope=?12 WHERE id=?13 AND revision=?14",
            params![atom.content, updated_at, encode_subject_scope(atom.subject_scope), encode_epistemic_form(atom.epistemic_form), encode_attribution(atom.attribution), encode_speech_act(atom.discourse.speech_act), encode_source_mode(atom.discourse.source_mode), encode_polarity(atom.discourse.polarity), encode_conditionality(atom.discourse.conditionality), encode_fictionality(atom.discourse.fictionality), encode_verification_status(atom.verification_status), encode_temporal_scope(atom.temporal_scope), atom.id, expected_revision],
        ).map_err(|error| PortError(error.to_string()))?;
        if changed != 1 {
            return Err(PortError(format!(
                "stale memory target {} at revision {expected_revision}",
                atom.id
            )));
        }
        self.load_memory_atom(atom.id)?
            .ok_or_else(|| PortError(format!("memory {} disappeared after CAS update", atom.id)))
    }
    fn delete_memory(&mut self, id: i64) -> Result<(), PortError> {
        self.delete_memory_fenced(id)
    }
    fn delete_summary(&mut self, conversation_id: &str) -> Result<(), PortError> {
        self.database
            .connection()
            .execute(
                "DELETE FROM conversation_summaries WHERE conversation_id=?1",
                [conversation_id],
            )
            .map(|_| ())
            .map_err(|e| PortError(e.to_string()))
    }
    fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryRecord>, PortError> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        if query.trim().chars().count() < 3 {
            let escaped = query
                .trim()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{escaped}%");
            let mut statement = self.database.connection().prepare("SELECT id,content FROM memories WHERE content LIKE ?1 ESCAPE '\\' ORDER BY updated_at DESC,id DESC LIMIT ?2").map_err(|e| PortError(e.to_string()))?;
            return statement
                .query_map(
                    params![pattern, i64::try_from(limit).unwrap_or(i64::MAX)],
                    |row| {
                        Ok(MemoryRecord {
                            id: row.get(0)?,
                            content: row.get(1)?,
                        })
                    },
                )
                .map_err(|e| PortError(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| PortError(e.to_string()));
        }
        let phrase = format!("\"{}\"", query.trim().replace('"', "\"\""));
        let mut statement=self.database.connection().prepare("SELECT m.id,m.content FROM memories_fts f JOIN memories m ON m.id=f.rowid WHERE memories_fts MATCH ?1 ORDER BY bm25(memories_fts),m.updated_at DESC LIMIT ?2").map_err(|e| PortError(e.to_string()))?;
        statement
            .query_map(
                params![phrase, i64::try_from(limit).unwrap_or(i64::MAX)],
                |row| {
                    Ok(MemoryRecord {
                        id: row.get(0)?,
                        content: row.get(1)?,
                    })
                },
            )
            .map_err(|e| PortError(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| PortError(e.to_string()))
    }

    fn find_consolidation_candidates(
        &self,
        query: &str,
        limit: usize,
        now: i64,
    ) -> Result<Vec<MemoryCandidate>, PortError> {
        self.lifecycle_search(query, limit, now, false)
    }

    fn apply_action(
        &mut self,
        action: &MemoryAction,
        source: &EvidenceSource,
        now: i64,
    ) -> Result<Option<i64>, PortError> {
        if let MemoryAction::Add { content, .. } | MemoryAction::Supersede { content, .. } = action
            && !is_safe_persistent_content(content)
        {
            return Err(PortError(
                "refusing to persist secret-shaped memory content".into(),
            ));
        }
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(|error| PortError(error.to_string()))?;
        let result = match action {
            MemoryAction::Add { content, pinned } => {
                Some(apply_add(&transaction, content, *pinned, source, now)?)
            }
            MemoryAction::Reinforce { memory_id, pin } => Some(apply_reinforce(
                &transaction,
                *memory_id,
                *pin,
                source,
                now,
            )?),
            MemoryAction::Supersede {
                old_memory_id,
                content,
                pin_replacement,
            } => Some(apply_supersede(
                &transaction,
                *old_memory_id,
                content,
                *pin_replacement,
                source,
                now,
            )?),
            MemoryAction::Ignore => None,
        };
        transaction
            .commit()
            .map_err(|error| PortError(error.to_string()))?;
        Ok(result)
    }

    fn apply_action_versioned(
        &mut self,
        action: &MemoryAction,
        expected_target_revision: Option<i64>,
        source: &EvidenceSource,
        now: i64,
    ) -> Result<Option<i64>, PortError> {
        if let MemoryAction::Add { content, .. } | MemoryAction::Supersede { content, .. } = action
            && !is_safe_persistent_content(content)
        {
            return Err(PortError(
                "refusing to persist secret-shaped memory content".into(),
            ));
        }
        let action_has_target = matches!(
            action,
            MemoryAction::Reinforce { .. } | MemoryAction::Supersede { .. }
        );
        if action_has_target != expected_target_revision.is_some() {
            return Err(PortError(
                "versioned lifecycle action must provide an expected revision exactly for target actions"
                    .into(),
            ));
        }
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(|error| PortError(error.to_string()))?;
        let result = match action {
            MemoryAction::Add { content, pinned } => {
                Some(apply_add(&transaction, content, *pinned, source, now)?)
            }
            MemoryAction::Reinforce { memory_id, pin } => Some(apply_reinforce_versioned(
                &transaction,
                *memory_id,
                expected_target_revision.expect("target action checked above"),
                *pin,
                source,
                now,
            )?),
            MemoryAction::Supersede {
                old_memory_id,
                content,
                pin_replacement,
            } => Some(apply_supersede_versioned(
                &transaction,
                *old_memory_id,
                expected_target_revision.expect("target action checked above"),
                content,
                *pin_replacement,
                source,
                now,
            )?),
            MemoryAction::Ignore => None,
        };
        transaction
            .commit()
            .map_err(|error| PortError(error.to_string()))?;
        Ok(result)
    }

    fn record_recalled(
        &mut self,
        ids: &[i64],
        source: &EvidenceSource,
        now: i64,
    ) -> Result<(), PortError> {
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(|error| PortError(error.to_string()))?;
        for id in ids {
            insert_evidence(&transaction, *id, "recalled", 0.15, source, now)?;
        }
        transaction
            .commit()
            .map_err(|error| PortError(error.to_string()))
    }

    fn search_active_for_prompt(
        &self,
        query: &str,
        limit: usize,
        now: i64,
    ) -> Result<Vec<MemoryCandidate>, PortError> {
        self.lifecycle_search(query, limit, now, true)
    }

    fn run_maintenance(&mut self, now: i64, limit: usize) -> Result<MaintenanceReport, PortError> {
        if limit == 0 {
            return Ok(MaintenanceReport::default());
        }
        if self.maintenance_active_complete && self.maintenance_expired_complete {
            self.maintenance_active_complete = false;
            self.maintenance_expired_complete = false;
            self.maintenance_active_after = None;
            self.maintenance_expired_after = None;
        }
        let sql_limit = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let transaction = self
            .database
            .connection_mut()
            .transaction()
            .map_err(|error| PortError(error.to_string()))?;
        let mut active_rows = if self.maintenance_active_complete {
            Vec::new()
        } else {
            load_active_maintenance_rows(&transaction, self.maintenance_active_after, sql_limit)?
        };
        let active_remaining = active_rows.len() > limit;
        active_rows.truncate(limit);
        let next_active_after = if active_remaining {
            active_rows
                .last()
                .map(|(id, last_seen_at)| (*last_seen_at, *id))
        } else {
            None
        };
        let next_active_complete = !active_remaining;
        let mut dormant = 0;
        for (id, _) in active_rows {
            let evidence = load_evidence(&transaction, id)?;
            if should_become_dormant(&evidence, now) {
                dormant += transaction
                    .execute(
                        "UPDATE memories SET revision=revision+1,state='dormant',state_changed_at=?1,updated_at=MAX(updated_at,?1) WHERE id=?2 AND state='active' AND pinned=0",
                        params![now, id],
                    )
                    .map_err(|error| PortError(error.to_string()))?;
            }
        }
        let cutoff = now.saturating_sub(DORMANT_DELETE_AFTER_SECONDS);
        let mut expired_rows = if self.maintenance_expired_complete {
            Vec::new()
        } else {
            load_expired_maintenance_rows(
                &transaction,
                cutoff,
                self.maintenance_expired_after,
                sql_limit,
            )?
        };
        let expired_remaining = expired_rows.len() > limit;
        expired_rows.truncate(limit);
        let next_expired_after = if expired_remaining {
            expired_rows
                .last()
                .map(|(id, state_changed_at)| (*state_changed_at, *id))
        } else {
            None
        };
        let next_expired_complete = !expired_remaining;
        let mut deleted = 0;
        for (id, _) in expired_rows {
            deleted += transaction
                .execute("DELETE FROM memories WHERE id=?1", [id])
                .map_err(|error| PortError(error.to_string()))?;
        }
        transaction
            .commit()
            .map_err(|error| PortError(error.to_string()))?;
        self.maintenance_active_after = next_active_after;
        self.maintenance_expired_after = next_expired_after;
        self.maintenance_active_complete = next_active_complete;
        self.maintenance_expired_complete = next_expired_complete;
        Ok(MaintenanceReport {
            dormant,
            deleted,
            remaining: !(self.maintenance_active_complete && self.maintenance_expired_complete),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SqliteMemoryStore, delete_all_memories_in_transaction, load_evidence,
        load_evidence_for_memories,
    };
    use crate::Database;
    use pw_application::memory::{
        Attribution, CandidateOperation, CandidateProvenanceRelation, ClassificationRun,
        DORMANT_DELETE_AFTER_SECONDS, DiscourseFeatures, EpistemicForm, EvidenceSource,
        Fictionality, MemoryAction, MemoryAtom, MemoryDomain, MemoryState, MemoryStore,
        MemoryWriteClass, NewObservation, ObservationStore, PersistedCandidate, Polarity,
        ProvenanceLink, ProvisionalMemoryChangeSet, SourceMode, SourceSpan, SpeechAct,
        SubjectScope, TemporalScope, VerificationStatus, VersionedMemoryAction, memory_strength,
        prompt_rank, should_become_dormant,
    };

    #[test]
    fn summary_and_memory_survive_reopen_and_search_by_relevance() {
        let path = std::env::temp_dir().join(format!("pw-memory-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let mut store = SqliteMemoryStore::new(Database::open(&path).unwrap());
            store.database.connection().execute_batch("INSERT INTO conversations(id,created_at,updated_at) VALUES('chat',1,1); INSERT INTO messages(conversation_id,turn_id,role,content,created_at) VALUES('chat',1,'user','x',1);").unwrap();
            store.upsert_summary("chat", "旅行の要約", 1, 10).unwrap();
            store.upsert_memory(None, "猫が好き", 10).unwrap();
            store.upsert_memory(None, "犬の散歩", 11).unwrap();
        }
        let store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        assert_eq!(
            store.load_summary("chat").unwrap().unwrap().content,
            "旅行の要約"
        );
        assert_eq!(store.search("猫が好き", 1).unwrap()[0].content, "猫が好き");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn observation_claims_are_exclusive_and_expired_leases_are_reclaimable() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let observation_id = store
            .insert_observation(NewObservation::new("chat", 7, "I like tea", 10))
            .unwrap();

        let first = store
            .claim_next_observation("worker-a", 10, 30)
            .unwrap()
            .unwrap();
        assert_eq!(first.observation_id, observation_id);
        assert!(
            store
                .claim_next_observation("worker-b", 11, 30)
                .unwrap()
                .is_none()
        );

        let second = store
            .claim_next_observation("worker-b", 40, 30)
            .unwrap()
            .unwrap();
        assert_eq!(second.observation_id, observation_id);
        assert_ne!(first.attempt_token, second.attempt_token);
    }

    #[test]
    fn observation_due_time_tracks_retry_backoff_and_expired_leases() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        store
            .insert_observation(NewObservation::new("chat", 7, "I like tea", 10))
            .unwrap();
        let lease = store
            .claim_next_observation("worker", 10, 30)
            .unwrap()
            .unwrap();
        assert_eq!(store.next_observation_due_at(10).unwrap(), Some(40));

        store
            .retry_or_defer_observation(&lease, "temporary failure", 10, 3, 5)
            .unwrap();
        assert_eq!(store.next_observation_due_at(10).unwrap(), Some(15));
        assert!(
            store
                .claim_next_observation("worker", 14, 30)
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .claim_next_observation("worker", 15, 30)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn promotion_is_atomic_and_request_key_retries_are_idempotent() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let observation = NewObservation::new("chat", 8, "I like tea", 10);
        store.insert_observation(observation.clone()).unwrap();
        let lease = store
            .claim_next_observation("worker", 10, 30)
            .unwrap()
            .unwrap();
        let run = ClassificationRun::new(
            lease.observation_id,
            "test",
            10,
            &lease.canonical_input_hash,
        );
        let run_id = pw_application::memory::ObservationStore::begin_classification_run(
            &mut store, &lease, &run, 11,
        )
        .unwrap();
        let candidate_id = pw_application::memory::ObservationStore::persist_candidate(
            &mut store,
            PersistedCandidate {
                classification_run_id: run_id,
                ordinal: 0,
                atom: MemoryAtom {
                    id: 0,
                    revision: 1,
                    content: "I like tea".into(),
                    subject_scope: SubjectScope::UserSelf,
                    epistemic_form: EpistemicForm::FactClaim,
                    attribution: Attribution::User,
                    discourse: DiscourseFeatures {
                        speech_act: SpeechAct::Asserted,
                        source_mode: SourceMode::Direct,
                        polarity: Polarity::Affirmed,
                        conditionality: pw_application::memory::Conditionality::Actual,
                        fictionality: Fictionality::RealWorld,
                    },
                    verification_status: VerificationStatus::UserReported,
                    temporal_scope: TemporalScope::Stable,
                    lifecycle_state: MemoryState::Active,
                    source_spans: vec![SourceSpan {
                        source_id: lease.canonical_input_hash.clone(),
                        start: 0,
                        end: "I like tea".len(),
                    }],
                },
                target_memory_id: None,
                expected_target_revision: None,
                operation: CandidateOperation::Add,
                relation: CandidateProvenanceRelation::Originated,
                domain: MemoryDomain::SemanticUser,
                write_class: MemoryWriteClass::NormalExplicit,
                normalization_edits: Vec::new(),
            },
            11,
        )
        .unwrap()
        .id();
        let change_set = ProvisionalMemoryChangeSet {
            request_key: run.request_key.clone(),
            lease,
            classification_run_id: run_id,
            classifier_version: run.classifier_version.clone(),
            schema_version: run.schema_version,
            input_hash: run.input_hash.clone(),
            actions: vec![VersionedMemoryAction {
                action: MemoryAction::Add {
                    content: "I like tea".into(),
                    pinned: false,
                },
                expected_revision: None,
            }],
            provenance: vec![ProvenanceLink {
                candidate_id,
                relation: "originated".into(),
            }],
        };
        let first = store.promote(&change_set, 12).unwrap();
        assert_eq!(first.promoted_memory_ids.len(), 1);
        let second = store.promote(&change_set, 13).unwrap();
        assert!(second.already_applied);
        assert_eq!(second.promoted_memory_ids, first.promoted_memory_ids);
        let count: i64 = store
            .database
            .connection()
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn temporary_switch_tombstones_and_removes_a_real_promoted_add() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let source_text = "I like green tea";
        let observation = NewObservation::new("temporary-promoted", 8, source_text, 10);
        store.insert_observation(observation.clone()).unwrap();
        let lease = store
            .claim_next_observation("worker", 10, 30)
            .unwrap()
            .unwrap();
        let run = ClassificationRun::new(
            lease.observation_id,
            "test",
            10,
            &lease.canonical_input_hash,
        );
        let run_id = pw_application::memory::ObservationStore::begin_classification_run(
            &mut store, &lease, &run, 11,
        )
        .unwrap();
        let candidate_id = pw_application::memory::ObservationStore::persist_candidate(
            &mut store,
            PersistedCandidate {
                classification_run_id: run_id,
                ordinal: 0,
                atom: MemoryAtom {
                    id: 0,
                    revision: 1,
                    content: source_text.into(),
                    subject_scope: SubjectScope::UserSelf,
                    epistemic_form: EpistemicForm::FactClaim,
                    attribution: Attribution::User,
                    discourse: DiscourseFeatures {
                        speech_act: SpeechAct::Asserted,
                        source_mode: SourceMode::Direct,
                        polarity: Polarity::Affirmed,
                        conditionality: pw_application::memory::Conditionality::Actual,
                        fictionality: Fictionality::RealWorld,
                    },
                    verification_status: VerificationStatus::UserReported,
                    temporal_scope: TemporalScope::Stable,
                    lifecycle_state: MemoryState::Active,
                    source_spans: vec![SourceSpan {
                        source_id: lease.canonical_input_hash.clone(),
                        start: 0,
                        end: source_text.len(),
                    }],
                },
                target_memory_id: None,
                expected_target_revision: None,
                operation: CandidateOperation::Add,
                relation: CandidateProvenanceRelation::Originated,
                domain: MemoryDomain::SemanticUser,
                write_class: MemoryWriteClass::NormalExplicit,
                normalization_edits: Vec::new(),
            },
            11,
        )
        .unwrap()
        .id();
        let change_set = ProvisionalMemoryChangeSet {
            request_key: run.request_key.clone(),
            lease,
            classification_run_id: run_id,
            classifier_version: run.classifier_version.clone(),
            schema_version: run.schema_version,
            input_hash: run.input_hash.clone(),
            actions: vec![VersionedMemoryAction {
                action: MemoryAction::Add {
                    content: source_text.into(),
                    pinned: false,
                },
                expected_revision: None,
            }],
            provenance: vec![ProvenanceLink {
                candidate_id,
                relation: "originated".into(),
            }],
        };
        let promoted = store.promote(&change_set, 12).unwrap();
        assert_eq!(promoted.promoted_memory_ids.len(), 1);
        let memory_id = promoted.promoted_memory_ids[0];

        // This is the post-promotion race the temporary trigger must close:
        // provenance cascades away, so the trigger itself must tombstone the
        // final-support memory before it deletes the source graph.
        store.database.connection().execute(
            "INSERT INTO temporary_conversations(conversation_id,temporary,revision,updated_at) VALUES('temporary-promoted',1,1,13)",
            [],
        ).unwrap();
        let remaining: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE id=?1",
                [memory_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
        let tombstone: (i64, i64, i64) = store.database.connection().query_row(
            "SELECT generation,final_support_removed,pinned FROM memory_tombstones WHERE memory_id=?1",
            [memory_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(tombstone, (1, 1, 0));
        let observations: i64 = store.database.connection().query_row(
            "SELECT COUNT(*) FROM memory_observations WHERE conversation_id='temporary-promoted'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(observations, 0);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn promotion_policy_is_enforced_for_all_write_classes_and_domain_consents() {
        let cases = [
            (MemoryWriteClass::NormalExplicit, "normal_explicit"),
            (MemoryWriteClass::Inferred, "inferred"),
            (MemoryWriteClass::Personal, "personal"),
            (MemoryWriteClass::Sensitive, "sensitive"),
            (MemoryWriteClass::Secret, "secret"),
            (MemoryWriteClass::NeverStore, "never_store"),
        ];
        for (index, (class, label)) in cases.into_iter().enumerate() {
            for consent in ["allowed", "pending_approval", "never_store"] {
                let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
                store
                    .database
                    .connection()
                    .execute(
                        "UPDATE memory_domain_controls SET consent=?1 WHERE domain='semantic_user'",
                        [consent],
                    )
                    .unwrap();
                let content = format!("safe policy case {index} {consent}");
                store
                    .insert_observation(NewObservation::new("chat", 1, &content, 10))
                    .unwrap();
                let lease = store
                    .claim_next_observation("worker", 10, 30)
                    .unwrap()
                    .unwrap();
                let run = ClassificationRun::new(
                    lease.observation_id,
                    "policy",
                    1,
                    &lease.canonical_input_hash,
                );
                let run_id = store.begin_classification_run(&lease, &run, 10).unwrap();
                let persisted = store
                    .persist_candidate(
                        PersistedCandidate {
                            classification_run_id: run_id,
                            ordinal: 0,
                            atom: MemoryAtom {
                                id: 0,
                                revision: 1,
                                content: content.clone(),
                                subject_scope: SubjectScope::UserSelf,
                                epistemic_form: EpistemicForm::FactClaim,
                                attribution: Attribution::User,
                                discourse: DiscourseFeatures {
                                    speech_act: SpeechAct::Asserted,
                                    source_mode: SourceMode::Direct,
                                    polarity: Polarity::Affirmed,
                                    conditionality: pw_application::memory::Conditionality::Actual,
                                    fictionality: Fictionality::RealWorld,
                                },
                                verification_status: VerificationStatus::UserReported,
                                temporal_scope: TemporalScope::Stable,
                                lifecycle_state: MemoryState::Active,
                                source_spans: vec![SourceSpan {
                                    source_id: lease.canonical_input_hash.clone(),
                                    start: 0,
                                    end: content.len(),
                                }],
                            },
                            target_memory_id: None,
                            expected_target_revision: None,
                            operation: CandidateOperation::Add,
                            relation: CandidateProvenanceRelation::Originated,
                            domain: MemoryDomain::SemanticUser,
                            write_class: class,
                            normalization_edits: Vec::new(),
                        },
                        10,
                    )
                    .unwrap();
                assert!(
                    matches!(
                        persisted,
                        pw_application::memory::PersistCandidateOutcome::Persisted(_)
                    ),
                    "{label}/{consent}: {persisted:?}"
                );
                let candidate_id = persisted.id();
                let result = store
                    .promote(
                        &ProvisionalMemoryChangeSet {
                            request_key: run.request_key,
                            lease: lease.clone(),
                            classification_run_id: run_id,
                            classifier_version: run.classifier_version,
                            schema_version: run.schema_version,
                            input_hash: run.input_hash,
                            actions: vec![VersionedMemoryAction {
                                action: MemoryAction::Add {
                                    content,
                                    pinned: false,
                                },
                                expected_revision: None,
                            }],
                            provenance: vec![ProvenanceLink {
                                candidate_id,
                                relation: "originated".into(),
                            }],
                        },
                        11,
                    )
                    .unwrap();
                let auto =
                    matches!(class, MemoryWriteClass::NormalExplicit) && consent == "allowed";
                assert_eq!(
                    result.promoted_memory_ids.len(),
                    usize::from(auto),
                    "{label}/{consent}"
                );
                let state: (String, String) = store
                    .database
                    .connection()
                    .query_row(
                        "SELECT candidate_state,policy_state FROM memory_candidates WHERE id=?1",
                        [candidate_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                if auto {
                    assert_eq!(
                        state,
                        ("promoted".into(), "auto_approved".into()),
                        "{label}/{consent}"
                    );
                } else if consent == "never_store"
                    || matches!(
                        class,
                        MemoryWriteClass::Secret | MemoryWriteClass::NeverStore
                    )
                {
                    assert_eq!(
                        state,
                        ("rejected".into(), "rejected".into()),
                        "{label}/{consent}"
                    );
                } else {
                    assert_eq!(
                        state,
                        ("pending".into(), "pending_approval".into()),
                        "{label}/{consent}"
                    );
                }
            }
        }
    }

    #[test]
    fn promotion_rejects_a_candidate_from_an_expired_classification_run() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        store
            .insert_observation(NewObservation::new("chat", 80, "I like tea", 10))
            .unwrap();
        let first = store
            .claim_next_observation("worker", 10, 1)
            .unwrap()
            .unwrap();
        let first_run = ClassificationRun::new(
            first.observation_id,
            "test",
            10,
            &first.canonical_input_hash,
        );
        let first_run_id = pw_application::memory::ObservationStore::begin_classification_run(
            &mut store, &first, &first_run, 10,
        )
        .unwrap();
        store.database.connection().execute(
            "INSERT INTO memory_candidates(observation_id,classification_run_id,candidate_ordinal,content,subject_scope,epistemic_form,attribution,speech_act,source_mode,polarity,conditionality,fictionality,verification_status,temporal_scope,proposed_operation,proposed_relation,source_start,source_end,created_at,updated_at) VALUES(?1,?2,0,'I like tea','user_self','fact_claim','user','asserted','direct','affirmed','actual','real_world','user_reported','stable','add','originated',0,10,10,10)",
            [first.observation_id, first_run_id],
        ).unwrap();
        let stale_candidate: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT id FROM memory_candidates WHERE classification_run_id=?1",
                [first_run_id],
                |row| row.get(0),
            )
            .unwrap();
        let second = store
            .claim_next_observation("worker", 12, 30)
            .unwrap()
            .unwrap();
        let second_run = ClassificationRun::new(
            second.observation_id,
            "test",
            10,
            &second.canonical_input_hash,
        );
        let second_run_id = pw_application::memory::ObservationStore::begin_classification_run(
            &mut store,
            &second,
            &second_run,
            12,
        )
        .unwrap();
        let result = store.promote(
            &ProvisionalMemoryChangeSet {
                request_key: second_run.request_key.clone(),
                lease: second,
                classification_run_id: second_run_id,
                classifier_version: second_run.classifier_version,
                schema_version: second_run.schema_version,
                input_hash: second_run.input_hash,
                actions: vec![VersionedMemoryAction {
                    action: MemoryAction::Add {
                        content: "I like tea".into(),
                        pinned: false,
                    },
                    expected_revision: None,
                }],
                provenance: vec![ProvenanceLink {
                    candidate_id: stale_candidate,
                    relation: "originated".into(),
                }],
            },
            13,
        );
        assert!(result.is_err());
        let count: i64 = store
            .database
            .connection()
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn stale_deletion_generation_rolls_back_promotion_before_memory_mutation() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let observation = NewObservation::new("chat", 9, "I like coffee", 10);
        store.insert_observation(observation).unwrap();
        let lease = store
            .claim_next_observation("worker", 10, 30)
            .unwrap()
            .unwrap();
        let run = ClassificationRun::new(
            lease.observation_id,
            "test",
            10,
            &lease.canonical_input_hash,
        );
        let run_id = pw_application::memory::ObservationStore::begin_classification_run(
            &mut store, &lease, &run, 11,
        )
        .unwrap();
        let candidate_id = pw_application::memory::ObservationStore::persist_candidate(
            &mut store,
            PersistedCandidate {
                classification_run_id: run_id,
                ordinal: 0,
                atom: MemoryAtom {
                    id: 0,
                    revision: 1,
                    content: "I like coffee".into(),
                    subject_scope: SubjectScope::UserSelf,
                    epistemic_form: EpistemicForm::FactClaim,
                    attribution: Attribution::User,
                    discourse: DiscourseFeatures {
                        speech_act: SpeechAct::Asserted,
                        source_mode: SourceMode::Direct,
                        polarity: Polarity::Affirmed,
                        conditionality: pw_application::memory::Conditionality::Actual,
                        fictionality: Fictionality::RealWorld,
                    },
                    verification_status: VerificationStatus::UserReported,
                    temporal_scope: TemporalScope::Stable,
                    lifecycle_state: MemoryState::Active,
                    source_spans: vec![SourceSpan {
                        source_id: lease.canonical_input_hash.clone(),
                        start: 0,
                        end: "I like coffee".len(),
                    }],
                },
                target_memory_id: None,
                expected_target_revision: None,
                operation: CandidateOperation::Add,
                relation: CandidateProvenanceRelation::Originated,
                domain: MemoryDomain::SemanticUser,
                write_class: MemoryWriteClass::NormalExplicit,
                normalization_edits: Vec::new(),
            },
            11,
        )
        .unwrap()
        .id();
        store.database.connection().execute("UPDATE memory_observations SET deletion_generation=deletion_generation+1 WHERE id=?1", [lease.observation_id]).unwrap();
        let result = store.promote(
            &ProvisionalMemoryChangeSet {
                request_key: run.request_key,
                lease,
                classification_run_id: run_id,
                classifier_version: run.classifier_version.clone(),
                schema_version: run.schema_version,
                input_hash: run.input_hash.clone(),
                actions: vec![VersionedMemoryAction {
                    action: MemoryAction::Add {
                        content: "I like coffee".into(),
                        pinned: false,
                    },
                    expected_revision: None,
                }],
                provenance: vec![ProvenanceLink {
                    candidate_id,
                    relation: "originated".into(),
                }],
            },
            12,
        );
        assert!(result.is_err());
        let count: i64 = store
            .database
            .connection()
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn delete_all_fences_a_leased_add_candidate_without_target() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let old = store.upsert_memory(None, "old durable fact", 1).unwrap();
        store
            .insert_observation(NewObservation::new("chat", 44, "safe new fact", 10))
            .unwrap();
        let lease = store
            .claim_next_observation("worker", 10, 30)
            .unwrap()
            .unwrap();
        let run =
            ClassificationRun::new(lease.observation_id, "test", 1, &lease.canonical_input_hash);
        let run_id = store.begin_classification_run(&lease, &run, 10).unwrap();
        let candidate_id = store
            .persist_candidate(
                PersistedCandidate {
                    classification_run_id: run_id,
                    ordinal: 0,
                    atom: MemoryAtom {
                        id: 0,
                        revision: 1,
                        content: "safe new fact".into(),
                        subject_scope: SubjectScope::UserSelf,
                        epistemic_form: EpistemicForm::FactClaim,
                        attribution: Attribution::User,
                        discourse: DiscourseFeatures {
                            speech_act: SpeechAct::Asserted,
                            source_mode: SourceMode::Direct,
                            polarity: Polarity::Affirmed,
                            conditionality: pw_application::memory::Conditionality::Actual,
                            fictionality: Fictionality::RealWorld,
                        },
                        verification_status: VerificationStatus::UserReported,
                        temporal_scope: TemporalScope::Stable,
                        lifecycle_state: MemoryState::Active,
                        source_spans: vec![SourceSpan {
                            source_id: lease.canonical_input_hash.clone(),
                            start: 0,
                            end: 13,
                        }],
                    },
                    target_memory_id: None,
                    expected_target_revision: None,
                    operation: CandidateOperation::Add,
                    relation: CandidateProvenanceRelation::Originated,
                    domain: MemoryDomain::SemanticUser,
                    write_class: MemoryWriteClass::NormalExplicit,
                    normalization_edits: Vec::new(),
                },
                10,
            )
            .unwrap()
            .id();
        let transaction = store.database.connection_mut().transaction().unwrap();
        assert_eq!(
            delete_all_memories_in_transaction(&transaction, 11).unwrap(),
            1
        );
        transaction.commit().unwrap();
        let result = store.promote(
            &ProvisionalMemoryChangeSet {
                request_key: run.request_key,
                lease,
                classification_run_id: run_id,
                classifier_version: run.classifier_version,
                schema_version: run.schema_version,
                input_hash: run.input_hash,
                actions: vec![VersionedMemoryAction {
                    action: MemoryAction::Add {
                        content: "safe new fact".into(),
                        pinned: false,
                    },
                    expected_revision: None,
                }],
                provenance: vec![ProvenanceLink {
                    candidate_id,
                    relation: "originated".into(),
                }],
            },
            12,
        );
        assert!(result.is_err());
        let remaining: i64 = store
            .database
            .connection()
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
        let tombstone: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM memory_tombstones WHERE memory_id=?1",
                [old],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tombstone, 1);
    }

    #[test]
    fn update_refreshes_the_fts_index() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let id = store.upsert_memory(None, "紅茶が好き", 1).unwrap();
        store.update_memory(id, "コーヒーが好き", 2).unwrap();
        assert!(store.search("紅茶", 10).unwrap().is_empty());
        assert_eq!(store.search("コーヒー", 10).unwrap().len(), 1);
        store.delete_memory(id).unwrap();
        assert!(store.search("コーヒー", 10).unwrap().is_empty());
    }

    #[test]
    fn typed_projection_round_trips_and_rejects_stale_cas() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let id = store.upsert_memory(None, "typed projection", 1).unwrap();
        let mut atom = store.load_memory_atom(id).unwrap().unwrap();
        assert_eq!(atom.revision, 1);
        assert_eq!(atom.subject_scope, SubjectScope::LegacyUnknown);
        atom.subject_scope = SubjectScope::UserSelf;
        atom.epistemic_form = EpistemicForm::Belief;
        atom.attribution = Attribution::User;
        atom.verification_status = VerificationStatus::UserReported;
        atom.content = "typed projection updated".into();
        let updated = store.update_memory_atom_cas(&atom, 1, 2).unwrap();
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.subject_scope, SubjectScope::UserSelf);
        assert_eq!(updated.epistemic_form, EpistemicForm::Belief);
        store
            .database
            .connection()
            .execute(
                "UPDATE memories SET pinned=1,state_changed_at=7 WHERE id=?1",
                [id],
            )
            .unwrap();
        let semantic = store.update_memory_atom_cas(&updated, 2, 3).unwrap();
        let lifecycle_columns: (i64, i64, Option<i64>) = store
            .database
            .connection()
            .query_row(
                "SELECT pinned,state_changed_at,superseded_by FROM memories WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(lifecycle_columns, (1, 7, None));
        let mut invalid_transition = semantic.clone();
        invalid_transition.lifecycle_state = MemoryState::Dormant;
        assert!(
            store
                .update_memory_atom_cas(&invalid_transition, semantic.revision, 4)
                .is_err()
        );
        for verification_status in [
            VerificationStatus::ExternallyCorroborated,
            VerificationStatus::ExternallyContradicted,
        ] {
            let mut external = semantic.clone();
            external.verification_status = verification_status;
            assert!(
                store
                    .update_memory_atom_cas(&external, external.revision, 4)
                    .is_err()
            );
        }
        assert!(store.update_memory_atom_cas(&atom, 1, 3).is_err());
        assert_eq!(
            store.search("updated", 10).unwrap()[0].content,
            "typed projection updated"
        );
    }

    #[test]
    fn versioned_reinforce_rejects_a_target_changed_after_validation() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let source = EvidenceSource::new("default", 1);
        let id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "stale target".into(),
                    pinned: false,
                },
                &source,
                1,
            )
            .unwrap()
            .unwrap();
        let observed_revision = store.load_memory_atom(id).unwrap().unwrap().revision;

        // Simulates another writer committing after validation but before the
        // lifecycle mutation starts.
        store
            .update_memory(id, "changed after validation", 2)
            .unwrap();
        let error = store
            .apply_action_versioned(
                &MemoryAction::Reinforce {
                    memory_id: id,
                    pin: false,
                },
                Some(observed_revision),
                &EvidenceSource::new("default", 2),
                3,
            )
            .unwrap_err();
        assert!(error.0.contains("stale memory target"));
        let mention_count: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT mention_count FROM memories WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mention_count, 1);
    }

    #[test]
    fn versioned_supersede_marks_old_target_and_creates_active_replacement() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let source = EvidenceSource::new("default", 1);
        let old_id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "I like cats".into(),
                    pinned: true,
                },
                &source,
                1,
            )
            .unwrap()
            .unwrap();
        let expected_revision = store.load_memory_atom(old_id).unwrap().unwrap().revision;

        let replacement_id = store
            .apply_action_versioned(
                &MemoryAction::Supersede {
                    old_memory_id: old_id,
                    content: "I like dogs".into(),
                    pin_replacement: false,
                },
                Some(expected_revision),
                &EvidenceSource::new("default", 2),
                2,
            )
            .unwrap()
            .unwrap();

        let old: (String, i64, Option<i64>) = store
            .database
            .connection()
            .query_row(
                "SELECT state,pinned,superseded_by FROM memories WHERE id=?1",
                [old_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(old, ("superseded".into(), 0, Some(replacement_id)));
        assert_eq!(
            store
                .load_memory_atom(replacement_id)
                .unwrap()
                .unwrap()
                .lifecycle_state,
            MemoryState::Active
        );
    }

    #[test]
    fn versioned_supersede_replaces_a_dormant_same_source_row() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let old_id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "I like cats".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 1),
                1,
            )
            .unwrap()
            .unwrap();
        let replacement_source = EvidenceSource::new("default", 2);
        let dormant_id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "I like dogs".into(),
                    pinned: false,
                },
                &replacement_source,
                2,
            )
            .unwrap()
            .unwrap();
        store
            .database
            .connection()
            .execute(
                "UPDATE memories SET state='dormant',state_changed_at=3 WHERE id=?1",
                [dormant_id],
            )
            .unwrap();
        let expected_revision = store.load_memory_atom(old_id).unwrap().unwrap().revision;

        let replacement_id = store
            .apply_action_versioned(
                &MemoryAction::Supersede {
                    old_memory_id: old_id,
                    content: "I like dogs".into(),
                    pin_replacement: true,
                },
                Some(expected_revision),
                &replacement_source,
                4,
            )
            .unwrap()
            .unwrap();

        assert_eq!(replacement_id, dormant_id);
        let replacement: (String, i64, Option<i64>) = store
            .database
            .connection()
            .query_row(
                "SELECT state,pinned,superseded_by FROM memories WHERE id=?1",
                [replacement_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(replacement, ("active".into(), 1, None));
        assert_eq!(
            store
                .load_memory_atom(replacement_id)
                .unwrap()
                .unwrap()
                .lifecycle_state,
            MemoryState::Active
        );
    }

    #[test]
    fn versioned_supersede_replaces_a_superseded_same_source_row() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let old_id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "I like cats".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 1),
                1,
            )
            .unwrap()
            .unwrap();
        let replacement_source = EvidenceSource::new("default", 2);
        let superseded_id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "I like dogs".into(),
                    pinned: false,
                },
                &replacement_source,
                2,
            )
            .unwrap()
            .unwrap();
        store
            .database
            .connection()
            .execute(
                "UPDATE memories SET state='superseded',state_changed_at=3 WHERE id=?1",
                [superseded_id],
            )
            .unwrap();
        let expected_revision = store.load_memory_atom(old_id).unwrap().unwrap().revision;

        let replacement_id = store
            .apply_action_versioned(
                &MemoryAction::Supersede {
                    old_memory_id: old_id,
                    content: "I like dogs".into(),
                    pin_replacement: false,
                },
                Some(expected_revision),
                &replacement_source,
                4,
            )
            .unwrap()
            .unwrap();

        assert_eq!(replacement_id, superseded_id);
        assert_eq!(
            store
                .load_memory_atom(replacement_id)
                .unwrap()
                .unwrap()
                .lifecycle_state,
            MemoryState::Active
        );
    }

    #[test]
    fn versioned_supersede_never_uses_its_old_target_as_the_replacement() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let source = EvidenceSource::new("default", 1);
        let old_id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "same content".into(),
                    pinned: true,
                },
                &source,
                1,
            )
            .unwrap()
            .unwrap();
        let expected_revision = store.load_memory_atom(old_id).unwrap().unwrap().revision;

        let error = store
            .apply_action_versioned(
                &MemoryAction::Supersede {
                    old_memory_id: old_id,
                    content: "same content".into(),
                    pin_replacement: false,
                },
                Some(expected_revision),
                &EvidenceSource::new("default", 2),
                2,
            )
            .unwrap_err();

        let old: (String, Option<i64>) = store
            .database
            .connection()
            .query_row(
                "SELECT state,superseded_by FROM memories WHERE id=?1",
                [old_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(error.0.contains("itself as the replacement"));
        assert_eq!(old, ("active".into(), None));
    }

    #[test]
    fn duplicate_source_targeted_pin_still_applies_the_pin_transition() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let source = EvidenceSource::new("default", 1);
        let id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "pin this".into(),
                    pinned: false,
                },
                &source,
                1,
            )
            .unwrap()
            .unwrap();
        let expected_revision = store.load_memory_atom(id).unwrap().unwrap().revision;

        store
            .apply_action_versioned(
                &MemoryAction::Reinforce {
                    memory_id: id,
                    pin: true,
                },
                Some(expected_revision),
                &source,
                2,
            )
            .unwrap();

        let target: (i64, String, i64, i64) = store
            .database
            .connection()
            .query_row(
                "SELECT pinned,state,mention_count,revision FROM memories WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(target, (1, "active".into(), 1, expected_revision + 1));
    }

    #[test]
    fn duplicate_source_reinforce_revives_a_dormant_memory() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let source = EvidenceSource::new("default", 1);
        let id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "revive this".into(),
                    pinned: false,
                },
                &source,
                1,
            )
            .unwrap()
            .unwrap();
        store
            .database
            .connection()
            .execute(
                "UPDATE memories SET state='dormant',state_changed_at=2 WHERE id=?1",
                [id],
            )
            .unwrap();
        let expected_revision = store.load_memory_atom(id).unwrap().unwrap().revision;

        store
            .apply_action_versioned(
                &MemoryAction::Reinforce {
                    memory_id: id,
                    pin: false,
                },
                Some(expected_revision),
                &source,
                3,
            )
            .unwrap();

        let target: (String, Option<i64>, i64, i64) = store
            .database
            .connection()
            .query_row(
                "SELECT state,state_changed_at,mention_count,revision FROM memories WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(target, ("active".into(), None, 1, expected_revision + 1));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn prompt_recall_evidence_never_changes_strength_or_dormancy() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "recall does not retain this memory".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 1),
                0,
            )
            .unwrap()
            .unwrap();
        let now = 31 * 86_400;
        let before = load_evidence(store.database.connection(), id).unwrap();
        for turn_id in 2..102 {
            store
                .record_recalled(&[id], &EvidenceSource::new("default", turn_id), now)
                .unwrap();
        }
        let after = load_evidence(store.database.connection(), id).unwrap();
        assert_eq!(memory_strength(&before, now), memory_strength(&after, now));
        assert_eq!(
            should_become_dormant(&before, now),
            should_become_dormant(&after, now)
        );
    }

    #[test]
    fn japanese_short_queries_use_escaped_like_fallback() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        store.upsert_memory(None, "猫が好き", 1).unwrap();
        store.upsert_memory(None, "100%確実_です", 2).unwrap();
        assert_eq!(store.search("猫", 10).unwrap()[0].content, "猫が好き");
        assert_eq!(store.search("%", 10).unwrap()[0].content, "100%確実_です");
        assert_eq!(store.search("_", 10).unwrap()[0].content, "100%確実_です");
    }

    #[test]
    fn fact_upsert_does_not_duplicate_identical_source_content() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let first = store.upsert_memory(None, "猫が好き", 1).unwrap();
        let second = store.upsert_memory(None, "猫が好き", 2).unwrap();
        assert_eq!(first, second);
        assert_eq!(store.search("猫", 10).unwrap().len(), 1);
    }

    #[test]
    fn secret_shaped_content_is_rejected_before_persistence() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        assert!(
            store
                .upsert_memory(None, "Authorization: Bearer abc", 1)
                .is_err()
        );
        assert!(store.search("Authorization", 10).unwrap().is_empty());
        let id = store.upsert_memory(None, "私は猫が好き", 1).unwrap();
        assert!(store.update_memory(id, "APIキー=x", 2).is_err());
        assert!(
            store
                .upsert_summary("missing", "パスワード=x", 1, 1)
                .is_err()
        );
    }

    #[test]
    fn temporary_conversation_never_enters_the_durable_observation_ledger() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        store
            .database
            .connection()
            .execute(
                "INSERT INTO temporary_conversations(conversation_id,temporary,revision,updated_at) VALUES('temporary',1,1,1)",
                [],
            )
            .unwrap();
        assert!(
            store
                .insert_observation(NewObservation::new("temporary", 1, "do not persist", 1))
                .is_err()
        );
        let count: i64 = store
            .database
            .connection()
            .query_row("SELECT COUNT(*) FROM memory_observations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn temporary_switch_fences_two_connection_observation_race_and_all_derived_state() {
        let path =
            std::env::temp_dir().join(format!("pw-temporary-fence-{}.sqlite3", std::process::id()));
        let mut first = SqliteMemoryStore::new(Database::open(&path).unwrap());
        first
            .insert_observation(NewObservation::new("race", 1, "safe queued input", 10))
            .unwrap();
        let observation_id: i64 = first
            .database
            .connection()
            .query_row(
                "SELECT id FROM memory_observations WHERE conversation_id='race'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        first.database.connection().execute(
            "INSERT INTO memory_classification_runs(observation_id,classifier_version,schema_version,input_hash,lease_attempt_token,transport_outcome,created_at) VALUES(?1,'test',1,'hash','token','pending',10)", [observation_id],
        ).unwrap();
        let run_id: i64 = first
            .database
            .connection()
            .query_row("SELECT id FROM memory_classification_runs", [], |row| {
                row.get(0)
            })
            .unwrap();
        first.database.connection().execute(
            "INSERT INTO memory_candidates(observation_id,classification_run_id,candidate_ordinal,content,subject_scope,epistemic_form,attribution,speech_act,source_mode,polarity,conditionality,fictionality,verification_status,temporal_scope,proposed_operation,proposed_relation,source_start,source_end,created_at,updated_at) VALUES(?1,?2,0,'safe queued input','user_self','fact_claim','user','asserted','direct','affirmed','actual','real_world','user_reported','stable','add','originated',0,17,10,10)", [observation_id, run_id],
        ).unwrap();
        first.database.connection().execute(
            "INSERT INTO memory_promotions(request_key,observation_id,classifier_version,schema_version,input_hash,classification_run_id,change_set_fingerprint,status,result_memory_ids,created_at,committed_at) VALUES('race',?1,'test',1,'hash',?2,'f','committed','[]',10,10)", [observation_id, run_id],
        ).unwrap();
        let second = Database::open(&path).unwrap();
        second.connection().execute(
            "INSERT INTO commitments(conversation_id,content,status,created_at,updated_at) VALUES('race','safe commitment','open',10,10)", [],
        ).unwrap();
        second.connection().execute(
            "INSERT INTO dialogue_states(conversation_id,expires_at,updated_at) VALUES('race',20,10)", [],
        ).unwrap();
        // Connection two flips the privacy fence immediately before connection
        // one attempts its next durable insert. The trigger deletes the queued
        // graph atomically; conditional INSERT rejects the late writer.
        second.connection().execute(
            "INSERT INTO temporary_conversations(conversation_id,temporary,revision,updated_at) VALUES('race',1,1,11)", [],
        ).unwrap();
        assert!(
            first
                .insert_observation(NewObservation::new("race", 2, "late input", 12))
                .is_err()
        );
        for table in [
            "memory_observations",
            "memory_candidates",
            "memory_promotions",
            "dialogue_states",
            "commitments",
        ] {
            let count: i64 = first
                .database
                .connection()
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table} WHERE {}",
                        if table == "memory_observations"
                            || table == "dialogue_states"
                            || table == "commitments"
                        {
                            "conversation_id='race'"
                        } else {
                            "1=1"
                        }
                    ),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "temporary fence left {table}");
        }
        drop(first);
        drop(second);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reinforce_revives_dormant_memory_once_per_turn() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let first_source = EvidenceSource::new("default", 7);
        let id = store
            .apply_action(
                &MemoryAction::Add {
                    content: "猫が好き".into(),
                    pinned: false,
                },
                &first_source,
                1,
            )
            .unwrap()
            .unwrap();
        store
            .database
            .connection()
            .execute(
                "UPDATE memories SET state='dormant',state_changed_at=2 WHERE id=?1",
                [id],
            )
            .unwrap();
        let second_source = EvidenceSource::new("default", 8);
        store
            .apply_action(
                &MemoryAction::Reinforce {
                    memory_id: id,
                    pin: false,
                },
                &second_source,
                3,
            )
            .unwrap();
        store
            .apply_action(
                &MemoryAction::Reinforce {
                    memory_id: id,
                    pin: false,
                },
                &second_source,
                99,
            )
            .unwrap();
        let candidate = store
            .find_consolidation_candidates("猫", 10, 3)
            .unwrap()
            .remove(0);
        assert_eq!(candidate.state, MemoryState::Active);
        assert_eq!(candidate.mention_count, 2);
        assert_eq!(candidate.last_seen_at, 3);
        let count: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM memory_evidence WHERE memory_id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn superseded_and_expired_rows_never_reach_prompt_search() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let source = EvidenceSource::new("default", 9);
        let old = store
            .apply_action(
                &MemoryAction::Add {
                    content: "猫が好き".into(),
                    pinned: true,
                },
                &source,
                1,
            )
            .unwrap()
            .unwrap();
        let new = store
            .apply_action(
                &MemoryAction::Supersede {
                    old_memory_id: old,
                    content: "犬が好き".into(),
                    pin_replacement: false,
                },
                &source,
                2,
            )
            .unwrap()
            .unwrap();
        store
            .apply_action(
                &MemoryAction::Supersede {
                    old_memory_id: old,
                    content: "犬が好き".into(),
                    pin_replacement: false,
                },
                &source,
                99,
            )
            .unwrap();
        let changed_at: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT state_changed_at FROM memories WHERE id=?1",
                [old],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(changed_at, 2);
        assert!(
            store
                .search_active_for_prompt("猫", 10, 2)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.search_active_for_prompt("犬", 10, 2).unwrap()[0].id,
            new
        );
        store
            .run_maintenance(2 + DORMANT_DELETE_AFTER_SECONDS, 100)
            .unwrap();
        assert!(
            store
                .find_consolidation_candidates("猫", 10, i64::MAX)
                .unwrap()
                .is_empty()
        );
        let fts_count: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH ?1",
                ["\"猫が好き\""],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 0);
    }

    #[test]
    fn supersede_timestamps_never_move_backwards() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let old = store
            .apply_action(
                &MemoryAction::Add {
                    content: "prefers green tea".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 20),
                100,
            )
            .unwrap()
            .unwrap();
        store
            .database
            .connection()
            .execute("UPDATE memories SET state_changed_at=80 WHERE id=?1", [old])
            .unwrap();
        store
            .apply_action(
                &MemoryAction::Supersede {
                    old_memory_id: old,
                    content: "prefers black tea".into(),
                    pin_replacement: false,
                },
                &EvidenceSource::new("default", 21),
                50,
            )
            .unwrap();
        let timestamps = store
            .database
            .connection()
            .query_row(
                "SELECT state_changed_at,updated_at FROM memories WHERE id=?1",
                [old],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap();
        assert_eq!(timestamps, (100, 100));
    }

    #[test]
    fn fts_bm25_normalization_and_prompt_rank_are_applied() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let lexical_best = store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee coffee coffee coffee".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 30),
                0,
            )
            .unwrap()
            .unwrap();
        let stronger_but_lexically_worse = store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee preference alongside hiking music books travel cooking".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 31),
                0,
            )
            .unwrap()
            .unwrap();
        for turn_id in 32..36 {
            store
                .apply_action(
                    &MemoryAction::Reinforce {
                        memory_id: stronger_but_lexically_worse,
                        pin: false,
                    },
                    &EvidenceSource::new("default", turn_id),
                    0,
                )
                .unwrap();
        }
        let candidates = store
            .find_consolidation_candidates("coffee", 10, 120 * 86_400)
            .unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id, lexical_best);
        assert!((candidates[0].lexical_relevance - 1.0).abs() < f64::EPSILON);
        assert!((candidates[1].lexical_relevance - 0.0).abs() < f64::EPSILON);
        assert!(candidates[0].strength < candidates[1].strength);
        let best_rank = prompt_rank(candidates[0].lexical_relevance, candidates[0].strength);
        let worst_rank = prompt_rank(candidates[1].lexical_relevance, candidates[1].strength);
        assert!((best_rank - 0.85).abs() < f64::EPSILON);
        assert!((worst_rank - 0.30).abs() < f64::EPSILON);

        let mut tied_store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let weaker = tied_store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee cats".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 40),
                0,
            )
            .unwrap()
            .unwrap();
        let stronger = tied_store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee dogs".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 41),
                0,
            )
            .unwrap()
            .unwrap();
        tied_store
            .apply_action(
                &MemoryAction::Reinforce {
                    memory_id: stronger,
                    pin: false,
                },
                &EvidenceSource::new("default", 42),
                0,
            )
            .unwrap();
        let tied = tied_store
            .search_active_for_prompt("coffee", 10, 120 * 86_400)
            .unwrap();
        assert_eq!(
            tied.iter().map(|item| item.id).collect::<Vec<_>>(),
            [stronger, weaker]
        );
        assert!(
            tied.iter()
                .all(|item| (item.lexical_relevance - 1.0).abs() < f64::EPSILON)
        );
        assert!(tied[0].strength > tied[1].strength);
    }

    #[test]
    fn contradiction_statement_discovers_the_previous_memory_safely() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let old = store
            .apply_action(
                &MemoryAction::Add {
                    content: "私は猫が好き".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 1),
                1,
            )
            .unwrap()
            .unwrap();

        let candidates = store
            .find_consolidation_candidates("私は犬が好き", 5, 2)
            .unwrap();
        assert!(candidates.iter().any(|candidate| candidate.id == old));

        for query in ["", "\" OR *", "猫", "🦀🦀🦀"] {
            assert!(store.find_consolidation_candidates(query, 5, 2).is_ok());
        }
    }

    #[test]
    fn prompt_rerank_oversamples_beyond_the_final_limit() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        for turn in 1..=5 {
            store
                .apply_action(
                    &MemoryAction::Add {
                        content: format!(
                            "coffee preference coffee preference coffee preference lexical-{turn}"
                        ),
                        pinned: false,
                    },
                    &EvidenceSource::new("default", turn),
                    0,
                )
                .unwrap();
        }
        let strong = store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee preference alongside hiking".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 10),
                0,
            )
            .unwrap()
            .unwrap();
        for turn in 11..=30 {
            store
                .apply_action(
                    &MemoryAction::Reinforce {
                        memory_id: strong,
                        pin: false,
                    },
                    &EvidenceSource::new("default", turn),
                    0,
                )
                .unwrap();
        }
        store
            .apply_action(
                &MemoryAction::Add {
                    content:
                        "coffee only with a deliberately unrelated and very long tail of words"
                            .into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 31),
                0,
            )
            .unwrap();

        let final_five = store
            .search_active_for_prompt("coffee preference", 5, 120 * 86_400)
            .unwrap();
        assert_eq!(final_five.len(), 5);
        assert!(final_five.iter().any(|candidate| candidate.id == strong));
    }

    #[test]
    fn batch_evidence_load_groups_all_candidate_evidence() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let first = store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee cats".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 1),
                1,
            )
            .unwrap()
            .unwrap();
        let second = store
            .apply_action(
                &MemoryAction::Add {
                    content: "coffee dogs".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 2),
                2,
            )
            .unwrap()
            .unwrap();
        store
            .apply_action(
                &MemoryAction::Reinforce {
                    memory_id: second,
                    pin: false,
                },
                &EvidenceSource::new("default", 3),
                3,
            )
            .unwrap();

        let evidence =
            load_evidence_for_memories(store.database.connection(), &[second, first]).unwrap();

        assert_eq!(evidence[&first].len(), 1);
        assert_eq!(evidence[&second].len(), 2);
        assert!(evidence[&second][0].id < evidence[&second][1].id);
    }

    #[test]
    fn maintenance_cursor_reaches_a_weak_row_after_one_hundred_strong_rows() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        for turn in 1..=100 {
            let id = store
                .apply_action(
                    &MemoryAction::Add {
                        content: format!("strong memory {turn}"),
                        pinned: false,
                    },
                    &EvidenceSource::new("default", turn),
                    0,
                )
                .unwrap()
                .unwrap();
            store
                .apply_action(
                    &MemoryAction::Reinforce {
                        memory_id: id,
                        pin: false,
                    },
                    &EvidenceSource::new("default", turn + 100),
                    0,
                )
                .unwrap();
        }
        let weak = store
            .apply_action(
                &MemoryAction::Add {
                    content: "weak memory after the first page".into(),
                    pinned: false,
                },
                &EvidenceSource::new("default", 1_000),
                0,
            )
            .unwrap()
            .unwrap();

        assert_eq!(store.run_maintenance(31 * 86_400, 100).unwrap().dormant, 0);
        assert_eq!(store.run_maintenance(31 * 86_400, 100).unwrap().dormant, 1);
        let state: String = store
            .database
            .connection()
            .query_row("SELECT state FROM memories WHERE id=?1", [weak], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(state, "dormant");
    }

    #[test]
    fn pin_secret_filter_and_recall_idempotency_are_enforced() {
        let mut store = SqliteMemoryStore::new(Database::open_in_memory().unwrap());
        let source = EvidenceSource::new("default", 10);
        let pinned = store
            .apply_action(
                &MemoryAction::Add {
                    content: "猫が好き".into(),
                    pinned: true,
                },
                &source,
                0,
            )
            .unwrap()
            .unwrap();
        assert!(
            store
                .apply_action(
                    &MemoryAction::Add {
                        content: "Authorization: Bearer raw-secret".into(),
                        pinned: false,
                    },
                    &EvidenceSource::new("default", 11),
                    1,
                )
                .is_err()
        );
        store
            .record_recalled(&[pinned], &EvidenceSource::new("default", 12), 2)
            .unwrap();
        store
            .record_recalled(&[pinned], &EvidenceSource::new("default", 12), 2)
            .unwrap();
        store.run_maintenance(i64::MAX / 2, 100).unwrap();
        assert_eq!(
            store
                .search_active_for_prompt("猫", 10, i64::MAX / 2)
                .unwrap()[0]
                .id,
            pinned
        );
        let recalled: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM memory_evidence WHERE memory_id=?1 AND kind='recalled'",
                [pinned],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recalled, 1);
        let unsafe_count: i64 = store
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE content LIKE '%raw-secret%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(unsafe_count, 0);
    }
}
