//! TTS worker: synthesizes sentences ahead of playback and streams
//! `speech-audio` items to the character window.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::diagnostics::QueueMetrics;
use pw_application::recovery::{
    FeatureHealthSupervisor, HealthTransition, SystemClock, TimeJitter,
};
use pw_application::speech_synthesis::{SpeechAudioSink, SpeechSynthesisQueue};
use pw_contracts::{
    RUNTIME_HEALTH_EVENT, RuntimeHealthEventDto, SCHEMA_VERSION, SpeechAudioEventDto,
    SpeechStopEventDto, TtsEngineKind, TtsSettingsDto, TtsStateEventDto,
};
use pw_domain::reply::TurnId;
use pw_domain::runtime_health::{FailureCode, RuntimeFailure, RuntimeFeature};
use pw_platform::paths::AppDataLayout;
use pw_tts::{
    AivisSpeechClient, CachedSpeechSynthesizer, DEFAULT_MAX_ENTRIES, EngineClient,
    IrodoriTtsClient, SynthesisParams, TtsClientConfig, WavCache,
};
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime};

/// Sent to the character window only (single-window event).
pub const AUDIO_EVENT: &str = "speech-audio";
/// Sent to the character window only (single-window event).
pub const STOP_EVENT: &str = "speech-stop";
/// Diagnostics broadcast (degraded-state banner).
pub const STATE_EVENT: &str = "tts-state";

const CHARACTER_WINDOW: &str = "character";
const TTS_QUEUE_CAPACITY: usize = 8;
// AivisSpeech can take more than five seconds for a long unsplit sentence,
// especially during model warm-up. Keep synthesis finite without mistaking a
// slow but healthy inference for an adapter failure.
const ADAPTER_TIMEOUT: Duration = Duration::from_secs(30);
// Queue admission has a separate, shorter bound so one stalled adapter cannot
// hold the conversation producer for the full synthesis timeout.
const BACKPRESSURE_TIMEOUT: Duration = Duration::from_secs(5);
const BACKPRESSURE_POLL_INTERVAL: Duration = Duration::from_millis(10);

