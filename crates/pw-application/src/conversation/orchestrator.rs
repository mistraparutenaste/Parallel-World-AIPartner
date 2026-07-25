//! Sole owner of the conversation state machine (基本設計 5章).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pw_domain::conversation::ConversationState;
use pw_domain::reply::{
    ReplyEvent, ReplyParser, SentenceSplitter, TurnId, TurnTracker, strip_emoji,
};

use super::ports::{ChatMessage, ChatRole, ConversationEvents, LlmClient};
use super::prompt::PromptBuilder;
use super::routing::{
    ConfiguredResponsePipeline, DialogueClassifier, IntentRouter, default_response_pipeline,
};
use crate::memory::MemoryContext;

/// Tuning for [`ConversationOrchestrator`].
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub prompt: PromptBuilder,
    /// Maximum number of past messages kept for context.
    pub max_history_messages: usize,
    /// Remove emoji from spoken text (display and TTS safety).
    pub strip_emoji: bool,
}

/// Runs conversation turns synchronously; the Tauri layer provides
/// the worker thread and the cancel handle. Turns are serialized, so
/// a stale stream can never interleave with a newer turn; cancelled
/// output is additionally suppressed via the cancel flag and turn id
/// tagging on every event.
pub struct ConversationOrchestrator<L, E> {
    config: OrchestratorConfig,
    llm: L,
    events: E,
    cancel: Arc<AtomicBool>,
    tracker: TurnTracker,
    state: ConversationState,
    history: VecDeque<ChatMessage>,
    router: IntentRouter,
    dialogue_classifier: DialogueClassifier,
    response_pipeline: ConfiguredResponsePipeline,
}

