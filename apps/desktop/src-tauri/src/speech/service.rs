//! Lifecycle owner of the speech pipeline worker.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use pw_application::recovery::{BackoffDecision, BackoffPolicy, Clock, RandomSource};
use pw_application::speech::{
    PipelineDiagnostics, SpeechEvents, SpeechPipeline, SpeechPipelineConfig, run_pipeline,
};
use pw_audio::capture::{CaptureError, CaptureSession, start_capture_with_failures};
use pw_audio::devices::{InputDeviceInfo, list_input_devices};
use pw_audio::frame_source::CaptureFrameSource;
use pw_audio::recovery::{FailureQueueMetrics, FailureSender, failure_channel};
use pw_contracts::{
    AudioDiagnosticsDto, AudioLevelEventDto, DeviceFallbackEventDto, RUNTIME_HEALTH_EVENT,
    RuntimeHealthEventDto, SCHEMA_VERSION, SttPhaseDto, SttStateEventDto, TranscriptEventDto,
};
use pw_domain::runtime_health::{FailureCode, RuntimeFailure, RuntimeFeature, RuntimeHealth};
use pw_domain::speech::RejectionReason;
use pw_platform::paths::AppDataLayout;
use pw_stt_sherpa::{ReazonSpeechRecognizer, RecognizerModelPaths, SherpaError, SileroVad};
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime};

pub const TRANSCRIPT_EVENT: &str = "stt-transcript";
pub const LEVEL_EVENT: &str = "stt-level";
pub const STATE_EVENT: &str = "stt-state";
pub const DEVICE_FALLBACK_EVENT: &str = "stt-device-fallback";

/// Emit a level event every N frames (~256 ms at 32 ms frames).
const LEVEL_EVERY_N_FRAMES: u64 = 8;

/// Resolved model locations under the app data `models/` directory.
pub struct SttModelPaths {
    pub vad_model: PathBuf,
    pub recognizer_dir: PathBuf,
}

impl SttModelPaths {
    #[must_use]
    pub fn under(layout: &AppDataLayout) -> Self {
        Self {
            vad_model: layout.models.join("vad/silero-vad-v5/silero_vad.onnx"),
            recognizer_dir: layout.models.join("stt/reazonspeech-k2-v2"),
        }
    }
}

struct RunningPipeline {
    active_device_id: Arc<Mutex<String>>,
    cancel: Arc<AtomicBool>,
    capture_enabled: Arc<AtomicBool>,
    diagnostics: Arc<PipelineDiagnostics>,
    dropped_samples: Arc<AtomicU64>,
    failure_metrics: Arc<Mutex<Arc<FailureQueueMetrics>>>,
    failure_queue_dropped: Arc<AtomicU64>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeechFailure {
    Audio,
    VadModel,
    SttModel,
    VadRuntime,
    SttRuntime,
    Stopped,
}

/// Persistent health state for the two features owned by the speech worker.
///
/// Keeping both records for the lifetime of the worker makes repeated events
/// idempotent and prevents an audio recovery from resetting STT history.
struct HealthRegistry {
    audio_input: RuntimeHealth,
    speech_to_text: RuntimeHealth,
}

impl HealthRegistry {
    const fn new() -> Self {
        Self {
            audio_input: RuntimeHealth::new(RuntimeFeature::AudioInput),
            speech_to_text: RuntimeHealth::new(RuntimeFeature::SpeechToText),
        }
    }

    fn snapshots(&self, attempts: u8) -> [RuntimeHealthEventDto; 2] {
        [
            RuntimeHealthEventDto::from((&self.audio_input, attempts)),
            RuntimeHealthEventDto::from((&self.speech_to_text, attempts)),
        ]
    }

    fn mark_pipeline_healthy(&mut self, now_ms: u64) -> [RuntimeHealthEventDto; 2] {
        self.audio_input.mark_healthy(now_ms);
        self.speech_to_text.mark_healthy(now_ms);
        self.snapshots(0)
    }

    fn mark_failure(
        &mut self,
        failure: SpeechFailure,
        now_ms: u64,
        attempts: u8,
        circuit_open: bool,
    ) -> RuntimeHealthEventDto {
        let (health, runtime_failure) = match failure {
            SpeechFailure::Audio => (
                &mut self.audio_input,
                RuntimeFailure::transient(FailureCode::Unavailable),
            ),
            SpeechFailure::VadModel | SpeechFailure::SttModel => (
                &mut self.speech_to_text,
                RuntimeFailure::permanent(FailureCode::MissingModel),
            ),
            SpeechFailure::VadRuntime | SpeechFailure::SttRuntime => (
                &mut self.speech_to_text,
                RuntimeFailure::transient(FailureCode::Internal),
            ),
            SpeechFailure::Stopped => unreachable!("stop uses mark_stopped"),
        };
        health.mark_failed(&runtime_failure, now_ms);
        let mut event = RuntimeHealthEventDto::from((&*health, attempts));
        event.circuit_open = circuit_open;
        event
    }

