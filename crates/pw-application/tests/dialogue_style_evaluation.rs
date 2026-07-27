use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pw_application::PortError;
use pw_application::conversation::{
    ChatMessage, ChatRole, ClosingPreference, ConversationEvents, ConversationOrchestrator,
    DialogueClassifier, DialogueTurnKind, LlmClient, OrchestratorConfig, PromptBuilder,
    QuestionPolicy, TurnStyleContract,
};
use pw_application::memory::MemoryContext;
use pw_domain::conversation::ConversationState;
use pw_domain::reply::{ReplyControl, TurnId};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Fixture {
    name: String,
    persona: String,
    history: Vec<String>,
    utterance: String,
    expected_turn_kind: String,
    expected_question_policy: String,
    expected_closing_preference: String,
    expected_recent_question_endings: u8,
}

#[derive(Default, Clone)]
struct Events;

impl ConversationEvents for Events {
    fn on_state(&self, _: ConversationState) {}
    fn on_user_message(&self, _: TurnId, _: &str) {}
    fn on_control(&self, _: TurnId, _: &ReplyControl) {}
    fn on_sentence(&self, _: TurnId, _: &str) {}
    fn on_reply_complete(&self, _: TurnId, _: &str) {}
    fn on_cancelled(&self, _: TurnId) {}
    fn on_error(&self, _: TurnId, _: &str) {}
}

struct RecordingLlm {
    calls: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<Vec<ChatMessage>>>>,
}

impl LlmClient for RecordingLlm {
    fn stream_chat(
        &mut self,
        messages: &[ChatMessage],
        _: &AtomicBool,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<(), PortError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.prompts.lock().unwrap().push(messages.to_vec());
        on_delta("A fixed synthetic reply.");
        Ok(())
    }
}

fn fixtures() -> Vec<Fixture> {
    serde_json::from_str(include_str!("fixtures/dialogue_style_cases.json"))
        .expect("dialogue style fixture must be valid JSON")
}

fn expected_contract(fixture: &Fixture) -> TurnStyleContract {
    TurnStyleContract {
        turn_kind: match fixture.expected_turn_kind.as_str() {
            "greeting" => DialogueTurnKind::Greeting,
            "compliment" => DialogueTurnKind::Compliment,
            "casual_observation" => DialogueTurnKind::CasualObservation,
            "answer_or_request" => DialogueTurnKind::AnswerOrRequest,
            "requested_questioning" => DialogueTurnKind::RequestedQuestioning,
            unexpected => panic!("{} has unknown turn kind {unexpected}", fixture.name),
        },
        question_policy: match fixture.expected_question_policy.as_str() {
            "avoid_question_ending" => QuestionPolicy::AvoidQuestionEnding,
            "clarification_only_if_materially_necessary" => {
                QuestionPolicy::ClarificationOnlyIfMateriallyNecessary
            }
            "contentful_question_only_if_no_recent_question" => {
                QuestionPolicy::ContentfulQuestionOnlyIfNoRecentQuestion
            }
            "question_requested" => QuestionPolicy::QuestionRequested,
            unexpected => panic!("{} has unknown question policy {unexpected}", fixture.name),
        },
        closing_preference: match fixture.expected_closing_preference.as_str() {
            "declarative" => ClosingPreference::Declarative,
            "question_permitted" => ClosingPreference::QuestionPermitted,
            unexpected => panic!(
                "{} has unknown closing preference {unexpected}",
                fixture.name
            ),
        },
        recent_assistant_question_endings: fixture.expected_recent_question_endings,
    }
}

fn assert_serialized_contract(prompt: &[ChatMessage], fixture: &Fixture) {
    let contracts: Vec<_> = prompt
        .iter()
        .filter(|message| message.content.starts_with("<turn_style_contract>\n"))
        .collect();
    assert_eq!(
        contracts.len(),
        1,
        "{} must include one turn-style contract tag",
        fixture.name
    );
    assert!(
        !contracts[0].content.contains(&fixture.persona),
        "{} must keep persona text out of the turn-style contract",
        fixture.name
    );
    let payload = contracts[0]
        .content
        .strip_prefix("<turn_style_contract>\n")
        .and_then(|content| content.strip_suffix("\n</turn_style_contract>"))
        .expect("turn-style contract must retain its tagged JSON boundary");
    let actual: serde_json::Value =
        serde_json::from_str(payload).expect("turn-style contract must contain JSON");
    let expected = serde_json::json!({
        "turn_kind": fixture.expected_turn_kind.as_str(),
        "question_policy": fixture.expected_question_policy.as_str(),
        "closing_preference": fixture.expected_closing_preference.as_str(),
        "recent_assistant_question_endings": fixture.expected_recent_question_endings,
    });
    assert_eq!(
        actual, expected,
        "{} must serialize the exact fixture contract",
        fixture.name
    );
}

fn assert_history_is_role_preserving_and_unique(
    prompt: &[ChatMessage],
    expected_history: &[String],
    fixture_name: &str,
) {
    let assistant_history: Vec<_> = prompt
        .iter()
        .filter(|message| message.role == ChatRole::Assistant)
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(
        assistant_history,
        expected_history
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        "{fixture_name} must preserve ordered assistant history"
    );
    for history_item in expected_history {
        let occurrences: Vec<_> = prompt
            .iter()
            .filter(|message| message.content == *history_item)
            .collect();
        assert_eq!(
            occurrences.len(),
            1,
            "{fixture_name} history item {history_item:?} must appear exactly once"
        );
        assert_eq!(
            occurrences[0].role,
            ChatRole::Assistant,
            "{fixture_name} history item {history_item:?} must retain its Assistant role"
        );
    }
}

