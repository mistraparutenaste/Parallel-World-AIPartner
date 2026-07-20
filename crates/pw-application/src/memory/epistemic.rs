use super::MemoryState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryAtom {
    pub id: i64,
    pub revision: i64,
    pub content: String,
    pub subject_scope: SubjectScope,
    pub epistemic_form: EpistemicForm,
    pub attribution: Attribution,
    pub discourse: DiscourseFeatures,
    pub verification_status: VerificationStatus,
    pub temporal_scope: TemporalScope,
    pub lifecycle_state: MemoryState,
    pub source_spans: Vec<SourceSpan>,
}

impl MemoryAtom {
    #[must_use]
    pub fn legacy(id: i64, content: String, lifecycle_state: MemoryState) -> Self {
        Self {
            id,
            revision: 1,
            content,
            subject_scope: SubjectScope::LegacyUnknown,
            epistemic_form: EpistemicForm::LegacyUntyped,
            attribution: Attribution::Unknown,
            discourse: DiscourseFeatures {
                source_mode: SourceMode::Reported,
                ..DiscourseFeatures::default()
            },
            verification_status: VerificationStatus::Unknown,
            temporal_scope: TemporalScope::Unknown,
            lifecycle_state,
            source_spans: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscourseFeatures {
    pub speech_act: SpeechAct,
    pub source_mode: SourceMode,
    pub polarity: Polarity,
    pub conditionality: Conditionality,
    pub fictionality: Fictionality,
}

impl Default for DiscourseFeatures {
    fn default() -> Self {
        Self {
            speech_act: SpeechAct::Unknown,
            source_mode: SourceMode::Direct,
            polarity: Polarity::Unknown,
            conditionality: Conditionality::Unknown,
            fictionality: Fictionality::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    /// UTF-8 byte offsets into the accepted user utterance.
    pub source_id: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubjectScope {
    UserSelf,
    ExternalWorld,
    OtherPerson,
    FictionalSubject,
    LegacyUnknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpistemicForm {
    FactClaim,
    Belief,
    Impression,
    PredictionOrHunch,
    Metaphor,
    Emotion,
    LegacyUntyped,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attribution {
    User,
    Assistant,
    NamedThirdParty,
    ExternalSource,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerificationStatus {
    NotApplicable,
    UserReported,
    UnverifiedExternalClaim,
    ExternallyCorroborated,
    ExternallyContradicted,
    Disputed,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemporalScope {
    Stable,
    Current,
    Past,
    Future,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechAct {
    Asserted,
    Questioned,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceMode {
    Direct,
    Reported,
    Quoted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Polarity {
    Affirmed,
    Negated,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conditionality {
    Actual,
    Hypothetical,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fictionality {
    RealWorld,
    Fictional,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_defaults_preserve_unknown_attribution() {
        let memory = MemoryAtom::legacy(7, "legacy".into(), MemoryState::Active);
        assert_eq!(memory.subject_scope, SubjectScope::LegacyUnknown);
        assert_eq!(memory.epistemic_form, EpistemicForm::LegacyUntyped);
        assert_eq!(memory.attribution, Attribution::Unknown);
        assert_eq!(memory.discourse.source_mode, SourceMode::Reported);
        assert_eq!(memory.discourse.speech_act, SpeechAct::Unknown);
        assert_eq!(memory.discourse.polarity, Polarity::Unknown);
        assert_eq!(memory.discourse.conditionality, Conditionality::Unknown);
    }
}