    fn mark_stopped(&mut self, now_ms: u64, attempts: u8) -> [RuntimeHealthEventDto; 2] {
        self.audio_input.mark_stopped(now_ms);
        self.speech_to_text.mark_stopped(now_ms);
        let mut events = self.snapshots(attempts);
        for event in &mut events {
            event.circuit_open = false;
        }
        events
    }
}

struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |value| {
                u64::try_from(value.as_millis()).unwrap_or(u64::MAX)
            })
    }
}

struct Jitter(u64);
impl RandomSource for Jitter {
    fn uniform_inclusive(&mut self, upper: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % upper.saturating_add(1)
    }
}

trait DeviceSelector {
    fn devices(&self) -> Vec<InputDeviceInfo>;
}

struct ProductionDeviceSelector;
impl DeviceSelector for ProductionDeviceSelector {
    fn devices(&self) -> Vec<InputDeviceInfo> {
        list_input_devices()
    }
}

trait CaptureFactory {
    type Session: CaptureLifecycle;

    fn open(
        &self,
        device_id: Option<&str>,
        failures: Option<FailureSender>,
    ) -> Result<Self::Session, CaptureError>;
}

trait CaptureLifecycle {
    fn stop_and_join(&mut self);
}

impl CaptureLifecycle for CaptureSession {
    fn stop_and_join(&mut self) {
        CaptureSession::stop_and_join(self);
    }
}

struct ProductionCaptureFactory;
impl CaptureFactory for ProductionCaptureFactory {
    type Session = CaptureSession;

