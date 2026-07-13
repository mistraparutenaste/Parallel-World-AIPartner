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
use pw_application::memory::{
    DEFAULT_MEMORY_LIMIT, JapanesePersistentFactGenerator, MemoryContext, MemoryStore,
    PersistentFactGenerator, RollingSummaryGenerator, SummaryGenerator, is_safe_persistent_content,
};
use pw_contracts::{
    ChatMessageEventDto, ChatRoleDto, ConversationStateEventDto, LlmSettingsDto, SCHEMA_VERSION,
};
use pw_domain::conversation::ConversationState;
use pw_domain::reply::{ReplyControl, TurnId};
use pw_llm::{LlmClientConfig, OpenAiCompatClient};
use pw_platform::paths::AppDataLayout;
use pw_storage::{Database, SqliteConversationHistory, SqliteMemoryStore};
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime};

use crate::commands::character::CharacterState;

pub const MESSAGE_EVENT: &str = "chat-message";
pub const STATE_EVENT: &str = "conversation-state";

/// Kept messages of context (user + assistant combined).
const MAX_HISTORY_MESSAGES: usize = 20;
const DEFAULT_CONVERSATION_ID: &str = "default";

fn load_memory_context<M: MemoryStore>(memory: &M, query: &str) -> MemoryContext {
    let memories = memory
        .search(query, DEFAULT_MEMORY_LIMIT)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "memory search failed; continuing without long-term memory");
            Vec::new()
        });
    let summary = memory
        .load_summary(DEFAULT_CONVERSATION_ID)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "summary restore failed; continuing without summary");
            None
        });
    MemoryContext {
        user_settings: None,
        memories: memories.into_iter().map(|item| item.content).collect(),
        summary: summary.map(|item| item.content),
    }
}

const SUMMARY_RECENT_MESSAGES: usize = 4;
const SUMMARY_BATCH_MESSAGES: usize = 8;
const SUMMARY_MAX_CHARS: usize = 2_000;

