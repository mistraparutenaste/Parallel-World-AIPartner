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
    pub tone_hint: Option<String>,
    pub relevant_facts: Vec<String>,
}

impl SurfaceContext {
    /// # Errors
    /// Returns an error for unbounded or blank surface data.
    pub fn validate(&self) -> Result<(), PortError> {
        if !is_bounded_text(&self.response_mode, MAX_SURFACE_HINT_CHARS)
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

/// A failure-isolated planned preparation pipeline.
pub struct ResponsePipeline<P, R, S> {
    planner: Option<P>,
    retriever: Option<R>,
    realizer: Option<S>,
    pending: Option<Receiver<(P, R, S, Option<PreparedResponse>)>>,
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
                "Explain available tool-related next steps",
                "Do not claim a tool was used unless its result is present",
            ),
            TurnKind::Proactive => (
                "Offer a low-pressure relevant check-in",
                "Keep the check-in optional and concise",
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
            tone_hint: Some(plan.goal.clone()),
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

    use super::*;

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
            tone_hint: None,
            relevant_facts: vec!["x".repeat(MAX_SURFACE_FACT_CHARS + 1)],
        };
        assert!(surface.validate().is_err());
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

    struct SlowPlanner;
    impl ResponsePlanner for SlowPlanner {
        fn plan(
            &mut self,
            kind: TurnKind,
            _utterance: &str,
            _context: &MemoryContext,
        ) -> Result<ResponsePlan, PortError> {
            thread::sleep(Duration::from_millis(10));
            Ok(ResponsePlan {
                kind,
                goal: "goal".into(),
                retrieval_query: None,
                directives: Vec::new(),
            })
        }
    }

    #[test]
    fn elapsed_planning_budget_falls_back() {
        let mut pipeline = ResponsePipeline::new(
            SlowPlanner,
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