    fn open(
        &self,
        device_id: Option<&str>,
        failures: Option<FailureSender>,
    ) -> Result<CaptureSession, CaptureError> {
        start_capture_with_failures(device_id, failures)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecoveryCycleEvent {
    Fallback {
        preferred_device_id: Option<String>,
        active_device_id: Option<String>,
    },
    Healthy,
}

struct RecoveryCycle<D: DeviceSelector, C: CaptureFactory> {
    selector: D,
    factory: C,
    preferred_device_id: Option<String>,
    capture_enabled: Arc<AtomicBool>,
    active_device_id: Arc<Mutex<String>>,
    generation: u64,
    current_generation: Arc<AtomicU64>,
    session: Option<C::Session>,
}

impl<D: DeviceSelector, C: CaptureFactory> RecoveryCycle<D, C> {
    fn new(
        selector: D,
        factory: C,
        preferred_device_id: Option<String>,
        capture_enabled: Arc<AtomicBool>,
        active_device_id: Arc<Mutex<String>>,
        generation: u64,
        current_generation: Arc<AtomicU64>,
    ) -> Self {
        Self {
            selector,
            factory,
            preferred_device_id,
            capture_enabled,
            active_device_id,
            generation,
            current_generation,
            session: None,
        }
    }

    fn recover_once(
        &mut self,
        failures: Option<FailureSender>,
        events: &mut Vec<RecoveryCycleEvent>,
    ) -> Result<(), (SpeechFailure, String)> {
        if self.current_generation.load(Ordering::Acquire) != self.generation {
            return Err((SpeechFailure::Stopped, "stale speech generation".into()));
        }
        if let Some(mut session) = self.session.take() {
            session.stop_and_join();
        }
        let resolved = resolve_device(
            self.preferred_device_id.as_deref(),
            &self.selector.devices(),
        );
        if resolved.fallback {
            events.push(RecoveryCycleEvent::Fallback {
                preferred_device_id: self.preferred_device_id.clone(),
                active_device_id: resolved.active_id.clone(),
            });
        }
        let session = self
            .factory
            .open(resolved.open_id.as_deref(), failures)
            .map_err(|error| (SpeechFailure::Audio, error.to_string()))?;
        *self
            .active_device_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            resolved.active_id.unwrap_or_else(|| "default".to_owned());
        self.session = Some(session);
        events.push(RecoveryCycleEvent::Healthy);
        Ok(())
    }

    fn take_session(&mut self) -> C::Session {
        self.session
            .take()
            .expect("recover_once succeeded without a capture session")
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ResolvedDevice {
    open_id: Option<String>,
    active_id: Option<String>,
    fallback: bool,
}

fn resolve_device(preferred: Option<&str>, devices: &[InputDeviceInfo]) -> ResolvedDevice {
    if let Some(id) = preferred
        && devices.iter().any(|device| device.id == id)
    {
        return ResolvedDevice {
            open_id: Some(id.to_owned()),
            active_id: Some(id.to_owned()),
            fallback: false,
        };
    }
    let active_id = devices
        .iter()
        .find(|device| device.is_default)
        .map(|device| device.id.clone());
    ResolvedDevice {
        open_id: None,
        active_id,
        fallback: preferred.is_some(),
    }
}

/// Managed state: at most one running speech pipeline.
pub struct SpeechService {
    running: Mutex<Option<RunningPipeline>>,
    generation: Arc<AtomicU64>,
}

impl Default for SpeechService {
    fn default() -> Self {
        Self {
            running: Mutex::new(None),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl SpeechService {
    fn lock(&self) -> MutexGuard<'_, Option<RunningPipeline>> {
        self.running.lock().unwrap_or_else(|poisoned| {
            // Counters remain usable even if a worker panicked.
            poisoned.into_inner()
        })
    }

    /// Starts the pipeline on a worker thread. Model loading happens
    /// on the worker; state transitions arrive as `stt-state` events.
    ///
    /// # Errors
    ///
    /// Returns an error when a pipeline is already running.
    pub fn start<R: Runtime>(
        &self,
        app: AppHandle<R>,
        paths: SttModelPaths,
        device_id: Option<String>,
    ) -> Result<(), String> {
        let mut guard = self.lock();
        // A worker that stopped on its own leaves a cancelled entry.
        if guard
            .as_ref()
            .is_some_and(|running| running.cancel.load(Ordering::Relaxed))
            && let Some(mut stopped) = guard.take()
            && let Some(worker) = stopped.worker.take()
        {
            let _ = worker.join();
        }
        if guard.is_some() {
            return Err("speech pipeline is already running".to_owned());
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let capture_enabled = Arc::new(AtomicBool::new(true));
        let diagnostics = Arc::new(PipelineDiagnostics::default());
        let dropped_samples = Arc::new(AtomicU64::new(0));
        let failure_metrics = Arc::new(Mutex::new(Arc::new(FailureQueueMetrics::default())));
        let failure_queue_dropped = Arc::new(AtomicU64::new(0));
        let active_device_id = Arc::new(Mutex::new(
            device_id.clone().unwrap_or_else(|| "default".to_owned()),
        ));
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;

        let worker = PipelineWorker {
            app,
            paths,
            device_id,
            cancel: Arc::clone(&cancel),
            capture_enabled: Arc::clone(&capture_enabled),
            active_device_id: Arc::clone(&active_device_id),
            diagnostics: Arc::clone(&diagnostics),
            dropped_samples: Arc::clone(&dropped_samples),
            failure_metrics: Arc::clone(&failure_metrics),
            failure_queue_dropped: Arc::clone(&failure_queue_dropped),
            generation,
            current_generation: Arc::clone(&self.generation),
        };
        let worker = std::thread::Builder::new()
            .name("pw-speech-pipeline".into())
            .spawn(move || worker.run())
            .map_err(|error| format!("failed to spawn speech worker: {error}"))?;

        *guard = Some(RunningPipeline {
            active_device_id,
            cancel,
            capture_enabled,
            diagnostics,
            dropped_samples,
            failure_metrics,
            failure_queue_dropped,
            worker: Some(worker),
        });
        Ok(())
    }

    /// Requests the running pipeline to stop.
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Some(mut running) = self.lock().take() {
            running.cancel.store(true, Ordering::Relaxed);
            if let Some(worker) = running.worker.take() {
                let _ = worker.join();
            }
        }
    }

    /// Enables or disables capture (mute / TTS playback guard).
    pub fn set_capture_enabled(&self, enabled: bool) {
        if let Some(running) = self.lock().as_ref() {
            running.capture_enabled.store(enabled, Ordering::Relaxed);
        }
    }

    #[must_use]
    pub fn diagnostics(&self) -> AudioDiagnosticsDto {
        let guard = self.lock();
        match guard.as_ref() {
            Some(running) => AudioDiagnosticsDto {
                schema_version: SCHEMA_VERSION,
                running: !running.cancel.load(Ordering::Relaxed),
                capture_enabled: running.capture_enabled.load(Ordering::Relaxed),
                frames_processed: running.diagnostics.frames_processed.load(Ordering::Relaxed),
                segments_completed: running
                    .diagnostics
                    .segments_completed
                    .load(Ordering::Relaxed),
                transcripts_accepted: running
                    .diagnostics
                    .transcripts_accepted
                    .load(Ordering::Relaxed),
                transcripts_rejected: running
                    .diagnostics
                    .transcripts_rejected
                    .load(Ordering::Relaxed),
                dropped_samples: running.dropped_samples.load(Ordering::Relaxed),
                failure_queue_depth: running
                    .failure_metrics
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .depth(),
                failure_queue_dropped: aggregate_counter(
                    running.failure_queue_dropped.load(Ordering::Relaxed),
                    running
                        .failure_metrics
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .dropped(),
                ),
            },
            None => AudioDiagnosticsDto {
                schema_version: SCHEMA_VERSION,
                running: false,
                capture_enabled: false,
                frames_processed: 0,
                segments_completed: 0,
                transcripts_accepted: 0,
                transcripts_rejected: 0,
                dropped_samples: 0,
                failure_queue_depth: 0,
                failure_queue_dropped: 0,
            },
        }
    }

    #[must_use]
    pub fn active_device_id(&self) -> String {
        self.lock().as_ref().map_or_else(
            || "inactive".to_owned(),
            |running| {
                running
                    .active_device_id
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
            },
        )
    }
}

struct PipelineWorker<R: Runtime> {
    app: AppHandle<R>,
    paths: SttModelPaths,
    device_id: Option<String>,
    cancel: Arc<AtomicBool>,
    capture_enabled: Arc<AtomicBool>,
    active_device_id: Arc<Mutex<String>>,
    diagnostics: Arc<PipelineDiagnostics>,
    dropped_samples: Arc<AtomicU64>,
    failure_metrics: Arc<Mutex<Arc<FailureQueueMetrics>>>,
    failure_queue_dropped: Arc<AtomicU64>,
    generation: u64,
    current_generation: Arc<AtomicU64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerInterruption {
    Continue,
    Stopped,
    Stale,
}

const fn worker_interruption(cancelled: bool, generation: u64, current: u64) -> WorkerInterruption {
    if cancelled {
        WorkerInterruption::Stopped
    } else if generation != current {
        WorkerInterruption::Stale
    } else {
        WorkerInterruption::Continue
    }
}

impl<R: Runtime> PipelineWorker<R> {
    fn run(self) {
        if self.is_stale() {
            return;
        }
        emit_state(&self.app, SttPhaseDto::Starting, None);
        let mut health = HealthRegistry::new();
        emit_health_events(&self.app, health.snapshots(0));
        let clock = SystemClock;
        let mut policy = BackoffPolicy::new(&clock, Jitter(clock.now_ms().max(1)));
        let mut recovery = RecoveryCycle::new(
            ProductionDeviceSelector,
            ProductionCaptureFactory,
            self.device_id.clone(),
            Arc::clone(&self.capture_enabled),
            Arc::clone(&self.active_device_id),
            self.generation,
            Arc::clone(&self.current_generation),
        );
        loop {
            match self.interruption() {
                WorkerInterruption::Continue => {}
                WorkerInterruption::Stopped => {
                    emit_state(&self.app, SttPhaseDto::Stopped, None);
                    emit_health_events(
                        &self.app,
                        health.mark_stopped(SystemClock.now_ms(), policy.attempts()),
                    );
                    break;
                }
                WorkerInterruption::Stale => break,
            }
            let result = self.build_and_run(&mut recovery, &mut health, || policy.record_healthy());
            if self.interruption() == WorkerInterruption::Stale {
                break;
            }
            let failure = match result {
                Ok(()) if self.cancel.load(Ordering::Acquire) => SpeechFailure::Stopped,
                Ok(()) => SpeechFailure::Audio,
                Err((failure, message)) => {
                    tracing::warn!(%message, ?failure, "speech pipeline unavailable");
                    failure
                }
            };
            if failure == SpeechFailure::Stopped {
                emit_state(&self.app, SttPhaseDto::Stopped, None);
                emit_health_events(
                    &self.app,
                    health.mark_stopped(SystemClock.now_ms(), policy.attempts()),
                );
                break;
            }
            if matches!(failure, SpeechFailure::VadModel | SpeechFailure::SttModel) {
                emit_state(&self.app, SttPhaseDto::Unavailable, Some(failure.message()));
                emit_health_event(
                    &self.app,
                    health.mark_failure(failure, SystemClock.now_ms(), policy.attempts(), true),
                );
                break;
            }
            match policy.record_failure() {
                BackoffDecision::CircuitOpen => {
                    emit_state(&self.app, SttPhaseDto::Unavailable, Some(failure.message()));
                    emit_health_event(
                        &self.app,
                        health.mark_failure(failure, SystemClock.now_ms(), policy.attempts(), true),
                    );
                    break;
                }
                BackoffDecision::RetryAfter(delay) => {
                    emit_health_event(
                        &self.app,
                        health.mark_failure(
                            failure,
                            SystemClock.now_ms(),
                            policy.attempts(),
                            false,
                        ),
                    );
                    emit_state(
                        &self.app,
                        SttPhaseDto::Starting,
                        Some("音声認識を再初期化しています".into()),
                    );
                    if self.wait_retry(delay) {
                        continue;
                    }
                    if self.interruption() == WorkerInterruption::Stopped {
                        emit_state(&self.app, SttPhaseDto::Stopped, None);
                        emit_health_events(
                            &self.app,
                            health.mark_stopped(SystemClock.now_ms(), policy.attempts()),
                        );
                    }
                    break;
                }
            }
        }
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn is_stale(&self) -> bool {
        self.current_generation.load(Ordering::Acquire) != self.generation
    }

    fn interruption(&self) -> WorkerInterruption {
        worker_interruption(
            self.cancel.load(Ordering::Acquire),
            self.generation,
            self.current_generation.load(Ordering::Acquire),
        )
    }

    fn wait_retry(&self, delay: Duration) -> bool {
        let deadline = std::time::Instant::now() + delay;
        while std::time::Instant::now() < deadline {
            if self.cancel.load(Ordering::Acquire) || self.is_stale() {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    fn build_and_run<F>(
        &self,
        recovery: &mut RecoveryCycle<ProductionDeviceSelector, ProductionCaptureFactory>,
        health: &mut HealthRegistry,
        mut on_healthy: F,
    ) -> Result<(), (SpeechFailure, String)>
    where
        F: FnMut(),
    {
        let vad = SileroVad::new(&self.paths.vad_model, 0.5)
            .map_err(|error| classify_model_error(&error, true))?;
        let recognizer = ReazonSpeechRecognizer::new(&RecognizerModelPaths::in_directory(
            &self.paths.recognizer_dir,
        ))
        .map_err(|error| classify_model_error(&error, false))?;

        let (failure_tx, failure_rx, metrics) = failure_channel(4);
        let mut failure_metrics = self
            .failure_metrics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.failure_queue_dropped
            .fetch_add(failure_metrics.dropped(), Ordering::Relaxed);
        *failure_metrics = metrics;
        drop(failure_metrics);
        let mut recovery_events = Vec::new();
        recovery.recover_once(Some(failure_tx), &mut recovery_events)?;
        for event in &recovery_events {
            if let RecoveryCycleEvent::Fallback {
                preferred_device_id,
                active_device_id,
            } = event
            {
                let _ = self.app.emit(
                    DEVICE_FALLBACK_EVENT,
                    DeviceFallbackEventDto {
                        schema_version: SCHEMA_VERSION,
                        preferred_device_id: preferred_device_id.clone(),
                        active_device_id: active_device_id.clone(),
                    },
                );
            }
        }
        let session = recovery.take_session();
        let dropped_counter = Arc::clone(&session.dropped_samples);
        let source = CaptureFrameSource::new(session)
            .map_err(|error| (SpeechFailure::Audio, error.to_string()))?;
        let session_cancel = Arc::new(AtomicBool::new(false));
        // Mirror the capture drop counter into the service counter.
        let mirror = self.mirror_drops(dropped_counter, Arc::clone(&session_cancel));

        let events = TauriSpeechEvents {
            app: self.app.clone(),
            frame_count: AtomicU64::new(0),
            generation: self.generation,
            current_generation: Arc::clone(&self.current_generation),
        };
        if self.is_stale() {
            session_cancel.store(true, Ordering::Release);
            let _ = mirror.join();
            return Ok(());
        }
        emit_state(&self.app, SttPhaseDto::Listening, None);
        emit_health_events(
            &self.app,
            health.mark_pipeline_healthy(SystemClock.now_ms()),
        );
        on_healthy();

        let pipeline = SpeechPipeline::new(
            SpeechPipelineConfig::default(),
            vad,
            recognizer,
            events,
            Arc::clone(&recovery.capture_enabled),
            Arc::clone(&self.diagnostics),
        );
        let result = std::thread::scope(|scope| {
            let pipeline_cancel = Arc::clone(&session_cancel);
            let done = Arc::new(AtomicBool::new(false));
            let pipeline_done = Arc::clone(&done);
            let handle = scope.spawn(move || {
                run_pipeline(source, pipeline, &pipeline_cancel);
                pipeline_done.store(true, Ordering::Release);
            });
            let mut stream_failure = None;
            while !done.load(Ordering::Acquire) && !self.cancel.load(Ordering::Acquire) {
                if let Some(failure) = failure_rx.try_recv() {
                    stream_failure = Some(failure);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            session_cancel.store(true, Ordering::Release);
            let _ = handle.join();
            if self.cancel.load(Ordering::Acquire) {
                Ok(())
            } else if stream_failure.is_some() {
                Err((
                    SpeechFailure::Audio,
                    "audio input stream disconnected".into(),
                ))
            } else {
                Err((
                    SpeechFailure::SttRuntime,
                    "speech pipeline stopped unexpectedly".into(),
                ))
            }
        });
        session_cancel.store(true, Ordering::Release);
        let _ = mirror.join();
        result
    }

    fn mirror_drops(
        &self,
        source: Arc<AtomicU64>,
        session_cancel: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        let target = Arc::clone(&self.dropped_samples);
        let completed = target.load(Ordering::Relaxed);
        let cancel = Arc::clone(&self.cancel);
        std::thread::Builder::new()
            .name("pw-speech-drop-mirror".into())
            .spawn(move || {
                while !cancel.load(Ordering::Relaxed) && !session_cancel.load(Ordering::Relaxed) {
                    target.store(
                        aggregate_counter(completed, source.load(Ordering::Relaxed)),
                        Ordering::Relaxed,
                    );
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                target.store(
                    aggregate_counter(completed, source.load(Ordering::Relaxed)),
                    Ordering::Relaxed,
                );
            })
            .expect("failed to spawn speech diagnostics mirror")
    }
}

const fn aggregate_counter(completed: u64, current: u64) -> u64 {
    completed.saturating_add(current)
}

impl SpeechFailure {
    fn message(self) -> String {
        match self {
            Self::VadModel | Self::SttModel => model_error_message(),
            Self::Audio => "音声入力を利用できません".into(),
            Self::VadRuntime => "音声区間検出を初期化できません".into(),
            Self::SttRuntime => "音声認識を初期化できません".into(),
            Self::Stopped => "音声認識を停止しました".into(),
        }
    }
}

fn classify_model_error(error: &SherpaError, vad: bool) -> (SpeechFailure, String) {
    let failure = match (error, vad) {
        (SherpaError::ModelMissing(_), true) => SpeechFailure::VadModel,
        (SherpaError::ModelMissing(_), false) => SpeechFailure::SttModel,
        (SherpaError::Init(_), true) => SpeechFailure::VadRuntime,
        (SherpaError::Init(_), false) => SpeechFailure::SttRuntime,
    };
    (failure, model_error(error))
}

fn model_error_message() -> String {
    "音声認識モデルがありません。`node tools/scripts/download-stt-models.mjs` で配置してください。"
        .into()
}

fn emit_health_event<R: Runtime>(app: &AppHandle<R>, payload: RuntimeHealthEventDto) {
    let _ = app.emit(RUNTIME_HEALTH_EVENT, payload);
}

fn emit_health_events<R: Runtime, const N: usize>(
    app: &AppHandle<R>,
    payloads: [RuntimeHealthEventDto; N],
) {
    for payload in payloads {
        emit_health_event(app, payload);
    }
}

fn model_error(error: &SherpaError) -> String {
    format!("{error}. `node tools/scripts/download-stt-models.mjs` でモデルを配置してください。")
}

fn emit_state<R: Runtime>(app: &AppHandle<R>, phase: SttPhaseDto, message: Option<String>) {
    let payload = SttStateEventDto {
        schema_version: SCHEMA_VERSION,
        phase,
        message,
    };
    // Broadcast exactly once: every window-scoped listener receives
    // one copy. Emitting per window would deliver duplicates to
    // listeners registered with the default `Any` target.
    if let Err(error) = app.emit(STATE_EVENT, payload) {
        tracing::warn!(%error, "failed to emit stt state");
    }
}

struct TauriSpeechEvents<R: Runtime> {
    app: AppHandle<R>,
    frame_count: AtomicU64,
    generation: u64,
    current_generation: Arc<AtomicU64>,
}

impl<R: Runtime> SpeechEvents for TauriSpeechEvents<R> {
    fn on_level(&self, rms: f32) {
        if self.current_generation.load(Ordering::Acquire) != self.generation {
            return;
        }
        let count = self.frame_count.fetch_add(1, Ordering::Relaxed);
        if !count.is_multiple_of(LEVEL_EVERY_N_FRAMES) {
            return;
        }
        let payload = AudioLevelEventDto {
            schema_version: SCHEMA_VERSION,
            rms,
        };
        let _ = self.app.emit_to(
            EventTarget::webview_window("settings"),
            LEVEL_EVENT,
            payload,
        );
    }

    fn on_speech_started(&self) {}

    fn on_transcript(&self, text: &str) {
        if self.current_generation.load(Ordering::Acquire) != self.generation {
            return;
        }
        let payload = TranscriptEventDto {
            schema_version: SCHEMA_VERSION,
            text: text.to_owned(),
        };
        // Single broadcast; see emit_state for the rationale.
        let _ = self.app.emit(TRANSCRIPT_EVENT, payload);
        // 音声からの応答生成: 確定発話を会話サービスへ投入する。
        let chat = self.app.state::<crate::chat::ChatService>();
        if let Err(error) = chat.submit(&self.app, text.to_owned()) {
            tracing::warn!(%error, "failed to submit transcript to chat");
        }
    }

    fn on_rejected(&self, reason: RejectionReason) {
        tracing::debug!(?reason, "transcript rejected");
    }

    fn on_error(&self, message: &str) {
        tracing::warn!(%message, "speech pipeline error");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use pw_audio::capture::CaptureError;
    use pw_audio::devices::InputDeviceInfo;
    use pw_platform::paths::AppDataLayout;

    use super::{
        CaptureFactory, CaptureLifecycle, DeviceSelector, HealthRegistry, RecoveryCycle,
        RecoveryCycleEvent, SpeechFailure, SpeechService, SttModelPaths, WorkerInterruption,
        aggregate_counter, resolve_device, worker_interruption,
    };
    use pw_contracts::{HealthStatusDto, RuntimeFeatureDto};

    #[test]
    fn missing_preferred_device_falls_back_without_overwriting_preference() {
        let devices = vec![InputDeviceInfo {
            id: "default".into(),
            name: "Built in".into(),
            is_default: true,
        }];
        let resolved = resolve_device(Some("usb-mic"), &devices);
        assert!(resolved.fallback);
        assert_eq!(resolved.open_id, None);
        assert_eq!(resolved.active_id.as_deref(), Some("default"));
    }

    #[test]
    fn available_preferred_device_remains_active() {
        let devices = vec![InputDeviceInfo {
            id: "usb-mic".into(),
            name: "USB".into(),
            is_default: false,
        }];
        let resolved = resolve_device(Some("usb-mic"), &devices);
        assert!(!resolved.fallback);
        assert_eq!(resolved.open_id.as_deref(), Some("usb-mic"));
    }

    #[test]
    fn model_paths_derive_from_the_layout() {
        let layout = AppDataLayout::under(std::path::PathBuf::from("Root"));
        let paths = SttModelPaths::under(&layout);
        assert!(
            paths
                .vad_model
                .ends_with("models/vad/silero-vad-v5/silero_vad.onnx")
        );
        assert!(
            paths
                .recognizer_dir
                .ends_with("models/stt/reazonspeech-k2-v2")
        );
    }

    #[test]
    fn diagnostics_report_not_running_by_default() {
        let service = SpeechService::default();
        let diagnostics = service.diagnostics();
        assert!(!diagnostics.running);
        assert_eq!(diagnostics.frames_processed, 0);
    }

    #[test]
    fn stop_and_mute_are_noops_without_a_pipeline() {
        let service = SpeechService::default();
        service.stop();
        service.set_capture_enabled(false);
        assert!(!service.diagnostics().running);
    }

    #[test]
    fn audio_disconnect_and_rebuild_preserve_stt_health() {
        let mut health = HealthRegistry::new();
        let initial = health.mark_pipeline_healthy(10);
        assert_eq!(initial.len(), 2);

        let failed = health.mark_failure(SpeechFailure::Audio, 20, 1, false);
        assert_eq!(failed.feature, RuntimeFeatureDto::AudioInput);
        assert_eq!(failed.status, HealthStatusDto::Recovering);

        let rebuilt = health.mark_pipeline_healthy(30);
        assert_eq!(rebuilt.len(), 2);
        let stt = rebuilt
            .iter()
            .find(|event| event.feature == RuntimeFeatureDto::SpeechToText)
            .unwrap();
        assert_eq!(stt.status, HealthStatusDto::Healthy);
        assert_eq!(stt.changed_at_ms, 10, "unchanged STT health stays stable");
        let audio = rebuilt
            .iter()
            .find(|event| event.feature == RuntimeFeatureDto::AudioInput)
            .unwrap();
        assert_eq!(audio.changed_at_ms, 30);
    }

    #[test]
    fn stop_transitions_both_features_without_opening_a_circuit() {
        let mut health = HealthRegistry::new();
        health.mark_pipeline_healthy(10);
        let events = health.mark_stopped(20, 7);
        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .all(|event| event.status == HealthStatusDto::Stopped)
        );
        assert!(events.iter().all(|event| !event.circuit_open));
    }

    #[test]
    fn circuit_opens_on_exactly_the_eighth_failure() {
        let mut health = HealthRegistry::new();
        for attempts in 1..8 {
            let event =
                health.mark_failure(SpeechFailure::Audio, u64::from(attempts), attempts, false);
            assert!(!event.circuit_open);
        }
        let event = health.mark_failure(SpeechFailure::Audio, 8, 8, true);
        assert!(event.circuit_open);
        assert_eq!(event.attempts, 8);
    }

    #[test]
    fn explicit_stop_wins_over_generation_staleness() {
        assert_eq!(worker_interruption(true, 1, 2), WorkerInterruption::Stopped);
    }

    #[test]
    fn stale_configuration_suppresses_old_worker_transitions() {
        assert_eq!(worker_interruption(false, 1, 2), WorkerInterruption::Stale);
        assert_eq!(
            worker_interruption(false, 2, 2),
            WorkerInterruption::Continue
        );
    }

    #[test]
    fn diagnostics_counters_remain_monotonic_across_sessions() {
        assert_eq!(aggregate_counter(7, 3), 10);
        assert_eq!(aggregate_counter(10, 2), 12);
        assert_eq!(aggregate_counter(u64::MAX, 1), u64::MAX);
    }

    #[derive(Default)]
    struct FakeDeviceSelector(Vec<InputDeviceInfo>);

    impl DeviceSelector for FakeDeviceSelector {
        fn devices(&self) -> Vec<InputDeviceInfo> {
            self.0.clone()
        }
    }

    #[derive(Default)]
    struct FakeCaptureState {
        opened: Vec<Option<String>>,
        stopped: usize,
    }

    struct FakeCapture {
        state: Arc<Mutex<FakeCaptureState>>,
        stopped: bool,
    }

    impl CaptureLifecycle for FakeCapture {
        fn stop_and_join(&mut self) {
            if !self.stopped {
                self.state.lock().unwrap().stopped += 1;
                self.stopped = true;
            }
        }
    }

    impl Drop for FakeCapture {
        fn drop(&mut self) {
            self.stop_and_join();
        }
    }

    struct FakeCaptureFactory(Arc<Mutex<FakeCaptureState>>);

    impl CaptureFactory for FakeCaptureFactory {
        type Session = FakeCapture;

        fn open(
            &self,
            device_id: Option<&str>,
            _failures: Option<pw_audio::recovery::FailureSender>,
        ) -> Result<Self::Session, CaptureError> {
            self.0
                .lock()
                .unwrap()
                .opened
                .push(device_id.map(str::to_owned));
            Ok(FakeCapture {
                state: Arc::clone(&self.0),
                stopped: false,
            })
        }
    }

    #[test]
    fn production_recovery_cycle_stops_old_session_and_preserves_runtime_state() {
        let state = Arc::new(Mutex::new(FakeCaptureState::default()));
        let enabled = Arc::new(AtomicBool::new(false));
        let active_device_id = Arc::new(Mutex::new("requested".to_owned()));
        let generation = Arc::new(AtomicU64::new(7));
        let mut events = Vec::new();
        let mut cycle = RecoveryCycle::new(
            FakeDeviceSelector(vec![InputDeviceInfo {
                id: "usb-mic".into(),
                name: "USB".into(),
                is_default: false,
            }]),
            FakeCaptureFactory(Arc::clone(&state)),
            Some("usb-mic".into()),
            Arc::clone(&enabled),
            Arc::clone(&active_device_id),
            7,
            Arc::clone(&generation),
        );

        assert!(cycle.recover_once(None, &mut events).is_ok());
        cycle.selector = FakeDeviceSelector(vec![InputDeviceInfo {
            id: "default".into(),
            name: "Built in".into(),
            is_default: true,
        }]);
        assert!(cycle.recover_once(None, &mut events).is_ok());

        assert_eq!(state.lock().unwrap().stopped, 1);
        assert_eq!(*active_device_id.lock().unwrap(), "default");
        assert_eq!(
            state.lock().unwrap().opened,
            vec![Some("usb-mic".into()), None]
        );
        assert!(
            !enabled.load(Ordering::Relaxed),
            "mute state is shared, not reset"
        );
        enabled.store(true, Ordering::Relaxed);
        assert!(enabled.load(Ordering::Relaxed));
        assert_eq!(
            events,
            vec![
                RecoveryCycleEvent::Healthy,
                RecoveryCycleEvent::Fallback {
                    preferred_device_id: Some("usb-mic".into()),
                    active_device_id: Some("default".into()),
                },
                RecoveryCycleEvent::Healthy,
            ]
        );

        generation.store(8, Ordering::Release);
        assert!(cycle.recover_once(None, &mut events).is_err());
        assert_eq!(events.len(), 3, "stale generations publish nothing");
    }
}
