//! Conversation worker: owns the orchestrator, emits UI events and
//! maps control JSON onto the character.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use pw_application::conversation::{
    ChatMessage, ChatRole, ConversationEvents, ConversationOrchestrator, OrchestratorConfig,
    PromptBuilder,
};
use pw_application::history::{ConversationHistory, MessageRole, StoredTurn};
use pw_contracts::{
    ChatMessageEventDto, ChatRoleDto, ConversationStateEventDto, LlmSettingsDto, SCHEMA_VERSION,
};
use pw_domain::conversation::ConversationState;
use pw_domain::reply::{ReplyControl, TurnId};
use pw_llm::{LlmClientConfig, OpenAiCompatClient};
use pw_platform::paths::AppDataLayout;
use pw_storage::{Database, SqliteConversationHistory};
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime};

use crate::commands::character::CharacterState;

pub const MESSAGE_EVENT: &str = "chat-message";
pub const STATE_EVENT: &str = "conversation-state";

/// Kept messages of context (user + assistant combined).
const MAX_HISTORY_MESSAGES: usize = 20;
const DEFAULT_CONVERSATION_ID: &str = "default";

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

fn persist_completed_turn<H: ConversationHistory>(
    history: &mut H,
    conversation_id: &str,
    turn: TurnId,
    user_text: &str,
    assistant_text: &str,
) -> Result<(), pw_application::PortError> {
    history.store_completed_turn(&StoredTurn {
        conversation_id: conversation_id.to_owned(),
        turn_id: turn.value(),
        user_content: user_text.to_owned(),
        assistant_content: assistant_text.to_owned(),
        created_at: unix_timestamp(),
    })
}

fn load_recent_history<H: ConversationHistory>(
    history: &H,
    conversation_id: &str,
    limit: usize,
) -> Result<Vec<ChatMessage>, pw_application::PortError> {
    let messages = history.list_messages(conversation_id)?;
    Ok(messages
        .into_iter()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|message| {
            ChatMessage::new(
                match message.role {
                    MessageRole::User => ChatRole::User,
                    MessageRole::Assistant => ChatRole::Assistant,
                },
                message.content,
            )
        })
        .collect())
}

struct PersistentConversationEvents<E, H> {
    inner: E,
    history: Mutex<H>,
    conversation_id: String,
    pending_users: Mutex<HashMap<TurnId, (String, Option<String>)>>,
}

impl<E, H> PersistentConversationEvents<E, H> {
    fn new(inner: E, history: H, conversation_id: impl Into<String>) -> Self {
        Self {
            inner,
            history: Mutex::new(history),
            conversation_id: conversation_id.into(),
            pending_users: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn history(&self) -> MutexGuard<'_, H> {
        self.history.lock().unwrap()
    }
}

impl<E: ConversationEvents, H: ConversationHistory> ConversationEvents
    for PersistentConversationEvents<E, H>
{
    fn on_state(&self, state: ConversationState) {
        self.inner.on_state(state);
    }
    fn on_user_message(&self, turn: TurnId, text: &str) {
        let mut pending = self.pending_users.lock().unwrap();
        let retries: Vec<_> = pending
            .iter()
            .filter_map(|(id, (user, assistant))| {
                assistant
                    .as_ref()
                    .map(|assistant| (*id, user.clone(), assistant.clone()))
            })
            .collect();
        for (retry_turn, user, assistant) in retries {
            if persist_completed_turn(
                &mut *self.history.lock().unwrap(),
                &self.conversation_id,
                retry_turn,
                &user,
                &assistant,
            )
            .is_ok()
            {
                pending.remove(&retry_turn);
            }
        }
        pending.insert(turn, (text.to_owned(), None));
        self.inner.on_user_message(turn, text);
    }
    fn on_control(&self, turn: TurnId, control: &ReplyControl) {
        self.inner.on_control(turn, control);
    }
    fn on_sentence(&self, turn: TurnId, sentence: &str) {
        self.inner.on_sentence(turn, sentence);
    }
    fn on_reply_complete(&self, turn: TurnId, speech_text: &str) {
        self.inner.on_reply_complete(turn, speech_text);
        let mut pending = self.pending_users.lock().unwrap();
        let Some((user_text, assistant)) = pending.get_mut(&turn) else {
            return;
        };
        *assistant = Some(speech_text.to_owned());
        if let Err(error) = persist_completed_turn(
            &mut *self.history.lock().unwrap(),
            &self.conversation_id,
            turn,
            user_text,
            speech_text,
        ) {
            tracing::warn!(%error, "conversation history persistence degraded to memory");
        } else {
            pending.remove(&turn);
        }
    }
    fn on_cancelled(&self, turn: TurnId) {
        self.pending_users.lock().unwrap().remove(&turn);
        self.inner.on_cancelled(turn);
    }
    fn on_error(&self, turn: TurnId, message: &str) {
        self.pending_users.lock().unwrap().remove(&turn);
        self.inner.on_error(turn, message);
    }
}

enum Command {
    Submit(String, u64),
    Shutdown,
}

struct Worker {
    tx: Sender<Command>,
    settings_fingerprint: String,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    fn shutdown(mut self) -> Result<(), String> {
        let _ = self.tx.send(Command::Shutdown);
        self.thread.take().map_or(Ok(()), |thread| {
            thread
                .join()
                .map_err(|_| "conversation worker panicked during shutdown".to_owned())
        })
    }
}

/// Managed state: at most one conversation worker.
pub struct ChatService {
    worker: Mutex<Option<Worker>>,
    cancel: Arc<AtomicBool>,
    fallback_turn_id: AtomicU64,
}

impl Default for ChatService {
    fn default() -> Self {
        Self {
            worker: Mutex::new(None),
            cancel: Arc::new(AtomicBool::new(false)),
            fallback_turn_id: AtomicU64::new(1_u64 << 63),
        }
    }
}

fn fingerprint(settings: &LlmSettingsDto) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        settings.base_url,
        settings.model,
        settings.allow_remote,
        settings.system_prompt,
        settings.character_prompt,
        settings.strip_emoji
    )
}