#[test]
#[should_panic(expected = "must serialize the exact fixture contract")]
fn serialized_contract_assertion_rejects_a_default_looking_contract() {
    let fixture = Fixture {
        name: "wrong_default_contract".into(),
        persona: "synthetic persona".into(),
        history: Vec::new(),
        utterance: "synthetic request".into(),
        expected_turn_kind: "answer_or_request".into(),
        expected_question_policy: "clarification_only_if_materially_necessary".into(),
        expected_closing_preference: "declarative".into(),
        expected_recent_question_endings: 0,
    };
    let prompt = vec![ChatMessage::new(
        ChatRole::User,
        "<turn_style_contract>\n{\"turn_kind\":\"greeting\",\"question_policy\":\"avoid_question_ending\",\"closing_preference\":\"declarative\",\"recent_assistant_question_endings\":0}\n</turn_style_contract>",
    )];

    assert_serialized_contract(&prompt, &fixture);
}

#[test]
#[should_panic(expected = "must appear exactly once")]
fn history_assertion_rejects_a_duplicate_under_the_user_role() {
    let prompt = vec![
        ChatMessage::new(ChatRole::Assistant, "Synthetic history."),
        ChatMessage::new(ChatRole::User, "Synthetic history."),
    ];

    assert_history_is_role_preserving_and_unique(
        &prompt,
        &["Synthetic history.".into()],
        "user-role duplicate",
    );
}

#[test]
#[should_panic(expected = "must appear exactly once")]
fn history_assertion_rejects_a_duplicate_under_the_system_role() {
    let prompt = vec![
        ChatMessage::new(ChatRole::Assistant, "Synthetic history."),
        ChatMessage::new(ChatRole::System, "Synthetic history."),
    ];

    assert_history_is_role_preserving_and_unique(
        &prompt,
        &["Synthetic history.".into()],
        "system-role duplicate",
    );
}

/// Regression guard for the persona-effectiveness work: two contrasting
/// personas must yield different prompts for the same input, while the
/// app-owned turn-style contract stays identical.
#[test]
fn contrasting_personas_produce_different_prompts_with_identical_contracts() {
    let classifier = DialogueClassifier;
    let utterance = "今日は雨ですね";
    let quiet_persona = "プロフィール:\n- 名前: 燈\n- 話し方: ぶっきらぼうで口数が少ない\n会話の傾向:\n- 返事はひとこと、ふたことのごく短いものにする";
    let talkative_persona = "プロフィール:\n- 名前: ひまり\n- 話し方: 明るく饒舌で、感嘆詞が多い\n会話の傾向:\n- 話し好きで、具体例や余談も交えてたっぷり話す";
    let contract = classifier.classify(utterance, &[]);

    let build = |persona: &str| {
        PromptBuilder {
            system_rules: "Keep replies factual and concise.".into(),
            character_prompt: persona.into(),
        }
        .build_with_context_and_turn_style(
            &[],
            utterance,
            &MemoryContext::default(),
            &contract,
        )
    };
    let quiet_prompt = build(quiet_persona);
    let talkative_prompt = build(talkative_persona);

    assert_eq!(quiet_prompt.len(), talkative_prompt.len());
    let differing: Vec<usize> = quiet_prompt
        .iter()
        .zip(&talkative_prompt)
        .enumerate()
        .filter(|(_, (quiet, talkative))| quiet != talkative)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        differing,
        [1],
        "exactly the persona system message must differ between the prompts"
    );
    assert_eq!(quiet_prompt[1].content, quiet_persona);
    assert_eq!(talkative_prompt[1].content, talkative_persona);
    assert_eq!(quiet_prompt[1].role, ChatRole::System);
}

#[test]
fn fixture_cases_keep_personas_history_and_current_turn_boundaries() {
    let classifier = DialogueClassifier;

    for fixture in fixtures() {
        let history: Vec<_> = fixture
            .history
            .iter()
            .map(|message| ChatMessage::new(ChatRole::Assistant, message))
            .collect();
        let contract = classifier.classify(&fixture.utterance, &history);
        assert_eq!(
            contract,
            expected_contract(&fixture),
            "{} selected an unexpected turn-style contract",
            fixture.name
        );

        let calls = Arc::new(AtomicUsize::new(0));
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let config = OrchestratorConfig {
            prompt: PromptBuilder {
                system_rules: "Keep replies factual and concise.".into(),
                character_prompt: fixture.persona.clone(),
            },
            max_history_messages: 8,
            strip_emoji: false,
            max_reply_chars: 0,
        };
        let mut orchestrator = ConversationOrchestrator::new_with_history(
            config,
            RecordingLlm {
                calls: Arc::clone(&calls),
                prompts: Arc::clone(&prompts),
            },
            Events,
            Arc::new(AtomicBool::new(false)),
            history,
        );

        orchestrator.submit_user_text(&fixture.utterance);

        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "{} must use exactly one streamed completion",
            fixture.name
        );
        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1, "{} must record one prompt", fixture.name);
        let prompt = &prompts[0];

        assert_eq!(
            prompt
                .iter()
                .filter(|message| {
                    message.role == ChatRole::System && message.content == fixture.persona
                })
                .count(),
            1,
            "{} must preserve the persona byte-for-byte in its original system message",
            fixture.name
        );
        assert_serialized_contract(prompt, &fixture);
        assert_history_is_role_preserving_and_unique(prompt, &fixture.history, &fixture.name);
        assert_eq!(
            prompt.last(),
            Some(&ChatMessage::new(ChatRole::User, &fixture.utterance)),
            "{} must end with the current user utterance",
            fixture.name
        );
        assert_eq!(
            prompt
                .iter()
                .filter(|message| {
                    message.role == ChatRole::User && message.content == fixture.utterance
                })
                .count(),
            1,
            "{} must have exactly one final current-user message",
            fixture.name
        );
    }
}
