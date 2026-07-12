//! Conversation worker: owns the orchestrator, emits UI events and
//! maps control JSON onto the character.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};

use pw_application::conversation::{
    ConversationEvents, ConversationOrchestrator, OrchestratorConfig, PromptBuilder,
};
use pw_contracts::{
    ChatMessageEventDto, ChatRoleDto, ConversationStateEventDto, LlmSettingsDto, SCHEMA_VERSION,
};
use pw_domain::conversation::ConversationState;
use pw_domain::reply::{ReplyControl, TurnId};
use pw_llm::{LlmClientConfig, OpenAiCompatClient};
use pw_platform::paths::AppDataLayout;
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime};

use crate::commands::character::CharacterState;

pub const MESSAGE_EVENT: &str = "chat-message";
pub const STATE_EVENT: &str = "conversation-state";

/// Kept messages of context (user + assistant combined).
const MAX_HISTORY_MESSAGES: usize = 20;

enum Command {
    Submit(String),
    Shutdown,
}

struct Worker {
    tx: Sender<Command>,
    settings_fingerprint: String,
}

/// Managed state: at most one conversation worker.
#[derive(Default)]
pub struct ChatService {
    worker: Mutex<Option<Worker>>,
    cancel: Arc<AtomicBool>,
}

fn fingerprint(settings: &LlmSettingsDto) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        settings.base_url,
        settings.model,
        settings.allow_remote,
        settings.system_prompt,
        settings.character_prompt
    )
}

impl ChatService {
    fn lock(&self) -> MutexGuard<'_, Option<Worker>> {
        self.worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Queues a user utterance, starting (or restarting) the worker
    /// with the current settings when needed.
    ///
    /// # Errors
    ///
    /// Returns an error message when the worker cannot be started.
    pub fn submit<R: Runtime>(&self, app: &AppHandle<R>, text: String) -> Result<(), String> {
        let layout = app.state::<AppDataLayout>();
        let settings = super::settings::load_llm_settings(&layout);
        let wanted = fingerprint(&settings);

        let mut guard = self.lock();
        let restart = match guard.as_ref() {
            Some(worker) => worker.settings_fingerprint != wanted,
            None => true,
        };
        if restart {
            if let Some(worker) = guard.take() {
                let _ = worker.tx.send(Command::Shutdown);
            }
            *guard = Some(self.start_worker(app.clone(), &settings)?);
        }
        let Some(worker) = guard.as_ref() else {
            return Err("conversation worker is not available".to_owned());
        };
        worker
            .tx
            .send(Command::Submit(text))
            .map_err(|_| "conversation worker is not available".to_owned())
    }

    /// Cancels the in-flight turn (生成途中で停止).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn start_worker<R: Runtime>(
        &self,
        app: AppHandle<R>,
        settings: &LlmSettingsDto,
    ) -> Result<Worker, String> {
        let llm = OpenAiCompatClient::new(LlmClientConfig {
            base_url: settings.base_url.clone(),
            model: settings.model.clone(),
            allow_remote: settings.allow_remote,
            ..LlmClientConfig::default()
        })
        .map_err(|error| error.to_string())?;

        let character_prompt = with_character_abilities(&app, settings.character_prompt.clone());
        let config = OrchestratorConfig {
            prompt: PromptBuilder {
                system_rules: settings.system_prompt.clone(),
                character_prompt,
            },
            max_history_messages: MAX_HISTORY_MESSAGES,
        };
        let cancel = Arc::clone(&self.cancel);
        let (tx, rx) = channel::<Command>();
        let events = TauriConversationEvents { app };

        std::thread::Builder::new()
            .name("pw-conversation".into())
            .spawn(move || {
                let mut orchestrator = ConversationOrchestrator::new(config, llm, events, cancel);
                while let Ok(command) = rx.recv() {
                    match command {
                        Command::Submit(text) => {
                            orchestrator.recover();
                            orchestrator.submit_user_text(&text);
                        }
                        Command::Shutdown => break,
                    }
                }
            })
            .map_err(|error| format!("failed to spawn conversation worker: {error}"))?;

        Ok(Worker {
            tx,
            settings_fingerprint: fingerprint(settings),
        })
    }
}

