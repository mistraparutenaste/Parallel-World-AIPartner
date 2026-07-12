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
    ConversationEvents, ConversationOrchestrator, OrchestratorConfig, PromptBuilder,
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
        allow_remote: false,
        timeout: Duration::from_mins(3),
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
