use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use pw_application::PortError;
use pw_application::conversation::{
    ChatMessage, ConversationEvents, ConversationOrchestrator, ExistingContextRetriever,
    FixedSurfaceRealizer, LexicalResponsePlanner, LlmClient, OrchestratorConfig, PlanningBudget,
    PromptBuilder, ResponsePlan, ResponsePlanner, TurnKind, response_pipeline,
};
use pw_application::memory::MemoryContext;
use pw_domain::conversation::ConversationState;
use pw_domain::reply::{ReplyControl, TurnId};

#[derive(Default, Clone)]
struct Events {
    replies: Arc<Mutex<Vec<String>>>,
}

impl ConversationEvents for Events {
    fn on_state(&self, _: ConversationState) {}
    fn on_user_message(&self, _: TurnId, _: &str) {}
    fn on_control(&self, _: TurnId, _: &ReplyControl) {}
    fn on_sentence(&self, _: TurnId, _: &str) {}
    fn on_reply_complete(&self, _: TurnId, text: &str) {
        self.replies.lock().unwrap().push(text.to_owned());
    }
    fn on_cancelled(&self, _: TurnId) {}
    fn on_error(&self, _: TurnId, _: &str) {}
}

struct SingleStreamLlm {
    calls: Arc<AtomicUsize>,
}

impl LlmClient for SingleStreamLlm {
    fn stream_chat(
        &mut self,
        _: &[ChatMessage],
        _: &AtomicBool,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<(), PortError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        on_delta("one streamed reply。");
        Ok(())
    }
}

fn config() -> OrchestratorConfig {
    OrchestratorConfig {
        prompt: PromptBuilder {
            system_rules: "rules".into(),
            character_prompt: "character".into(),
        },
        max_history_messages: 8,
        strip_emoji: false,
    }
}

#[test]
fn ordinary_turn_uses_one_stream_and_completes_without_planning() {
    let calls = Arc::new(AtomicUsize::new(0));
    let events = Events::default();
    let mut orchestrator = ConversationOrchestrator::new(
        config(),
        SingleStreamLlm {
            calls: Arc::clone(&calls),
        },
        events.clone(),
        Arc::new(AtomicBool::new(false)),
    );

    orchestrator.submit_user_text("hello");

    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        events.replies.lock().unwrap().as_slice(),
        ["one streamed reply。"]
    );
}

struct SlowPlanner;

impl ResponsePlanner for SlowPlanner {
    fn plan(
        &mut self,
        kind: TurnKind,
        _: &str,
        _: &MemoryContext,
    ) -> Result<ResponsePlan, PortError> {
        thread::sleep(Duration::from_millis(20));
        Ok(ResponsePlan {
            kind,
            goal: "bounded goal".into(),
            retrieval_query: None,
            directives: vec!["bounded directive".into()],
        })
    }
}

#[test]
fn planned_timeout_falls_back_to_the_same_single_stream() {
    let calls = Arc::new(AtomicUsize::new(0));
    let events = Events::default();
    let pipeline = response_pipeline(
        SlowPlanner,
        ExistingContextRetriever,
        FixedSurfaceRealizer,
        PlanningBudget {
            max_elapsed: Duration::from_millis(1),
        },
    );
    let mut orchestrator = ConversationOrchestrator::new_with_response_pipeline(
        config(),
        SingleStreamLlm {
            calls: Arc::clone(&calls),
        },
        events.clone(),
        Arc::new(AtomicBool::new(false)),
        pipeline,
    );

    orchestrator.submit_user_text("remember this");

    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        events.replies.lock().unwrap().as_slice(),
        ["one streamed reply。"]
    );
}

#[test]
fn default_planner_remains_local_and_bounded() {
    let mut planner = LexicalResponsePlanner;
    let plan = planner
        .plan(TurnKind::Memory, "remember", &MemoryContext::default())
        .expect("memory plan");
    assert!(plan.validate().is_ok());
}
