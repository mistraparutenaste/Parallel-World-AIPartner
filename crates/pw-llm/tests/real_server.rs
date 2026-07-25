//! Manual verification against a real OpenAI-compatible server.
//!
//! ```text
//! PW_LLM_BASE_URL=http://127.0.0.1:1234/v1 PW_LLM_MODEL=<model id> \
//! cargo test -p pw-llm --test real_server -- --ignored --nocapture
//! ```

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pw_application::conversation::{
    ChatMessage, ChatRole, ConversationEvents, ConversationOrchestrator, OrchestratorConfig,
    PromptBuilder,
};
use pw_domain::conversation::ConversationState;
use pw_domain::reply::{ReplyControl, TurnId};
use pw_llm::{LlmClientConfig, OpenAiCompatClient};

#[derive(Default)]
struct Log {
    sentences: Mutex<Vec<String>>,
    controls: Mutex<Vec<ReplyControl>>,
    states: Mutex<Vec<ConversationState>>,
    errors: Mutex<Vec<String>>,
    completions: Mutex<Vec<String>>,
}

struct Events(Arc<Log>);

impl ConversationEvents for Events {
    fn on_state(&self, state: ConversationState) {
        self.0.states.lock().unwrap().push(state);
    }
    fn on_user_message(&self, _turn: TurnId, _text: &str) {}
    fn on_control(&self, _turn: TurnId, control: &ReplyControl) {
        println!("control: {control:?}");
        self.0.controls.lock().unwrap().push(control.clone());
    }
    fn on_sentence(&self, _turn: TurnId, sentence: &str) {
        println!("sentence: {sentence}");
        self.0.sentences.lock().unwrap().push(sentence.to_owned());
    }
    fn on_reply_complete(&self, _turn: TurnId, speech_text: &str) {
        println!("complete: {speech_text}");
        self.0
            .completions
            .lock()
            .unwrap()
            .push(speech_text.to_owned());
    }
    fn on_cancelled(&self, _turn: TurnId) {}
    fn on_error(&self, _turn: TurnId, message: &str) {
        self.0.errors.lock().unwrap().push(message.to_owned());
    }
}

#[test]
#[ignore = "requires a running OpenAI-compatible server"]
fn full_turn_against_the_real_server() {
    let base_url =
        std::env::var("PW_LLM_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:1234/v1".into());
    let model = std::env::var("PW_LLM_MODEL").expect("set PW_LLM_MODEL");

    let llm = OpenAiCompatClient::new(LlmClientConfig {
        base_url,
        model,
        api_key: None,
        allow_remote: false,
        timeout: Duration::from_mins(3),
        ..LlmClientConfig::default()
    })
    .unwrap();

    let config = OrchestratorConfig {
        prompt: PromptBuilder {
            system_rules: "あなたはデスクトップに常駐するAIパートナーです。\
応答の1行目には {\"emotion\":\"表情名\",\"intensity\":0.0から1.0,\"motion\":\"モーション名\"} \
という制御JSONだけを出力し、空行を1行挟んでから本文を書いてください。\
本文は日本語の話し言葉で、短く自然な文にしてください。"
                .into(),
            character_prompt: "あなたの名前はエプシロン。明るく丁寧な口調で話す。\n\
利用できる表情(emotion): Angry, Blushing, f01, f02, Normal, Sad, Smile, Surprised\n\
利用できるモーション(motion): Idle, FlickUp, Flick, Tap, Flick3, FlickDown, Shake"
                .into(),
        },
        max_history_messages: 20,
        strip_emoji: true,
    };

    let log = Arc::new(Log::default());
    let mut orchestrator = ConversationOrchestrator::new(
        config,
        llm,
        Events(Arc::clone(&log)),
        Arc::new(AtomicBool::new(false)),
    );

    orchestrator.submit_user_text("こんにちは。聞こえていますか？短く答えてください。");

    let errors = log.errors.lock().unwrap();
    assert!(errors.is_empty(), "errors: {errors:?}");
    let sentences = log.sentences.lock().unwrap();
    assert!(!sentences.is_empty(), "no sentences received");
    assert_eq!(orchestrator.state(), ConversationState::Idle);
    println!(
        "control preludes: {} / sentences: {}",
        log.controls.lock().unwrap().len(),
        sentences.len()
    );
}

