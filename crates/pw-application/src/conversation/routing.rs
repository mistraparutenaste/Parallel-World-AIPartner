//! Conditional response planning kept off the ordinary chat path.
//!
//! A planned turn still has exactly one streamed completion.  Planning only
//! produces a bounded, deterministic prompt surface; it never emits user
//! visible output and failure deliberately falls back to the ordinary prompt.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError, sync_channel};
use std::thread;
use std::time::Duration;

use crate::PortError;
use crate::memory::MemoryContext;

use super::{ChatMessage, ChatRole};

const MAX_PLAN_GOAL_CHARS: usize = 160;
const MAX_PLAN_QUERY_CHARS: usize = 240;
const MAX_PLAN_DIRECTIVES: usize = 4;
const MAX_DIRECTIVE_CHARS: usize = 160;
const MAX_SURFACE_HINT_CHARS: usize = 320;
const MAX_SURFACE_FACTS: usize = 4;
const MAX_SURFACE_FACT_CHARS: usize = 240;

/// Turn types that warrant bounded response preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnKind {
    Simple,
    Memory,
    Commitment,
    Correction,
    DecisionSupport,
    Tool,
    Proactive,
}

impl TurnKind {
    #[must_use]
    pub const fn requires_planning(self) -> bool {
        !matches!(self, Self::Simple)
    }

    #[must_use]
    pub const fn surface_label(self) -> &'static str {
        match self {
            Self::Simple => "ordinary conversation",
            Self::Memory => "memory-aware response",
            Self::Commitment => "commitment follow-up",
            Self::Correction => "correction",
            Self::DecisionSupport => "decision support",
            Self::Tool => "tool-related response",
            Self::Proactive => "proactive check-in",
        }
    }
}

/// The response-ending constraints selected for one user turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnStyleContract {
    pub turn_kind: DialogueTurnKind,
    pub question_policy: QuestionPolicy,
    pub closing_preference: ClosingPreference,
    pub recent_assistant_question_endings: u8,
}

/// Conservative categories for deterministic dialogue-style selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogueTurnKind {
    Greeting,
    Compliment,
    CasualObservation,
    AnswerOrRequest,
    RequestedQuestioning,
}

/// Whether a response may close with a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionPolicy {
    AvoidQuestionEnding,
    ClarificationOnlyIfMateriallyNecessary,
    ContentfulQuestionOnlyIfNoRecentQuestion,
    QuestionRequested,
}

/// Preferred response-ending form for one turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosingPreference {
    Declarative,
    QuestionPermitted,
}

/// Pure, deterministic lexical classifier for dialogue-ending constraints.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DialogueClassifier;

impl DialogueClassifier {
    #[must_use]
    pub fn classify(&self, current_utterance: &str, history: &[ChatMessage]) -> TurnStyleContract {
        let recent_assistant_question_endings = u8::try_from(
            history
                .iter()
                .rev()
                .filter(|message| message.role == ChatRole::Assistant)
                .take(3)
                .filter(|message| assistant_ends_with_question(&message.content))
                .count(),
        )
        .unwrap_or(u8::MAX);
        let normalized = current_utterance.trim().to_lowercase();
        let (turn_kind, question_policy, closing_preference) = if is_greeting(&normalized) {
            (
                DialogueTurnKind::Greeting,
                QuestionPolicy::AvoidQuestionEnding,
                ClosingPreference::Declarative,
            )
        } else if is_requested_questioning(&normalized) {
            (
                DialogueTurnKind::RequestedQuestioning,
                QuestionPolicy::QuestionRequested,
                ClosingPreference::QuestionPermitted,
            )
        } else if is_compliment(&normalized) {
            (
                DialogueTurnKind::Compliment,
                QuestionPolicy::AvoidQuestionEnding,
                ClosingPreference::Declarative,
            )
        } else if is_casual_observation(&normalized) {
            if recent_assistant_question_endings == 0 {
                (
                    DialogueTurnKind::CasualObservation,
                    QuestionPolicy::ContentfulQuestionOnlyIfNoRecentQuestion,
                    ClosingPreference::QuestionPermitted,
                )
            } else {
                (
                    DialogueTurnKind::CasualObservation,
                    QuestionPolicy::AvoidQuestionEnding,
                    ClosingPreference::Declarative,
                )
            }
        } else {
            (
                DialogueTurnKind::AnswerOrRequest,
                QuestionPolicy::ClarificationOnlyIfMateriallyNecessary,
                ClosingPreference::Declarative,
            )
        };

        TurnStyleContract {
            turn_kind,
            question_policy,
            closing_preference,
            recent_assistant_question_endings,
        }
    }
}

fn is_greeting(text: &str) -> bool {
    !has_concrete_request(text)
        && matches!(
            trim_terminal_punctuation(text),
            "hello" | "hello there" | "hi" | "hey" | "こんにちは" | "おはよう"
        )
}

fn is_compliment(text: &str) -> bool {
    !has_concrete_request(text)
        && text.chars().count() <= 48
        && contains_any(
            text,
            &[
                "thank you",
                "thanks",
                "great",
                "nice",
                "ありがとう",
                "すごい",
                "分かりやすい",
                "わかりやすい",
            ],
        )
}

fn is_casual_observation(text: &str) -> bool {
    !has_concrete_request(text)
        && text.chars().count() <= 48
        && contains_any(
            text,
            &[
                "weather",
                "rain",
                "sunny",
                "today is",
                "it's ",
                "it is ",
                "天気",
                "今日は",
                "雨",
                "暑い",
                "寒い",
                "疲れた",
                "眠い",
            ],
        )
}

fn is_requested_questioning(text: &str) -> bool {
    is_affirmative_command(
        text,
        &[
            "ask me questions",
            "ask me a question",
            "ask me one question at a time",
            "please ask me questions",
            "please ask me a question",
            "can you ask me questions",
            "could you ask me questions",
            "i want you to ask me questions",
            "interview me",
            "please interview me",
            "can you interview me",
            "could you interview me",
        ],
    ) || matches!(
        trim_terminal_punctuation(text),
        "質問して"
            | "質問してください"
            | "質問を一つずつして"
            | "質問を一つずつして、計画を整理して"
            | "質問で確認して"
    )
}

