use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use super::{MemoryAction, MemoryCandidate, is_safe_persistent_content};
use crate::PortError;
use crate::conversation::{ChatMessage, ChatRole, LlmClient};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(from = "StrictProposedAction")]
pub enum ProposedAction {
    Add {
        content: String,
    },
    Reinforce {
        memory_id: i64,
    },
    Supersede {
        old_memory_id: i64,
        content: String,
    },
    Pin {
        memory_id: Option<i64>,
        content: Option<String>,
    },
    Ignore,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum StrictProposedAction {
    Add {
        content: String,
    },
    Reinforce {
        memory_id: i64,
    },
    Supersede {
        old_memory_id: i64,
        content: String,
    },
    Pin {
        memory_id: Option<i64>,
        content: Option<String>,
    },
    Ignore {},
}

impl From<StrictProposedAction> for ProposedAction {
    fn from(action: StrictProposedAction) -> Self {
        match action {
            StrictProposedAction::Add { content } => Self::Add { content },
            StrictProposedAction::Reinforce { memory_id } => Self::Reinforce { memory_id },
            StrictProposedAction::Supersede {
                old_memory_id,
                content,
            } => Self::Supersede {
                old_memory_id,
                content,
            },
            StrictProposedAction::Pin { memory_id, content } => Self::Pin { memory_id, content },
            StrictProposedAction::Ignore {} => Self::Ignore,
        }
    }
}

#[allow(clippy::missing_errors_doc)]
pub trait MemoryClassifier: Send {
    fn classify(
        &mut self,
        statement: &str,
        candidates: &[MemoryCandidate],
    ) -> Result<ProposedAction, PortError>;
}

impl MemoryClassifier for Box<dyn MemoryClassifier> {
    fn classify(
        &mut self,
        statement: &str,
        candidates: &[MemoryCandidate],
    ) -> Result<ProposedAction, PortError> {
        self.as_mut().classify(statement, candidates)
    }
}

pub struct HybridConsolidator<C> {
    classifier: C,
}

impl<C: MemoryClassifier> HybridConsolidator<C> {
    #[must_use]
    pub fn new(classifier: C) -> Self {
        Self { classifier }
    }

    #[must_use]
    pub fn decide(&mut self, statement: &str, candidates: &[MemoryCandidate]) -> MemoryAction {
        self.classifier
            .classify(statement, candidates)
            .ok()
            .and_then(|proposal| validate(proposal, statement, candidates))
            .unwrap_or_else(|| exact_match_fallback(statement, candidates))
    }
}

fn validate(
    proposal: ProposedAction,
    statement: &str,
    candidates: &[MemoryCandidate],
) -> Option<MemoryAction> {
    match proposal {
        ProposedAction::Add { content } => {
            validated_content(&content, statement).then_some(MemoryAction::Add {
                content,
                pinned: false,
            })
        }
        ProposedAction::Reinforce { memory_id } => {
            allowed_id(memory_id, candidates).then_some(MemoryAction::Reinforce {
                memory_id,
                pin: false,
            })
        }
        ProposedAction::Supersede {
            old_memory_id,
            content,
        } => (allowed_id(old_memory_id, candidates) && validated_content(&content, statement))
            .then_some(MemoryAction::Supersede {
                old_memory_id,
                content,
                pin_replacement: has_explicit_pin_intent(statement),
            }),
        ProposedAction::Pin { memory_id, content } if has_explicit_pin_intent(statement) => {
            match (memory_id, content) {
                (Some(memory_id), None) if allowed_id(memory_id, candidates) => {
                    Some(MemoryAction::Reinforce {
                        memory_id,
                        pin: true,
                    })
                }
                (None, Some(content)) if validated_content(&content, statement) => {
                    Some(MemoryAction::Add {
                        content,
                        pinned: true,
                    })
                }
                _ => None,
            }
        }
        ProposedAction::Pin { .. } => None,
        ProposedAction::Ignore => Some(MemoryAction::Ignore),
    }
}

fn allowed_id(id: i64, candidates: &[MemoryCandidate]) -> bool {
    candidates.iter().any(|candidate| candidate.id == id)
}

fn validated_content(content: &str, statement: &str) -> bool {
    let normalized_content = normalize(content);
    !normalized_content.is_empty()
        && is_safe_persistent_content(content)
        && normalize(statement).contains(&normalized_content)
}

fn exact_match_fallback(statement: &str, candidates: &[MemoryCandidate]) -> MemoryAction {
    let normalized_statement = normalize(statement);
    if normalized_statement.is_empty() {
        return MemoryAction::Ignore;
    }
    candidates
        .iter()
        .find(|candidate| normalize(&candidate.content) == normalized_statement)
        .map_or(MemoryAction::Ignore, |candidate| MemoryAction::Reinforce {
            memory_id: candidate.id,
            pin: false,
        })
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            !character.is_whitespace()
                && !matches!(character, '。' | '、' | '!' | '！' | '?' | '？')
        })
        .flat_map(char::to_lowercase)
        .collect()
}