impl ChatService {
    fn reserve_turn_id(&self, database_path: &Path) -> u64 {
        let durable = Database::open(database_path)
            .map_err(|error| error.to_string())
            .and_then(|database| {
                SqliteConversationHistory::new(database)
                    .reserve_turn_id(DEFAULT_CONVERSATION_ID, unix_timestamp())
                    .map_err(|error| error.to_string())
            });
        match durable {
            Ok(id) => id,
            Err(error) => {
                let id = self.fallback_turn_id.fetch_add(1, Ordering::SeqCst);
                tracing::warn!(%error, turn_id = id, "durable turn allocator unavailable; using process fallback range");
                id
            }
        }
    }
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
                worker.shutdown()?;
            }
            *guard = Some(self.start_worker(app.clone(), &settings)?);
        }
        let Some(worker) = guard.as_ref() else {
            return Err("conversation worker is not available".to_owned());
        };
        let database_path = layout.data.join("parallel-world.sqlite3");
        let turn_id = self.reserve_turn_id(&database_path);
        worker
            .tx
            .send(Command::Submit(text, turn_id))
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
            strip_emoji: settings.strip_emoji,
        };
        let cancel = Arc::clone(&self.cancel);
        let (tx, rx) = channel::<Command>();
        let database_path = app
            .state::<AppDataLayout>()
            .data
            .join("parallel-world.sqlite3");
        let database = Database::open(&database_path).or_else(|error| {
            tracing::warn!(%error, path = %database_path.display(), "conversation history unavailable; using temporary history");
            Database::open_in_memory()
        }).map_err(|error| format!("failed to initialize conversation history: {error}"))?;
        let history = SqliteConversationHistory::new(database);
        let seed = load_recent_history(&history, DEFAULT_CONVERSATION_ID, MAX_HISTORY_MESSAGES)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "conversation history restore failed; continuing without restored history");
                Vec::new()
            });
        let last_turn_id = history.max_turn_id(DEFAULT_CONVERSATION_ID).unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to restore turn sequence; starting from temporary sequence");
            None
        }).unwrap_or(0);
        let events = PersistentConversationEvents::new(
            TauriConversationEvents { app },
            history,
            DEFAULT_CONVERSATION_ID,
        );

        let thread = std::thread::Builder::new()
            .name("pw-conversation".into())
            .spawn(move || {
                let mut orchestrator = ConversationOrchestrator::new_with_history_after(
                    config,
                    llm,
                    events,
                    cancel,
                    seed,
                    last_turn_id,
                );
                while let Ok(command) = rx.recv() {
                    match command {
                        Command::Submit(text, turn_id) => {
                            orchestrator.recover();
                            orchestrator.submit_user_text_with_id(&text, turn_id);
                        }
                        Command::Shutdown => break,
                    }
                }
            })
            .map_err(|error| format!("failed to spawn conversation worker: {error}"))?;

        Ok(Worker {
            tx,
            settings_fingerprint: fingerprint(settings),
            thread: Some(thread),
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
        // Sentence-level read-ahead: synthesis of this sentence runs
        // while earlier ones are still playing (基本設計 8章).
        self.app
            .state::<crate::tts::TtsService>()
            .enqueue(&self.app, turn, sentence);
    }

    fn on_reply_complete(&self, _turn: TurnId, _speech_text: &str) {}

    fn on_cancelled(&self, _turn: TurnId) {}

    fn on_error(&self, _turn: TurnId, message: &str) {
        self.emit_state(ConversationState::LlmUnavailable, Some(message.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use pw_application::history::ConversationHistory;
    use pw_domain::reply::TurnTracker;
    use pw_storage::{Database, SqliteConversationHistory};

    use super::*;

    #[derive(Default)]
    struct NoopEvents {
        errors: Mutex<Vec<String>>,
    }

    impl ConversationEvents for NoopEvents {
        fn on_state(&self, _: ConversationState) {}
        fn on_user_message(&self, _: TurnId, _: &str) {}
        fn on_control(&self, _: TurnId, _: &ReplyControl) {}
        fn on_sentence(&self, _: TurnId, _: &str) {}
        fn on_reply_complete(&self, _: TurnId, _: &str) {}
        fn on_cancelled(&self, _: TurnId) {}
        fn on_error(&self, _: TurnId, message: &str) {
            self.errors.lock().unwrap().push(message.to_owned());
        }
    }

    #[test]
    fn persists_only_completed_user_and_assistant_messages() {
        let history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
        let events = PersistentConversationEvents::new(NoopEvents::default(), history, "chat");
        let mut tracker = TurnTracker::new();
        let completed = tracker.begin_turn();
        events.on_user_message(completed, "kept user");
        events.on_reply_complete(completed, "kept assistant");
        let cancelled = tracker.begin_turn();
        events.on_user_message(cancelled, "cancelled user");
        events.on_sentence(cancelled, "partial");
        events.on_cancelled(cancelled);

        let messages = events.history().list_messages("chat").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "kept user");
        assert_eq!(messages[1].content, "kept assistant");
        assert!(messages.iter().all(|message| message.id.is_some()));
    }

    #[test]
    fn restored_messages_are_limited_to_recent_prompt_history() {
        let mut history = SqliteConversationHistory::new(Database::open_in_memory().unwrap());
        let mut tracker = TurnTracker::new();
        persist_completed_turn(
            &mut history,
            "chat",
            tracker.begin_turn(),
            "old",
            "old reply",
        )
        .unwrap();
        persist_completed_turn(
            &mut history,
            "chat",
            tracker.begin_turn(),
            "new",
            "new reply",
        )
        .unwrap();

        let restored = load_recent_history(&history, "chat", 2).unwrap();

        assert_eq!(
            restored
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            ["new", "new reply"]
        );
    }

    #[test]
    fn worker_restart_waits_for_queued_turn_before_history_is_read() {
        let (tx, rx) = channel();
        let persisted = Arc::new(AtomicBool::new(false));
        let worker_persisted = Arc::clone(&persisted);
        let thread = std::thread::spawn(move || {
            while let Ok(command) = rx.recv() {
                match command {
                    Command::Submit(_, _) => worker_persisted.store(true, Ordering::SeqCst),
                    Command::Shutdown => break,
                }
            }
        });
        let worker = Worker {
            tx,
            settings_fingerprint: "old".into(),
            thread: Some(thread),
        };
        worker.tx.send(Command::Submit("turn".into(), 1)).unwrap();

        worker.shutdown().unwrap();

        assert!(
            persisted.load(Ordering::SeqCst),
            "new worker would have seeded too early"
        );
    }

    #[test]
    fn allocator_falls_back_on_database_failure_and_never_collides_after_recovery() {
        let service = ChatService::default();
        let invalid = std::env::temp_dir()
            .join("missing-parent")
            .join("db.sqlite3");
        let first = service.reserve_turn_id(&invalid);
        let second = service.reserve_turn_id(&invalid);
        assert!(first >= (1_u64 << 63));
        assert_eq!(second, first + 1);

        let path = std::env::temp_dir().join(format!(
            "pw-allocator-recovery-{}.sqlite3",
            std::process::id()
        ));
        let recovered = service.reserve_turn_id(&path);
        assert!(recovered < (1_u64 << 63));
        assert_ne!(recovered, first);
        let _ = std::fs::remove_file(path);
    }
}