fn is_affirmative_command(text: &str, commands: &[&str]) -> bool {
    let text = trim_terminal_punctuation(text);
    commands.iter().any(|command| {
        text == *command
            || text
                .strip_prefix(command)
                .is_some_and(|suffix| suffix.starts_with(" to "))
    })
}

fn has_concrete_request(text: &str) -> bool {
    assistant_ends_with_question(text)
        || [
            "can you ",
            "could you ",
            "please ",
            "help me ",
            "recommend ",
            "show ",
            "tell me ",
            "explain ",
            "suggest ",
            "give me ",
            "i need ",
        ]
        .iter()
        .any(|prefix| text.starts_with(prefix))
        || ["教えて", "してください", "してほしい", "相談したい"]
            .iter()
            .any(|suffix| trim_terminal_punctuation(text).ends_with(suffix))
}

fn trim_terminal_punctuation(text: &str) -> &str {
    text.trim_end_matches(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '.' | '!'
                    | '。'
                    | '！'
                    | '…'
                    | '"'
                    | '\''
                    | ')'
                    | '）'
                    | ']'
                    | '】'
                    | '}'
                    | '」'
                    | '』'
            )
    })
}

fn assistant_ends_with_question(content: &str) -> bool {
    let content = trim_terminal_punctuation(content);
    content.ends_with(['?', '？'])
        || ["ですか", "ますか", "ませんか", "でしょうか", "ましょうか"]
            .iter()
            .any(|suffix| content.ends_with(suffix))
}

/// Deterministic, conservative lexical router.
///
/// It only opts into planning for explicit cues.  Everything else is simple
/// conversation and therefore cannot incur planning or retrieval work.
#[derive(Debug, Default, Clone, Copy)]
pub struct IntentRouter;

impl IntentRouter {
    #[must_use]
    pub fn classify(&self, text: &str) -> TurnKind {
        let normalized = text.trim().to_lowercase();
        if normalized.is_empty() {
            return TurnKind::Simple;
        }
        if contains_any(&normalized, &["覚えて", "記憶", "remember", "memory"]) {
            TurnKind::Memory
        } else if contains_any(
            &normalized,
            &[
                "約束",
                "あとで",
                "リマインド",
                "todo",
                "remind",
                "commitment",
            ],
        ) {
            TurnKind::Commitment
        } else if contains_any(
            &normalized,
            &["訂正", "違う", "間違い", "correct", "actually"],
        ) {
            TurnKind::Correction
        } else if contains_any(
            &normalized,
            &[
                "どちら",
                "比較",
                "決め",
                "選ぶ",
                "recommend",
                "decide",
                "compare",
            ],
        ) {
            TurnKind::DecisionSupport
        } else if contains_any(
            &normalized,
            &["ツール", "検索して", "調べて", "tool", "search ", "look up"],
        ) {
            TurnKind::Tool
        } else if contains_any(&normalized, &["先に", "話しかけ", "proactive", "check in"]) {
            TurnKind::Proactive
        } else {
            TurnKind::Simple
        }
    }
}

fn contains_any(text: &str, cues: &[&str]) -> bool {
    cues.iter().any(|cue| text.contains(cue))
}

/// Bounded instructions produced by a response planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponsePlan {
    pub kind: TurnKind,
    pub goal: String,
    pub retrieval_query: Option<String>,
    pub directives: Vec<String>,
}

impl ResponsePlan {
    /// # Errors
    /// Returns an error when externally supplied plan data exceeds the prompt
    /// surface contract or attempts to describe an ordinary turn.
    pub fn validate(&self) -> Result<(), PortError> {
        if !self.kind.requires_planning()
            || !is_bounded_text(&self.goal, MAX_PLAN_GOAL_CHARS)
            || self
                .retrieval_query
                .as_deref()
                .is_some_and(|query| !is_bounded_text(query, MAX_PLAN_QUERY_CHARS))
            || self.directives.len() > MAX_PLAN_DIRECTIVES
            || self
                .directives
                .iter()
                .any(|directive| !is_bounded_text(directive, MAX_DIRECTIVE_CHARS))
        {
            return Err(PortError("invalid bounded response plan".into()));
        }
        Ok(())
    }
}

/// Bounded context that affects wording, never durable state or transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceContext {
    pub response_mode: String,
    /// Task goal for this turn. Kept separate from `tone_hint` so task
    /// wording never masquerades as a tone instruction.
    pub goal: Option<String>,
    pub tone_hint: Option<String>,
    pub relevant_facts: Vec<String>,
}

/// Bounded dialogue/commitment facts supplied only to planned turns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoundedStateContext {
    pub mood: Option<String>,
    pub reaction: Option<String>,
    pub relationship_score: Option<i64>,
    pub reflection_cursor: Option<String>,
    pub open_commitments: Vec<String>,
}

impl BoundedStateContext {
    /// # Errors
    /// Returns an error for an out-of-range relationship score, an unbounded
    /// optional field, or too many/unbounded open commitments.
    pub fn validate(&self) -> Result<(), PortError> {
        if self
            .relationship_score
            .is_some_and(|score| !(-100..=100).contains(&score))
            || [
                self.mood.as_deref(),
                self.reaction.as_deref(),
                self.reflection_cursor.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|value| value.trim().is_empty() || value.chars().count() > 96)
            || self.open_commitments.len() > 4
            || self
                .open_commitments
                .iter()
                .any(|value| value.trim().is_empty() || value.chars().count() > 160)
        {
            return Err(PortError("invalid bounded state context".into()));
        }
        Ok(())
    }

    fn as_facts(&self) -> Vec<String> {
        let mut facts = Vec::new();
        if let Some(mood) = &self.mood {
            facts.push(format!("dialogue.mood={mood}"));
        }
        if let Some(reaction) = &self.reaction {
            facts.push(format!("dialogue.reaction={reaction}"));
        }
        if let Some(score) = self.relationship_score {
            facts.push(format!("dialogue.relationship_score={score}"));
        }
        if let Some(cursor) = &self.reflection_cursor {
            facts.push(format!("dialogue.reflection_cursor={cursor}"));
        }
        facts.extend(
            self.open_commitments
                .iter()
                .map(|commitment| format!("commitment.open={commitment}")),
        );
        facts
    }
}