enum Command {
    Sentence { turn: TurnId, text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueAdmission {
    Immediate,
    Backpressured,
    Cancelled,
}

fn enqueue_command(
    tx: &SyncSender<Command>,
    command: Command,
    invalid_up_to: &AtomicU64,
    timeout: Duration,
) -> Result<QueueAdmission, TrySendError<Command>> {
    enqueue_command_with_backpressure_observer(tx, command, invalid_up_to, timeout, || {})
}

fn enqueue_command_with_backpressure_observer(
    tx: &SyncSender<Command>,
    mut command: Command,
    invalid_up_to: &AtomicU64,
    timeout: Duration,
    mut on_backpressure: impl FnMut(),
) -> Result<QueueAdmission, TrySendError<Command>> {
    let started = Instant::now();
    let mut backpressured = false;
    loop {
        let turn = match &command {
            Command::Sentence { turn, .. } => *turn,
        };
        if turn.value() <= invalid_up_to.load(Ordering::SeqCst) {
            return Ok(QueueAdmission::Cancelled);
        }
        match tx.try_send(command) {
            Ok(()) => {
                return Ok(if backpressured {
                    QueueAdmission::Backpressured
                } else {
                    QueueAdmission::Immediate
                });
            }
            Err(TrySendError::Full(returned)) => {
                command = returned;
                if !backpressured {
                    backpressured = true;
                    on_backpressure();
                }
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(TrySendError::Full(command));
                }
                std::thread::sleep(remaining.min(BACKPRESSURE_POLL_INTERVAL));
            }
            Err(TrySendError::Disconnected(returned)) => {
                return Err(TrySendError::Disconnected(returned));
            }
        }
    }
}

fn emit_audio_if_current(
    event_gate: &Mutex<()>,
    invalid_up_to: &AtomicU64,
    turn: TurnId,
    emit: impl FnOnce(),
) -> bool {
    let _guard = event_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if turn.value() <= invalid_up_to.load(Ordering::SeqCst) {
        return false;
    }
    emit();
    true
}

struct Worker {
    tx: SyncSender<Command>,
    settings_fingerprint: String,
    cancel: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    fn shutdown(mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        drop(self.tx);
        if let Some(thread) = self.thread.take() {
            // Each adapter request has a finite timeout. Never detach a stale
            // synthesis worker that could emit audio after its replacement.
            let _ = thread.join();
        }
    }
}

/// Managed state: at most one synthesis worker.
///
/// Latest-turn registration is atomic. Stop invalidation never waits for the
/// worker lock, so a full producer queue can be cancelled immediately. The
/// invalidation watermark is shared with the worker, which drops queued
/// sentences at or below it before synthesizing them.
pub struct TtsService {
    worker: Mutex<Option<Worker>>,
    settings_fingerprint: Mutex<Option<String>>,
    latest_turn: AtomicU64,
    invalid_up_to: Arc<AtomicU64>,
    event_gate: Arc<Mutex<()>>,
    dropped_sentences: AtomicU64,
    text_only_turn: AtomicU64,
    health: Arc<Mutex<FeatureHealthSupervisor<SystemClock, TimeJitter>>>,
    queue_metrics: Arc<QueueMetrics>,
}

impl Default for TtsService {
    fn default() -> Self {
        Self {
            worker: Mutex::new(None),
            settings_fingerprint: Mutex::new(None),
            latest_turn: AtomicU64::new(0),
            invalid_up_to: Arc::new(AtomicU64::new(0)),
            event_gate: Arc::new(Mutex::new(())),
            dropped_sentences: AtomicU64::new(0),
            text_only_turn: AtomicU64::new(0),
            health: Arc::new(Mutex::new(FeatureHealthSupervisor::new(
                RuntimeFeature::TextToSpeech,
                SystemClock,
                TimeJitter::default(),
            ))),
            queue_metrics: Arc::new(QueueMetrics::new("tts", TTS_QUEUE_CAPACITY)),
        }
    }
}

fn fingerprint(settings: &TtsSettingsDto) -> String {
    let engine = match settings.engine {
        TtsEngineKind::Aivis => "aivis",
        TtsEngineKind::Irodori => "irodori",
    };
    let volume = settings.volume.to_bits().to_string();
    let speed = settings.speed.to_bits().to_string();
    let mut result = String::new();
    for field in [
        engine,
        settings.base_url.as_str(),
        settings.voice_id.as_str(),
        settings.irodori_lora_adapter.as_str(),
        volume.as_str(),
        speed.as_str(),
    ] {
        result.push_str(&field.len().to_string());
        result.push(':');
        result.push_str(field);
    }
    result
}

fn engine_client(settings: &TtsSettingsDto) -> Result<EngineClient, String> {
    let config = TtsClientConfig {
        base_url: settings.base_url.clone(),
        timeout: ADAPTER_TIMEOUT,
    };
    match settings.engine {
        TtsEngineKind::Aivis => AivisSpeechClient::new(&config)
            .map(EngineClient::Aivis)
            .map_err(|error| error.to_string()),
        TtsEngineKind::Irodori => {
            let client = if settings.irodori_lora_adapter.trim().is_empty() {
                IrodoriTtsClient::new(&config)
            } else {
                IrodoriTtsClient::with_lora_adapter(&config, &settings.irodori_lora_adapter)
            };
            client
                .map(EngineClient::Irodori)
                .map_err(|error| error.to_string())
        }
    }
}

impl TtsService {
    /// Stops work tied to an old engine configuration and gives the new
    /// configuration an independent health circuit. Repeated submissions with
    /// the same fingerprint retain their current backoff state.
    fn refresh_configuration(&self, wanted: &str) {
        let mut current = self
            .settings_fingerprint
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current.as_deref() == Some(wanted) {
            return;
        }

        let mut worker = self.lock();
        if let Some(stale) = worker.take() {
            stale.shutdown();
        }
        self.queue_metrics.reset_depth();
        self.text_only_turn.store(0, Ordering::Relaxed);
        *self
            .health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = FeatureHealthSupervisor::new(
            RuntimeFeature::TextToSpeech,
            SystemClock,
            TimeJitter::default(),
        );
        *current = Some(wanted.to_owned());
    }

    /// Clears the application-owned TTS circuit.
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
    pub fn queue_metrics(&self) -> pw_contracts::QueueMetricsDto {
        self.queue_metrics.snapshot()
    }
    fn lock(&self) -> MutexGuard<'_, Option<Worker>> {
        self.worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    fn register_turn_locked(&self, guard: &MutexGuard<'_, Option<Worker>>, turn: TurnId) -> bool {
        self.register_turn_locked_with(guard, turn, || {})
    }

