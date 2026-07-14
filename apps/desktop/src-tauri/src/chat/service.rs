//! Conversation worker: owns the orchestrator, emits UI events and
//! maps control JSON onto the character.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
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
    redact_persistent_content,
};
use pw_application::recovery::{
    FeatureHealthSupervisor, HealthTransition, SystemClock, TimeJitter,
};
use pw_contracts::{
    ChatMessageEventDto, ChatRoleDto, ConversationStateEventDto, LlmSettingsDto,
    RUNTIME_HEALTH_EVENT, RuntimeHealthEventDto, SCHEMA_VERSION,
};
use pw_domain::conversation::ConversationState;
use pw_domain::reply::{ReplyControl, TurnId};
use pw_domain::runtime_health::{FailureCode, RuntimeFailure, RuntimeFeature};
use pw_llm::{LlmClientConfig, OpenAiCompatClient};
use pw_platform::paths::AppDataLayout;
use pw_storage::{Database, SqliteConversationHistory, SqliteMemoryStore};
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime};

use crate::character::CharacterCapabilities;
use crate::commands::character::{CharacterState, EXPRESSION_EVENT, MOTION_EVENT};
use crate::diagnostics::QueueMetrics;

pub const MESSAGE_EVENT: &str = "chat-message";
pub const STATE_EVENT: &str = "conversation-state";
const CHARACTER_WEBVIEW: &str = "character";

/// Kept messages of context (user + assistant combined).
const MAX_HISTORY_MESSAGES: usize = 20;
const DEFAULT_CONVERSATION_ID: &str = "default";
const SUBMIT_QUEUE_CAPACITY: usize = 8;
const CONVERSATION_QUEUE_CAPACITY: usize = 8;
const ENRICHMENT_QUEUE_CAPACITY: usize = 1;
const ENRICHMENT_PENDING_CAPACITY: usize = 64;
const ADAPTER_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(2);

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
#[derive(Clone)]
struct EnrichmentSender {
    wake: SyncSender<()>,
    pending: Arc<Mutex<Option<Vec<String>>>>,
    metrics: Arc<QueueMetrics>,
}