/// Provider used by a response retriever; implementations must return
/// metadata only and never a transcript.
pub trait PlannedStateContextProvider: Send {
    /// # Errors
    /// Returns an error when bounded companion state cannot be retrieved.
    fn retrieve_state(&mut self, plan: &ResponsePlan) -> Result<BoundedStateContext, PortError>;
}

/// Decorates an existing retriever with companion state. It is invoked only
/// after `ResponsePipeline` has classified a turn as planned.
pub struct StateAwareRetriever<R, P> {
    inner: R,
    state: P,
}

impl<R, P> StateAwareRetriever<R, P> {
    #[must_use]
    pub const fn new(inner: R, state: P) -> Self {
        Self { inner, state }
    }
}

impl<R, P> ResponseContextRetriever for StateAwareRetriever<R, P>
where
    R: ResponseContextRetriever,
    P: PlannedStateContextProvider,
{
    fn retrieve(
        &mut self,
        plan: &ResponsePlan,
        context: &MemoryContext,
    ) -> Result<MemoryContext, PortError> {
        let mut context = self.inner.retrieve(plan, context)?.bounded();
        let state = self.state.retrieve_state(plan)?;
        state.validate()?;
        context.memories.extend(state.as_facts());
        Ok(context.bounded())
    }
}

impl SurfaceContext {
    /// # Errors
    /// Returns an error for unbounded or blank surface data.
    pub fn validate(&self) -> Result<(), PortError> {
        if !is_bounded_text(&self.response_mode, MAX_SURFACE_HINT_CHARS)
            || self
                .goal
                .as_deref()
                .is_some_and(|goal| !is_bounded_text(goal, MAX_SURFACE_HINT_CHARS))
            || self
                .tone_hint
                .as_deref()
                .is_some_and(|hint| !is_bounded_text(hint, MAX_SURFACE_HINT_CHARS))
            || self.relevant_facts.len() > MAX_SURFACE_FACTS
            || self
                .relevant_facts
                .iter()
                .any(|fact| !is_bounded_text(fact, MAX_SURFACE_FACT_CHARS))
        {
            return Err(PortError("invalid bounded surface context".into()));
        }
        Ok(())
    }
}

fn is_bounded_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= maximum
}

/// Optional planning port. Implementations may use a local model, but must
/// not stream user-visible output.
pub trait ResponsePlanner: Send {
    /// # Errors
    /// Returns an error when a bounded response plan cannot be produced.
    fn plan(
        &mut self,
        kind: TurnKind,
        utterance: &str,
        context: &MemoryContext,
    ) -> Result<ResponsePlan, PortError>;
}

impl<T: ResponsePlanner + ?Sized> ResponsePlanner for Box<T> {
    fn plan(
        &mut self,
        kind: TurnKind,
        utterance: &str,
        context: &MemoryContext,
    ) -> Result<ResponsePlan, PortError> {
        (**self).plan(kind, utterance, context)
    }
}

/// Optional context retrieval port for planned turns only.
pub trait ResponseContextRetriever: Send {
    /// # Errors
    /// Returns an error when planned-turn context cannot be retrieved.
    fn retrieve(
        &mut self,
        plan: &ResponsePlan,
        context: &MemoryContext,
    ) -> Result<MemoryContext, PortError>;
}

impl<T: ResponseContextRetriever + ?Sized> ResponseContextRetriever for Box<T> {
    fn retrieve(
        &mut self,
        plan: &ResponsePlan,
        context: &MemoryContext,
    ) -> Result<MemoryContext, PortError> {
        (**self).retrieve(plan, context)
    }
}

/// Optional surface realization port. It must return bounded instructions
/// rather than direct assistant text so the existing streaming path remains
/// the sole source of reply/TTS events.
pub trait SurfaceRealizer: Send {
    /// # Errors
    /// Returns an error when a bounded surface context cannot be realized.
    fn realize(&mut self, plan: &ResponsePlan) -> Result<SurfaceContext, PortError>;
}

impl<T: SurfaceRealizer + ?Sized> SurfaceRealizer for Box<T> {
    fn realize(&mut self, plan: &ResponsePlan) -> Result<SurfaceContext, PortError> {
        (**self).realize(plan)
    }
}

/// A wall-clock budget for all non-streaming planned preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningBudget {
    pub max_elapsed: Duration,
}

impl Default for PlanningBudget {
    fn default() -> Self {
        Self {
            max_elapsed: Duration::from_millis(30),
        }
    }
}

/// Result passed to the normal prompt builder after planning succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedResponse {
    pub context: MemoryContext,
    pub surface: SurfaceContext,
}

/// Ports plus outcome returned by one background planned-preparation attempt.
type PendingPreparation<P, R, S> = (P, R, S, Option<PreparedResponse>);

/// A failure-isolated planned preparation pipeline.
pub struct ResponsePipeline<P, R, S> {
    planner: Option<P>,
    retriever: Option<R>,
    realizer: Option<S>,
    pending: Option<Receiver<PendingPreparation<P, R, S>>>,
    budget: PlanningBudget,
}

impl<P, R, S> ResponsePipeline<P, R, S> {
    #[must_use]
    pub const fn new(planner: P, retriever: R, realizer: S, budget: PlanningBudget) -> Self {
        Self {
            planner: Some(planner),
            retriever: Some(retriever),
            realizer: Some(realizer),
            pending: None,
            budget,
        }
    }
}