    fn register_turn_locked_with(
        &self,
        _guard: &MutexGuard<'_, Option<Worker>>,
        turn: TurnId,
        emit_stop: impl FnOnce(),
    ) -> bool {
        let _event_guard = self
            .event_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous_turn = self.latest_turn.fetch_max(turn.value(), Ordering::SeqCst);
        if previous_turn != 0 && previous_turn < turn.value() {
            self.invalid_up_to
                .fetch_max(previous_turn, Ordering::SeqCst);
            emit_stop();
            return true;
        }
        false
    }

    fn stop_core(&self) {
        // This load is stop's linearization point. Turns registered before it
        // are invalidated; a newer turn registered after it is subsequent work.
        let latest = self.latest_turn.load(Ordering::SeqCst);
        self.invalid_up_to.fetch_max(latest, Ordering::SeqCst);
    }

    fn stop_with(&self, emit_stop: impl FnOnce()) {
        let _guard = self
            .event_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.stop_core();
        emit_stop();
    }

    /// Stops the worker and runs one destructive cache operation while new
    /// synthesis submissions are excluded by the worker lock.
    ///
    /// # Errors
    ///
    /// Returns the operation's error.
    pub(crate) fn with_exclusive_reset<T>(
        &self,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        // Cancel a producer waiting on a full queue before waiting for the lock
        // that producer currently owns.
        self.stop_core();
        let mut guard = self.lock();
        // Include any registration that won the worker lock immediately before
        // this reset acquired it.
        self.stop_core();
        if let Some(worker) = guard.take() {
            worker.shutdown();
        }
        self.queue_metrics.reset_depth();
        operation()
    }

    /// Queues one sentence for synthesis. No-op while TTS is disabled;
    /// engine failures degrade to text-only via `tts-state`.
    pub fn enqueue<R: Runtime>(&self, app: &AppHandle<R>, turn: TurnId, text: &str) {
        let layout = app.state::<AppDataLayout>();
        let settings = super::settings::load_tts_settings(&layout);
        if !settings.enabled {
            return;
        }
        let wanted = fingerprint(&settings);
        self.refresh_configuration(&wanted);
        if !self
            .health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .can_attempt()
        {
            self.text_only_turn.store(turn.value(), Ordering::Relaxed);
            emit_state(
                app,
                false,
                Some("tts is recovering; continuing this turn text-only".to_owned()),
            );
            return;
        }
        let mut guard = self.lock();
        self.register_turn_locked_with(&guard, turn, || emit_stop(app));
        if turn.value() <= self.invalid_up_to.load(Ordering::SeqCst) {
            return;
        }
        let text_only = self.text_only_turn.load(Ordering::Relaxed);
        if text_only == turn.value() {
            return;
        }
        if turn.value() > text_only {
            let _ = self.text_only_turn.compare_exchange(
                text_only,
                0,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }

        let restart = match guard.as_ref() {
            Some(worker) => worker.settings_fingerprint != wanted,
            None => true,
        };
        if restart {
            if let Some(worker) = guard.take() {
                worker.shutdown();
                self.queue_metrics.reset_depth();
            }
            match self.start_worker(app.clone(), &settings) {
                Ok(worker) => *guard = Some(worker),
                Err(message) => {
                    emit_state(app, false, Some(message));
                    emit_tts_health(app, &self.health, false);
                    return;
                }
            }
        }
        let disconnected = guard.as_ref().is_some_and(|worker| {
            self.queue_metrics.enqueued();
            let admission = enqueue_command(
                &worker.tx,
                Command::Sentence {
                    turn,
                    text: text.to_owned(),
                },
                &self.invalid_up_to,
                BACKPRESSURE_TIMEOUT,
            );
            self.handle_queue_admission(app, turn, admission)
        });
        if disconnected {
            if let Some(worker) = guard.take() {
                worker.shutdown();
            }
            self.queue_metrics.reset_depth();
        }
    }

    fn handle_queue_admission<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        turn: TurnId,
        admission: Result<QueueAdmission, TrySendError<Command>>,
    ) -> bool {
        match admission {
            Ok(QueueAdmission::Immediate) => false,
            Ok(QueueAdmission::Backpressured) => {
                self.queue_metrics.busy();
                false
            }
            Ok(QueueAdmission::Cancelled) => {
                self.queue_metrics.dequeued();
                false
            }
            Err(error) => {
                let adapter_failure = enqueue_error_is_adapter_failure(&error);
                self.queue_metrics.dequeued();
                let disconnected = match error {
                    TrySendError::Full(_) => {
                        self.queue_metrics.busy();
                        self.queue_metrics.dropped();
                        self.dropped_sentences.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            turn = turn.value(),
                            "tts queue remained full; skipped one sentence without invalidating the turn"
                        );
                        false
                    }
                    TrySendError::Disconnected(_) => {
                        self.queue_metrics.dropped();
                        emit_state(app, false, Some("tts worker is not available".to_owned()));
                        true
                    }
                };
                if adapter_failure {
                    emit_tts_health(app, &self.health, false);
                }
                disconnected
            }
        }
    }