struct MeasurementScenario {
    name: &'static str,
    assistant_history: &'static [&'static str],
    utterance: &'static str,
}

fn build_dialogue_measurement_orchestrator(
    base_url: &str,
    model: &str,
    assistant_history: &[&str],
    events: Events,
) -> ConversationOrchestrator<OpenAiCompatClient, Events> {
    let llm = OpenAiCompatClient::new(LlmClientConfig {
        base_url: base_url.to_owned(),
        model: model.to_owned(),
        api_key: None,
        allow_remote: false,
        timeout: Duration::from_mins(3),
        ..LlmClientConfig::default()
    })
    .expect("valid opted-in local OpenAI-compatible server configuration");
    let history = assistant_history
        .iter()
        .map(|message| ChatMessage::new(ChatRole::Assistant, *message))
        .collect();

    ConversationOrchestrator::new_with_history(
        OrchestratorConfig {
            prompt: PromptBuilder {
                system_rules: "Respond helpfully and follow the character profile.".into(),
                character_prompt: "You are a discreet executive secretary. Preserve your natural wording while assisting the user.".into(),
            },
            max_history_messages: 20,
            strip_emoji: true,
        },
        llm,
        events,
        Arc::new(AtomicBool::new(false)),
        history,
    )
}

fn ends_with_question(reply: &str) -> bool {
    let reply = reply.trim_end();
    matches!(reply.chars().last(), Some('?' | '？'))
        || reply
            .strip_suffix('。')
            .is_some_and(|sentence| sentence.ends_with('か'))
}

#[test]
fn identifies_ascii_fullwidth_and_japanese_question_endings() {
    assert!(ends_with_question("What is the plan?"));
    assert!(ends_with_question("計画は何ですか？"));
    assert!(ends_with_question("計画は何ですか。"));
    assert!(ends_with_question("どのような計画でしょうか。"));
    assert!(!ends_with_question("今日は雨ですね。"));
}

#[test]
#[ignore = "requires PW_LLM_DIALOGUE_EVAL=1 and a running OpenAI-compatible server"]
fn measures_question_endings_for_dialogue_contract_cases() {
    if std::env::var("PW_LLM_DIALOGUE_EVAL").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping dialogue measurement: set PW_LLM_DIALOGUE_EVAL=1 with a local server configuration"
        );
        return;
    }

    let base_url = std::env::var("PW_LLM_BASE_URL").expect("set PW_LLM_BASE_URL after opting in");
    let model = std::env::var("PW_LLM_MODEL").expect("set PW_LLM_MODEL after opting in");
    let scenarios = [
        MeasurementScenario {
            name: "japanese_greeting_after_assistant_question",
            assistant_history: &["May I organize anything else?"],
            utterance: "こんにちは",
        },
        MeasurementScenario {
            name: "casual_observation_without_recent_question",
            assistant_history: &[],
            utterance: "今日は雨ですね",
        },
        MeasurementScenario {
            name: "explicit_requested_questioning",
            assistant_history: &[],
            utterance: "質問を一つずつして、計画を整理して",
        },
    ];

    for scenario in scenarios {
        let log = Arc::new(Log::default());
        let mut orchestrator = build_dialogue_measurement_orchestrator(
            &base_url,
            &model,
            scenario.assistant_history,
            Events(Arc::clone(&log)),
        );

        orchestrator.submit_user_text(scenario.utterance);

        let reply = log.completions.lock().unwrap();
        assert_eq!(reply.len(), 1, "case={}", scenario.name);
        let reply = reply[0].clone();
        assert!(
            log.errors.lock().unwrap().is_empty(),
            "case={}",
            scenario.name
        );
        assert!(!reply.trim().is_empty(), "case={}", scenario.name);
        assert_eq!(
            orchestrator.state(),
            ConversationState::Idle,
            "case={}",
            scenario.name
        );
        println!(
            "dialogue_measurement case={} reply_chars={} question_ending={}",
            scenario.name,
            reply.chars().count(),
            ends_with_question(&reply),
        );
    }
}
