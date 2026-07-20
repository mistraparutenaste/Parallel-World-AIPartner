use super::consolidation::ProposedAction;
use super::{
    Attribution, Conditionality, MemoryAtom, MemoryCandidate, Polarity, SourceMode, SourceSpan,
    SpeechAct, SubjectScope, VerificationStatus, is_safe_persistent_content,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateRelation {
    Same,
    Refines,
    Contradicts,
    ChangesStance,
    Unrelated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    AssistantAttributionForbidden,
    ExternalCorroborationWithoutEvidence,
    MarkerPreservationUnproven,
    ModalMarkerRemoved,
    ControlCharacterForbidden,
    UnsafePersistentContent,
}

pub fn validate_candidate(
    candidate: &TypedCandidate,
    source: &str,
    targets: &[MemoryCandidate],
) -> Result<(), ValidationError> {
    validate_spans(&candidate.atom.source_spans, source)?;
    validate_normalization(candidate, source)?;
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

fn validate_spans(spans: &[SourceSpan], source: &str) -> Result<(), ValidationError> {
    if spans.is_empty() {
        return Err(ValidationError::MissingSourceSpan);
    }
    for span in spans {
        if span.start > span.end || span.end > source.len() {
            return Err(ValidationError::SourceSpanOutOfBounds);
        }
        if !source.is_char_boundary(span.start) || !source.is_char_boundary(span.end) {
            return Err(ValidationError::SourceSpanNotCharBoundary);
        }
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
        while edit_index < edits.len() && edits[edit_index].end <= span.start {
            return Err(ValidationError::NormalizationTraceDoesNotCoverChangedBytes);
        }
        while edit_index < edits.len() && edits[edit_index].start < span.end {
            let edit = edits[edit_index];
            if edit.start < cursor || edit.end > span.end || edit.start > edit.end {
                return Err(ValidationError::NormalizationTraceDoesNotCoverChangedBytes);
            }
            if !source.is_char_boundary(edit.start) || !source.is_char_boundary(edit.end) {
                return Err(ValidationError::SourceSpanNotCharBoundary);
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
    if candidate.atom.subject_scope == SubjectScope::ExternalWorld
        && candidate.atom.verification_status == VerificationStatus::ExternallyCorroborated
    {
        return Err(ValidationError::ExternalCorroborationWithoutEvidence);
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

fn validate_target(
    candidate: &TypedCandidate,
    targets: &[MemoryCandidate],
) -> Result<(), ValidationError> {
    let Some(target_memory_id) = candidate.target_memory_id else {
        return Ok(());
    };
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
        DiscourseFeatures, EpistemicForm, Fictionality, MemoryState, TemporalScope,
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
        value.target_memory_id = Some(3);
        value.expected_target_revision = Some(1);
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
            validate_candidate(&value, "I like cats", &[target]),
            Err(ValidationError::StaleTargetRevision { .. })
        ));
    }

    #[test]
    fn preserves_negation_and_question_markers() {
        let mut value = candidate("猫は好きではない？");
        value.atom.discourse.polarity = Polarity::Affirmed;
        value.atom.discourse.speech_act = SpeechAct::Asserted;
        assert_eq!(
            validate_candidate(&value, "猫は好きではない？", &[]),
            Err(ValidationError::MarkerPreservationUnproven)
        );
    }
}