    /// Stops playback immediately: invalidates every queued sentence
    /// up to the latest turn and tells the character window to halt.
    pub fn stop<R: Runtime>(&self, app: &AppHandle<R>) {
        self.stop_with(|| emit_stop(app));
    }

    fn start_worker<R: Runtime>(
        &self,
        app: AppHandle<R>,
        settings: &TtsSettingsDto,
    ) -> Result<Worker, String> {
        let client = engine_client(settings)?;
        let layout = app.state::<AppDataLayout>();
        let cache = WavCache::new(layout.cache.join("tts"), DEFAULT_MAX_ENTRIES);
        let synthesizer = CachedSpeechSynthesizer::new(
            client,
            cache,
            &settings.voice_id,
            SynthesisParams {
                volume: settings.volume,
                speed: settings.speed,
            },
        );
        let invalid = Arc::clone(&self.invalid_up_to);
        let event_gate = Arc::clone(&self.event_gate);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (tx, rx) = sync_channel::<Command>(TTS_QUEUE_CAPACITY);
        let sink = TauriSpeechAudioSink {
            app,
            health: Arc::clone(&self.health),
            invalid: Arc::clone(&invalid),
            event_gate,
        };
        let queue_metrics = Arc::clone(&self.queue_metrics);

        let thread = std::thread::Builder::new()
            .name("pw-tts".into())
            .spawn(move || {
                let mut queue = SpeechSynthesisQueue::new(synthesizer, sink);
                while let Ok(command) = rx.recv() {
                    queue_metrics.dequeued();
                    if worker_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    match command {
                        Command::Sentence { turn, text } => {
                            if turn.value() <= invalid.load(Ordering::SeqCst) {
                                continue;
                            }
                            queue.push_sentence(turn, &text);
                        }
                    }
                }
            })
            .map_err(|error| format!("failed to spawn tts worker: {error}"))?;

        Ok(Worker {
            tx,
            settings_fingerprint: fingerprint(settings),
            cancel,
            thread: Some(thread),
        })
    }
}

fn enqueue_error_is_adapter_failure(error: &TrySendError<Command>) -> bool {
    matches!(error, TrySendError::Disconnected(_))
}

fn emit_stop<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit_to(
        EventTarget::webview_window(CHARACTER_WINDOW),
        STOP_EVENT,
        SpeechStopEventDto {
            schema_version: SCHEMA_VERSION,
        },
    );
}

fn emit_state<R: Runtime>(app: &AppHandle<R>, available: bool, message: Option<String>) {
    let _ = app.emit(
        STATE_EVENT,
        TtsStateEventDto {
            schema_version: SCHEMA_VERSION,
            available,
            message,
        },
    );
}

fn emit_tts_health<R: Runtime>(
    app: &AppHandle<R>,
    registry: &Mutex<FeatureHealthSupervisor<SystemClock, TimeJitter>>,
    healthy: bool,
) {
    let mut health = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let transition = if healthy {
        health.record_success()
    } else {
        match health.record_failure(RuntimeFailure::transient(FailureCode::Unavailable)) {
            pw_application::recovery::HealthUpdate::Changed {
                health, attempts, ..
            } => HealthTransition::Changed { health, attempts },
            pw_application::recovery::HealthUpdate::Unchanged { .. } => HealthTransition::Unchanged,
        }
    };
    if let HealthTransition::Changed { health, attempts } = transition {
        let _ = app.emit(
            RUNTIME_HEALTH_EVENT,
            RuntimeHealthEventDto::from((&health, attempts)),
        );
    }
}

struct TauriSpeechAudioSink<R: Runtime> {
    app: AppHandle<R>,
    health: Arc<Mutex<FeatureHealthSupervisor<SystemClock, TimeJitter>>>,
    invalid: Arc<AtomicU64>,
    event_gate: Arc<Mutex<()>>,
}