impl<P, R, S> ResponsePipeline<P, R, S>
where
    P: ResponsePlanner + 'static,
    R: ResponseContextRetriever + 'static,
    S: SurfaceRealizer + 'static,
{
    /// Runs only a planned turn. Any error, malformed payload, or elapsed
    /// budget returns `None`, which is deliberately the old prompt path.
    #[must_use]
    pub fn try_prepare(
        &mut self,
        kind: TurnKind,
        utterance: &str,
        context: &MemoryContext,
    ) -> Option<PreparedResponse> {
        if !kind.requires_planning() {
            return None;
        }
        self.reclaim_pending();
        if self.pending.is_some() {
            return None;
        }
        let planner = self.planner.take()?;
        let retriever = self.retriever.take()?;
        let realizer = self.realizer.take()?;
        let (sender, receiver) = sync_channel(1);
        let utterance = utterance.to_owned();
        let context = context.clone();
        thread::spawn(move || {
            let mut planner = planner;
            let mut retriever = retriever;
            let mut realizer = realizer;
            let prepared = catch_unwind(AssertUnwindSafe(|| {
                let plan = planner.plan(kind, &utterance, &context).ok()?;
                if plan.kind != kind {
                    return None;
                }
                plan.validate().ok()?;
                let context = retriever.retrieve(&plan, &context).ok()?.bounded();
                let surface = realizer.realize(&plan).ok()?;
                surface.validate().ok()?;
                Some(PreparedResponse { context, surface })
            }))
            .ok()
            .flatten();
            let _ = sender.send((planner, retriever, realizer, prepared));
        });

        match receiver.recv_timeout(self.budget.max_elapsed) {
            Ok((planner, retriever, realizer, prepared)) => {
                self.planner = Some(planner);
                self.retriever = Some(retriever);
                self.realizer = Some(realizer);
                prepared
            }
            Err(RecvTimeoutError::Timeout) => {
                self.pending = Some(receiver);
                None
            }
            Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    fn reclaim_pending(&mut self) {
        let Some(receiver) = self.pending.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok((planner, retriever, realizer, _prepared)) => {
                self.planner = Some(planner);
                self.retriever = Some(retriever);
                self.realizer = Some(realizer);
            }
            Err(TryRecvError::Empty) => self.pending = Some(receiver),
            Err(TryRecvError::Disconnected) => {}
        }
    }
}

/// Default planner: the route itself is enough to select safe, fixed goals.
/// It intentionally never makes a second LLM request.
#[derive(Debug, Default)]
pub struct LexicalResponsePlanner;

impl ResponsePlanner for LexicalResponsePlanner {
    fn plan(
        &mut self,
        kind: TurnKind,
        _utterance: &str,
        _context: &MemoryContext,
    ) -> Result<ResponsePlan, PortError> {
        let (goal, directive) = match kind {
            TurnKind::Memory => (
                "Answer with bounded recalled context when relevant",
                "Separate recalled context from current user intent",
            ),
            TurnKind::Commitment => (
                "Clarify commitment status and next step",
                "Do not claim a commitment was completed without evidence",
            ),
            TurnKind::Correction => (
                "Acknowledge and apply the correction",
                "Preserve uncertainty when the correction cannot be verified",
            ),
            TurnKind::DecisionSupport => (
                "Compare options and state trade-offs",
                "Do not invent facts or preferences",
            ),
            TurnKind::Tool => (
                "Answer the current request with only relevant tool actions",
                "Do not claim a tool was used or executed unless its result is present",
            ),
            TurnKind::Proactive => (
                "Respond to concrete observed context with one concise, self-contained utterance",
                "Do not append a generic menu or next-topic offer",
            ),
            TurnKind::Simple => return Err(PortError("simple turns must not be planned".into())),
        };
        Ok(ResponsePlan {
            kind,
            goal: goal.into(),
            retrieval_query: None,
            directives: vec![directive.into()],
        })
    }
}

/// Default retrieval preserves the already-bounded memory context. Storage
/// retrieval remains outside the response thread and can be plugged in later.
#[derive(Debug, Default)]
pub struct ExistingContextRetriever;

impl ResponseContextRetriever for ExistingContextRetriever {
    fn retrieve(
        &mut self,
        _plan: &ResponsePlan,
        context: &MemoryContext,
    ) -> Result<MemoryContext, PortError> {
        Ok(context.clone())
    }
}

/// Default realizer maps only fixed plan fields into a bounded system hint.
#[derive(Debug, Default)]
pub struct FixedSurfaceRealizer;

impl SurfaceRealizer for FixedSurfaceRealizer {
    fn realize(&mut self, plan: &ResponsePlan) -> Result<SurfaceContext, PortError> {
        Ok(SurfaceContext {
            response_mode: plan.kind.surface_label().into(),
            goal: Some(plan.goal.clone()),
            tone_hint: None,
            relevant_facts: plan.directives.clone(),
        })
    }
}

pub type ConfiguredResponsePipeline = ResponsePipeline<
    Box<dyn ResponsePlanner>,
    Box<dyn ResponseContextRetriever>,
    Box<dyn SurfaceRealizer>,
>;

#[must_use]
pub fn response_pipeline(
    planner: impl ResponsePlanner + 'static,
    retriever: impl ResponseContextRetriever + 'static,
    realizer: impl SurfaceRealizer + 'static,
    budget: PlanningBudget,
) -> ConfiguredResponsePipeline {
    ResponsePipeline::new(
        Box::new(planner),
        Box::new(retriever),
        Box::new(realizer),
        budget,
    )
}

#[must_use]
pub fn default_response_pipeline() -> ConfiguredResponsePipeline {
    response_pipeline(
        LexicalResponsePlanner,
        ExistingContextRetriever,
        FixedSurfaceRealizer,
        PlanningBudget::default(),
    )
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::super::{ChatMessage, ChatRole};
    use super::*;

    #[test]
    fn contract_counts_only_the_last_three_assistant_question_endings() {
        let history = vec![
            ChatMessage::new(ChatRole::Assistant, "older question?"),
            ChatMessage::new(ChatRole::User, "intervening user"),
            ChatMessage::new(ChatRole::Assistant, "recent question?"),
            ChatMessage::new(ChatRole::Assistant, "recent statement."),
            ChatMessage::new(ChatRole::Assistant, "newest question？"),
        ];

        let contract = DialogueClassifier.classify("Explain the trade-off.", &history);

        assert_eq!(contract.recent_assistant_question_endings, 2);
        assert_eq!(contract.turn_kind, DialogueTurnKind::AnswerOrRequest);
        assert_eq!(
            contract.question_policy,
            QuestionPolicy::ClarificationOnlyIfMateriallyNecessary
        );
        assert_eq!(contract.closing_preference, ClosingPreference::Declarative);
    }

    #[test]
    fn explicit_request_for_questions_can_override_cadence_preference() {
        let history = vec![ChatMessage::new(ChatRole::Assistant, "What is the goal?")];

        let contract = DialogueClassifier.classify(
            "Ask me one question at a time to clarify the plan.",
            &history,
        );

        assert_eq!(contract.turn_kind, DialogueTurnKind::RequestedQuestioning);
        assert_eq!(contract.question_policy, QuestionPolicy::QuestionRequested);
        assert_eq!(
            contract.closing_preference,
            ClosingPreference::QuestionPermitted
        );
    }

    #[test]
    fn japanese_greeting_avoids_a_question_ending() {
        let contract = DialogueClassifier.classify("こんにちは", &[]);

        assert_eq!(contract.turn_kind, DialogueTurnKind::Greeting);
        assert_eq!(
            contract.question_policy,
            QuestionPolicy::AvoidQuestionEnding
        );
        assert_eq!(contract.closing_preference, ClosingPreference::Declarative);
    }

    #[test]
    fn japanese_compliment_avoids_a_question_ending() {
        let contract = DialogueClassifier.classify("説明が分かりやすいです", &[]);

        assert_eq!(contract.turn_kind, DialogueTurnKind::Compliment);
        assert_eq!(
            contract.question_policy,
            QuestionPolicy::AvoidQuestionEnding
        );
        assert_eq!(contract.closing_preference, ClosingPreference::Declarative);
    }

    #[test]
    fn casual_observation_allows_a_contentful_question_without_recent_questions() {
        let contract = DialogueClassifier.classify("今日は雨です", &[]);

        assert_eq!(contract.turn_kind, DialogueTurnKind::CasualObservation);
        assert_eq!(
            contract.question_policy,
            QuestionPolicy::ContentfulQuestionOnlyIfNoRecentQuestion
        );
        assert_eq!(
            contract.closing_preference,
            ClosingPreference::QuestionPermitted
        );
        assert_eq!(contract.recent_assistant_question_endings, 0);
    }

    #[test]
    fn casual_observation_avoids_a_question_after_a_recent_assistant_question() {
        let history = [ChatMessage::new(ChatRole::Assistant, "How are you?")];

        let contract = DialogueClassifier.classify("今日は雨です", &history);

        assert_eq!(contract.turn_kind, DialogueTurnKind::CasualObservation);
        assert_eq!(
            contract.question_policy,
            QuestionPolicy::AvoidQuestionEnding
        );
        assert_eq!(contract.closing_preference, ClosingPreference::Declarative);
        assert_eq!(contract.recent_assistant_question_endings, 1);
    }

    #[test]
    fn ordinary_questions_and_requests_do_not_request_questioning() {
        for utterance in ["どうすればいい？", "相談したい", "help me decide"] {
            let contract = DialogueClassifier.classify(utterance, &[]);

            assert_eq!(
                contract.turn_kind,
                DialogueTurnKind::AnswerOrRequest,
                "{utterance}"
            );
            assert_eq!(
                contract.question_policy,
                QuestionPolicy::ClarificationOnlyIfMateriallyNecessary,
                "{utterance}"
            );
            assert_eq!(
                contract.closing_preference,
                ClosingPreference::Declarative,
                "{utterance}"
            );
        }
    }

    #[test]
    fn negated_or_referenced_questioning_does_not_request_questions() {
        for utterance in [
            "Please do not ask me questions.",
            "I dislike it when people ask me questions.",
            "We talked about why you ask me questions.",
            "Ask me questions is a phrase we discussed.",
            "\"Ask me questions\" is a phrase we discussed.",
        ] {
            let contract = DialogueClassifier.classify(utterance, &[]);

            assert_eq!(
                contract.turn_kind,
                DialogueTurnKind::AnswerOrRequest,
                "{utterance}"
            );
            assert_eq!(
                contract.question_policy,
                QuestionPolicy::ClarificationOnlyIfMateriallyNecessary,
                "{utterance}"
            );
            assert_eq!(
                contract.closing_preference,
                ClosingPreference::Declarative,
                "{utterance}"
            );
        }
    }

    #[test]
    fn ordinary_command_with_an_appraisal_word_remains_answer_or_request() {
        let contract = DialogueClassifier.classify("recommend nice restaurants", &[]);

        assert_eq!(contract.turn_kind, DialogueTurnKind::AnswerOrRequest);
        assert_eq!(
            contract.question_policy,
            QuestionPolicy::ClarificationOnlyIfMateriallyNecessary
        );
        assert_eq!(contract.closing_preference, ClosingPreference::Declarative);
    }

    #[test]
    fn compliment_with_show_as_a_noun_remains_a_compliment() {
        let contract = DialogueClassifier.classify("Great show today!", &[]);

        assert_eq!(contract.turn_kind, DialogueTurnKind::Compliment);
        assert_eq!(
            contract.question_policy,
            QuestionPolicy::AvoidQuestionEnding
        );
        assert_eq!(contract.closing_preference, ClosingPreference::Declarative);
    }

    #[test]
    fn japanese_explicit_questioning_remains_requested() {
        let contract = DialogueClassifier.classify("質問を一つずつして", &[]);

        assert_eq!(contract.turn_kind, DialogueTurnKind::RequestedQuestioning);
        assert_eq!(contract.question_policy, QuestionPolicy::QuestionRequested);
        assert_eq!(
            contract.closing_preference,
            ClosingPreference::QuestionPermitted
        );
    }

    #[test]
    fn japanese_requested_questioning_with_follow_on_goal_is_requested() {
        let contract = DialogueClassifier.classify("質問を一つずつして、計画を整理して", &[]);

        assert_eq!(contract.turn_kind, DialogueTurnKind::RequestedQuestioning);
        assert_eq!(contract.question_policy, QuestionPolicy::QuestionRequested);
        assert_eq!(
            contract.closing_preference,
            ClosingPreference::QuestionPermitted
        );
    }

    #[test]
    fn japanese_negated_or_referenced_follow_on_questioning_stays_ordinary() {
        for utterance in [
            "質問を一つずつしてほしくない",
            "質問を一つずつして、計画を整理してほしくない",
            "「質問を一つずつして、計画を整理して」は依頼文の例です",
            "質問を一つずつして、計画を整理してという例を説明して",
        ] {
            let contract = DialogueClassifier.classify(utterance, &[]);

            assert_eq!(
                contract.turn_kind,
                DialogueTurnKind::AnswerOrRequest,
                "{utterance}"
            );
            assert_eq!(
                contract.question_policy,
                QuestionPolicy::ClarificationOnlyIfMateriallyNecessary,
                "{utterance}"
            );
            assert_eq!(
                contract.closing_preference,
                ClosingPreference::Declarative,
                "{utterance}"
            );
        }
    }

    #[test]
    fn questioning_commands_override_appraisal_and_observation_subject_words() {
        for utterance in [
            "ask me questions to discuss the weather",
            "ask me questions to discuss great ideas",
        ] {
            let contract = DialogueClassifier.classify(utterance, &[]);

            assert_eq!(
                contract.turn_kind,
                DialogueTurnKind::RequestedQuestioning,
                "{utterance}"
            );
            assert_eq!(
                contract.question_policy,
                QuestionPolicy::QuestionRequested,
                "{utterance}"
            );
            assert_eq!(
                contract.closing_preference,
                ClosingPreference::QuestionPermitted,
                "{utterance}"
            );
        }
    }

    #[test]
    fn natural_japanese_polite_question_command_is_requested() {
        let contract = DialogueClassifier.classify("質問してください", &[]);

        assert_eq!(contract.turn_kind, DialogueTurnKind::RequestedQuestioning);
        assert_eq!(contract.question_policy, QuestionPolicy::QuestionRequested);
        assert_eq!(
            contract.closing_preference,
            ClosingPreference::QuestionPermitted
        );
    }

    #[test]
    fn japanese_polite_question_command_negation_quotes_and_references_stay_ordinary() {
        for utterance in [
            "質問しないでください",
            "質問してくださいとは言っていません",
            "「質問してください」は依頼文の例です",
            "質問してくださいという表現を説明して",
        ] {
            let contract = DialogueClassifier.classify(utterance, &[]);

            assert_eq!(
                contract.turn_kind,
                DialogueTurnKind::AnswerOrRequest,
                "{utterance}"
            );
            assert_eq!(
                contract.question_policy,
                QuestionPolicy::ClarificationOnlyIfMateriallyNecessary,
                "{utterance}"
            );
            assert_eq!(
                contract.closing_preference,
                ClosingPreference::Declarative,
                "{utterance}"
            );
        }
    }

    #[test]
    fn requests_with_compliment_or_observation_words_remain_answer_or_request() {
        for utterance in ["show weather forecast", "I need nice advice"] {
            let contract = DialogueClassifier.classify(utterance, &[]);

            assert_eq!(
                contract.turn_kind,
                DialogueTurnKind::AnswerOrRequest,
                "{utterance}"
            );
            assert_eq!(
                contract.question_policy,
                QuestionPolicy::ClarificationOnlyIfMateriallyNecessary,
                "{utterance}"
            );
            assert_eq!(
                contract.closing_preference,
                ClosingPreference::Declarative,
                "{utterance}"
            );
        }
    }

    #[test]
    fn assistant_question_ending_detection_handles_terminal_variants() {
        assert!(assistant_ends_with_question("ASCII?"));
        assert!(assistant_ends_with_question("全角？"));
        assert!(assistant_ends_with_question("quoted question?\")  "));
        assert!(!assistant_ends_with_question("not a question."));
    }

    #[test]
    fn assistant_question_ending_detects_japanese_period_interrogatives() {
        for question in [
            "リストアップしておくのはいかがでしょうか。",
            "こちらでよろしいですか。",
            "明日確認しますか。",
            "少し休みませんか。",
        ] {
            assert!(assistant_ends_with_question(question), "{question}");
        }
        for declarative in ["傘を持っていきましょう。", "またいつか。"] {
            assert!(!assistant_ends_with_question(declarative), "{declarative}");
        }
    }

    #[test]
    fn assistant_question_ending_counts_mashouka_and_japanese_quote_closer_for_cadence() {
        for assistant_question in ["ほかにお手伝いしましょうか。", "次は何を確認しますか？」"]
        {
            assert!(
                assistant_ends_with_question(assistant_question),
                "{assistant_question}"
            );
            let contract = DialogueClassifier.classify(
                "今日は雨です",
                &[ChatMessage::new(ChatRole::Assistant, assistant_question)],
            );
            assert_eq!(contract.turn_kind, DialogueTurnKind::CasualObservation);
            assert_eq!(
                contract.question_policy,
                QuestionPolicy::AvoidQuestionEnding,
                "{assistant_question}"
            );
            assert_eq!(
                contract.closing_preference,
                ClosingPreference::Declarative,
                "{assistant_question}"
            );
            assert_eq!(contract.recent_assistant_question_endings, 1);
        }

        for declarative in ["またいつか。", "いきましょう。"] {
            assert!(!assistant_ends_with_question(declarative), "{declarative}");
        }
    }

    #[test]
    fn router_only_plans_explicit_intents() {
        let router = IntentRouter;
        assert_eq!(router.classify("こんにちは"), TurnKind::Simple);
        assert_eq!(router.classify("これを覚えて"), TurnKind::Memory);
        assert_eq!(
            router.classify("候補を比較して決めたい"),
            TurnKind::DecisionSupport
        );
        assert_eq!(
            router.classify("前の内容は違う、訂正する"),
            TurnKind::Correction
        );
    }

    #[test]
    fn response_plan_and_surface_context_reject_unbounded_values() {
        let plan = ResponsePlan {
            kind: TurnKind::Memory,
            goal: "x".repeat(MAX_PLAN_GOAL_CHARS + 1),
            retrieval_query: None,
            directives: Vec::new(),
        };
        assert!(plan.validate().is_err());
        let surface = SurfaceContext {
            response_mode: "memory".into(),
            goal: None,
            tone_hint: None,
            relevant_facts: vec!["x".repeat(MAX_SURFACE_FACT_CHARS + 1)],
        };
        assert!(surface.validate().is_err());
    }

    #[test]
    fn lexical_planner_keeps_tool_and_proactive_turns_concrete() {
        let mut planner = LexicalResponsePlanner;
        let context = MemoryContext::default();
        let tool = planner
            .plan(TurnKind::Tool, "search this", &context)
            .expect("tool plan");
        let proactive = planner
            .plan(TurnKind::Proactive, "check in", &context)
            .expect("proactive plan");
        let tool_text = std::iter::once(tool.goal)
            .chain(tool.directives)
            .collect::<Vec<_>>()
            .join(" ");
        let proactive_text = std::iter::once(proactive.goal)
            .chain(proactive.directives)
            .collect::<Vec<_>>()
            .join(" ");

        assert!(tool_text.contains("current request"));
        assert!(!tool_text.contains("available tool-related next steps"));
        assert!(proactive_text.contains("concrete observed context"));
        assert!(proactive_text.contains("self-contained"));
        assert!(!proactive_text.contains("Offer a low-pressure"));
    }

    struct CountingPlanner(usize);
    impl ResponsePlanner for CountingPlanner {
        fn plan(
            &mut self,
            kind: TurnKind,
            _utterance: &str,
            _context: &MemoryContext,
        ) -> Result<ResponsePlan, PortError> {
            self.0 += 1;
            Ok(ResponsePlan {
                kind,
                goal: "goal".into(),
                retrieval_query: None,
                directives: Vec::new(),
            })
        }
    }
    #[derive(Default)]
    struct PassThrough;
    impl ResponseContextRetriever for PassThrough {
        fn retrieve(
            &mut self,
            _plan: &ResponsePlan,
            context: &MemoryContext,
        ) -> Result<MemoryContext, PortError> {
            Ok(context.clone())
        }
    }
    impl SurfaceRealizer for PassThrough {
        fn realize(&mut self, _plan: &ResponsePlan) -> Result<SurfaceContext, PortError> {
            Ok(SurfaceContext {
                response_mode: "planned".into(),
                goal: None,
                tone_hint: None,
                relevant_facts: Vec::new(),
            })
        }
    }

    struct PanicRetriever;
    impl ResponseContextRetriever for PanicRetriever {
        fn retrieve(
            &mut self,
            _plan: &ResponsePlan,
            _context: &MemoryContext,
        ) -> Result<MemoryContext, PortError> {
            panic!("simple turns must not retrieve context")
        }
    }
    struct PanicRealizer;
    impl SurfaceRealizer for PanicRealizer {
        fn realize(&mut self, _plan: &ResponsePlan) -> Result<SurfaceContext, PortError> {
            panic!("simple turns must not realize a surface")
        }
    }

    #[test]
    fn simple_turn_skips_every_planning_port() {
        let mut pipeline = ResponsePipeline::new(
            CountingPlanner(0),
            PanicRetriever,
            PanicRealizer,
            PlanningBudget::default(),
        );
        assert!(
            pipeline
                .try_prepare(TurnKind::Simple, "hello", &MemoryContext::default())
                .is_none()
        );
        assert_eq!(pipeline.planner.as_ref().unwrap().0, 0);
    }

    struct FixedState;
    impl PlannedStateContextProvider for FixedState {
        fn retrieve_state(
            &mut self,
            _plan: &ResponsePlan,
        ) -> Result<BoundedStateContext, PortError> {
            Ok(BoundedStateContext {
                mood: Some("positive".into()),
                reaction: None,
                relationship_score: Some(4),
                reflection_cursor: None,
                open_commitments: vec!["資料を確認する".into()],
            })
        }
    }

    #[test]
    fn state_context_is_retrieved_only_for_planned_turns() {
        let mut pipeline = ResponsePipeline::new(
            CountingPlanner(0),
            StateAwareRetriever::new(PassThrough, FixedState),
            PassThrough,
            PlanningBudget::default(),
        );
        assert!(
            pipeline
                .try_prepare(TurnKind::Simple, "hello", &MemoryContext::default())
                .is_none()
        );
        let prepared = pipeline
            .try_prepare(TurnKind::Memory, "remember", &MemoryContext::default())
            .expect("planned state context");
        assert!(
            prepared
                .context
                .memories
                .iter()
                .any(|fact| fact.contains("commitment.open"))
        );
    }

    struct DeterministicTimeoutPlanner {
        started: std::sync::mpsc::SyncSender<()>,
        release: std::sync::mpsc::Receiver<()>,
        finished: std::sync::mpsc::SyncSender<()>,
    }
    impl ResponsePlanner for DeterministicTimeoutPlanner {
        fn plan(
            &mut self,
            kind: TurnKind,
            _utterance: &str,
            _context: &MemoryContext,
        ) -> Result<ResponsePlan, PortError> {
            self.started
                .send(())
                .map_err(|_| PortError("test did not await planner start".into()))?;
            self.release
                .recv()
                .map_err(|_| PortError("test did not release planner".into()))?;
            let plan = ResponsePlan {
                kind,
                goal: "goal".into(),
                retrieval_query: None,
                directives: Vec::new(),
            };
            self.finished
                .send(())
                .map_err(|_| PortError("test did not await planner finish".into()))?;
            Ok(plan)
        }
    }

    #[test]
    fn elapsed_planning_budget_falls_back() {
        let (started_sender, started_receiver) = std::sync::mpsc::sync_channel(1);
        let (release_sender, release_receiver) = std::sync::mpsc::sync_channel(1);
        let (finished_sender, finished_receiver) = std::sync::mpsc::sync_channel(1);
        let mut pipeline = ResponsePipeline::new(
            DeterministicTimeoutPlanner {
                started: started_sender,
                release: release_receiver,
                finished: finished_sender,
            },
            PassThrough,
            PassThrough,
            PlanningBudget {
                max_elapsed: Duration::from_millis(1),
            },
        );
        assert!(
            pipeline
                .try_prepare(TurnKind::Memory, "remember", &MemoryContext::default())
                .is_none()
        );
        started_receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("planner must start before it is released");
        release_sender.send(()).expect("planner must be releasable");
        finished_receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("planner must finish after release");
        for _ in 0..1000 {
            pipeline.reclaim_pending();
            if pipeline.pending.is_none() {
                break;
            }
            thread::yield_now();
        }
        assert!(
            pipeline.pending.is_none(),
            "timed-out worker must terminate"
        );
    }

    struct FirstTurnBlockingPlanner {
        calls: usize,
        started: std::sync::mpsc::SyncSender<()>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl ResponsePlanner for FirstTurnBlockingPlanner {
        fn plan(
            &mut self,
            kind: TurnKind,
            _utterance: &str,
            _context: &MemoryContext,
        ) -> Result<ResponsePlan, PortError> {
            self.calls += 1;
            if self.calls == 1 {
                self.started
                    .send(())
                    .map_err(|_| PortError("test did not await planner start".into()))?;
                self.release
                    .recv()
                    .map_err(|_| PortError("test did not release planner".into()))?;
            }
            Ok(ResponsePlan {
                kind,
                goal: "goal".into(),
                retrieval_query: None,
                directives: Vec::new(),
            })
        }
    }

    struct CountingRetriever(usize);
    impl ResponseContextRetriever for CountingRetriever {
        fn retrieve(
            &mut self,
            _plan: &ResponsePlan,
            context: &MemoryContext,
        ) -> Result<MemoryContext, PortError> {
            self.0 += 1;
            Ok(context.clone())
        }
    }

    struct CountingRealizer(usize);
    impl SurfaceRealizer for CountingRealizer {
        fn realize(&mut self, _plan: &ResponsePlan) -> Result<SurfaceContext, PortError> {
            self.0 += 1;
            Ok(SurfaceContext {
                response_mode: "recovered-planned".into(),
                goal: None,
                tone_hint: None,
                relevant_facts: Vec::new(),
            })
        }
    }

    #[test]
    fn timeout_reclaims_ports_for_a_later_planned_turn() {
        let (started_sender, started_receiver) = std::sync::mpsc::sync_channel(1);
        let (release_sender, release_receiver) = std::sync::mpsc::sync_channel(1);
        let mut pipeline = ResponsePipeline::new(
            FirstTurnBlockingPlanner {
                calls: 0,
                started: started_sender,
                release: release_receiver,
            },
            CountingRetriever(0),
            CountingRealizer(0),
            PlanningBudget {
                max_elapsed: Duration::from_millis(5),
            },
        );

        assert!(
            pipeline
                .try_prepare(TurnKind::Memory, "first", &MemoryContext::default())
                .is_none()
        );
        started_receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("first planner must still be pending");
        assert!(
            pipeline
                .try_prepare(TurnKind::Memory, "while-pending", &MemoryContext::default())
                .is_none(),
            "a still-running worker must not drop its returned ports"
        );
        release_sender
            .send(())
            .expect("pending planner must be releasable");
        thread::sleep(Duration::from_millis(10));

        let prepared = pipeline
            .try_prepare(TurnKind::Memory, "recovered", &MemoryContext::default())
            .expect("the returned ports must prepare the next planned turn");
        assert_eq!(prepared.surface.response_mode, "recovered-planned");
        assert_eq!(
            pipeline.planner.as_ref().expect("planner restored").calls,
            2
        );
        assert_eq!(
            pipeline.retriever.as_ref().expect("retriever restored").0,
            2
        );
        assert_eq!(pipeline.realizer.as_ref().expect("realizer restored").0, 2);
    }

    struct MalformedPlanner;
    impl ResponsePlanner for MalformedPlanner {
        fn plan(
            &mut self,
            _kind: TurnKind,
            _utterance: &str,
            _context: &MemoryContext,
        ) -> Result<ResponsePlan, PortError> {
            Ok(ResponsePlan {
                kind: TurnKind::Simple,
                goal: "not allowed".into(),
                retrieval_query: None,
                directives: Vec::new(),
            })
        }
    }

    #[test]
    fn malformed_plan_falls_back_before_retrieval() {
        let mut pipeline = ResponsePipeline::new(
            MalformedPlanner,
            PassThrough,
            PassThrough,
            PlanningBudget::default(),
        );
        assert!(
            pipeline
                .try_prepare(TurnKind::Memory, "remember", &MemoryContext::default())
                .is_none()
        );
    }

    struct MismatchedKindPlanner(TurnKind);
    impl ResponsePlanner for MismatchedKindPlanner {
        fn plan(
            &mut self,
            _kind: TurnKind,
            _utterance: &str,
            _context: &MemoryContext,
        ) -> Result<ResponsePlan, PortError> {
            Ok(ResponsePlan {
                kind: self.0,
                goal: "wrong route".into(),
                retrieval_query: None,
                directives: Vec::new(),
            })
        }
    }

    #[test]
    fn every_mismatched_planned_kind_falls_back_before_retrieval() {
        for (requested, returned) in [
            (TurnKind::Memory, TurnKind::Commitment),
            (TurnKind::Commitment, TurnKind::Correction),
            (TurnKind::Correction, TurnKind::DecisionSupport),
            (TurnKind::DecisionSupport, TurnKind::Tool),
            (TurnKind::Tool, TurnKind::Proactive),
            (TurnKind::Proactive, TurnKind::Memory),
        ] {
            let mut pipeline = ResponsePipeline::new(
                MismatchedKindPlanner(returned),
                CountingRetriever(0),
                CountingRealizer(0),
                PlanningBudget::default(),
            );
            assert!(
                pipeline
                    .try_prepare(requested, "planned", &MemoryContext::default())
                    .is_none(),
                "{requested:?} must reject a {returned:?} plan"
            );
            assert_eq!(
                pipeline.retriever.as_ref().expect("retriever restored").0,
                0,
                "{requested:?} must reject a {returned:?} plan before retrieval"
            );
            assert_eq!(
                pipeline.realizer.as_ref().expect("realizer restored").0,
                0,
                "{requested:?} must reject a {returned:?} plan before realization"
            );
        }
    }
}