impl EnrichmentSender {
    fn replace_latest(&self, text: String) -> Result<(), ()> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let batch = pending.get_or_insert_with(Vec::new);
        if batch.iter().any(|existing| existing == &text) {
            self.metrics.coalesced();
            return Ok(());
        } else if batch.len() < ENRICHMENT_PENDING_CAPACITY {
            batch.push(text);
            self.metrics.enqueued();
        } else {
            self.metrics.dropped();
            return Err(());
        }
        drop(pending);
        match self.wake.try_send(()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(())) => {
                self.metrics.busy();
                Ok(())
            }
            Err(TrySendError::Disconnected(())) => {
                let discarded = self
                    .pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .map_or(0, |batch| batch.len());
                for _ in 0..discarded {
                    self.metrics.dequeued();
                    self.metrics.dropped();
                }
                Err(())
            }
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn run_enrichment(
    database_path: &Path,
    rx: Receiver<()>,
    pending: Arc<Mutex<Option<Vec<String>>>>,
    metrics: Arc<QueueMetrics>,
) {
    while rx.recv().is_ok() {
        let Some(user_texts) = pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            continue;
        };
        for user_text in user_texts {
            metrics.dequeued();
            if let Err(error) = process_enrichment_job(database_path, &user_text) {
                tracing::warn!(%error, "memory enrichment job failed; worker remains available");
            }
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn run_context_worker<M: MemoryStore>(
    memory: M,
    rx: Receiver<Command>,
    conversation_tx: SyncSender<Command>,
    submit_metrics: Arc<QueueMetrics>,
    context_metrics: Arc<QueueMetrics>,
    conversation_metrics: Arc<QueueMetrics>,
) {
    while let Ok(command) = rx.recv() {
        submit_metrics.dequeued();
        match command {
            Command::Submit(text, turn_id) => {
                context_metrics.enqueued();
                let context = load_memory_context(&memory, &text);
                context_metrics.dequeued();
                // Account for the distinct prepared-conversation queue.
                // Increment before send so a fast consumer cannot underflow depth.
                conversation_metrics.enqueued();
                if conversation_tx
                    .send(Command::Prepared(text, turn_id, context))
                    .is_err()
                {
                    conversation_metrics.dequeued();
                    conversation_metrics.dropped();
                    break;
                }
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
        user_content: redact_persistent_content(user_text),
        assistant_content: redact_persistent_content(assistant_text),
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
    enrichment: Option<EnrichmentSender>,
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
        enrichment: Option<EnrichmentSender>,
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
                && enrichment.replace_latest(user_text.clone()).is_err()
            {
                tracing::warn!(
                    "memory enrichment worker unavailable; conversation remains available"
                );
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
}

fn enqueue_submit(
    tx: &SyncSender<Command>,
    metrics: &QueueMetrics,
    text: String,
    turn_id: u64,
) -> Result<(), String> {
    metrics.enqueued();
    tx.try_send(Command::Submit(text, turn_id))
        .map_err(|error| match error {
            TrySendError::Full(_) => {
                metrics.dequeued();
                metrics.busy();
                metrics.dropped();
                "conversation is busy; please retry".to_owned()
            }
            TrySendError::Disconnected(_) => {
                metrics.dequeued();
                metrics.dropped();
                "conversation worker is not available".to_owned()
            }
        })
}

struct Worker {
    tx: SyncSender<Command>,
    settings_fingerprint: String,
    thread: Option<std::thread::JoinHandle<()>>,
    context_thread: Option<std::thread::JoinHandle<()>>,
    enrichment_thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    fn shutdown(mut self) -> Result<(), String> {
        // Dropping the sole submit sender disconnects the bounded queue even when full.
        // The context worker then drops its conversation sender, cascading shutdown.
        drop(self.tx);
        let result = join_worker(self.context_thread.take(), "context");
        let conversation = join_worker(self.thread.take(), "conversation");
        let enrichment = join_worker(self.enrichment_thread.take(), "enrichment");
        result.and(conversation).and(enrichment)
    }
}

fn join_worker(thread: Option<std::thread::JoinHandle<()>>, name: &str) -> Result<(), String> {
    let Some(thread) = thread else {
        return Ok(());
    };
    // Adapter I/O has a finite total timeout. Always join so a reset cannot
    // leave stale workers mutating history after the replacement starts.
    thread
        .join()
        .map_err(|_| format!("{name} worker panicked during shutdown"))
}

/// Managed state: at most one conversation worker.
pub struct ChatService {
    operation: Mutex<()>,
    worker: Mutex<Option<Worker>>,
    cancel: Arc<AtomicBool>,
    fallback_turn_id: AtomicU64,
    health: Arc<Mutex<FeatureHealthSupervisor<SystemClock, TimeJitter>>>,
    submit_metrics: Arc<QueueMetrics>,
    context_metrics: Arc<QueueMetrics>,
    conversation_metrics: Arc<QueueMetrics>,
    enrichment_metrics: Arc<QueueMetrics>,
}

impl Default for ChatService {
    fn default() -> Self {
        Self {
            operation: Mutex::new(()),
            worker: Mutex::new(None),
            cancel: Arc::new(AtomicBool::new(false)),
            fallback_turn_id: AtomicU64::new(1_u64 << 63),
            health: Arc::new(Mutex::new(FeatureHealthSupervisor::new(
                RuntimeFeature::LanguageModel,
                SystemClock,
                TimeJitter::default(),
            ))),
            submit_metrics: Arc::new(QueueMetrics::new("chat_submit", SUBMIT_QUEUE_CAPACITY)),
            context_metrics: Arc::new(QueueMetrics::new("chat_context", SUBMIT_QUEUE_CAPACITY)),
            conversation_metrics: Arc::new(QueueMetrics::new(
                "chat_conversation",
                CONVERSATION_QUEUE_CAPACITY,
            )),
            enrichment_metrics: Arc::new(QueueMetrics::new(
                "chat_enrichment",
                ENRICHMENT_PENDING_CAPACITY,
            )),
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
    /// Clears the application-owned LLM circuit.
    ///
    /// # Errors
    /// Returns an error when the circuit is not open.
    pub fn rearm(&self) -> Result<HealthTransition, &'static str> {
        self.health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .rearm()
    }
    #[must_use]
    pub fn health_snapshot(&self) -> RuntimeHealthEventDto {
        let health = self
            .health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut event = RuntimeHealthEventDto::from((health.health(), health.attempts()));
        event.circuit_open = health.circuit_open();
        event
    }
    pub fn queue_metrics(&self) -> Vec<pw_contracts::QueueMetricsDto> {
        [
            &self.submit_metrics,
            &self.context_metrics,
            &self.conversation_metrics,
            &self.enrichment_metrics,
        ]
        .into_iter()
        .map(|metrics| metrics.snapshot())
        .collect()
    }
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
        let result = self.lock().take().map_or(Ok(()), Worker::shutdown);
        self.cancel.store(false, Ordering::SeqCst);
        result
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
        if !self
            .health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .can_attempt()
        {
            return Err("language model is recovering; retry after the backoff period".to_owned());
        }
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
        enqueue_submit(&worker.tx, &self.submit_metrics, text, turn_id)
    }

    /// Cancels the in-flight turn (生成途中で停止).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    #[allow(clippy::too_many_lines)]
    fn start_worker<R: Runtime>(
        &self,
        app: AppHandle<R>,
        settings: &LlmSettingsDto,
    ) -> Result<Worker, String> {
        let llm = OpenAiCompatClient::new(LlmClientConfig {
            base_url: settings.base_url.clone(),
            model: settings.model.clone(),
            allow_remote: settings.allow_remote,
            timeout: ADAPTER_TIMEOUT,
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
        let (tx, context_rx) = sync_channel::<Command>(SUBMIT_QUEUE_CAPACITY);
        let (conversation_tx, rx) = sync_channel::<Command>(CONVERSATION_QUEUE_CAPACITY);
        let (enrichment_wake, enrichment_rx) = sync_channel::<()>(ENRICHMENT_QUEUE_CAPACITY);
        let enrichment_pending = Arc::new(Mutex::new(None));
        let enrichment_tx = EnrichmentSender {
            wake: enrichment_wake,
            pending: Arc::clone(&enrichment_pending),
            metrics: Arc::clone(&self.enrichment_metrics),
        };
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
            TauriConversationEvents {
                runtime: AppConversationEventRuntime { app },
                health: Arc::clone(&self.health),
            },
            history,
            DEFAULT_CONVERSATION_ID,
            Some(enrichment_tx),
        );

        let context_thread = std::thread::Builder::new()
            .name("pw-memory-context".into())
            .spawn({
                let submit_metrics = Arc::clone(&self.submit_metrics);
                let context_metrics = Arc::clone(&self.context_metrics);
                let conversation_metrics = Arc::clone(&self.conversation_metrics);
                move || {
                    run_context_worker(
                        memory,
                        context_rx,
                        conversation_tx,
                        submit_metrics,
                        context_metrics,
                        conversation_metrics,
                    );
                }
            })
            .map_err(|error| format!("failed to spawn memory context worker: {error}"))?;

        let enrichment_path = database_path.clone();
        let enrichment_thread = std::thread::Builder::new()
            .name("pw-memory-enrichment".into())
            .spawn({
                let metrics = Arc::clone(&self.enrichment_metrics);
                move || run_enrichment(&enrichment_path, enrichment_rx, enrichment_pending, metrics)
            })
            .map_err(|error| format!("failed to spawn memory enrichment worker: {error}"))?;

        let conversation_metrics_for_worker = Arc::clone(&self.conversation_metrics);
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
                    conversation_metrics_for_worker.dequeued();
                    match command {
                        Command::Prepared(text, turn_id, context) => {
                            orchestrator.recover();
                            orchestrator.submit_user_text_with_context(&text, turn_id, &context);
                        }
                        Command::Submit(..) => {}
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
fn character_instruction(base: &str, capabilities: &CharacterCapabilities) -> String {
    let mut lines = vec![base.to_owned()];
    if !capabilities.expressions.is_empty() {
        lines.push(format!(
            "利用できる表情(emotion): {}",
            capabilities.expressions.join(", ")
        ));
    }
    if !capabilities.motions.is_empty() {
        lines.push(format!(
            "利用できるモーション(motion): {}",
            capabilities.motions.join(", ")
        ));
    }
    lines.join("\n")
}

fn with_character_abilities<R: Runtime>(app: &AppHandle<R>, base: String) -> String {
    let state = app.state::<CharacterState>();
    match state.manifest_summary() {
        Some(capabilities) => character_instruction(&base, &capabilities),
        _ => base,
    }
}

fn dispatch_character_control(
    capabilities: &CharacterCapabilities,
    renderer: &str,
    control: &ReplyControl,
    mut emit: impl FnMut(&'static str, &str),
) {
    if let Some(emotion) = &control.emotion {
        if capabilities
            .expressions
            .iter()
            .any(|known| known == emotion)
        {
            emit(EXPRESSION_EVENT, emotion);
        } else {
            tracing::warn!(
                renderer = %renderer,
                control = "emotion",
                name = %emotion,
                "ignoring unsupported character control"
            );
        }
    }
    if let Some(motion) = &control.motion {
        if capabilities.motions.iter().any(|known| known == motion) {
            emit(MOTION_EVENT, motion);
        } else {
            tracing::warn!(
                renderer = %renderer,
                control = "motion",
                name = %motion,
                "ignoring unsupported character control"
            );
        }
    }
}

trait ConversationEventRuntime: Send {
    fn character_context(&self) -> Option<crate::commands::character::CharacterControlContext>;
    fn emit_to_webview(
        &self,
        target: &'static str,
        event: &'static str,
        name: &str,
    ) -> Result<(), String>;
    fn emit_chat_message(&self, payload: ChatMessageEventDto);
    fn emit_conversation_state(&self, payload: ConversationStateEventDto);
    fn emit_runtime_health(&self, payload: RuntimeHealthEventDto);
    fn enqueue_speech(&self, turn: TurnId, sentence: &str);
}

struct AppConversationEventRuntime<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> ConversationEventRuntime for AppConversationEventRuntime<R> {
    fn character_context(&self) -> Option<crate::commands::character::CharacterControlContext> {
        self.app.state::<CharacterState>().control_context()
    }

    fn emit_to_webview(
        &self,
        target: &'static str,
        event: &'static str,
        name: &str,
    ) -> Result<(), String> {
        self.app
            .emit_to(EventTarget::webview_window(target), event, name)
            .map_err(|error| error.to_string())
    }

    fn emit_chat_message(&self, payload: ChatMessageEventDto) {
        let _ = self.app.emit(MESSAGE_EVENT, payload);
    }

    fn emit_conversation_state(&self, payload: ConversationStateEventDto) {
        let _ = self.app.emit(STATE_EVENT, payload);
    }

    fn emit_runtime_health(&self, payload: RuntimeHealthEventDto) {
        let _ = self.app.emit(RUNTIME_HEALTH_EVENT, payload);
    }

    fn enqueue_speech(&self, turn: TurnId, sentence: &str) {
        self.app
            .state::<crate::tts::TtsService>()
            .enqueue(&self.app, turn, sentence);
    }
}

struct TauriConversationEvents<A: ConversationEventRuntime> {
    runtime: A,
    health: Arc<Mutex<FeatureHealthSupervisor<SystemClock, TimeJitter>>>,
}

impl<A: ConversationEventRuntime> TauriConversationEvents<A> {
    fn emit_health(&self, healthy: bool) {
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transition = if healthy {
            health.record_success()
        } else {
            match health.record_failure(RuntimeFailure::transient(FailureCode::Unavailable)) {
                pw_application::recovery::HealthUpdate::Changed {
                    health, attempts, ..
                } => HealthTransition::Changed { health, attempts },
                pw_application::recovery::HealthUpdate::Unchanged { .. } => {
                    HealthTransition::Unchanged
                }
            }
        };
        if let HealthTransition::Changed { health, attempts } = transition {
            self.runtime
                .emit_runtime_health(RuntimeHealthEventDto::from((&health, attempts)));
        }
    }
    fn emit_message(&self, turn: TurnId, role: ChatRoleDto, text: &str) {
        let payload = ChatMessageEventDto {
            schema_version: SCHEMA_VERSION,
            turn_id: turn.value(),
            message_id: None,
            role,
            text: text.to_owned(),
        };
        // Single broadcast: see the event conventions note.
        self.runtime.emit_chat_message(payload);
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
        self.runtime.emit_conversation_state(payload);
    }
}

impl<A: ConversationEventRuntime> ConversationEvents for TauriConversationEvents<A> {
    fn on_state(&self, state: ConversationState) {
        self.emit_state(state, None);
    }

    fn on_user_message(&self, turn: TurnId, text: &str) {
        self.emit_message(turn, ChatRoleDto::User, text);
    }

    fn on_control(&self, _turn: TurnId, control: &ReplyControl) {
        let Some(context) = self.runtime.character_context() else {
            if let Some(emotion) = &control.emotion {
                tracing::warn!(
                    renderer = "unavailable",
                    control = "emotion",
                    name = %emotion,
                    "ignoring character control before capabilities are loaded"
                );
            }
            if let Some(motion) = &control.motion {
                tracing::warn!(
                    renderer = "unavailable",
                    control = "motion",
                    name = %motion,
                    "ignoring character control before capabilities are loaded"
                );
            }
            return;
        };
        dispatch_character_control(
            &context.capabilities,
            context.renderer,
            control,
            |event, name| {
                if let Err(error) = self.runtime.emit_to_webview(CHARACTER_WEBVIEW, event, name) {
                    tracing::warn!(
                        %error,
                        renderer = %context.renderer,
                        control = %event,
                        name = %name,
                        "failed to emit character control"
                    );
                }
            },
        );
    }

    fn on_sentence(&self, turn: TurnId, sentence: &str) {
        self.emit_message(turn, ChatRoleDto::Assistant, sentence);
        // Sentence-level read-ahead: synthesis of this sentence runs
        // while earlier ones are still playing (基本設計 8章).
        self.runtime.enqueue_speech(turn, sentence);
    }

    fn on_reply_complete(&self, _turn: TurnId, _speech_text: &str) {
        self.emit_health(true);
    }

    fn on_cancelled(&self, _turn: TurnId) {}

    fn on_error(&self, _turn: TurnId, message: &str) {
        self.emit_health(false);
        self.emit_state(ConversationState::LlmUnavailable, Some(message.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;

    use super::*;
    use pw_application::PortError;
    use pw_application::conversation::LlmClient;
    use pw_application::history::ConversationHistory;
    use pw_application::memory::{MemoryRecord, StoredSummary};
    use pw_contracts::MotionGroupDto;
    use pw_domain::reply::TurnTracker;
    use pw_storage::{Database, SqliteConversationHistory};

    #[test]
    fn production_llm_timeout_allows_local_streaming_inference() {
        assert!(ADAPTER_TIMEOUT >= std::time::Duration::from_secs(30));
    }

    fn static_capabilities() -> crate::character::CharacterCapabilities {
        crate::character::CharacterCapabilities {
            expressions: vec!["neutral".into(), "happy".into()],
            motions: Vec::new(),
        }
    }

    fn live2d_capabilities() -> crate::character::CharacterCapabilities {
        crate::character::CharacterCapabilities {
            expressions: vec!["neutral".into(), "happy".into()],
            motions: vec!["Idle".into(), "Tap".into()],
        }
    }

    #[test]
    fn static_character_instruction_lists_expressions_without_motion_line() {
        let instruction = character_instruction("base", &static_capabilities());

        assert!(instruction.contains("利用できる表情(emotion): neutral, happy"));
        assert!(!instruction.contains("利用できるモーション(motion):"));
    }

    #[test]
    fn live2d_character_instruction_lists_expressions_and_motions() {
        let instruction = character_instruction("base", &live2d_capabilities());

        assert!(instruction.contains("利用できる表情(emotion): neutral, happy"));
        assert!(instruction.contains("利用できるモーション(motion): Idle, Tap"));
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedCharacterEvent {
        target: String,
        event: String,
        name: String,
    }

    struct RecordingConversationRuntime {
        state: Arc<CharacterState>,
        attempts: Arc<Mutex<Vec<RecordedCharacterEvent>>>,
        fail_emit: bool,
        speech: Arc<Mutex<Vec<(TurnId, String)>>>,
    }

    impl ConversationEventRuntime for RecordingConversationRuntime {
        fn character_context(&self) -> Option<crate::commands::character::CharacterControlContext> {
            self.state.control_context()
        }

        fn emit_to_webview(
            &self,
            target: &'static str,
            event: &'static str,
            name: &str,
        ) -> Result<(), String> {
            self.attempts.lock().unwrap().push(RecordedCharacterEvent {
                target: target.into(),
                event: event.into(),
                name: name.into(),
            });
            if self.fail_emit {
                Err("injected character event failure".into())
            } else {
                Ok(())
            }
        }

        fn emit_chat_message(&self, _payload: ChatMessageEventDto) {}

        fn emit_conversation_state(&self, _payload: ConversationStateEventDto) {}

        fn emit_runtime_health(&self, _payload: RuntimeHealthEventDto) {}

        fn enqueue_speech(&self, turn: TurnId, sentence: &str) {
            self.speech.lock().unwrap().push((turn, sentence.into()));
        }
    }

    fn live2d_character() -> crate::character::ResolvedCharacter {
        crate::character::ResolvedCharacter {
            id: "live2d-test".into(),
            display_name: "Live2D Test".into(),
            profile_root: PathBuf::from("C:/characters/live2d-test"),
            renderer: crate::character::ResolvedRenderer::Live2d {
                model_path: PathBuf::from("C:/characters/live2d-test/model.model3.json"),
                default_expression: Some("neutral".into()),
                expressions: vec!["neutral".into(), "happy".into()],
                motion_groups: vec![MotionGroupDto {
                    name: "Tap".into(),
                    motion_count: 1,
                }],
            },
        }
    }

    fn static_character() -> crate::character::ResolvedCharacter {
        crate::character::ResolvedCharacter {
            id: "static-test".into(),
            display_name: "Static Test".into(),
            profile_root: PathBuf::from("C:/characters/static-test"),
            renderer: crate::character::ResolvedRenderer::StaticImage {
                default_expression: "neutral".into(),
                expressions: vec![
                    crate::character::ResolvedStaticExpression {
                        name: "neutral".into(),
                        image_path: PathBuf::from("C:/characters/static-test/neutral.png"),
                    },
                    crate::character::ResolvedStaticExpression {
                        name: "happy".into(),
                        image_path: PathBuf::from("C:/characters/static-test/happy.png"),
                    },
                ],
                width: 512,
                height: 1024,
            },
        }
    }

    fn test_health() -> Arc<Mutex<FeatureHealthSupervisor<SystemClock, TimeJitter>>> {
        Arc::new(Mutex::new(FeatureHealthSupervisor::new(
            RuntimeFeature::LanguageModel,
            SystemClock,
            TimeJitter::default(),
        )))
    }

    fn recording_events(
        state: Arc<CharacterState>,
        attempts: Arc<Mutex<Vec<RecordedCharacterEvent>>>,
        fail_emit: bool,
        speech: Arc<Mutex<Vec<(TurnId, String)>>>,
    ) -> TauriConversationEvents<RecordingConversationRuntime> {
        TauriConversationEvents {
            runtime: RecordingConversationRuntime {
                state,
                attempts,
                fail_emit,
                speech,
            },
            health: test_health(),
        }
    }

    #[test]
    fn unknown_emotion_uses_cached_state_and_sends_no_webview_event() {
        let state = Arc::new(CharacterState::default());
        state.cache_manifest(static_character()).unwrap();
        let before = state.control_context();
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let events = recording_events(
            Arc::clone(&state),
            Arc::clone(&attempts),
            false,
            Arc::new(Mutex::new(Vec::new())),
        );
        let turn = TurnTracker::new().begin_turn();

        events.on_control(
            turn,
            &ReplyControl {
                emotion: Some("surprised".into()),
                intensity: None,
                motion: None,
            },
        );

        assert!(attempts.lock().unwrap().is_empty());
        assert_eq!(state.control_context(), before);
    }

    #[test]
    fn static_motion_uses_cached_state_and_sends_no_webview_event() {
        let state = Arc::new(CharacterState::default());
        state.cache_manifest(static_character()).unwrap();
        let before = state.control_context();
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let events = recording_events(
            Arc::clone(&state),
            Arc::clone(&attempts),
            false,
            Arc::new(Mutex::new(Vec::new())),
        );
        let turn = TurnTracker::new().begin_turn();

        events.on_control(
            turn,
            &ReplyControl {
                emotion: None,
                intensity: None,
                motion: Some("Tap".into()),
            },
        );

        assert!(attempts.lock().unwrap().is_empty());
        assert_eq!(state.control_context(), before);
    }

    #[test]
    fn valid_live2d_controls_target_only_the_character_webview() {
        let state = Arc::new(CharacterState::default());
        state.cache_manifest(live2d_character()).unwrap();
        let before = state.control_context();
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let events = recording_events(
            Arc::clone(&state),
            Arc::clone(&attempts),
            false,
            Arc::new(Mutex::new(Vec::new())),
        );
        let turn = TurnTracker::new().begin_turn();

        events.on_control(
            turn,
            &ReplyControl {
                emotion: Some("happy".into()),
                intensity: Some(0.75),
                motion: Some("Tap".into()),
            },
        );

        assert_eq!(
            *attempts.lock().unwrap(),
            [
                RecordedCharacterEvent {
                    target: "character".into(),
                    event: EXPRESSION_EVENT.into(),
                    name: "happy".into(),
                },
                RecordedCharacterEvent {
                    target: "character".into(),
                    event: MOTION_EVENT.into(),
                    name: "Tap".into(),
                },
            ]
        );
        assert_eq!(state.control_context(), before);
    }

    struct ScriptedLlm(&'static str);

    impl LlmClient for ScriptedLlm {
        fn stream_chat(
            &mut self,
            _messages: &[ChatMessage],
            _cancel: &AtomicBool,
            on_delta: &mut dyn FnMut(&str),
        ) -> Result<(), PortError> {
            on_delta(self.0);
            Ok(())
        }
    }

    #[test]
    fn character_emit_failure_does_not_stop_sentence_or_tts_enqueue() {
        let state = Arc::new(CharacterState::default());
        state.cache_manifest(live2d_character()).unwrap();
        let before = state.control_context();
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let speech = Arc::new(Mutex::new(Vec::new()));
        let events = recording_events(
            Arc::clone(&state),
            Arc::clone(&attempts),
            true,
            Arc::clone(&speech),
        );
        let mut orchestrator = ConversationOrchestrator::new(
            OrchestratorConfig {
                prompt: PromptBuilder {
                    system_rules: "rules".into(),
                    character_prompt: "character".into(),
                },
                max_history_messages: 4,
                strip_emoji: true,
            },
            ScriptedLlm("{\"emotion\":\"happy\"}\n続きます。"),
            events,
            Arc::new(AtomicBool::new(false)),
        );

        orchestrator.submit_user_text("話して");

        assert_eq!(attempts.lock().unwrap().len(), 1);
        assert_eq!(attempts.lock().unwrap()[0].target, "character");
        assert_eq!(speech.lock().unwrap().len(), 1);
        assert_eq!(speech.lock().unwrap()[0].1, "続きます。");
        assert_eq!(state.control_context(), before);
    }

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

    #[test]
    fn shutdown_failure_always_restores_cancel_flag() {
        let service = ChatService::default();
        let (tx, _rx) = sync_channel(8);
        *service.lock() = Some(Worker {
            tx,
            settings_fingerprint: String::new(),
            thread: Some(std::thread::spawn(|| panic!("injected"))),
            context_thread: None,
            enrichment_thread: None,
        });
        assert!(service.reset().is_err());
        assert!(!service.cancel.load(Ordering::SeqCst));
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
        let (tx, rx) = sync_channel(8);
        let pending = Arc::new(Mutex::new(Some(vec!["私は猫が好きです".to_owned()])));
        let worker_pending = Arc::clone(&pending);
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            run_enrichment(
                &worker_path,
                rx,
                worker_pending,
                Arc::new(QueueMetrics::new("test_enrichment", 8)),
            );
        });
        tx.send(()).unwrap();
        drop(tx);
        worker.join().unwrap();

        let store = SqliteMemoryStore::new(Database::open(&path).unwrap());
        let context = load_memory_context(&store, "猫");
        assert_eq!(context.memories, ["私は猫が好きです"]);
        assert!(context.summary.unwrap().contains("古い質問"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn phase5_restart_delete_and_export_acceptance() {
        let root =
            std::env::temp_dir().join(format!("pw-phase5-acceptance-{}", std::process::id()));
        let database_path = root.join("parallel-world.sqlite3");
        let snapshot_path = root.join("export.sqlite3");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let mut history = SqliteConversationHistory::new(Database::open(&database_path).unwrap());
        let mut tracker = TurnTracker::new();
        persist_completed_turn(
            &mut history,
            DEFAULT_CONVERSATION_ID,
            tracker.begin_turn(),
            "猫の話を覚えて",
            "覚えました",
        )
        .unwrap();
        drop(history);
        process_enrichment_job(&database_path, "私は猫が好き").unwrap();

        let reopened_history =
            SqliteConversationHistory::new(Database::open(&database_path).unwrap());
        let restored = load_recent_history(
            &reopened_history,
            DEFAULT_CONVERSATION_ID,
            MAX_HISTORY_MESSAGES,
        )
        .unwrap();
        assert_eq!(restored.len(), 2);
        let store = SqliteMemoryStore::new(Database::open(&database_path).unwrap());
        let context = load_memory_context(&store, "猫");
        let prompt = PromptBuilder {
            system_rules: "rules".into(),
            character_prompt: "character".into(),
        }
        .build_with_context(&restored, "次の質問", &context);
        assert!(prompt.iter().any(|message| message.content.contains("猫")));

        Database::open(&database_path)
            .unwrap()
            .backup_to(&snapshot_path)
            .unwrap();
        let snapshot = SqliteConversationHistory::new(Database::open(&snapshot_path).unwrap());
        assert_eq!(
            snapshot
                .list_messages(DEFAULT_CONVERSATION_ID)
                .unwrap()
                .len(),
            2
        );

        let mut history = SqliteConversationHistory::new(Database::open(&database_path).unwrap());
        history
            .delete_conversation(DEFAULT_CONVERSATION_ID)
            .unwrap();
        drop(history);
        let mut memory = SqliteMemoryStore::new(Database::open(&database_path).unwrap());
        memory.delete_summary(DEFAULT_CONVERSATION_ID).unwrap();
        for record in memory.search("猫", DEFAULT_MEMORY_LIMIT).unwrap() {
            memory.delete_memory(record.id).unwrap();
        }
        drop(memory);
        let reopened = SqliteConversationHistory::new(Database::open(&database_path).unwrap());
        assert!(
            reopened
                .list_messages(DEFAULT_CONVERSATION_ID)
                .unwrap()
                .is_empty()
        );
        let empty_context = load_memory_context(
            &SqliteMemoryStore::new(Database::open(&database_path).unwrap()),
            "猫",
        );
        assert!(empty_context.summary.is_none());
        assert!(empty_context.memories.is_empty());
        let next_prompt = PromptBuilder {
            system_rules: "rules".into(),
            character_prompt: "character".into(),
        }
        .build_with_context(&[], "新しい会話", &empty_context);
        assert!(
            !next_prompt
                .iter()
                .any(|message| message.content.contains("覚えて"))
        );

        let _ = std::fs::remove_dir_all(root);
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
        assert!(first.content.contains("[REDACTED]"));
        assert!(!first.content.contains("abc"));
        assert!(!first.content.contains("xyz"));
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
        let (tx, rx) = sync_channel(8);
        let (conversation_tx, conversation_rx) = sync_channel(8);
        let worker = std::thread::spawn(move || {
            run_context_worker(
                SlowMemory,
                rx,
                conversation_tx,
                Arc::new(QueueMetrics::new("test_submit", 8)),
                Arc::new(QueueMetrics::new("test_context", 8)),
                Arc::new(QueueMetrics::new("test_conversation", 8)),
            );
        });
        let started = std::time::Instant::now();
        tx.send(Command::Submit("query".into(), 1)).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
        assert!(matches!(
            conversation_rx.recv().unwrap(),
            Command::Prepared(_, 1, _)
        ));
        drop(tx);
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
    fn persisted_history_and_export_never_contain_raw_credentials() {
        let root = std::env::temp_dir().join(format!("pw-redacted-export-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source.sqlite3");
        let export = root.join("export.sqlite3");
        let mut history = SqliteConversationHistory::new(Database::open(&source).unwrap());
        let mut tracker = TurnTracker::new();
        persist_completed_turn(
            &mut history,
            DEFAULT_CONVERSATION_ID,
            tracker.begin_turn(),
            "keep token=\"raw Secret123\", secret value rawValue789; APIキーは japaneseRaw123, next",
            "Authorization: Basic rawAssistant456;",
        )
        .unwrap();
        drop(history);
        Database::open(&source).unwrap().backup_to(&export).unwrap();
        for path in [&source, &export] {
            let history = SqliteConversationHistory::new(Database::open(path).unwrap());
            let joined = history
                .list_messages(DEFAULT_CONVERSATION_ID)
                .unwrap()
                .into_iter()
                .map(|m| m.content)
                .collect::<Vec<_>>()
                .join(" ");
            assert!(joined.contains("[REDACTED]"));
            assert!(!joined.contains("rawSecret123"));
            assert!(!joined.contains("raw Secret123"));
            assert!(!joined.contains("rawValue789"));
            assert!(!joined.contains("japaneseRaw123"));
            assert!(!joined.contains("rawAssistant456"));
        }
        let _ = std::fs::remove_dir_all(root);
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
        let (tx, rx) = sync_channel(8);
        let persisted = Arc::new(AtomicBool::new(false));
        let worker_persisted = Arc::clone(&persisted);
        let thread = std::thread::spawn(move || {
            while let Ok(command) = rx.recv() {
                match command {
                    Command::Submit(_, _) => worker_persisted.store(true, Ordering::SeqCst),
                    Command::Prepared(_, _, _) => {}
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

    #[test]
    fn bounded_submit_queue_returns_busy_without_silently_dropping_user_text() {
        let (tx, rx) = sync_channel(1);
        let metrics = QueueMetrics::new("test_submit", 1);
        enqueue_submit(&tx, &metrics, "first".to_owned(), 1).unwrap();

        let error = enqueue_submit(&tx, &metrics, "second".to_owned(), 2).unwrap_err();

        assert_eq!(error, "conversation is busy; please retry");
        let Command::Submit(text, turn_id) = rx.recv().unwrap() else {
            panic!("expected queued user submission");
        };
        assert_eq!((text.as_str(), turn_id), ("first", 1));
    }

    #[test]
    fn enrichment_coalescing_keeps_all_distinct_pending_facts() {
        let (wake, rx) = sync_channel(1);
        let pending = Arc::new(Mutex::new(None));
        let sender = EnrichmentSender {
            wake,
            pending: Arc::clone(&pending),
            metrics: Arc::new(QueueMetrics::new("test_enrichment", 1)),
        };
        sender.replace_latest("old".into()).unwrap();
        sender.replace_latest("latest".into()).unwrap();
        rx.recv().unwrap();
        assert_eq!(
            pending.lock().unwrap().take(),
            Some(vec!["old".to_owned(), "latest".to_owned()])
        );
        assert!(
            rx.try_recv().is_err(),
            "one wake coalesces multiple updates"
        );
    }

    #[test]
    fn enrichment_metrics_track_the_bounded_pending_work_not_the_wake_token() {
        let (wake, _rx) = sync_channel(1);
        let metrics = Arc::new(QueueMetrics::new(
            "test_enrichment",
            ENRICHMENT_PENDING_CAPACITY,
        ));
        let sender = EnrichmentSender {
            wake,
            pending: Arc::new(Mutex::new(None)),
            metrics: Arc::clone(&metrics),
        };
        for index in 0..ENRICHMENT_PENDING_CAPACITY {
            sender.replace_latest(format!("fact-{index}")).unwrap();
        }
        assert!(sender.replace_latest("overflow".into()).is_err());
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.depth, ENRICHMENT_PENDING_CAPACITY);
        assert_eq!(snapshot.capacity, ENRICHMENT_PENDING_CAPACITY);
        assert_eq!(snapshot.dropped, 1);
    }

    #[test]
    fn shutdown_does_not_block_sending_control_into_a_full_queue() {
        let (tx, rx) = sync_channel(1);
        tx.send(Command::Submit("queued".into(), 1)).unwrap();
        let context = std::thread::spawn(move || while rx.recv().is_ok() {});
        let worker = Worker {
            tx,
            settings_fingerprint: String::new(),
            thread: None,
            context_thread: Some(context),
            enrichment_thread: None,
        };
        let started = std::time::Instant::now();
        worker.shutdown().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