impl<R: Runtime> SpeechAudioSink for TauriSpeechAudioSink<R> {
    fn on_audio(&self, turn: TurnId, seq: u32, wav_path: &Path, text: &str) {
        emit_audio_if_current(&self.event_gate, &self.invalid, turn, || {
            emit_state(&self.app, true, None);
            emit_tts_health(&self.app, &self.health, true);
            let payload = SpeechAudioEventDto {
                schema_version: SCHEMA_VERSION,
                turn_id: turn.value(),
                seq,
                wav_path: wav_path.to_string_lossy().into_owned(),
                text: text.to_owned(),
            };
            let _ = self.app.emit_to(
                EventTarget::webview_window(CHARACTER_WINDOW),
                AUDIO_EVENT,
                payload,
            );
        });
    }

    fn on_stop(&self) {
        emit_stop(&self.app);
    }

    fn on_error(&self, _turn: TurnId, message: &str) {
        tracing::warn!(%message, "tts synthesis failed; continuing text-only");
        emit_state(&self.app, false, Some(message.to_owned()));
        emit_tts_health(&self.app, &self.health, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_fingerprint_changes_for_engine_or_voice() {
        let base = crate::tts::settings::default_tts_settings();
        let mut changed_engine = base.clone();
        changed_engine.engine = pw_contracts::TtsEngineKind::Irodori;
        let mut changed_voice = base.clone();
        changed_voice.voice_id = "different".to_owned();
        let mut changed_lora = base.clone();
        changed_lora.engine = pw_contracts::TtsEngineKind::Irodori;
        changed_lora.irodori_lora_adapter = "adapters/character-a".to_owned();

        assert_ne!(fingerprint(&base), fingerprint(&changed_engine));
        assert_ne!(fingerprint(&base), fingerprint(&changed_voice));
        assert_ne!(fingerprint(&changed_engine), fingerprint(&changed_lora));
    }

    #[test]
    fn worker_fingerprint_separates_fields_containing_the_old_delimiter() {
        let mut left = crate::tts::settings::default_tts_settings();
        left.engine = TtsEngineKind::Irodori;
        left.voice_id = "x".to_owned();
        left.irodori_lora_adapter = "y|z".to_owned();
        let mut right = left.clone();
        right.voice_id = "x|y".to_owned();
        right.irodori_lora_adapter = "z".to_owned();

        assert_ne!(fingerprint(&left), fingerprint(&right));
    }

    #[test]
    fn engine_change_rearms_an_open_aivis_circuit_before_the_next_attempt() {
        let service = TtsService::default();
        let aivis = crate::tts::settings::default_tts_settings();
        let mut irodori = aivis.clone();
        irodori.engine = TtsEngineKind::Irodori;
        irodori.base_url = crate::tts::settings::default_base_url(TtsEngineKind::Irodori).into();
        irodori.voice_id = "voice-a".into();

        service.refresh_configuration(&fingerprint(&aivis));
        {
            let mut health = service.health.lock().unwrap();
            health.record_failure(RuntimeFailure::permanent(FailureCode::Unavailable));
            assert!(!health.can_attempt());
        }

        service.refresh_configuration(&fingerprint(&irodori));

        assert!(service.health.lock().unwrap().can_attempt());
    }

    #[test]
    fn same_configuration_preserves_health_backoff() {
        let service = TtsService::default();
        let wanted = fingerprint(&crate::tts::settings::default_tts_settings());
        service.refresh_configuration(&wanted);
        service
            .health
            .lock()
            .unwrap()
            .record_failure(RuntimeFailure::permanent(FailureCode::Unavailable));

        service.refresh_configuration(&wanted);

        assert!(!service.health.lock().unwrap().can_attempt());
    }

    #[test]
    fn production_tts_timeout_covers_slow_local_synthesis_without_extending_backpressure() {
        assert!(ADAPTER_TIMEOUT >= Duration::from_secs(30));
        assert_eq!(BACKPRESSURE_TIMEOUT, Duration::from_secs(5));
    }

    #[test]
    fn queue_backpressure_is_not_classified_as_an_adapter_failure() {
        let full = TrySendError::Full(Command::Sentence {
            turn: pw_domain::reply::TurnTracker::new().begin_turn(),
            text: "busy".into(),
        });
        assert!(!enqueue_error_is_adapter_failure(&full));
    }

    #[test]
    fn full_queue_waits_for_capacity_without_invalidating_the_turn() {
        let (tx, rx) = sync_channel(TTS_QUEUE_CAPACITY);
        let turn = pw_domain::reply::TurnTracker::new().begin_turn();
        let invalid_up_to = AtomicU64::new(0);
        for index in 0..TTS_QUEUE_CAPACITY {
            tx.try_send(Command::Sentence {
                turn,
                text: format!("queued-{index}"),
            })
            .unwrap();
        }
        let receiver = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            (0..=TTS_QUEUE_CAPACITY)
                .map(|_| match rx.recv().unwrap() {
                    Command::Sentence { text, .. } => text,
                })
                .collect::<Vec<_>>()
        });

        let started = std::time::Instant::now();
        let admission = enqueue_command(
            &tx,
            Command::Sentence {
                turn,
                text: "backpressured".into(),
            },
            &invalid_up_to,
            Duration::from_secs(1),
        );
        let elapsed = started.elapsed();
        let delivered = receiver.join().unwrap();

        assert_eq!(admission.unwrap(), QueueAdmission::Backpressured);
        assert_eq!(
            delivered,
            [
                "queued-0",
                "queued-1",
                "queued-2",
                "queued-3",
                "queued-4",
                "queued-5",
                "queued-6",
                "queued-7",
                "backpressured",
            ]
        );
        assert_eq!(invalid_up_to.load(Ordering::Relaxed), 0);
        assert!(
            elapsed >= Duration::from_millis(50),
            "queue admission did not apply backpressure: {elapsed:?}"
        );
    }