/// Appends the loaded character's expression / motion names so the
/// model can emit control JSON the renderer understands.
fn with_character_abilities<R: Runtime>(app: &AppHandle<R>, base: String) -> String {
    let state = app.state::<CharacterState>();
    match state.manifest_summary() {
        Some((expressions, motions)) if !expressions.is_empty() || !motions.is_empty() => {
            format!(
                "{base}\n利用できる表情(emotion): {}\n利用できるモーション(motion): {}",
                expressions.join(", "),
                motions.join(", ")
            )
        }
        _ => base,
    }
}

struct TauriConversationEvents<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> TauriConversationEvents<R> {
    fn emit_message(&self, turn: TurnId, role: ChatRoleDto, text: &str) {
        let payload = ChatMessageEventDto {
            schema_version: SCHEMA_VERSION,
            turn_id: turn.value(),
            role,
            text: text.to_owned(),
        };
        // Single broadcast: see the event conventions note.
        let _ = self.app.emit(MESSAGE_EVENT, payload);
    }

    fn emit_state(&self, state: ConversationState, message: Option<String>) {
        let dto = match state {
            ConversationState::Starting => pw_contracts::ConversationStateDto::Starting,
            ConversationState::Idle => pw_contracts::ConversationStateDto::Idle,
            ConversationState::Listening => pw_contracts::ConversationStateDto::Listening,
            ConversationState::Transcribing => pw_contracts::ConversationStateDto::Transcribing,
            ConversationState::Thinking => pw_contracts::ConversationStateDto::Thinking,
            ConversationState::Speaking => pw_contracts::ConversationStateDto::Speaking,
            ConversationState::Muted => pw_contracts::ConversationStateDto::Muted,
            ConversationState::Interrupting => pw_contracts::ConversationStateDto::Interrupting,
            ConversationState::Cancelled => pw_contracts::ConversationStateDto::Cancelled,
            ConversationState::Recovering => pw_contracts::ConversationStateDto::Recovering,
            ConversationState::SttUnavailable => pw_contracts::ConversationStateDto::SttUnavailable,
            ConversationState::LlmUnavailable => pw_contracts::ConversationStateDto::LlmUnavailable,
            ConversationState::TtsUnavailable => pw_contracts::ConversationStateDto::TtsUnavailable,
            ConversationState::RendererUnavailable => {
                pw_contracts::ConversationStateDto::RendererUnavailable
            }
        };
        let payload = ConversationStateEventDto {
            schema_version: SCHEMA_VERSION,
            state: dto,
            message,
        };
        let _ = self.app.emit(STATE_EVENT, payload);
    }
}

impl<R: Runtime> ConversationEvents for TauriConversationEvents<R> {
    fn on_state(&self, state: ConversationState) {
        self.emit_state(state, None);
    }

    fn on_user_message(&self, turn: TurnId, text: &str) {
        self.emit_message(turn, ChatRoleDto::User, text);
    }

    fn on_control(&self, _turn: TurnId, control: &ReplyControl) {
        // Map the control prelude onto the character window; unknown
        // names are ignored by the renderer.
        if let Some(emotion) = &control.emotion {
            let _ = self.app.emit_to(
                EventTarget::webview_window("character"),
                "character-expression",
                emotion.clone(),
            );
        }
        if let Some(motion) = &control.motion {
            let _ = self.app.emit_to(
                EventTarget::webview_window("character"),
                "character-motion",
                motion.clone(),
            );
        }
    }

    fn on_sentence(&self, turn: TurnId, sentence: &str) {
        self.emit_message(turn, ChatRoleDto::Assistant, sentence);
    }

    fn on_reply_complete(&self, _turn: TurnId, _speech_text: &str) {}

    fn on_cancelled(&self, _turn: TurnId) {}

    fn on_error(&self, _turn: TurnId, message: &str) {
        self.emit_state(ConversationState::LlmUnavailable, Some(message.to_owned()));
    }
}