#[must_use]
pub fn has_explicit_pin_intent(statement: &str) -> bool {
    let statement = normalize(statement);
    ["覚えておいて", "記憶しておいて", "忘れないで"]
        .iter()
        .any(|phrase| statement.contains(&normalize(phrase)))
}

pub struct LlmMemoryClassifier<L> {
    llm: L,
    cancel: Arc<AtomicBool>,
}

impl<L: LlmClient> LlmMemoryClassifier<L> {
    #[must_use]
    pub fn new(llm: L) -> Self {
        Self {
            llm,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    #[must_use]
    pub fn new_with_cancel(llm: L, cancel: Arc<AtomicBool>) -> Self {
        Self { llm, cancel }
    }
}

impl<L: LlmClient> MemoryClassifier for LlmMemoryClassifier<L> {
    fn classify(
        &mut self,
        statement: &str,
        candidates: &[MemoryCandidate],
    ) -> Result<ProposedAction, PortError> {
        let system = ChatMessage::new(
            ChatRole::System,
            concat!(
                "Return exactly one JSON object and no prose. Allowed schema: ",
                "{\"operation\":\"add\",\"content\":string} | ",
                "{\"operation\":\"reinforce\",\"memory_id\":integer} | ",
                "{\"operation\":\"supersede\",\"old_memory_id\":integer,\"content\":string} | ",
                "{\"operation\":\"pin\",\"memory_id\":integer|null,\"content\":string|null} | ",
                "{\"operation\":\"ignore\"}. Do not add fields."
            ),
        );
        let candidate_values = candidates
            .iter()
            .map(|candidate| {
                serde_json::json!({
                    "id": candidate.id,
                    "content": candidate.content,
                })
            })
            .collect::<Vec<_>>();
        let user = ChatMessage::new(
            ChatRole::User,
            serde_json::json!({
                "statement": statement,
                "candidates": candidate_values,
            })
            .to_string(),
        );
        let mut output = String::new();
        self.llm
            .stream_chat(&[system, user], &self.cancel, &mut |delta| {
                output.push_str(delta);
            })?;
        let json = strip_optional_json_fence(&output)?;
        serde_json::from_str(json)
            .map_err(|error| PortError(format!("invalid memory classifier output: {error}")))
    }
}

fn strip_optional_json_fence(output: &str) -> Result<&str, PortError> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Err(PortError("empty memory classifier output".into()));
    }
    if let Some(body) = trimmed.strip_prefix("```json\n") {
        return body
            .strip_suffix("\n```")
            .map(str::trim)
            .filter(|body| !body.is_empty())
            .ok_or_else(|| PortError("invalid JSON Markdown fence".into()));
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use crate::conversation::{ChatMessage, LlmClient};
    use crate::{
        PortError,
        memory::{MemoryAction, MemoryCandidate, MemoryState},
    };

    use super::*;

    fn candidate(id: i64, content: &str) -> MemoryCandidate {
        MemoryCandidate {
            id,
            revision: Some(1),
            content: content.into(),
            state: MemoryState::Active,
            pinned: false,
            mention_count: 1,
            last_seen_at: 0,
            lexical_relevance: 1.0,
            strength: 1.0,
        }
    }

    enum FakeResult {
        Action(ProposedAction),
        Failure,
    }

    struct FakeClassifier(FakeResult);

    impl FakeClassifier {
        fn returns(action: ProposedAction) -> Self {
            Self(FakeResult::Action(action))
        }

        fn fails() -> Self {
            Self(FakeResult::Failure)
        }
    }

    impl MemoryClassifier for FakeClassifier {
        fn classify(
            &mut self,
            _: &str,
            _: &[MemoryCandidate],
        ) -> Result<ProposedAction, PortError> {
            match &self.0 {
                FakeResult::Action(action) => Ok(action.clone()),
                FakeResult::Failure => Err(PortError("classifier unavailable".into())),
            }
        }
    }

    #[test]
    fn invalid_classifier_id_falls_back_without_mutation() {
        let candidates = vec![candidate(1, "猫が好き")];
        let mut consolidator =
            HybridConsolidator::new(FakeClassifier::returns(ProposedAction::Reinforce {
                memory_id: 999,
            }));
        assert_eq!(
            consolidator.decide("猫が好き", &candidates),
            MemoryAction::Reinforce {
                memory_id: 1,
                pin: false,
            }
        );
    }

    #[test]
    fn semantic_or_destructive_fallback_is_forbidden() {
        let candidates = vec![candidate(1, "猫が好き")];
        let mut consolidator = HybridConsolidator::new(FakeClassifier::fails());
        assert_eq!(
            consolidator.decide("犬が好き", &candidates),
            MemoryAction::Ignore
        );
    }

    #[test]
    fn pin_requires_deterministic_explicit_intent() {
        let mut consolidator =
            HybridConsolidator::new(FakeClassifier::returns(ProposedAction::Pin {
                memory_id: None,
                content: Some("猫が好き".into()),
            }));
        assert_eq!(consolidator.decide("猫が好き", &[]), MemoryAction::Ignore);
        assert_eq!(
            consolidator.decide("猫が好き。覚えておいて", &[]),
            MemoryAction::Add {
                content: "猫が好き".into(),
                pinned: true,
            }
        );
    }

    #[test]
    fn valid_actions_require_allowlisted_ids_safe_content_and_statement_substrings() {
        let candidates = vec![candidate(1, "猫が好き")];
        let mut pin = HybridConsolidator::new(FakeClassifier::returns(ProposedAction::Pin {
            memory_id: Some(1),
            content: None,
        }));
        assert_eq!(
            pin.decide("猫が好き。記憶しておいて", &candidates),
            MemoryAction::Reinforce {
                memory_id: 1,
                pin: true,
            }
        );

        for content in ["", "犬が好き", "password=VerySecretPassword123456"] {
            let mut add = HybridConsolidator::new(FakeClassifier::returns(ProposedAction::Add {
                content: content.into(),
            }));
            assert_eq!(
                add.decide("猫が好き", &candidates),
                MemoryAction::Reinforce {
                    memory_id: 1,
                    pin: false
                }
            );
        }
    }

    #[test]
    fn supersede_pins_replacement_only_with_explicit_intent() {
        let candidates = vec![candidate(1, "私は猫が好き")];
        for (statement, expected_pin) in
            [("私は犬が好き", false), ("私は犬が好き。忘れないで", true)]
        {
            let mut consolidator =
                HybridConsolidator::new(FakeClassifier::returns(ProposedAction::Supersede {
                    old_memory_id: 1,
                    content: "私は犬が好き".into(),
                }));
            assert_eq!(
                consolidator.decide(statement, &candidates),
                MemoryAction::Supersede {
                    old_memory_id: 1,
                    content: "私は犬が好き".into(),
                    pin_replacement: expected_pin,
                }
            );
        }
    }

    #[test]
    fn supersede_rejects_unknown_ids_and_ungrounded_content() {
        let candidates = vec![candidate(1, "私は猫が好き")];
        for proposal in [
            ProposedAction::Supersede {
                old_memory_id: 999,
                content: "私は犬が好き".into(),
            },
            ProposedAction::Supersede {
                old_memory_id: 1,
                content: "私は鳥が好き".into(),
            },
        ] {
            let mut consolidator = HybridConsolidator::new(FakeClassifier::returns(proposal));
            assert_eq!(
                consolidator.decide("私は犬が好き", &candidates),
                MemoryAction::Ignore
            );
        }
    }

    struct StaticLlm(Result<&'static str, &'static str>);

    impl LlmClient for StaticLlm {
        fn stream_chat(
            &mut self,
            _: &[ChatMessage],
            _: &AtomicBool,
            on_delta: &mut dyn FnMut(&str),
        ) -> Result<(), PortError> {
            match self.0 {
                Ok(output) => {
                    on_delta(output);
                    Ok(())
                }
                Err(message) => Err(PortError(message.into())),
            }
        }
    }

    #[test]
    fn llm_classifier_accepts_one_json_object_and_rejects_extra_prose() {
        let candidates = vec![candidate(1, "猫が好き")];
        let mut valid = LlmMemoryClassifier::new(StaticLlm(Ok(
            "```json\n{\"operation\":\"reinforce\",\"memory_id\":1}\n```",
        )));
        assert_eq!(
            valid.classify("猫が好き", &candidates).unwrap(),
            ProposedAction::Reinforce { memory_id: 1 },
        );
        let mut invalid = LlmMemoryClassifier::new(StaticLlm(Ok(
            "承知しました。{\"operation\":\"reinforce\",\"memory_id\":1}",
        )));
        assert!(invalid.classify("猫が好き", &candidates).is_err());
    }

    #[test]
    fn llm_classifier_rejects_unknown_fields_empty_output_and_transport_failure() {
        for result in [
            Ok("{\"operation\":\"ignore\",\"reason\":\"extra\"}"),
            Ok("  "),
            Err("offline"),
        ] {
            let mut classifier = LlmMemoryClassifier::new(StaticLlm(result));
            assert!(classifier.classify("猫が好き", &[]).is_err());
        }
    }

    #[test]
    fn boxed_classifier_delegates() {
        let classifier: Box<dyn MemoryClassifier> =
            Box::new(FakeClassifier::returns(ProposedAction::Ignore));
        let mut consolidator = HybridConsolidator::new(classifier);
        assert_eq!(consolidator.decide("雑談", &[]), MemoryAction::Ignore);
    }
}