    #[test]
    fn exclusive_reset_clears_queue_depth_and_propagates_the_operation_result() {
        let service = TtsService::default();
        service.queue_metrics.enqueued();
        assert_eq!(service.queue_metrics().depth, 1);

        let value = service
            .with_exclusive_reset(|| Ok::<_, String>(42))
            .unwrap();
        assert_eq!(value, 42);
        assert_eq!(service.queue_metrics().depth, 0);
        assert_eq!(
            service.with_exclusive_reset(|| Err::<(), _>("cleanup failed".to_owned())),
            Err("cleanup failed".to_owned())
        );
    }

    #[test]
    fn exclusive_reset_invalidates_a_racing_turn_and_blocks_same_turn_restart() {
        let service = Arc::new(TtsService::default());
        let mut tracker = pw_domain::reply::TurnTracker::new();
        let old_turn = tracker.begin_turn();
        let racing_turn = tracker.begin_turn();
        {
            let guard = service.lock();
            service.register_turn_locked(&guard, old_turn);
        }

        let (registered_tx, registered_rx) = std::sync::mpsc::channel();
        let (release_registration_tx, release_registration_rx) = std::sync::mpsc::channel();
        let registering_service = Arc::clone(&service);
        let registration = std::thread::spawn(move || {
            let guard = registering_service.lock();
            registering_service.register_turn_locked(&guard, racing_turn);
            registered_tx.send(()).unwrap();
            release_registration_rx.recv().unwrap();
        });
        registered_rx.recv().unwrap();

        let (reset_entered_tx, reset_entered_rx) = std::sync::mpsc::channel();
        let (release_reset_tx, release_reset_rx) = std::sync::mpsc::channel();
        let resetting_service = Arc::clone(&service);
        let reset = std::thread::spawn(move || {
            resetting_service
                .with_exclusive_reset(|| {
                    reset_entered_tx.send(()).unwrap();
                    release_reset_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap();
        });
        assert!(
            reset_entered_rx
                .recv_timeout(Duration::from_millis(50))
                .is_err(),
            "reset entered while latest-turn registration held the worker lock"
        );
        release_registration_tx.send(()).unwrap();
        registration.join().unwrap();
        reset_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            service.invalid_up_to.load(Ordering::Relaxed),
            racing_turn.value()
        );

        let (admission_tx, admission_rx) = std::sync::mpsc::channel();
        let later_service = Arc::clone(&service);
        let later_sentence = std::thread::spawn(move || {
            let guard = later_service.lock();
            later_service.register_turn_locked(&guard, racing_turn);
            let (tx, _rx) = sync_channel(1);
            let admission = enqueue_command(
                &tx,
                Command::Sentence {
                    turn: racing_turn,
                    text: "must remain stopped".into(),
                },
                &later_service.invalid_up_to,
                Duration::ZERO,
            );
            admission_tx.send(admission).unwrap();
        });
        assert!(
            admission_rx
                .recv_timeout(Duration::from_millis(50))
                .is_err(),
            "same-turn sentence bypassed the exclusive reset lock"
        );
        release_reset_tx.send(()).unwrap();
        reset.join().unwrap();
        assert!(matches!(
            admission_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Ok(QueueAdmission::Cancelled)
        ));
        later_sentence.join().unwrap();
    }

    #[test]
    fn delayed_older_turn_cannot_move_the_latest_turn_watermark_backwards() {
        let service = TtsService::default();
        let mut tracker = pw_domain::reply::TurnTracker::new();
        let first = tracker.begin_turn();
        let delayed = tracker.begin_turn();
        let latest = tracker.begin_turn();
        {
            let guard = service.lock();
            service.register_turn_locked(&guard, first);
            service.register_turn_locked(&guard, latest);
            service.register_turn_locked(&guard, delayed);
        }

        assert_eq!(service.latest_turn.load(Ordering::Relaxed), latest.value());
        service.with_exclusive_reset(|| Ok(())).unwrap();
        assert_eq!(
            service.invalid_up_to.load(Ordering::Relaxed),
            latest.value()
        );
    }

    #[test]
    fn stop_path_ignores_the_worker_lock_and_unblocks_reset_after_backpressure() {
        let service = Arc::new(TtsService::default());
        let (tx, _rx) = sync_channel(TTS_QUEUE_CAPACITY);
        let turn = pw_domain::reply::TurnTracker::new().begin_turn();
        for index in 0..TTS_QUEUE_CAPACITY {
            tx.try_send(Command::Sentence {
                turn,
                text: format!("queued-{index}"),
            })
            .unwrap();
        }
        let (backpressured_tx, backpressured_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let producer_service = Arc::clone(&service);
        let producer = std::thread::spawn(move || {
            let guard = producer_service.lock();
            producer_service.register_turn_locked(&guard, turn);
            let result = enqueue_command_with_backpressure_observer(
                &tx,
                Command::Sentence {
                    turn,
                    text: "cancelled".into(),
                },
                &producer_service.invalid_up_to,
                Duration::from_secs(1),
                || backpressured_tx.send(()).unwrap(),
            );
            drop(guard);
            result_tx.send(result).unwrap();
        });
        backpressured_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        let (stopped_tx, stopped_rx) = std::sync::mpsc::channel();
        let (reset_tx, reset_rx) = std::sync::mpsc::channel();
        let reset_service = Arc::clone(&service);
        let reset = std::thread::spawn(move || {
            reset_service.stop_with(|| {});
            stopped_tx.send(()).unwrap();
            let result = reset_service.with_exclusive_reset(|| Ok::<_, String>(()));
            reset_tx.send(result).unwrap();
        });

        let stopped_promptly = stopped_rx.recv_timeout(Duration::from_millis(100)).is_ok();
        let admission = result_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let reset_promptly = reset_rx.recv_timeout(Duration::from_millis(250)).is_ok();
        producer.join().unwrap();
        reset.join().unwrap();

        assert!(
            stopped_promptly,
            "stop core waited for the producer-held worker lock"
        );
        assert!(matches!(admission, Ok(QueueAdmission::Cancelled)));
        assert!(
            reset_promptly,
            "exclusive reset did not progress after stop cancelled backpressure"
        );
    }

    #[test]
    fn explicit_stop_prevents_a_racing_old_turn_audio_event() {
        let service = Arc::new(TtsService::default());
        let turn = pw_domain::reply::TurnTracker::new().begin_turn();
        service.latest_turn.store(turn.value(), Ordering::SeqCst);
        let events = Arc::new(Mutex::new(Vec::new()));
        let (stop_emitting_tx, stop_emitting_rx) = std::sync::mpsc::channel();
        let (release_stop_tx, release_stop_rx) = std::sync::mpsc::channel();
        let (stop_returned_tx, stop_returned_rx) = std::sync::mpsc::channel();
        let stopping_service = Arc::clone(&service);
        let stop_events = Arc::clone(&events);
        let stopper = std::thread::spawn(move || {
            stopping_service.stop_with(|| {
                stop_events.lock().unwrap().push("stop");
                stop_emitting_tx.send(()).unwrap();
                release_stop_rx.recv().unwrap();
            });
            stop_returned_tx.send(()).unwrap();
        });
        stop_emitting_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        let (audio_result_tx, audio_result_rx) = std::sync::mpsc::channel();
        let audio_gate = Arc::clone(&service.event_gate);
        let invalid = Arc::clone(&service.invalid_up_to);
        let audio_events = Arc::clone(&events);
        let audio = std::thread::spawn(move || {
            let emitted = emit_audio_if_current(&audio_gate, &invalid, turn, || {
                audio_events.lock().unwrap().push("audio");
            });
            audio_result_tx.send(emitted).unwrap();
        });
        assert!(
            audio_result_rx
                .recv_timeout(Duration::from_millis(50))
                .is_err(),
            "audio event bypassed the in-flight stop event"
        );

        release_stop_tx.send(()).unwrap();
        stop_returned_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert!(
            !audio_result_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
        );
        stopper.join().unwrap();
        audio.join().unwrap();
        assert_eq!(*events.lock().unwrap(), ["stop"]);
    }

    #[test]
    fn newer_turn_stop_prevents_a_racing_old_turn_audio_event() {
        let service = Arc::new(TtsService::default());
        let mut tracker = pw_domain::reply::TurnTracker::new();
        let old_turn = tracker.begin_turn();
        let new_turn = tracker.begin_turn();
        {
            let guard = service.lock();
            assert!(!service.register_turn_locked(&guard, old_turn));
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let (stop_emitting_tx, stop_emitting_rx) = std::sync::mpsc::channel();
        let (release_stop_tx, release_stop_rx) = std::sync::mpsc::channel();
        let switching_service = Arc::clone(&service);
        let stop_events = Arc::clone(&events);
        let switcher = std::thread::spawn(move || {
            let guard = switching_service.lock();
            switching_service.register_turn_locked_with(&guard, new_turn, || {
                stop_events.lock().unwrap().push("stop");
                stop_emitting_tx.send(()).unwrap();
                release_stop_rx.recv().unwrap();
            });
        });
        stop_emitting_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        let (audio_result_tx, audio_result_rx) = std::sync::mpsc::channel();
        let audio_gate = Arc::clone(&service.event_gate);
        let invalid = Arc::clone(&service.invalid_up_to);
        let audio_events = Arc::clone(&events);
        let audio = std::thread::spawn(move || {
            let emitted = emit_audio_if_current(&audio_gate, &invalid, old_turn, || {
                audio_events.lock().unwrap().push("audio");
            });
            audio_result_tx.send(emitted).unwrap();
        });
        assert!(
            audio_result_rx
                .recv_timeout(Duration::from_millis(50))
                .is_err(),
            "old audio event bypassed the in-flight newer-turn stop event"
        );

        release_stop_tx.send(()).unwrap();
        switcher.join().unwrap();
        assert!(
            !audio_result_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
        );
        audio.join().unwrap();
        assert_eq!(*events.lock().unwrap(), ["stop"]);
    }

    #[test]
    fn stalled_queue_times_out_without_invalidating_the_turn() {
        let (tx, _rx) = sync_channel(TTS_QUEUE_CAPACITY);
        let turn = pw_domain::reply::TurnTracker::new().begin_turn();
        let invalid_up_to = AtomicU64::new(0);
        for index in 0..TTS_QUEUE_CAPACITY {
            tx.try_send(Command::Sentence {
                turn,
                text: format!("queued-{index}"),
            })
            .unwrap();
        }

        let started = Instant::now();
        let result = enqueue_command(
            &tx,
            Command::Sentence {
                turn,
                text: "timed-out".into(),
            },
            &invalid_up_to,
            Duration::from_millis(30),
        );

        assert!(matches!(result, Err(TrySendError::Full(_))));
        assert!(started.elapsed() < Duration::from_millis(200));
        assert_eq!(invalid_up_to.load(Ordering::Relaxed), 0);
    }

    struct ActiveGuard(Arc<AtomicU64>);
    impl Drop for ActiveGuard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn shutdown_joins_a_slow_adapter_worker_instead_of_detaching_it() {
        let (tx, _rx) = sync_channel(1);
        let active = Arc::new(AtomicU64::new(1));
        let worker_active = Arc::clone(&active);
        let thread = std::thread::spawn(move || {
            let _guard = ActiveGuard(worker_active);
            std::thread::sleep(Duration::from_millis(2_050));
        });
        let worker = Worker {
            tx,
            settings_fingerprint: "test".into(),
            cancel: Arc::new(AtomicBool::new(false)),
            thread: Some(thread),
        };

        worker.shutdown();

        assert_eq!(active.load(Ordering::SeqCst), 0);
    }
}