impl<L, E> ConversationOrchestrator<L, E>
where
    L: LlmClient,
    E: ConversationEvents,
{
    pub fn new(config: OrchestratorConfig, llm: L, events: E, cancel: Arc<AtomicBool>) -> Self {
        Self::new_with_history(config, llm, events, cancel, Vec::new())
    }

    /// Builds an orchestrator with explicitly configured planned-turn ports.
    /// Ordinary turns still bypass these ports and use one streamed LLM call.
    pub fn new_with_response_pipeline(
        config: OrchestratorConfig,
        llm: L,
        events: E,
        cancel: Arc<AtomicBool>,
        response_pipeline: ConfiguredResponsePipeline,
    ) -> Self {
        Self::new_with_history_after_and_response_pipeline(
            config,
            llm,
            events,
            cancel,
            Vec::new(),
            0,
            response_pipeline,
        )
    }

    /// Builds an orchestrator with previously confirmed messages as prompt context.
    pub fn new_with_history(
        config: OrchestratorConfig,
        llm: L,
        events: E,
        cancel: Arc<AtomicBool>,
        history: Vec<ChatMessage>,
    ) -> Self {
        let mut history: VecDeque<_> = history.into();
        while history.len() > config.max_history_messages {
            history.pop_front();
        }
        Self::new_with_history_after(config, llm, events, cancel, history.into(), 0)
    }

    /// Builds with restored prompt history and the largest persisted turn id.
    pub fn new_with_history_after(
        config: OrchestratorConfig,
        llm: L,
        events: E,
        cancel: Arc<AtomicBool>,
        history: Vec<ChatMessage>,
        last_turn_id: u64,
    ) -> Self {
        Self::new_with_history_after_and_response_pipeline(
            config,
            llm,
            events,
            cancel,
            history,
            last_turn_id,
            default_response_pipeline(),
        )
    }

    /// Restores prompt history and injects planned-turn ports.  This keeps
    /// the legacy constructors source-compatible while allowing adapters to
    /// provide bounded retrieval or surface realization.
    pub fn new_with_history_after_and_response_pipeline(
        config: OrchestratorConfig,
        llm: L,
        events: E,
        cancel: Arc<AtomicBool>,
        history: Vec<ChatMessage>,
        last_turn_id: u64,
        response_pipeline: ConfiguredResponsePipeline,
    ) -> Self {
        let mut history: VecDeque<_> = history.into();
        while history.len() > config.max_history_messages {
            history.pop_front();
        }
        let orchestrator = Self {
            config,
            llm,
            events,
            cancel,
            tracker: TurnTracker::after(last_turn_id),
            state: ConversationState::Idle,
            history,
            router: IntentRouter,
            dialogue_classifier: DialogueClassifier,
            response_pipeline,
        };
        orchestrator.events.on_state(orchestrator.state);
        orchestrator
    }

    #[must_use]
    pub fn state(&self) -> ConversationState {
        self.state
    }

    /// Runs one full turn for the given user utterance.
    pub fn submit_user_text(&mut self, text: &str) -> TurnId {
        let turn = self.tracker.begin_turn();
        self.submit_user_text_for_turn(text, turn, &MemoryContext::default())
    }

    /// Runs a turn using an id already reserved by durable storage.
    pub fn submit_user_text_with_id(&mut self, text: &str, turn_id: u64) -> TurnId {
        let turn = self.tracker.begin_reserved(turn_id);
        self.submit_user_text_for_turn(text, turn, &MemoryContext::default())
    }

    pub fn submit_user_text_with_context(
        &mut self,
        text: &str,
        turn_id: u64,
        context: &MemoryContext,
    ) -> TurnId {
        let turn = self.tracker.begin_reserved(turn_id);
        self.submit_user_text_for_turn(text, turn, context)
    }

    fn submit_user_text_for_turn(
        &mut self,
        text: &str,
        turn: TurnId,
        context: &MemoryContext,
    ) -> TurnId {
        self.cancel.store(false, Ordering::Relaxed);
        self.events.on_user_message(turn, text);
        self.set_state(ConversationState::Thinking);

        let history: Vec<ChatMessage> = self.history.iter().cloned().collect();
        let kind = self.router.classify(text);
        let turn_style = self.dialogue_classifier.classify(text, &history);
        let prepared = self.response_pipeline.try_prepare(kind, text, context);
        let messages = if let Some(prepared) = prepared {
            self.config
                .prompt
                .build_with_context_surface_and_turn_style(
                    &history,
                    text,
                    &prepared.context,
                    &prepared.surface,
                    &turn_style,
                )
        } else {
            self.config.prompt.build_with_context_and_turn_style(
                &history,
                text,
                context,
                &turn_style,
            )
        };

        let mut parser = ReplyParser::new();
        let mut splitter = SentenceSplitter::new();
        let mut speech_text = String::new();
        let mut spoke = false;

        let cancel = Arc::clone(&self.cancel);
        let events = &self.events;
        let state_cell = &mut self.state;
        let strip = self.config.strip_emoji;
        let result = {
            let mut on_delta = |delta: &str| {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                for event in parser.push(delta) {
                    match event {
                        ReplyEvent::Control(control) => {
                            events.on_control(turn, &control);
                        }
                        ReplyEvent::Speech(chunk) => {
                            let chunk = if strip { strip_emoji(&chunk) } else { chunk };
                            speech_text.push_str(&chunk);
                            for sentence in splitter.push(&chunk) {
                                if cancel.load(Ordering::Relaxed) {
                                    break;
                                }
                                if !spoke {
                                    spoke = true;
                                    *state_cell = ConversationState::Speaking;
                                    events.on_state(ConversationState::Speaking);
                                }
                                events.on_sentence(turn, &sentence);
                            }
                        }
                    }
                }
            };
            self.llm.stream_chat(&messages, &cancel, &mut on_delta)
        };

        if self.cancel.load(Ordering::Relaxed) {
            return self.finish_cancelled(turn, text, &speech_text);
        }

        match result {
            Ok(()) => {
                let mut trailing = Vec::new();
                for event in parser.finish() {
                    if let ReplyEvent::Speech(chunk) = event {
                        let chunk = if self.config.strip_emoji {
                            strip_emoji(&chunk)
                        } else {
                            chunk
                        };
                        speech_text.push_str(&chunk);
                        trailing.extend(splitter.push(&chunk));
                    }
                }
                trailing.extend(splitter.finish());
                for sentence in trailing {
                    if self.cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    self.events.on_sentence(turn, &sentence);
                }
                if self.cancel.load(Ordering::Relaxed) {
                    return self.finish_cancelled(turn, text, &speech_text);
                }
                self.events.on_reply_complete(turn, speech_text.trim());
                self.record_turn(text, &speech_text);
                self.set_state(ConversationState::Idle);
            }
            Err(error) => {
                self.events.on_error(turn, &error.to_string());
                // Keep history and settings so the conversation can
                // resume after reconnection (設計spec 8章).
                self.record_turn(text, "");
                self.set_state(ConversationState::LlmUnavailable);
            }
        }
        turn
    }

    fn finish_cancelled(&mut self, turn: TurnId, user_text: &str, partial: &str) -> TurnId {
        self.tracker.invalidate();
        self.events.on_cancelled(turn);
        self.set_state(ConversationState::Cancelled);
        let _ = partial;
        self.record_turn(user_text, "");
        self.set_state(ConversationState::Idle);
        turn
    }

    /// Returns to Idle after a degraded state (reconnection).
    pub fn recover(&mut self) {
        if self.state == ConversationState::LlmUnavailable {
            self.set_state(ConversationState::Idle);
        }
    }

    fn record_turn(&mut self, user_text: &str, assistant_speech: &str) {
        self.history
            .push_back(ChatMessage::new(ChatRole::User, user_text));
        if !assistant_speech.trim().is_empty() {
            self.history.push_back(ChatMessage::new(
                ChatRole::Assistant,
                assistant_speech.trim(),
            ));
        }
        while self.history.len() > self.config.max_history_messages {
            self.history.pop_front();
        }
    }

    fn set_state(&mut self, state: ConversationState) {
        if self.state != state {
            self.state = state;
            self.events.on_state(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use pw_domain::conversation::ConversationState;
    use pw_domain::reply::{ReplyControl, TurnId};

    use super::super::ports::{ChatMessage, ChatRole, ConversationEvents, LlmClient};
    use super::super::prompt::PromptBuilder;
    use super::super::routing::{
        ExistingContextRetriever, FixedSurfaceRealizer, PlanningBudget, ResponsePlan,
        ResponsePlanner, TurnKind, response_pipeline,
    };
    use super::{ConversationOrchestrator, OrchestratorConfig};
    use crate::PortError;
    use crate::memory::MemoryContext;

    const EXPECTED_CONVERSATIONAL_STYLE_POLICY: &str = "自然な話し言葉で、短く一度に一つの話題に答える。フィラーや相づちは必要な場合のみ使う。フィラーは控えめに使い、短い返答では一つまでとし、毎回同じ表現を繰り返さない。説明の羅列、箇条書き、メタ発言、定型的な書き出し、頼まれていない話題の提案、サービスメニューのような言い回しを避ける。必要なときだけ自然な確認質問を一つ添え、習慣的な締めの質問や「今日は何をしますか」のような定型質問を繰り返さない。不明な事実は推測と明示し、約束・実行・感情を偽らない。";

    /// Emits scripted chunks, honouring the cancel flag.
    struct ScriptedLlm {
        chunks: Vec<&'static str>,
        received_prompts: Arc<Mutex<Vec<Vec<ChatMessage>>>>,
        fail: bool,
    }

    impl LlmClient for ScriptedLlm {
        fn stream_chat(
            &mut self,
            messages: &[ChatMessage],
            cancel: &AtomicBool,
            on_delta: &mut dyn FnMut(&str),
        ) -> Result<(), PortError> {
            self.received_prompts
                .lock()
                .unwrap()
                .push(messages.to_vec());
            if self.fail {
                return Err(PortError("connection refused".into()));
            }
            for chunk in &self.chunks {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
                on_delta(chunk);
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct Recording {
        states: Mutex<Vec<ConversationState>>,
        sentences: Mutex<Vec<(TurnId, String)>>,
        controls: Mutex<Vec<ReplyControl>>,
        completions: Mutex<Vec<String>>,
        cancellations: Mutex<Vec<TurnId>>,
        errors: Mutex<Vec<String>>,
        /// When set, cancel is triggered after this many sentences.
        cancel_after_sentences: Option<(usize, Arc<AtomicBool>)>,
    }

    struct Events(Arc<Recording>);

    impl ConversationEvents for Events {
        fn on_state(&self, state: ConversationState) {
            self.0.states.lock().unwrap().push(state);
        }
        fn on_user_message(&self, _turn: TurnId, _text: &str) {}
        fn on_control(&self, _turn: TurnId, control: &ReplyControl) {
            self.0.controls.lock().unwrap().push(control.clone());
        }
        fn on_sentence(&self, turn: TurnId, sentence: &str) {
            let mut sentences = self.0.sentences.lock().unwrap();
            sentences.push((turn, sentence.to_owned()));
            if let Some((count, cancel)) = &self.0.cancel_after_sentences
                && sentences.len() >= *count
            {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        fn on_reply_complete(&self, _turn: TurnId, speech_text: &str) {
            self.0.completions.lock().unwrap().push(speech_text.into());
        }
        fn on_cancelled(&self, turn: TurnId) {
            self.0.cancellations.lock().unwrap().push(turn);
        }
        fn on_error(&self, _turn: TurnId, message: &str) {
            self.0.errors.lock().unwrap().push(message.to_owned());
        }
    }

    fn config() -> OrchestratorConfig {
        OrchestratorConfig {
            prompt: PromptBuilder {
                system_rules: "規則".into(),
                character_prompt: "キャラ".into(),
            },
            max_history_messages: 4,
            strip_emoji: true,
        }
    }

    #[test]
    fn a_turn_streams_control_sentences_and_completion() {
        let recording = Arc::new(Recording::default());
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let llm = ScriptedLlm {
            chunks: vec![
                "{\"emotion\":\"happy\",\"motion\":\"nod\"}\n\n",
                "おかえりなさい。",
                "今日は何を",
                "しますか？",
            ],
            received_prompts: Arc::clone(&prompts),
            fail: false,
        };
        let mut orchestrator =
            ConversationOrchestrator::new(config(), llm, Events(Arc::clone(&recording)), cancel);

        orchestrator.submit_user_text("ただいま");

        assert_eq!(
            recording.controls.lock().unwrap()[0].emotion.as_deref(),
            Some("happy")
        );
        let sentences: Vec<_> = recording
            .sentences
            .lock()
            .unwrap()
            .iter()
            .map(|(_, s)| s.clone())
            .collect();
        assert_eq!(sentences, ["おかえりなさい。", "今日は何をしますか？"]);
        assert_eq!(
            recording.completions.lock().unwrap()[0],
            "おかえりなさい。今日は何をしますか？"
        );
        assert_eq!(
            *recording.states.lock().unwrap(),
            [
                ConversationState::Idle,
                ConversationState::Thinking,
                ConversationState::Speaking,
                ConversationState::Idle,
            ]
        );
        // Prompt preserves its base order and adds one per-turn contract before the utterance.
        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0][0], ChatMessage::new(ChatRole::System, "規則"));
        assert_eq!(prompts[0][1], ChatMessage::new(ChatRole::System, "キャラ"));
        assert_eq!(
            prompts[0][2],
            ChatMessage::new(ChatRole::System, EXPECTED_CONVERSATIONAL_STYLE_POLICY)
        );
        assert_eq!(
            prompts[0]
                .iter()
                .filter(|message| message.content.starts_with("<turn_style_contract>\n"))
                .count(),
            1
        );
        assert_eq!(prompts[0].last().unwrap().content, "ただいま");
    }

    #[test]
    fn history_is_included_and_capped_on_later_turns() {
        let recording = Arc::new(Recording::default());
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let llm = ScriptedLlm {
            chunks: vec!["はい。"],
            received_prompts: Arc::clone(&prompts),
            fail: false,
        };
        let mut orchestrator = ConversationOrchestrator::new(
            config(),
            llm,
            Events(recording),
            Arc::new(AtomicBool::new(false)),
        );

        orchestrator.submit_user_text("一つ目");
        orchestrator.submit_user_text("二つ目");
        orchestrator.submit_user_text("三つ目");

        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 3);
        let third = &prompts[2];
        // max 4 history messages: (一つ目, はい。, 二つ目, はい。).
        for message in [
            ChatMessage::new(ChatRole::User, "一つ目"),
            ChatMessage::new(ChatRole::Assistant, "はい。"),
            ChatMessage::new(ChatRole::User, "二つ目"),
            ChatMessage::new(ChatRole::Assistant, "はい。"),
        ] {
            assert!(
                third.contains(&message),
                "missing history message: {message:?}"
            );
        }
        assert_eq!(third.last().unwrap().content, "三つ目");
    }

    #[test]
    fn seeded_history_survives_orchestrator_reconstruction() {
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let llm = ScriptedLlm {
            chunks: vec!["new reply"],
            received_prompts: Arc::clone(&prompts),
            fail: false,
        };
        let mut orchestrator = ConversationOrchestrator::new_with_history(
            config(),
            llm,
            Events(Arc::new(Recording::default())),
            Arc::new(AtomicBool::new(false)),
            vec![
                ChatMessage::new(super::super::ports::ChatRole::User, "saved user"),
                ChatMessage::new(super::super::ports::ChatRole::Assistant, "saved assistant"),
            ],
        );

        orchestrator.submit_user_text("new user");

        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0][0], ChatMessage::new(ChatRole::System, "規則"));
        assert_eq!(prompts[0][1], ChatMessage::new(ChatRole::System, "キャラ"));
        assert_eq!(
            prompts[0][2],
            ChatMessage::new(ChatRole::System, EXPECTED_CONVERSATIONAL_STYLE_POLICY)
        );
        assert!(prompts[0].contains(&ChatMessage::new(ChatRole::User, "saved user")));
        assert!(prompts[0].contains(&ChatMessage::new(ChatRole::Assistant, "saved assistant")));
        assert_eq!(prompts[0].last().unwrap().content, "new user");
    }

    #[test]
    fn restored_orchestrator_continues_after_persisted_turn_id() {
        let llm = ScriptedLlm {
            chunks: vec!["reply"],
            received_prompts: Arc::new(Mutex::new(Vec::new())),
            fail: false,
        };
        let mut orchestrator = ConversationOrchestrator::new_with_history_after(
            config(),
            llm,
            Events(Arc::new(Recording::default())),
            Arc::new(AtomicBool::new(false)),
            Vec::new(),
            41,
        );
        assert_eq!(orchestrator.submit_user_text("next").value(), 42);
    }

    #[test]
    fn cancelled_assistant_fragment_is_not_kept_in_later_prompts() {
        let cancel = Arc::new(AtomicBool::new(false));
        let recording = Arc::new(Recording {
            cancel_after_sentences: Some((1, Arc::clone(&cancel))),
            ..Recording::default()
        });
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let llm = ScriptedLlm {
            chunks: vec!["partial。", "discarded。"],
            received_prompts: Arc::clone(&prompts),
            fail: false,
        };
        let mut orchestrator =
            ConversationOrchestrator::new(config(), llm, Events(recording), Arc::clone(&cancel));

        orchestrator.submit_user_text("cancel me");
        cancel.store(false, Ordering::Relaxed);
        orchestrator.recover();
        orchestrator.submit_user_text("next");

        let second = &prompts.lock().unwrap()[1];
        assert!(second.iter().all(|message| message.content != "partial。"));
    }

    #[test]
    fn cancel_mid_stream_suppresses_remaining_output() {
        let cancel = Arc::new(AtomicBool::new(false));
        let recording = Arc::new(Recording {
            cancel_after_sentences: Some((1, Arc::clone(&cancel))),
            ..Recording::default()
        });
        let llm = ScriptedLlm {
            chunks: vec!["一文目。", "二文目。", "三文目。"],
            received_prompts: Arc::new(Mutex::new(Vec::new())),
            fail: false,
        };
        let mut orchestrator =
            ConversationOrchestrator::new(config(), llm, Events(Arc::clone(&recording)), cancel);

        let turn = orchestrator.submit_user_text("長い話をして");

        let sentences = recording.sentences.lock().unwrap();
        assert_eq!(sentences.len(), 1, "sentences: {sentences:?}");
        assert!(recording.completions.lock().unwrap().is_empty());
        assert_eq!(*recording.cancellations.lock().unwrap(), [turn]);
        let states = recording.states.lock().unwrap();
        assert!(states.contains(&ConversationState::Cancelled));
        assert_eq!(*states.last().unwrap(), ConversationState::Idle);
    }

    #[test]
    fn llm_failure_degrades_but_keeps_history() {
        let recording = Arc::new(Recording::default());
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let llm = ScriptedLlm {
            chunks: vec![],
            received_prompts: Arc::clone(&prompts),
            fail: true,
        };
        let mut orchestrator = ConversationOrchestrator::new(
            config(),
            llm,
            Events(Arc::clone(&recording)),
            Arc::new(AtomicBool::new(false)),
        );

        orchestrator.submit_user_text("聞こえてる？");

        assert_eq!(orchestrator.state(), ConversationState::LlmUnavailable);
        assert_eq!(recording.errors.lock().unwrap().len(), 1);

        orchestrator.recover();
        assert_eq!(orchestrator.state(), ConversationState::Idle);

        // History kept the failed turn's user message.
        orchestrator.submit_user_text("再挑戦");
        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 2);
        assert!(prompts[1].contains(&ChatMessage::new(ChatRole::User, "聞こえてる？")));
        assert_eq!(prompts[1].last().unwrap().content, "再挑戦");
    }

    #[test]
    fn emoji_are_stripped_from_spoken_sentences() {
        let recording = Arc::new(Recording::default());
        let llm = ScriptedLlm {
            chunks: vec!["こんにちは😊。", "今日も🎉がんばろう👍🏻。"],
            received_prompts: Arc::new(Mutex::new(Vec::new())),
            fail: false,
        };
        let mut orchestrator = ConversationOrchestrator::new(
            config(),
            llm,
            Events(Arc::clone(&recording)),
            Arc::new(AtomicBool::new(false)),
        );

        orchestrator.submit_user_text("やあ");

        let sentences: Vec<_> = recording
            .sentences
            .lock()
            .unwrap()
            .iter()
            .map(|(_, s)| s.clone())
            .collect();
        assert_eq!(sentences, ["こんにちは。", "今日もがんばろう。"]);
        assert_eq!(
            recording.completions.lock().unwrap()[0],
            "こんにちは。今日もがんばろう。"
        );
    }

    #[test]
    fn later_turns_carry_higher_turn_ids() {
        let recording = Arc::new(Recording::default());
        let llm = ScriptedLlm {
            chunks: vec!["一。", "二。"],
            received_prompts: Arc::new(Mutex::new(Vec::new())),
            fail: false,
        };
        let mut orchestrator = ConversationOrchestrator::new(
            config(),
            llm,
            Events(Arc::clone(&recording)),
            Arc::new(AtomicBool::new(false)),
        );

        let first = orchestrator.submit_user_text("最初");
        let second = orchestrator.submit_user_text("次");
        assert!(second > first);
        let sentences = recording.sentences.lock().unwrap();
        assert!(sentences.iter().take(2).all(|(turn, _)| *turn == first));
        assert!(sentences.iter().skip(2).all(|(turn, _)| *turn == second));
    }

    struct FailingPlanner;

    impl ResponsePlanner for FailingPlanner {
        fn plan(
            &mut self,
            _kind: TurnKind,
            _utterance: &str,
            _context: &MemoryContext,
        ) -> Result<ResponsePlan, PortError> {
            Err(PortError("planned preparation failed".into()))
        }
    }

    #[test]
    fn planned_preparation_failure_keeps_the_single_streamed_reply_path() {
        let recording = Arc::new(Recording::default());
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let llm = ScriptedLlm {
            chunks: vec!["fallback reply。"],
            received_prompts: Arc::clone(&prompts),
            fail: false,
        };
        let pipeline = response_pipeline(
            FailingPlanner,
            ExistingContextRetriever,
            FixedSurfaceRealizer,
            PlanningBudget::default(),
        );
        let mut orchestrator = ConversationOrchestrator::new_with_response_pipeline(
            config(),
            llm,
            Events(Arc::clone(&recording)),
            Arc::new(AtomicBool::new(false)),
            pipeline,
        );

        orchestrator.submit_user_text("これを覚えて");

        assert_eq!(
            recording.completions.lock().unwrap().as_slice(),
            ["fallback reply。"]
        );
        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1, "planning cannot create another LLM call");
        assert!(
            prompts[0]
                .iter()
                .all(|message| !message.content.contains("response_surface_context"))
        );
        assert!(
            prompts[0]
                .iter()
                .any(|message| message.content.starts_with("<turn_style_contract>\n"))
        );
    }

    #[test]
    fn planned_turn_adds_a_bounded_surface_but_still_calls_llm_once() {
        let recording = Arc::new(Recording::default());
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let llm = ScriptedLlm {
            chunks: vec!["memory reply。"],
            received_prompts: Arc::clone(&prompts),
            fail: false,
        };
        let mut orchestrator = ConversationOrchestrator::new(
            config(),
            llm,
            Events(recording),
            Arc::new(AtomicBool::new(false)),
        );

        orchestrator.submit_user_text("これを覚えて");

        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(
            prompts[0]
                .iter()
                .filter(|message| message.content.starts_with("<response_surface_context>\n"))
                .count(),
            1
        );
        assert_eq!(
            prompts[0]
                .iter()
                .filter(|message| message.content.starts_with("<turn_style_contract>\n"))
                .count(),
            1
        );
        assert_eq!(prompts[0].last().unwrap().content, "これを覚えて");
    }

    #[test]
    fn second_ordinary_turn_carries_one_contract_and_unchanged_assistant_history() {
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let llm = ScriptedLlm {
            chunks: vec!["first assistant reply?"],
            received_prompts: Arc::clone(&prompts),
            fail: false,
        };
        let mut orchestrator = ConversationOrchestrator::new(
            config(),
            llm,
            Events(Arc::new(Recording::default())),
            Arc::new(AtomicBool::new(false)),
        );

        orchestrator.submit_user_text("first ordinary request");
        orchestrator.submit_user_text("second ordinary request");

        assert_eq!(prompts.lock().unwrap().len(), 2);
        let second = &prompts.lock().unwrap()[1];
        assert_eq!(
            second
                .iter()
                .filter(|message| message.content.starts_with("<turn_style_contract>\n"))
                .count(),
            1
        );
        assert!(
            second
                .iter()
                .any(|message| message.content == "first assistant reply?")
        );
        assert_eq!(second.last().unwrap().content, "second ordinary request");
        assert_eq!(
            second
                .iter()
                .filter(|message| message.content == "first assistant reply?")
                .count(),
            1,
            "stored assistant history must be attached unchanged exactly once"
        );
    }
}
