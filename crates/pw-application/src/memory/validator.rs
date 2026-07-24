use super::consolidation::ProposedAction;
use super::{
    Attribution, Conditionality, MemoryAtom, MemoryCandidate, MemoryState, Polarity, SourceMode,
    SourceSpan, SpeechAct, VerificationStatus, is_safe_persistent_content,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateRelation {
    Same,
    Refines,
    Contradicts,
    ChangesStance,
    Unrelated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizationEdit {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedCandidate {
    pub atom: MemoryAtom,
    pub relation: CandidateRelation,
    pub target_memory_id: Option<i64>,
    pub expected_target_revision: Option<i64>,
    pub normalization_edits: Vec<NormalizationEdit>,
    pub proposed_action: ProposedAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    MissingSourceSpan,
    EmptySourceSpan,
    SourceSpanSourceMismatch {
        expected_source_id: String,
        actual_source_id: String,
    },
    SourceSpansNotStrictlyOrdered,
    DisjointSourceSpansForbidden,
    SourceSpanMustCoverSource,
    SourceSpanOutOfBounds,
    SourceSpanNotCharBoundary,
    NormalizationTraceDoesNotCoverChangedBytes,
    UnknownTargetId {
        target_memory_id: i64,
    },
    MissingExpectedTargetRevision {
        target_memory_id: i64,
    },
    MissingTargetRevisionProof {
        target_memory_id: i64,
        expected_revision: i64,
    },
    StaleTargetRevision {
        target_memory_id: i64,
        expected_revision: i64,
        actual_revision: i64,
    },
    UnexpectedTarget {
        target_memory_id: Option<i64>,
    },
    ActionTargetMismatch {
        action_target_memory_id: i64,
        candidate_target_memory_id: Option<i64>,
    },
    ActionContentMismatch,
    TargetLifecycleForbidden {
        target_memory_id: i64,
        state: MemoryState,
    },
    LifecycleTransitionForbidden {
        from: MemoryState,
        to: MemoryState,
    },
    AssistantAttributionForbidden,
    ExternalVerificationReserved,
    MarkerPreservationUnproven,
    ModalMarkerRemoved,
    ControlCharacterForbidden,
    UnsafePersistentContent,
}

/// # Errors
/// Returns an error under the same conditions as
/// [`validate_candidate_for_source`].
pub fn validate_candidate(
    candidate: &TypedCandidate,
    expected_source_id: &str,
    source: &str,
    targets: &[MemoryCandidate],
) -> Result<(), ValidationError> {
    validate_candidate_for_source(candidate, expected_source_id, source, targets)
}

/// Validates a typed candidate against one accepted source identity. Callers that
/// construct observations must use this entry point so source spans cannot cross
/// turn boundaries.
///
/// # Errors
/// Returns a [`ValidationError`] describing the first bounds, normalization,
/// marker-preservation, or target-lifecycle violation found.
pub fn validate_candidate_for_source(
    candidate: &TypedCandidate,
    expected_source_id: &str,
    source: &str,
    targets: &[MemoryCandidate],
) -> Result<(), ValidationError> {
    validate_spans(&candidate.atom.source_spans, expected_source_id, source)?;
    validate_normalization(candidate, source)?;
    validate_action_content(candidate)?;
    validate_markers(
        candidate,
        &source_text(&candidate.atom.source_spans, source),
    )?;
    validate_target(candidate, targets)?;
    if candidate
        .atom
        .content
        .chars()
        .any(|value| value.is_control() && !matches!(value, '\n' | '\r' | '\t'))
    {
        return Err(ValidationError::ControlCharacterForbidden);
    }
    if !is_safe_persistent_content(&candidate.atom.content) {
        return Err(ValidationError::UnsafePersistentContent);
    }
    Ok(())
}

fn validate_spans(
    spans: &[SourceSpan],
    expected_source_id: &str,
    source: &str,
) -> Result<(), ValidationError> {
    if spans.is_empty() {
        return Err(ValidationError::MissingSourceSpan);
    }
    // A memory atom is an auditable projection of one source clause.  Joining
    // disjoint slices would make the omitted text (including negation or
    // modality) invisible to the marker checks below.
    if spans.len() != 1 {
        return Err(ValidationError::DisjointSourceSpansForbidden);
    }
    let mut previous_end = None;
    for span in spans {
        if span.source_id != expected_source_id {
            return Err(ValidationError::SourceSpanSourceMismatch {
                expected_source_id: expected_source_id.into(),
                actual_source_id: span.source_id.clone(),
            });
        }
        if span.start >= span.end {
            return Err(ValidationError::EmptySourceSpan);
        }
        if span.end > source.len() {
            return Err(ValidationError::SourceSpanOutOfBounds);
        }
        if !source.is_char_boundary(span.start) || !source.is_char_boundary(span.end) {
            return Err(ValidationError::SourceSpanNotCharBoundary);
        }
        if previous_end.is_some_and(|end| span.start <= end) {
            return Err(ValidationError::SourceSpansNotStrictlyOrdered);
        }
        previous_end = Some(span.end);
    }
    let span = &spans[0];
    if span.start != 0 || span.end != source.len() {
        return Err(ValidationError::SourceSpanMustCoverSource);
    }
    Ok(())
}

fn validate_action_content(candidate: &TypedCandidate) -> Result<(), ValidationError> {
    let action_content = match &candidate.proposed_action {
        ProposedAction::Add { content } | ProposedAction::Supersede { content, .. } => {
            Some(content)
        }
        ProposedAction::Pin {
            content: Some(content),
            ..
        } => Some(content),
        ProposedAction::Reinforce { .. } | ProposedAction::Pin { .. } | ProposedAction::Ignore => {
            None
        }
    };
    if action_content.is_some_and(|content| content != &candidate.atom.content) {
        return Err(ValidationError::ActionContentMismatch);
    }
    Ok(())
}

fn source_text(spans: &[SourceSpan], source: &str) -> String {
    spans
        .iter()
        .map(|span| &source[span.start..span.end])
        .collect()
}

fn validate_normalization(candidate: &TypedCandidate, source: &str) -> Result<(), ValidationError> {
    let mut edits = candidate.normalization_edits.iter().collect::<Vec<_>>();
    edits.sort_by_key(|edit| (edit.start, edit.end));
    let mut edit_index = 0;
    let mut output = String::new();
    for span in &candidate.atom.source_spans {
        let mut cursor = span.start;
        if edit_index < edits.len() && edits[edit_index].end <= span.start {
            return Err(ValidationError::NormalizationTraceDoesNotCoverChangedBytes);
        }
        while edit_index < edits.len() && edits[edit_index].start < span.end {
            let edit = edits[edit_index];
            if edit.start < cursor || edit.end > span.end || edit.start >= edit.end {
                return Err(ValidationError::NormalizationTraceDoesNotCoverChangedBytes);
            }
            if !source.is_char_boundary(edit.start) || !source.is_char_boundary(edit.end) {
                return Err(ValidationError::SourceSpanNotCharBoundary);
            }
            if edit.replacement != source[edit.start..edit.end] {
                return Err(ValidationError::NormalizationTraceDoesNotCoverChangedBytes);
            }
            output.push_str(&source[cursor..edit.start]);
            output.push_str(&edit.replacement);
            cursor = edit.end;
            edit_index += 1;
        }
        output.push_str(&source[cursor..span.end]);
    }
    if edit_index != edits.len() || output != candidate.atom.content {
        return Err(ValidationError::NormalizationTraceDoesNotCoverChangedBytes);
    }
    Ok(())
}

fn validate_markers(candidate: &TypedCandidate, source: &str) -> Result<(), ValidationError> {
    if candidate.atom.attribution == Attribution::Assistant {
        return Err(ValidationError::AssistantAttributionForbidden);
    }
    if matches!(
        candidate.atom.verification_status,
        VerificationStatus::ExternallyCorroborated | VerificationStatus::ExternallyContradicted
    ) {
        return Err(ValidationError::ExternalVerificationReserved);
    }
    if contains_modal_marker(source) && !contains_modal_marker(&candidate.atom.content) {
        return Err(ValidationError::ModalMarkerRemoved);
    }
    let weakened = (contains_quote_or_reporting(source)
        && candidate.atom.discourse.source_mode == SourceMode::Direct)
        || (contains_question_marker(source)
            && candidate.atom.discourse.speech_act != SpeechAct::Questioned)
        || (contains_negation_marker(source)
            && candidate.atom.discourse.polarity != Polarity::Negated)
        || (contains_hypothetical_marker(source)
            && candidate.atom.discourse.conditionality != Conditionality::Hypothetical);
    if weakened {
        return Err(ValidationError::MarkerPreservationUnproven);
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn validate_target(
    candidate: &TypedCandidate,
    targets: &[MemoryCandidate],
) -> Result<(), ValidationError> {
    let creates_replacement = matches!(
        candidate.proposed_action,
        ProposedAction::Add { .. }
            | ProposedAction::Pin {
                memory_id: None,
                ..
            }
            | ProposedAction::Supersede { .. }
    );
    if creates_replacement && candidate.atom.lifecycle_state != MemoryState::Active {
        return Err(ValidationError::LifecycleTransitionForbidden {
            from: MemoryState::Active,
            to: candidate.atom.lifecycle_state,
        });
    }
    let action_target = match candidate.proposed_action {
        ProposedAction::Supersede { old_memory_id, .. } => Some(old_memory_id),
        ProposedAction::Reinforce { memory_id }
        | ProposedAction::Pin {
            memory_id: Some(memory_id),
            ..
        } => Some(memory_id),
        ProposedAction::Add { .. }
        | ProposedAction::Ignore
        | ProposedAction::Pin {
            memory_id: None, ..
        } => None,
    };
    if candidate.relation == CandidateRelation::Unrelated {
        if action_target.is_some()
            || candidate.target_memory_id.is_some()
            || candidate.expected_target_revision.is_some()
        {
            return Err(ValidationError::UnexpectedTarget {
                target_memory_id: candidate.target_memory_id,
            });
        }
        return Ok(());
    }
    let Some(action_target_memory_id) = action_target else {
        if candidate.target_memory_id.is_some() || candidate.expected_target_revision.is_some() {
            return Err(ValidationError::UnexpectedTarget {
                target_memory_id: candidate.target_memory_id,
            });
        }
        return Ok(());
    };
    if candidate.target_memory_id != Some(action_target_memory_id) {
        return Err(ValidationError::ActionTargetMismatch {
            action_target_memory_id,
            candidate_target_memory_id: candidate.target_memory_id,
        });
    }
    let target_memory_id = action_target_memory_id;
    let target = targets
        .iter()
        .find(|target| target.id == target_memory_id)
        .ok_or(ValidationError::UnknownTargetId { target_memory_id })?;
    let expected_revision = candidate
        .expected_target_revision
        .ok_or(ValidationError::MissingExpectedTargetRevision { target_memory_id })?;
    let actual_revision = target
        .revision
        .ok_or(ValidationError::MissingTargetRevisionProof {
            target_memory_id,
            expected_revision,
        })?;
    if actual_revision != expected_revision {
        return Err(ValidationError::StaleTargetRevision {
            target_memory_id,
            expected_revision,
            actual_revision,
        });
    }
    match candidate.proposed_action {
        ProposedAction::Reinforce { .. } | ProposedAction::Pin { .. } => {
            if target.state == MemoryState::Superseded {
                return Err(ValidationError::TargetLifecycleForbidden {
                    target_memory_id,
                    state: target.state,
                });
            }
            if candidate.atom.lifecycle_state != MemoryState::Active {
                return Err(ValidationError::LifecycleTransitionForbidden {
                    from: target.state,
                    to: candidate.atom.lifecycle_state,
                });
            }
        }
        ProposedAction::Supersede { .. } => {
            if target.state == MemoryState::Superseded {
                return Err(ValidationError::TargetLifecycleForbidden {
                    target_memory_id,
                    state: target.state,
                });
            }
            // The candidate is the newly-created replacement.  The target is
            // superseded by the storage action, while the replacement must be
            // born active.
            if candidate.atom.lifecycle_state != MemoryState::Active {
                return Err(ValidationError::LifecycleTransitionForbidden {
                    from: target.state,
                    to: candidate.atom.lifecycle_state,
                });
            }
        }
        ProposedAction::Add { .. } | ProposedAction::Ignore => {
            unreachable!("target-less actions returned early")
        }
    }
    Ok(())
}

fn contains_quote_or_reporting(source: &str) -> bool {
    source.contains(['「', '」', '『', '』', '"', '“', '”'])
        || [
            "と言った",
            "って言った",
            "と聞いた",
            "によると",
            "だそう",
            "らしい",
        ]
        .iter()
        .any(|marker| source.contains(marker))
}
fn contains_question_marker(source: &str) -> bool {
    source.contains(['?', '？', 'か'])
}
fn contains_negation_marker(source: &str) -> bool {
    [
        "じゃない",
        "ではない",
        "ない",
        "ません",
        "ぬ",
        "ず",
        "なかった",
        "ませんでした",
    ]
    .iter()
    .any(|marker| source.contains(marker))
}
fn contains_hypothetical_marker(source: &str) -> bool {
    ["もし", "なら", "たら", "れば", "かもしれない", "ようなら"]
        .iter()
        .any(|marker| source.contains(marker))
}
fn contains_modal_marker(source: &str) -> bool {
    [
        "と思う",
        "と思います",
        "気がする",
        "かもしれない",
        "ようだ",
        "みたい",
        "らしい",
    ]
    .iter()
    .any(|marker| source.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{
        DiscourseFeatures, EpistemicForm, Fictionality, MemoryState, SubjectScope, TemporalScope,
    };

    fn candidate(source: &str) -> TypedCandidate {
        TypedCandidate {
            atom: MemoryAtom {
                id: 0,
                revision: 1,
                content: source.into(),
                subject_scope: SubjectScope::UserSelf,
                epistemic_form: EpistemicForm::Belief,
                attribution: Attribution::User,
                discourse: DiscourseFeatures {
                    fictionality: Fictionality::RealWorld,
                    ..DiscourseFeatures::default()
                },
                verification_status: VerificationStatus::UserReported,
                temporal_scope: TemporalScope::Current,
                lifecycle_state: MemoryState::Active,
                source_spans: vec![SourceSpan {
                    source_id: "turn:1".into(),
                    start: 0,
                    end: source.len(),
                }],
            },
            relation: CandidateRelation::Unrelated,
            target_memory_id: None,
            expected_target_revision: None,
            normalization_edits: vec![],
            proposed_action: ProposedAction::Add {
                content: source.into(),
            },
        }
    }

    #[test]
    fn rejects_stale_target_revision() {
        let mut value = candidate("I like cats");
        value.relation = CandidateRelation::Same;
        value.target_memory_id = Some(3);
        value.expected_target_revision = Some(1);
        value.proposed_action = ProposedAction::Reinforce { memory_id: 3 };
        let target = MemoryCandidate {
            id: 3,
            revision: Some(2),
            content: "I like cats".into(),
            state: MemoryState::Active,
            pinned: false,
            mention_count: 1,
            last_seen_at: 0,
            lexical_relevance: 1.0,
            strength: 1.0,
        };
        assert!(matches!(
            validate_candidate(&value, "turn:1", "I like cats", &[target]),
            Err(ValidationError::StaleTargetRevision { .. })
        ));
    }

    #[test]
    fn preserves_negation_and_question_markers() {
        let mut value = candidate("猫は好きではない？");
        value.atom.discourse.polarity = Polarity::Affirmed;
        value.atom.discourse.speech_act = SpeechAct::Asserted;
        assert_eq!(
            validate_candidate(&value, "turn:1", "猫は好きではない？", &[]),
            Err(ValidationError::MarkerPreservationUnproven)
        );
    }

    #[test]
    fn rejects_a_source_span_from_another_turn() {
        let mut value = candidate("I like cats");
        value.atom.source_spans[0].source_id = "turn:other".into();

        assert!(matches!(
            validate_candidate_for_source(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::SourceSpanSourceMismatch { .. })
        ));
    }

    #[test]
    fn rejects_a_whole_span_replacement_that_invents_a_fact() {
        let mut value = candidate("I like cats");
        value.atom.content = "I own a yacht".into();
        value.normalization_edits = vec![NormalizationEdit {
            start: 0,
            end: "I like cats".len(),
            replacement: "I own a yacht".into(),
        }];

        assert_eq!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::NormalizationTraceDoesNotCoverChangedBytes)
        );
    }

    #[test]
    fn rejects_zero_width_normalization_insertions() {
        let mut value = candidate("I like cats");
        value.atom.content = "Actually, I like cats".into();
        value.normalization_edits = vec![NormalizationEdit {
            start: 0,
            end: 0,
            replacement: "Actually, ".into(),
        }];

        assert_eq!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::NormalizationTraceDoesNotCoverChangedBytes)
        );
    }

    #[test]
    fn rejects_add_content_that_differs_from_the_validated_atom() {
        let mut value = candidate("I like cats");
        value.proposed_action = ProposedAction::Add {
            content: "I own a yacht".into(),
        };

        assert_eq!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::ActionContentMismatch)
        );
    }

    #[test]
    fn rejects_supersede_content_that_differs_from_the_validated_atom() {
        let target = MemoryCandidate {
            id: 3,
            revision: Some(1),
            content: "I like cats".into(),
            state: MemoryState::Active,
            pinned: false,
            mention_count: 1,
            last_seen_at: 0,
            lexical_relevance: 1.0,
            strength: 1.0,
        };
        let mut value = candidate("I like dogs");
        value.relation = CandidateRelation::Contradicts;
        value.target_memory_id = Some(3);
        value.expected_target_revision = Some(1);
        value.proposed_action = ProposedAction::Supersede {
            old_memory_id: 3,
            content: "I own a yacht".into(),
        };

        assert_eq!(
            validate_candidate(&value, "turn:1", "I like dogs", &[target]),
            Err(ValidationError::ActionContentMismatch)
        );
    }

    #[test]
    fn rejects_targetless_pin_content_that_differs_from_the_validated_atom() {
        let mut value = candidate("I like cats");
        value.proposed_action = ProposedAction::Pin {
            memory_id: None,
            content: Some("I own a yacht".into()),
        };

        assert_eq!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::ActionContentMismatch)
        );
    }

    #[test]
    fn rejects_targeted_pin_content_that_differs_from_the_validated_atom() {
        let target = MemoryCandidate {
            id: 3,
            revision: Some(1),
            content: "I like cats".into(),
            state: MemoryState::Active,
            pinned: false,
            mention_count: 1,
            last_seen_at: 0,
            lexical_relevance: 1.0,
            strength: 1.0,
        };
        let mut value = candidate("I like cats");
        value.relation = CandidateRelation::Same;
        value.target_memory_id = Some(3);
        value.expected_target_revision = Some(1);
        value.proposed_action = ProposedAction::Pin {
            memory_id: Some(3),
            content: Some("I own a yacht".into()),
        };

        assert_eq!(
            validate_candidate(&value, "turn:1", "I like cats", &[target]),
            Err(ValidationError::ActionContentMismatch)
        );
    }

    #[test]
    fn rejects_disjoint_spans_that_elide_negation_and_modality() {
        let source = "I might not like cats";
        let mut value = candidate("I like cats");
        value.atom.source_spans = vec![
            SourceSpan {
                source_id: "turn:1".into(),
                start: 0,
                end: 2,
            },
            SourceSpan {
                source_id: "turn:1".into(),
                start: "I might not ".len(),
                end: source.len(),
            },
        ];

        assert_eq!(
            validate_candidate(&value, "turn:1", source, &[]),
            Err(ValidationError::DisjointSourceSpansForbidden)
        );
    }

    #[test]
    fn rejects_a_partial_span_that_elides_modal_context() {
        let source = "I might like cats";
        let mut value = candidate("like cats");
        value.atom.source_spans[0] = SourceSpan {
            source_id: "turn:1".into(),
            start: "I might ".len(),
            end: source.len(),
        };
        value.proposed_action = ProposedAction::Add {
            content: "like cats".into(),
        };

        assert_eq!(
            validate_candidate(&value, "turn:1", source, &[]),
            Err(ValidationError::SourceSpanMustCoverSource)
        );
    }

    #[test]
    fn rejects_additions_with_a_non_active_lifecycle_state() {
        let mut value = candidate("I like cats");
        value.atom.lifecycle_state = MemoryState::Dormant;
        assert!(matches!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::LifecycleTransitionForbidden {
                from: MemoryState::Active,
                to: MemoryState::Dormant,
            })
        ));

        value.atom.lifecycle_state = MemoryState::Superseded;
        assert!(matches!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::LifecycleTransitionForbidden {
                from: MemoryState::Active,
                to: MemoryState::Superseded,
            })
        ));
    }

    #[test]
    fn accepts_supersede_with_an_active_replacement_atom() {
        let target = MemoryCandidate {
            id: 3,
            revision: Some(1),
            content: "I like cats".into(),
            state: MemoryState::Active,
            pinned: false,
            mention_count: 1,
            last_seen_at: 0,
            lexical_relevance: 1.0,
            strength: 1.0,
        };
        let mut value = candidate("I like dogs");
        value.relation = CandidateRelation::Contradicts;
        value.target_memory_id = Some(3);
        value.expected_target_revision = Some(1);
        value.proposed_action = ProposedAction::Supersede {
            old_memory_id: 3,
            content: "I like dogs".into(),
        };

        assert_eq!(
            validate_candidate(&value, "turn:1", "I like dogs", &[target]),
            Ok(())
        );
    }

    #[test]
    fn rejects_target_fields_for_unrelated_additions() {
        let mut value = candidate("I like cats");
        value.target_memory_id = Some(3);
        value.expected_target_revision = Some(1);
        assert!(matches!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::UnexpectedTarget { .. })
        ));
    }

    #[test]
    fn rejects_a_targeted_action_when_the_relation_is_unrelated() {
        let mut value = candidate("I like cats");
        value.proposed_action = ProposedAction::Reinforce { memory_id: 3 };
        assert!(matches!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::UnexpectedTarget {
                target_memory_id: None,
            })
        ));
    }

    #[test]
    fn rejects_target_fields_for_add_actions() {
        let mut value = candidate("I like cats");
        value.relation = CandidateRelation::Same;
        value.target_memory_id = Some(3);
        value.expected_target_revision = Some(1);
        assert!(matches!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::UnexpectedTarget {
                target_memory_id: Some(3),
            })
        ));
    }

    #[test]
    fn rejects_an_external_verification_state_without_a_trusted_writer() {
        let mut value = candidate("I like cats");
        value.atom.verification_status = VerificationStatus::ExternallyContradicted;
        assert_eq!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::ExternalVerificationReserved)
        );
    }

    #[test]
    fn rejects_an_unknown_target_named_by_the_action() {
        let mut value = candidate("I like cats");
        value.relation = CandidateRelation::Same;
        value.target_memory_id = Some(99);
        value.expected_target_revision = Some(1);
        value.proposed_action = ProposedAction::Reinforce { memory_id: 99 };

        assert!(matches!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::UnknownTargetId {
                target_memory_id: 99
            })
        ));
    }

    #[test]
    fn rejects_a_target_id_that_disagrees_with_the_action() {
        let mut value = candidate("I like cats");
        value.relation = CandidateRelation::Same;
        value.target_memory_id = Some(3);
        value.expected_target_revision = Some(1);
        value.proposed_action = ProposedAction::Reinforce { memory_id: 4 };

        assert!(matches!(
            validate_candidate(&value, "turn:1", "I like cats", &[]),
            Err(ValidationError::ActionTargetMismatch {
                action_target_memory_id: 4,
                candidate_target_memory_id: Some(3),
            })
        ));
    }

    #[test]
    fn rejects_illegal_lifecycle_transitions_for_target_actions() {
        let target = MemoryCandidate {
            id: 3,
            revision: Some(1),
            content: "I like cats".into(),
            state: MemoryState::Active,
            pinned: false,
            mention_count: 1,
            last_seen_at: 0,
            lexical_relevance: 1.0,
            strength: 1.0,
        };
        let mut value = candidate("I like cats");
        value.relation = CandidateRelation::Same;
        value.target_memory_id = Some(3);
        value.expected_target_revision = Some(1);
        value.proposed_action = ProposedAction::Reinforce { memory_id: 3 };
        value.atom.lifecycle_state = MemoryState::Superseded;

        assert!(matches!(
            validate_candidate(&value, "turn:1", "I like cats", &[target]),
            Err(ValidationError::LifecycleTransitionForbidden {
                from: MemoryState::Active,
                to: MemoryState::Superseded,
            })
        ));
    }
}
