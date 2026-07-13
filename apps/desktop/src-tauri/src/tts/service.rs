//! TTS worker: synthesizes sentences ahead of playback and streams
//! `speech-audio` items to the character window.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::diagnostics::QueueMetrics;
use pw_application::recovery::{
    FeatureHealthSupervisor, HealthTransition, SystemClock, TimeJitter,
};
use pw_application::speech_synthesis::{SpeechAudioSink, SpeechSynthesisQueue};
use pw_contracts::{
    RUNTIME_HEALTH_EVENT, RuntimeHealthEventDto, SCHEMA_VERSION, SpeechAudioEventDto,
    SpeechStopEventDto, TtsSettingsDto, TtsStateEventDto,
};
use pw_domain::reply::TurnId;
use pw_domain::runtime_health::{FailureCode, RuntimeFailure, RuntimeFeature};
use pw_platform::paths::AppDataLayout;
use pw_tts::{
    AivisSpeechClient, CachedSpeechSynthesizer, DEFAULT_MAX_ENTRIES, SynthesisParams,
    TtsClientConfig, WavCache,
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
const ADAPTER_TIMEOUT: Duration = Duration::from_secs(5);

enum Command {
    Sentence { turn: TurnId, text: String },
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
            // The adapter has a finite total timeout. Never detach a stale
            // synthesis worker that could emit audio after its replacement.
            let _ = thread.join();
        }
    }
}

/// Managed state: at most one synthesis worker.
///
/// Stop must not wait behind queued synthesis, so the invalidation
/// watermark is shared with the worker: `stop()` raises it and emits
/// `speech-stop` from the calling thread; the worker drops queued
/// sentences at or below the watermark before synthesizing them.
pub struct TtsService {
    worker: Mutex<Option<Worker>>,
    latest_turn: AtomicU64,
    invalid_up_to: Arc<AtomicU64>,
    dropped_sentences: AtomicU64,
    text_only_turn: AtomicU64,
    health: Arc<Mutex<FeatureHealthSupervisor<SystemClock, TimeJitter>>>,
    queue_metrics: Arc<QueueMetrics>,
}

impl Default for TtsService {
    fn default() -> Self {
        Self {
            worker: Mutex::new(None),
            latest_turn: AtomicU64::new(0),
            invalid_up_to: Arc::new(AtomicU64::new(0)),
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
    format!(
        "{}|{}|{}|{}",
        settings.base_url, settings.style_id, settings.volume, settings.speed
    )
}

impl TtsService {
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

    /// Queues one sentence for synthesis. No-op while TTS is disabled;
    /// engine failures degrade to text-only via `tts-state`.
    pub fn enqueue<R: Runtime>(&self, app: &AppHandle<R>, turn: TurnId, text: &str) {
        let layout = app.state::<AppDataLayout>();
        let settings = super::settings::load_tts_settings(&layout);
        if !settings.enabled {
            return;
        }
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
        self.latest_turn.store(turn.value(), Ordering::Relaxed);
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

        let wanted = fingerprint(&settings);
        let mut guard = self.lock();
        let restart = match guard.as_ref() {
            Some(worker) => worker.settings_fingerprint != wanted,
            None => true,
        };
        if restart {
            if let Some(worker) = guard.take() {
                worker.shutdown();
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
        if let Some(worker) = guard.as_ref() {
            self.queue_metrics.enqueued();
            if let Err(error) = worker.tx.try_send(Command::Sentence {
                turn,
                text: text.to_owned(),
            }) {
                let adapter_failure = enqueue_error_is_adapter_failure(&error);
                self.queue_metrics.dequeued();
                match error {
                    TrySendError::Full(_) => {
                        self.queue_metrics.busy();
                        self.queue_metrics.dropped();
                        self.dropped_sentences.fetch_add(1, Ordering::Relaxed);
                        self.invalid_up_to
                            .fetch_max(turn.value(), Ordering::Relaxed);
                        if self.text_only_turn.swap(turn.value(), Ordering::Relaxed) != turn.value()
                        {
                            emit_state(
                                app,
                                false,
                                Some(
                                    "tts queue is busy; continuing this turn text-only".to_owned(),
                                ),
                            );
                        }
                    }
                    TrySendError::Disconnected(_) => {
                        self.queue_metrics.dropped();
                        emit_state(app, false, Some("tts worker is not available".to_owned()));
                    }
                }
                if adapter_failure {
                    emit_tts_health(app, &self.health, false);
                }
            }
        }
    }

    /// Stops playback immediately: invalidates every queued sentence
    /// up to the latest turn and tells the character window to halt.
    pub fn stop<R: Runtime>(&self, app: &AppHandle<R>) {
        let latest = self.latest_turn.load(Ordering::Relaxed);
        self.invalid_up_to.fetch_max(latest, Ordering::Relaxed);
        emit_stop(app);
    }

    fn start_worker<R: Runtime>(
        &self,
        app: AppHandle<R>,
        settings: &TtsSettingsDto,
    ) -> Result<Worker, String> {
        let client = AivisSpeechClient::new(&TtsClientConfig {
            base_url: settings.base_url.clone(),
            timeout: ADAPTER_TIMEOUT,
        })
        .map_err(|error| error.to_string())?;
        let layout = app.state::<AppDataLayout>();
        let cache = WavCache::new(layout.cache.join("tts"), DEFAULT_MAX_ENTRIES);
        let synthesizer = CachedSpeechSynthesizer::new(
            client,
            cache,
            settings.style_id,
            SynthesisParams {
                volume: settings.volume,
                speed: settings.speed,
            },
        );
        let invalid = Arc::clone(&self.invalid_up_to);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (tx, rx) = sync_channel::<Command>(TTS_QUEUE_CAPACITY);
        let sink = TauriSpeechAudioSink {
            app,
            health: Arc::clone(&self.health),
            invalid: Arc::clone(&invalid),
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
                            if turn.value() <= invalid.load(Ordering::Relaxed) {
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
}

impl<R: Runtime> SpeechAudioSink for TauriSpeechAudioSink<R> {
    fn on_audio(&self, turn: TurnId, seq: u32, wav_path: &Path, text: &str) {
        if turn.value() <= self.invalid.load(Ordering::Relaxed) {
            return;
        }
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
    fn production_tts_timeout_allows_local_synthesis() {
        assert!(ADAPTER_TIMEOUT >= Duration::from_secs(5));
    }

    #[test]
    fn queue_backpressure_is_not_classified_as_an_adapter_failure() {
        let full = TrySendError::Full(Command::Sentence {
            turn: pw_domain::reply::TurnTracker::new().begin_turn(),
            text: "busy".into(),
        });
        assert!(!enqueue_error_is_adapter_failure(&full));
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
