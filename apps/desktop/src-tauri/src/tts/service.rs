//! TTS worker: synthesizes sentences ahead of playback and streams
//! `speech-audio` items to the character window.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use pw_application::speech_synthesis::{SpeechAudioSink, SpeechSynthesisQueue};
use pw_contracts::{
    RUNTIME_HEALTH_EVENT, RuntimeHealthEventDto, SCHEMA_VERSION, SpeechAudioEventDto,
    SpeechStopEventDto, TtsSettingsDto, TtsStateEventDto,
};
use pw_domain::reply::TurnId;
use pw_domain::runtime_health::{FailureCode, RuntimeFailure, RuntimeFeature, RuntimeHealth};
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

enum Command {
    Sentence { turn: TurnId, text: String },
    Shutdown,
}

struct Worker {
    tx: SyncSender<Command>,
    settings_fingerprint: String,
}

/// Managed state: at most one synthesis worker.
///
/// Stop must not wait behind queued synthesis, so the invalidation
/// watermark is shared with the worker: `stop()` raises it and emits
/// `speech-stop` from the calling thread; the worker drops queued
/// sentences at or below the watermark before synthesizing them.
#[derive(Default)]
pub struct TtsService {
    worker: Mutex<Option<Worker>>,
    latest_turn: AtomicU64,
    invalid_up_to: Arc<AtomicU64>,
    dropped_sentences: AtomicU64,
}

fn fingerprint(settings: &TtsSettingsDto) -> String {
    format!(
        "{}|{}|{}|{}",
        settings.base_url, settings.style_id, settings.volume, settings.speed
    )
}

impl TtsService {
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
        self.latest_turn.store(turn.value(), Ordering::Relaxed);

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
            match self.start_worker(app.clone(), &settings) {
                Ok(worker) => *guard = Some(worker),
                Err(message) => {
                    emit_state(app, false, Some(message));
                    emit_tts_health(app, false);
                    return;
                }
            }
        }
        if let Some(worker) = guard.as_ref()
            && let Err(error) = worker.tx.try_send(Command::Sentence {
                turn,
                text: text.to_owned(),
            })
        {
            match error {
                TrySendError::Full(_) => {
                    self.dropped_sentences.fetch_add(1, Ordering::Relaxed);
                    emit_state(
                        app,
                        false,
                        Some("tts queue is busy; continuing text-only".to_owned()),
                    );
                    emit_tts_health(app, false);
                }
                TrySendError::Disconnected(_) => {
                    emit_state(app, false, Some("tts worker is not available".to_owned()));
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
            timeout: Duration::from_secs(30),
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
        let (tx, rx) = sync_channel::<Command>(TTS_QUEUE_CAPACITY);
        let sink = TauriSpeechAudioSink { app };

        std::thread::Builder::new()
            .name("pw-tts".into())
            .spawn(move || {
                let mut queue = SpeechSynthesisQueue::new(synthesizer, sink);
                while let Ok(command) = rx.recv() {
                    match command {
                        Command::Sentence { turn, text } => {
                            if turn.value() <= invalid.load(Ordering::Relaxed) {
                                continue;
                            }
                            queue.push_sentence(turn, &text);
                        }
                        Command::Shutdown => break,
                    }
                }
            })
            .map_err(|error| format!("failed to spawn tts worker: {error}"))?;

        Ok(Worker {
            tx,
            settings_fingerprint: fingerprint(settings),
        })
    }
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

fn emit_tts_health<R: Runtime>(app: &AppHandle<R>, healthy: bool) {
    let mut health = RuntimeHealth::new(RuntimeFeature::TextToSpeech);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        });
    if healthy {
        health.mark_healthy(now);
    } else {
        health.mark_failed(&RuntimeFailure::transient(FailureCode::Unavailable), now);
    }
    let _ = app.emit(
        RUNTIME_HEALTH_EVENT,
        RuntimeHealthEventDto::from((&health, u8::from(!healthy))),
    );
}

struct TauriSpeechAudioSink<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> SpeechAudioSink for TauriSpeechAudioSink<R> {
    fn on_audio(&self, turn: TurnId, seq: u32, wav_path: &Path, text: &str) {
        emit_state(&self.app, true, None);
        emit_tts_health(&self.app, true);
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
        emit_tts_health(&self.app, false);
    }
}