fn process_enrichment_job(database_path: &Path, user_text: &str) -> Result<(), String> {
    let history = SqliteConversationHistory::new(
        Database::open(database_path).map_err(|error| error.to_string())?,
    );
    let mut memory =
        SqliteMemoryStore::new(Database::open(database_path).map_err(|error| error.to_string())?);
    let mut facts = JapanesePersistentFactGenerator;
    for fact in facts
        .extract(user_text)
        .map_err(|error| error.to_string())?
    {
        if let Err(error) =
            memory.upsert_memory(Some(DEFAULT_CONVERSATION_ID), &fact, unix_timestamp())
        {
            tracing::warn!(%error, "persistent fact rejected; summary enrichment continues");
        }
    }
    let messages = history
        .list_messages(DEFAULT_CONVERSATION_ID)
        .map_err(|error| error.to_string())?;
    let stable_len = messages.len().saturating_sub(SUMMARY_RECENT_MESSAGES);
    let existing = memory
        .load_summary(DEFAULT_CONVERSATION_ID)
        .map_err(|error| error.to_string())?;
    let through = existing.as_ref().map_or(0, |item| item.through_message_id);
    let pending = messages[..stable_len]
        .iter()
        .filter(|item| item.id.is_some_and(|id| id > through))
        .take(SUMMARY_BATCH_MESSAGES)
        .collect::<Vec<_>>();
    if pending.is_empty() {
        return Ok(());
    }
    let prompt_messages = pending
        .iter()
        .filter(|item| is_safe_persistent_content(&item.content))
        .map(|item| {
            ChatMessage::new(
                match item.role {
                    MessageRole::User => ChatRole::User,
                    MessageRole::Assistant => ChatRole::Assistant,
                },
                item.content.clone(),
            )
        })
        .collect::<Vec<_>>();
    let delta = RollingSummaryGenerator
        .summarize(&prompt_messages)
        .map_err(|error| error.to_string())?;
    let merged = [
        existing.map(|item| item.content),
        (!delta.is_empty()).then_some(delta),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" / ");
    let bounded = merged
        .chars()
        .rev()
        .take(SUMMARY_MAX_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let through = pending
        .last()
        .and_then(|item| item.id)
        .ok_or_else(|| "pending summary message lacks id".to_owned())?;
    memory
        .upsert_summary(DEFAULT_CONVERSATION_ID, &bounded, through, unix_timestamp())
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn run_enrichment(database_path: &Path, rx: std::sync::mpsc::Receiver<String>) {
    while let Ok(user_text) = rx.recv() {
        if let Err(error) = process_enrichment_job(database_path, &user_text) {
            tracing::warn!(%error, "memory enrichment job failed; worker remains available");
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn run_context_worker<M: MemoryStore>(
    memory: M,
    rx: std::sync::mpsc::Receiver<Command>,
    conversation_tx: Sender<Command>,
) {
    while let Ok(command) = rx.recv() {
        match command {
            Command::Submit(text, turn_id) => {
                let context = load_memory_context(&memory, &text);
                if conversation_tx
                    .send(Command::Prepared(text, turn_id, context))
                    .is_err()
                {
                    break;
                }
            }
            Command::Shutdown => {
                let _ = conversation_tx.send(Command::Shutdown);
                break;
            }
            Command::Prepared(..) => {}
        }
    }
}

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
    enrichment: Option<Sender<String>>,
}

impl<E, H> PersistentConversationEvents<E, H> {
    #[cfg(test)]
    fn new(inner: E, history: H, conversation_id: impl Into<String>) -> Self {
        Self::new_with_enrichment(inner, history, conversation_id, None)
    }
    fn new_with_enrichment(
        inner: E,
        history: H,
        conversation_id: impl Into<String>,
        enrichment: Option<Sender<String>>,
    ) -> Self {
        Self {
            inner,
            history: Mutex::new(history),
            conversation_id: conversation_id.into(),
            pending_users: Mutex::new(HashMap::new()),
            enrichment,
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
            if let Some(enrichment) = &self.enrichment
                && let Err(error) = enrichment.send(user_text.clone())
            {
                tracing::warn!(%error, "memory enrichment worker unavailable; conversation remains available");
            }
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
    Prepared(String, u64, MemoryContext),
    Shutdown,
}

struct Worker {
    tx: Sender<Command>,
    settings_fingerprint: String,
    thread: Option<std::thread::JoinHandle<()>>,
    context_thread: Option<std::thread::JoinHandle<()>>,
    enrichment_thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    fn shutdown(mut self) -> Result<(), String> {
        let _ = self.tx.send(Command::Shutdown);
        let result = self.context_thread.take().map_or(Ok(()), |thread| {
            thread
                .join()
                .map_err(|_| "context worker panicked during shutdown".to_owned())
        });
        let conversation = self.thread.take().map_or(Ok(()), |thread| {
            thread
                .join()
                .map_err(|_| "conversation worker panicked during shutdown".to_owned())
        });
        let enrichment = self.enrichment_thread.take().map_or(Ok(()), |thread| {
            thread
                .join()
                .map_err(|_| "enrichment worker panicked during shutdown".to_owned())
        });
        result.and(conversation).and(enrichment)
    }
}

/// Managed state: at most one conversation worker.
pub struct ChatService {
    operation: Mutex<()>,
    worker: Mutex<Option<Worker>>,
    cancel: Arc<AtomicBool>,
    fallback_turn_id: AtomicU64,
}

impl Default for ChatService {
    fn default() -> Self {
        Self {
            operation: Mutex::new(()),
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
    /// Stops the active workers and clears their in-memory prompt state.
    ///
    /// # Errors
    /// Returns an error if a worker thread panicked while shutting down.
    pub fn reset(&self) -> Result<(), String> {
        let _operation = self
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.reset_locked()
    }

    fn reset_locked(&self) -> Result<(), String> {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(worker) = self.lock().take() {
            worker.shutdown()?;
        }
        self.cancel.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Runs a destructive operation while submissions are excluded.
    ///
    /// # Errors
    /// Returns a worker shutdown error or the operation's error.
    pub fn with_exclusive_reset<T>(
        &self,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = self
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.reset_locked()?;
        operation()
    }
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
        let _operation = self
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let (tx, context_rx) = channel::<Command>();
        let (conversation_tx, rx) = channel::<Command>();
        let (enrichment_tx, enrichment_rx) = channel::<String>();
        let database_path = app
            .state::<AppDataLayout>()
            .data
            .join("parallel-world.sqlite3");
        let database = Database::open(&database_path).or_else(|error| {
            tracing::warn!(%error, path = %database_path.display(), "conversation history unavailable; using temporary history");
            Database::open_in_memory()
        }).map_err(|error| format!("failed to initialize conversation history: {error}"))?;
        let history = SqliteConversationHistory::new(database);
        let memory = Database::open(&database_path).or_else(|error| {
            tracing::warn!(%error, "memory database unavailable; using empty temporary context");
            Database::open_in_memory()
        }).map(SqliteMemoryStore::new).map_err(|error| format!("failed to initialize temporary memory context: {error}"))?;
        let seed = load_recent_history(&history, DEFAULT_CONVERSATION_ID, MAX_HISTORY_MESSAGES)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "conversation history restore failed; continuing without restored history");
                Vec::new()
            });
        let last_turn_id = history.max_turn_id(DEFAULT_CONVERSATION_ID).unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to restore turn sequence; starting from temporary sequence");
            None
        }).unwrap_or(0);
        let events = PersistentConversationEvents::new_with_enrichment(
            TauriConversationEvents { app },
            history,
            DEFAULT_CONVERSATION_ID,
            Some(enrichment_tx),
        );

        let context_thread = std::thread::Builder::new()
            .name("pw-memory-context".into())
            .spawn(move || run_context_worker(memory, context_rx, conversation_tx))
            .map_err(|error| format!("failed to spawn memory context worker: {error}"))?;

        let enrichment_path = database_path.clone();
        let enrichment_thread = std::thread::Builder::new()
            .name("pw-memory-enrichment".into())
            .spawn(move || run_enrichment(&enrichment_path, enrichment_rx))
            .map_err(|error| format!("failed to spawn memory enrichment worker: {error}"))?;

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
                        Command::Prepared(text, turn_id, context) => {
                            orchestrator.recover();
                            orchestrator.submit_user_text_with_context(&text, turn_id, &context);
                        }
                        Command::Submit(..) => {}
                        Command::Shutdown => break,
                    }
                }
            })
            .map_err(|error| format!("failed to spawn conversation worker: {error}"))?;

        Ok(Worker {
            tx,
            settings_fingerprint: fingerprint(settings),
            thread: Some(thread),
            context_thread: Some(context_thread),
            enrichment_thread: Some(enrichment_thread),
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
            message_id: None,
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
    use pw_application::memory::{MemoryRecord, StoredSummary};
    use pw_domain::reply::TurnTracker;
    use pw_storage::{Database, SqliteConversationHistory};

    use super::*;

    #[test]
    fn destructive_operation_excludes_competing_operation_until_commit_boundary() {
        let service = Arc::new(ChatService::default());
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first = Arc::clone(&service);
        let thread = std::thread::spawn(move || {
            first
                .with_exclusive_reset(|| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap();
        });
        entered_rx.recv().unwrap();
        let second = Arc::clone(&service);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            second.with_exclusive_reset(|| Ok(())).unwrap();
            done_tx.send(()).unwrap();
        });
        assert!(
            done_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err()
        );
        release_tx.send(()).unwrap();
        done_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        thread.join().unwrap();
        waiter.join().unwrap();
    }

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

    struct FailingMemory;
    impl MemoryStore for FailingMemory {
        fn load_summary(
            &self,
            _: &str,
        ) -> Result<Option<StoredSummary>, pw_application::PortError> {
            Err(pw_application::PortError("failed".into()))
        }
        fn upsert_summary(
            &mut self,
            _: &str,
            _: &str,
            _: i64,
            _: i64,
        ) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn upsert_memory(
            &mut self,
            _: Option<&str>,
            _: &str,
            _: i64,
        ) -> Result<i64, pw_application::PortError> {
            unreachable!()
        }
        fn update_memory(
            &mut self,
            _: i64,
            _: &str,
            _: i64,
        ) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn delete_memory(&mut self, _: i64) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn delete_summary(&mut self, _: &str) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn search(
            &self,
            _: &str,
            _: usize,
        ) -> Result<Vec<MemoryRecord>, pw_application::PortError> {
            Err(pw_application::PortError("failed".into()))
        }
    }

    struct SlowMemory;
    impl MemoryStore for SlowMemory {
        fn load_summary(
            &self,
            _: &str,
        ) -> Result<Option<StoredSummary>, pw_application::PortError> {
            Ok(None)
        }
        fn upsert_summary(
            &mut self,
            _: &str,
            _: &str,
            _: i64,
            _: i64,
        ) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn upsert_memory(
            &mut self,
            _: Option<&str>,
            _: &str,
            _: i64,
        ) -> Result<i64, pw_application::PortError> {
            unreachable!()
        }
        fn update_memory(
            &mut self,
            _: i64,
            _: &str,
            _: i64,
        ) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn delete_memory(&mut self, _: i64) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn delete_summary(&mut self, _: &str) -> Result<(), pw_application::PortError> {
            unreachable!()
        }
        fn search(
            &self,
            _: &str,
            _: usize,
        ) -> Result<Vec<MemoryRecord>, pw_application::PortError> {
            std::thread::sleep(std::time::Duration::from_millis(200));
            Ok(Vec::new())
        }
    }

    #[test]
    fn memory_lookup_failure_keeps_the_conversation_path_available() {
        assert_eq!(
            load_memory_context(&FailingMemory, "hello"),
            MemoryContext::default()
        );
    }

    #[test]
    fn completed_turn_enrichment_survives_restart_and_is_loaded_into_context() {
        let path =
            std::env::temp_dir().join(format!("pw-enrichment-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        let mut tracker = TurnTracker::new();
        for (user, assistant) in [
            ("古い質問", "古い回答"),
            ("次の質問", "次の回答"),
            ("私は猫が好きです", "覚えました"),
        ] {
            persist_completed_turn(
                &mut history,
                DEFAULT_CONVERSATION_ID,
                tracker.begin_turn(),
                user,
                assistant,
            )
            .unwrap();
        }
        drop(history);
        let (tx, rx) = channel();
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || run_enrichment(&worker_path, rx));
        tx.send("私は猫が好きです".into()).unwrap();
        drop(tx);
        worker.join().unwrap();

        let store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        let context = load_memory_context(&store, "猫");
        assert_eq!(context.memories, ["私は猫が好きです"]);
        assert!(context.summary.unwrap().contains("古い質問"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn enrichment_recovers_after_open_failure_and_rolls_summary_forward_by_message_id() {
        let root =
            std::env::temp_dir().join(format!("pw-enrichment-recovery-{}", std::process::id()));
        let path = root.join("db.sqlite3");
        let _ = std::fs::remove_dir_all(&root);
        assert!(process_enrichment_job(&path, "私は猫が好きです").is_err());
        std::fs::create_dir_all(&root).unwrap();
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        let mut tracker = TurnTracker::new();
        for i in 0..7 {
            persist_completed_turn(
                &mut history,
                DEFAULT_CONVERSATION_ID,
                tracker.begin_turn(),
                &format!("私は項目{i}が好きです"),
                "了解",
            )
            .unwrap();
        }
        drop(history);
        process_enrichment_job(&path, "私は猫が好きです").unwrap();
        let first = SqliteMemoryStore::new(Database::open(&path).unwrap())
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        process_enrichment_job(&path, "私は犬が好きです").unwrap();
        let second = SqliteMemoryStore::new(Database::open(&path).unwrap())
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        assert!(second.through_message_id >= first.through_message_id);
        assert!(second.content.chars().count() <= SUMMARY_MAX_CHARS);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unsafe_only_summary_batch_advances_cursor_then_later_safe_batch_is_processed() {
        let path =
            std::env::temp_dir().join(format!("pw-summary-cursor-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        let mut tracker = TurnTracker::new();
        for (user, assistant) in [
            ("APIキー=abc", "token=xyz"),
            ("安全な質問", "安全な回答"),
            ("最近1", "最近1回答"),
        ] {
            persist_completed_turn(
                &mut history,
                DEFAULT_CONVERSATION_ID,
                tracker.begin_turn(),
                user,
                assistant,
            )
            .unwrap();
        }
        drop(history);
        process_enrichment_job(&path, "通常発話").unwrap();
        let first = SqliteMemoryStore::new(Database::open(&path).unwrap())
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        assert!(first.content.is_empty());
        let mut history = SqliteConversationHistory::new(Database::open(&path).unwrap());
        for (user, assistant) in [("追加1", "追加1回答"), ("追加2", "追加2回答")] {
            persist_completed_turn(
                &mut history,
                DEFAULT_CONVERSATION_ID,
                tracker.begin_turn(),
                user,
                assistant,
            )
            .unwrap();
        }
        drop(history);
        process_enrichment_job(&path, "通常発話").unwrap();
        let second = SqliteMemoryStore::new(Database::open(&path).unwrap())
            .load_summary(DEFAULT_CONVERSATION_ID)
            .unwrap()
            .unwrap();
        assert!(second.through_message_id > first.through_message_id);
        assert!(second.content.contains("安全な質問"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn slow_memory_search_does_not_block_submit_sender() {
        let (tx, rx) = channel();
        let (conversation_tx, conversation_rx) = channel();
        let worker =
            std::thread::spawn(move || run_context_worker(SlowMemory, rx, conversation_tx));
        let started = std::time::Instant::now();
        tx.send(Command::Submit("query".into(), 1)).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
        assert!(matches!(
            conversation_rx.recv().unwrap(),
            Command::Prepared(_, 1, _)
        ));
        tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
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
                    Command::Prepared(_, _, _) => {}
                    Command::Shutdown => break,
                }
            }
        });
        let worker = Worker {
            tx,
            settings_fingerprint: "old".into(),
            thread: Some(thread),
            context_thread: None,
            enrichment_thread: None,
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
