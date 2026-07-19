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
use pw_audio::capture::{
    CaptureError, CaptureSession, start_capture_with_failures_until_cancelled,
};
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
/// Base time a single startup stage may run without reporting progress.
/// Bounds hung platform/model calls, not total startup time: slow
/// stages get [`StartupStage::budget_multiplier`] times this budget,
/// and retries keep marking progress so they are never cut off.
const STARTUP_NO_PROGRESS_TIMEOUT: Duration = Duration::from_secs(30);
const STARTUP_WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(50);

struct SpeechState {
    current: Mutex<VersionedSpeechState>,
}

struct VersionedSpeechState {
    generation: u64,
    event: SttStateEventDto,
}

impl Default for SpeechState {
    fn default() -> Self {
        Self {
            current: Mutex::new(VersionedSpeechState {
                generation: 0,
                event: SttStateEventDto {
                    schema_version: SCHEMA_VERSION,
                    phase: SttPhaseDto::Stopped,
                    message: None,
                },
            }),
        }
    }
}

impl SpeechState {
    fn update_for_generation(
        &self,
        generation: u64,
        phase: SttPhaseDto,
        message: Option<String>,
    ) -> Option<SttStateEventDto> {
        let next = SttStateEventDto {
            schema_version: SCHEMA_VERSION,
            phase,
            message,
        };
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if generation < current.generation {
            return None;
        }
        current.generation = generation;
        current.event = next.clone();
        Some(next)
    }

    fn snapshot(&self) -> SttStateEventDto {
        self.current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .event
            .clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupWatchdogOutcome {
    Ready,
    Cancelled,
    TimedOut,
}

/// Startup stage reported by the worker; shown in timeout diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum StartupStage {
    WaitingPreviousShutdown = 0,
    LoadingVadModel = 1,
    LoadingRecognizerModel = 2,
    OpeningCapture = 3,
    RetryBackoff = 4,
}

impl StartupStage {
    const fn name(self) -> &'static str {
        match self {
            Self::WaitingPreviousShutdown => "waiting for previous pipeline shutdown",
            Self::LoadingVadModel => "loading vad model",
            Self::LoadingRecognizerModel => "loading recognizer model",
            Self::OpeningCapture => "opening audio capture",
            Self::RetryBackoff => "waiting to retry",
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::LoadingVadModel,
            2 => Self::LoadingRecognizerModel,
            3 => Self::OpeningCapture,
            4 => Self::RetryBackoff,
            _ => Self::WaitingPreviousShutdown,
        }
    }

    /// Model loading (and waiting out an old worker stuck inside it)
    /// can legitimately take tens of seconds on a cold file cache with
    /// the 150 MB recognizer being antivirus-scanned on first read.
    /// Only fast stages get the tight base budget.
    const fn budget_multiplier(self) -> u32 {
        match self {
            Self::WaitingPreviousShutdown
            | Self::LoadingVadModel
            | Self::LoadingRecognizerModel => 4,
            Self::OpeningCapture | Self::RetryBackoff => 1,
        }
    }
}

/// Monotonic startup progress shared between the worker and its
/// watchdog. The watchdog only gives up when no stage reports
/// progress for [`STARTUP_NO_PROGRESS_TIMEOUT`].
struct StartupProgress {
    epoch: std::time::Instant,
    last_progress_ms: AtomicU64,
    stage: std::sync::atomic::AtomicU8,
}

impl StartupProgress {
    fn new(stage: StartupStage) -> Self {
        Self {
            epoch: std::time::Instant::now(),
            last_progress_ms: AtomicU64::new(0),
            stage: std::sync::atomic::AtomicU8::new(stage as u8),
        }
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Records progress into a new stage.
    fn mark(&self, stage: StartupStage) {
        self.stage.store(stage as u8, Ordering::Release);
        self.touch();
    }

    /// Records progress within the current stage.
    fn touch(&self) {
        self.last_progress_ms
            .store(self.elapsed_ms(), Ordering::Release);
    }

    fn stalled_for(&self) -> Duration {
        Duration::from_millis(
            self.elapsed_ms()
                .saturating_sub(self.last_progress_ms.load(Ordering::Acquire)),
        )
    }

    fn stage(&self) -> StartupStage {
        StartupStage::from_u8(self.stage.load(Ordering::Acquire))
    }
}

fn wait_for_startup_watchdog(
    state: &SpeechState,
    cancel: &AtomicBool,
    startup_timed_out: &AtomicBool,
    generation: u64,
    current_generation: &AtomicU64,
    progress: &StartupProgress,
    no_progress_timeout: Duration,
) -> StartupWatchdogOutcome {
    loop {
        if cancel.load(Ordering::Acquire)
            || current_generation.load(Ordering::Acquire) != generation
        {
            return StartupWatchdogOutcome::Cancelled;
        }
        if state.snapshot().phase != SttPhaseDto::Starting {
            return StartupWatchdogOutcome::Ready;
        }
        let stalled = progress.stalled_for();
        let budget = no_progress_timeout * progress.stage().budget_multiplier();
        if stalled >= budget {
            return if current_generation
                .compare_exchange(
                    generation,
                    generation.wrapping_add(1),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                startup_timed_out.store(true, Ordering::Release);
                cancel.store(true, Ordering::Release);
                StartupWatchdogOutcome::TimedOut
            } else {
                StartupWatchdogOutcome::Cancelled
            };
        }
        std::thread::sleep(STARTUP_WATCHDOG_POLL_INTERVAL.min(budget.saturating_sub(stalled)));
    }
}

/// Resolved model locations under the app data `models/` directory.
#[derive(Clone)]
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

    fn mark_startup_timeout(&mut self, now_ms: u64) -> [RuntimeHealthEventDto; 2] {
        let failure = RuntimeFailure::transient(FailureCode::Timeout);
        self.audio_input.mark_failed(&failure, now_ms);
        self.speech_to_text.mark_failed(&failure, now_ms);
        let mut events = self.snapshots(1);
        for event in &mut events {
            event.circuit_open = true;
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

struct ProductionCaptureFactory {
    cancel: Arc<AtomicBool>,
}
impl CaptureFactory for ProductionCaptureFactory {
    type Session = CaptureSession;

    fn open(
        &self,
        device_id: Option<&str>,
        failures: Option<FailureSender>,
    ) -> Result<CaptureSession, CaptureError> {
        start_capture_with_failures_until_cancelled(device_id, failures, &self.cancel)
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

/// A start request queued while the previous pipeline is still
/// shutting down; completed by the deferred-start thread.
struct PendingStart {
    paths: SttModelPaths,
    device_id: Option<String>,
}

/// Managed state: at most one running speech pipeline.
pub struct SpeechService {
    lifecycle: Mutex<()>,
    running: Mutex<Option<RunningPipeline>>,
    active_cancel: Mutex<Option<Arc<AtomicBool>>>,
    pending_start: Mutex<Option<PendingStart>>,
    generation: Arc<AtomicU64>,
    state: Arc<SpeechState>,
}

impl Default for SpeechService {
    fn default() -> Self {
        Self {
            lifecycle: Mutex::new(()),
            running: Mutex::new(None),
            active_cancel: Mutex::new(None),
            pending_start: Mutex::new(None),
            generation: Arc::new(AtomicU64::new(0)),
            state: Arc::new(SpeechState::default()),
        }
    }
}

impl SpeechService {
    fn lock_lifecycle(&self) -> MutexGuard<'_, ()> {
        self.lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock(&self) -> MutexGuard<'_, Option<RunningPipeline>> {
        self.running.lock().unwrap_or_else(|poisoned| {
            // Counters remain usable even if a worker panicked.
            poisoned.into_inner()
        })
    }

    fn lock_active_cancel(&self) -> MutexGuard<'_, Option<Arc<AtomicBool>>> {
        self.active_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_pending_start(&self) -> MutexGuard<'_, Option<PendingStart>> {
        self.pending_start
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Starts the pipeline on a worker thread. Model loading happens
    /// on the worker; state transitions arrive as `stt-state` events.
    ///
    /// When the previous pipeline is still shutting down the request
    /// is queued and completed as soon as its worker exits, so a
    /// stop→start toggle (or a retry after a timeout) never fails
    /// with a transient error.
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
        // Start/stop commands only hold this lock while updating lifecycle
        // ownership. Native capture remains entirely on the worker thread.
        let _lifecycle = self.lock_lifecycle();
        let mut guard = self.lock();
        // Reap a completed worker without ever waiting for an unresponsive
        // platform/model backend on the command thread.
        let stopped_worker_finished = guard.as_ref().is_some_and(|running| {
            running.cancel.load(Ordering::Acquire)
                && running
                    .worker
                    .as_ref()
                    .is_none_or(std::thread::JoinHandle::is_finished)
        });
        if stopped_worker_finished
            && let Some(mut stopped) = guard.take()
            && let Some(worker) = stopped.worker.take()
        {
            let _ = worker.join();
        }
        if let Some(running) = guard.as_mut() {
            if !running.cancel.load(Ordering::Acquire) {
                return Err("speech pipeline is already running".to_owned());
            }
            // The old worker may be blocked inside a platform/model
            // call that cannot be interrupted; queue the request
            // instead of joining it on the command thread.
            let worker = running.worker.take();
            drop(guard);
            self.queue_start_while_stopping(app, worker, paths, device_id);
            return Ok(());
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let startup_timed_out = Arc::new(AtomicBool::new(false));
        *self.lock_active_cancel() = Some(Arc::clone(&cancel));
        let capture_enabled = Arc::new(AtomicBool::new(true));
        let diagnostics = Arc::new(PipelineDiagnostics::default());
        let dropped_samples = Arc::new(AtomicU64::new(0));
        let failure_metrics = Arc::new(Mutex::new(Arc::new(FailureQueueMetrics::default())));
        let failure_queue_dropped = Arc::new(AtomicU64::new(0));
        let active_device_id = Arc::new(Mutex::new(
            device_id.clone().unwrap_or_else(|| "default".to_owned()),
        ));
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Some(starting) =
            self.state
                .update_for_generation(generation, SttPhaseDto::Starting, None)
        {
            emit_state(&app, starting);
        }

        let progress = Arc::new(StartupProgress::new(StartupStage::LoadingVadModel));
        let worker = PipelineWorker {
            app: app.clone(),
            paths,
            device_id,
            cancel: Arc::clone(&cancel),
            startup_timed_out: Arc::clone(&startup_timed_out),
            capture_enabled: Arc::clone(&capture_enabled),
            active_device_id: Arc::clone(&active_device_id),
            diagnostics: Arc::clone(&diagnostics),
            dropped_samples: Arc::clone(&dropped_samples),
            failure_metrics: Arc::clone(&failure_metrics),
            failure_queue_dropped: Arc::clone(&failure_queue_dropped),
            progress: Arc::clone(&progress),
            generation,
            current_generation: Arc::clone(&self.generation),
            state: Arc::clone(&self.state),
        };
        let worker = match std::thread::Builder::new()
            .name("pw-speech-pipeline".into())
            .spawn(move || worker.run())
        {
            Ok(worker) => worker,
            Err(error) => {
                self.lock_active_cancel().take();
                let message = format!("failed to spawn speech worker: {error}");
                if let Some(unavailable) = self.state.update_for_generation(
                    generation,
                    SttPhaseDto::Unavailable,
                    Some(message.clone()),
                ) {
                    emit_state(&app, unavailable);
                }
                return Err(message);
            }
        };

        let watchdog_cancel = Arc::clone(&cancel);
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
        self.spawn_startup_watchdog(
            app,
            watchdog_cancel,
            startup_timed_out,
            generation,
            progress,
        );
        Ok(())
    }

    /// Records the start request and completes it once the previous
    /// worker has exited; state transitions keep flowing meanwhile.
    fn queue_start_while_stopping<R: Runtime>(
        &self,
        app: AppHandle<R>,
        worker: Option<JoinHandle<()>>,
        paths: SttModelPaths,
        device_id: Option<String>,
    ) {
        *self.lock_pending_start() = Some(PendingStart { paths, device_id });
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Some(starting) = self.state.update_for_generation(
            generation,
            SttPhaseDto::Starting,
            Some("前回の音声認識の終了を待っています".into()),
        ) {
            emit_state(&app, starting);
        }
        // The watchdog still supervises the wait: a worker that never
        // exits surfaces as a startup timeout instead of silence.
        let progress = Arc::new(StartupProgress::new(StartupStage::WaitingPreviousShutdown));
        self.spawn_startup_watchdog(
            app.clone(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            generation,
            progress,
        );
        if let Some(worker) = worker {
            spawn_deferred_start(app, worker);
        }
    }

    fn spawn_startup_watchdog<R: Runtime>(
        &self,
        app: AppHandle<R>,
        cancel: Arc<AtomicBool>,
        startup_timed_out: Arc<AtomicBool>,
        generation: u64,
        progress: Arc<StartupProgress>,
    ) {
        let state = Arc::clone(&self.state);
        let current_generation = Arc::clone(&self.generation);
        if let Err(error) = std::thread::Builder::new()
            .name("pw-speech-startup-watchdog".into())
            .spawn(move || {
                if wait_for_startup_watchdog(
                    &state,
                    &cancel,
                    &startup_timed_out,
                    generation,
                    &current_generation,
                    &progress,
                    STARTUP_NO_PROGRESS_TIMEOUT,
                ) == StartupWatchdogOutcome::TimedOut
                {
                    tracing::warn!(
                        stage = progress.stage().name(),
                        stalled_ms = progress.stalled_for().as_millis(),
                        "speech startup made no progress; marking unavailable"
                    );
                    let payload = state.update_for_generation(
                        generation.wrapping_add(1),
                        SttPhaseDto::Unavailable,
                        Some("音声認識の起動がタイムアウトしました。再試行してください。".into()),
                    );
                    if let Some(payload) = payload {
                        emit_state(&app, payload);
                    }
                    let mut health = HealthRegistry::new();
                    emit_health_events(&app, health.mark_startup_timeout(SystemClock.now_ms()));
                }
            })
        {
            tracing::warn!(%error, "failed to spawn speech startup watchdog");
        }
    }

    /// Requests the running pipeline to stop.
    pub fn stop(&self) -> SttStateEventDto {
        let _lifecycle = self.lock_lifecycle();
        // A stop also withdraws any start still waiting on the old
        // worker: the user's last request wins.
        *self.lock_pending_start() = None;
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Some(cancel) = self.lock_active_cancel().as_ref() {
            cancel.store(true, Ordering::Release);
        }
        self.state
            .update_for_generation(generation, SttPhaseDto::Stopped, None)
            .unwrap_or_else(|| self.state.snapshot())
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
            Some(running) => {
                // Read both metrics under a single short-lived lock.
                // Locking twice inside one struct literal deadlocks:
                // the first guard is a temporary that lives until the
                // end of the whole statement.
                let (failure_queue_depth, failure_metrics_dropped) = {
                    let metrics = running
                        .failure_metrics
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    (metrics.depth(), metrics.dropped())
                };
                AudioDiagnosticsDto {
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
                    failure_queue_depth,
                    failure_queue_dropped: aggregate_counter(
                        running.failure_queue_dropped.load(Ordering::Relaxed),
                        failure_metrics_dropped,
                    ),
                }
            }
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
                if running.cancel.load(Ordering::Acquire) {
                    "inactive".to_owned()
                } else {
                    running
                        .active_device_id
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone()
                }
            },
        )
    }

    #[must_use]
    pub fn current_state(&self) -> SttStateEventDto {
        self.state.snapshot()
    }
}

/// Test-only hooks for the lifecycle integration tests in
/// `tests/speech_lifecycle.rs`. Those tests need a mock Tauri app,
/// which links dialog code importing `TaskDialogIndirect`; only
/// integration test binaries receive the Common-Controls v6 manifest
/// (see `build.rs`), so they cannot live in the unit test module.
#[doc(hidden)]
impl SpeechService {
    /// Installs a cancelled pipeline whose worker blocks until the
    /// returned sender fires, emulating a shutdown stuck inside an
    /// uninterruptible platform call.
    pub fn testing_install_blocked_stopping_worker(&self) -> std::sync::mpsc::Sender<()> {
        let cancel = Arc::new(AtomicBool::new(true));
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::Builder::new()
            .name("pw-speech-test-blocked-worker".into())
            .spawn(move || {
                let _ = release_rx.recv();
            })
            .expect("failed to spawn blocked test worker");
        *self.lock_active_cancel() = Some(Arc::clone(&cancel));
        *self.lock() = Some(RunningPipeline {
            active_device_id: Arc::new(Mutex::new("test-device".to_owned())),
            cancel,
            capture_enabled: Arc::new(AtomicBool::new(true)),
            diagnostics: Arc::new(PipelineDiagnostics::default()),
            dropped_samples: Arc::new(AtomicU64::new(0)),
            failure_metrics: Arc::new(Mutex::new(Arc::new(FailureQueueMetrics::default()))),
            failure_queue_dropped: Arc::new(AtomicU64::new(0)),
            worker: Some(worker),
        });
        release_tx
    }

    #[must_use]
    pub fn testing_has_running_entry(&self) -> bool {
        self.lock().is_some()
    }

    #[must_use]
    pub fn testing_has_pending_start(&self) -> bool {
        self.lock_pending_start().is_some()
    }
}

/// Completes a queued start once the previous worker has exited.
/// Runs on its own thread because the join may block for as long as
/// the old worker is stuck inside an uninterruptible platform call.
fn spawn_deferred_start<R: Runtime>(app: AppHandle<R>, worker: JoinHandle<()>) {
    if let Err(error) = std::thread::Builder::new()
        .name("pw-speech-deferred-start".into())
        .spawn(move || {
            let _ = worker.join();
            let service = app.state::<SpeechService>();
            {
                let mut guard = service.lock();
                if guard.as_ref().is_some_and(|running| {
                    running.cancel.load(Ordering::Acquire) && running.worker.is_none()
                }) {
                    guard.take();
                }
            }
            let pending = service.lock_pending_start().take();
            let Some(pending) = pending else {
                return;
            };
            if let Err(error) = service.start(app.clone(), pending.paths, pending.device_id) {
                tracing::warn!(%error, "deferred speech start failed");
            }
        })
    {
        tracing::warn!(%error, "failed to spawn deferred speech start");
    }
}

/// Models kept alive across audio-only retry cycles; reloading the
/// ~150 MB recognizer on every capture retry would stall recovery.
struct LoadedModels {
    vad: SileroVad,
    recognizer: ReazonSpeechRecognizer,
}

struct PipelineWorker<R: Runtime> {
    app: AppHandle<R>,
    paths: SttModelPaths,
    device_id: Option<String>,
    cancel: Arc<AtomicBool>,
    startup_timed_out: Arc<AtomicBool>,
    capture_enabled: Arc<AtomicBool>,
    active_device_id: Arc<Mutex<String>>,
    diagnostics: Arc<PipelineDiagnostics>,
    dropped_samples: Arc<AtomicU64>,
    failure_metrics: Arc<Mutex<Arc<FailureQueueMetrics>>>,
    failure_queue_dropped: Arc<AtomicU64>,
    progress: Arc<StartupProgress>,
    generation: u64,
    current_generation: Arc<AtomicU64>,
    state: Arc<SpeechState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerInterruption {
    Continue,
    Stopped,
    Stale,
    TimedOut,
}

const fn worker_interruption(
    startup_timed_out: bool,
    cancelled: bool,
    generation: u64,
    current: u64,
) -> WorkerInterruption {
    if startup_timed_out {
        WorkerInterruption::TimedOut
    } else if cancelled {
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
        self.emit_state(SttPhaseDto::Starting, None);
        let mut health = HealthRegistry::new();
        emit_health_events(&self.app, health.snapshots(0));
        let clock = SystemClock;
        let mut policy = BackoffPolicy::new(&clock, Jitter(clock.now_ms().max(1)));
        let mut recovery = RecoveryCycle::new(
            ProductionDeviceSelector,
            ProductionCaptureFactory {
                cancel: Arc::clone(&self.cancel),
            },
            self.device_id.clone(),
            Arc::clone(&self.capture_enabled),
            Arc::clone(&self.active_device_id),
            self.generation,
            Arc::clone(&self.current_generation),
        );
        let mut models: Option<LoadedModels> = None;
        loop {
            match self.interruption() {
                WorkerInterruption::Continue => {}
                WorkerInterruption::Stopped => {
                    self.emit_stopped(&mut health, policy.attempts());
                    break;
                }
                WorkerInterruption::Stale | WorkerInterruption::TimedOut => break,
            }
            let result = match self.ensure_models(&mut models) {
                Ok(loaded) => self.build_and_run(loaded, &mut recovery, &mut health, || {
                    policy.record_healthy();
                }),
                Err(failure) => Err(failure),
            };
            if self.was_superseded() {
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
                self.emit_stopped(&mut health, policy.attempts());
                break;
            }
            if failure != SpeechFailure::Audio {
                // Model files or inference state are suspect; reload on
                // the next cycle. Audio failures keep the loaded models.
                models = None;
            }
            if matches!(failure, SpeechFailure::VadModel | SpeechFailure::SttModel) {
                self.emit_unavailable(&mut health, failure, policy.attempts());
                break;
            }
            match policy.record_failure() {
                BackoffDecision::CircuitOpen => {
                    self.emit_unavailable(&mut health, failure, policy.attempts());
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
                    self.emit_state(
                        SttPhaseDto::Starting,
                        Some("音声認識を再初期化しています".into()),
                    );
                    self.progress.mark(StartupStage::RetryBackoff);
                    if self.wait_retry(delay) {
                        continue;
                    }
                    if self.interruption() == WorkerInterruption::Stopped {
                        self.emit_stopped(&mut health, policy.attempts());
                    }
                    break;
                }
            }
        }
        self.cancel.store(true, Ordering::Release);
        if self.startup_timed_out.load(Ordering::Acquire) {
            // Marks when the uninterruptible call that exhausted the
            // watchdog budget finally returned.
            tracing::info!("speech worker exited after startup timeout");
        }
    }

    fn emit_stopped(&self, health: &mut HealthRegistry, attempts: u8) {
        self.emit_state(SttPhaseDto::Stopped, None);
        emit_health_events(
            &self.app,
            health.mark_stopped(SystemClock.now_ms(), attempts),
        );
    }

    fn emit_unavailable(&self, health: &mut HealthRegistry, failure: SpeechFailure, attempts: u8) {
        self.emit_state(SttPhaseDto::Unavailable, Some(failure.message()));
        emit_health_event(
            &self.app,
            health.mark_failure(failure, SystemClock.now_ms(), attempts, true),
        );
    }

    /// Loads VAD and recognizer models unless a previous cycle already
    /// left healthy instances behind.
    fn ensure_models<'m>(
        &self,
        models: &'m mut Option<LoadedModels>,
    ) -> Result<&'m mut LoadedModels, (SpeechFailure, String)> {
        if models.is_none() {
            let started = std::time::Instant::now();
            self.progress.mark(StartupStage::LoadingVadModel);
            let vad = SileroVad::new(&self.paths.vad_model, 0.5)
                .map_err(|error| classify_model_error(&error, true))?;
            self.progress.mark(StartupStage::LoadingRecognizerModel);
            let recognizer = ReazonSpeechRecognizer::new(&RecognizerModelPaths::in_directory(
                &self.paths.recognizer_dir,
            ))
            .map_err(|error| classify_model_error(&error, false))?;
            tracing::info!(
                elapsed_ms = started.elapsed().as_millis(),
                "speech models loaded"
            );
            *models = Some(LoadedModels { vad, recognizer });
        }
        Ok(models
            .as_mut()
            .expect("models were just loaded or already present"))
    }

    fn was_superseded(&self) -> bool {
        matches!(
            self.interruption(),
            WorkerInterruption::Stale | WorkerInterruption::TimedOut
        )
    }

    fn is_stale(&self) -> bool {
        self.current_generation.load(Ordering::Acquire) != self.generation
    }

    fn emit_state(&self, phase: SttPhaseDto, message: Option<String>) {
        if let Some(payload) = self
            .state
            .update_for_generation(self.generation, phase, message)
        {
            emit_state(&self.app, payload);
        }
    }

    fn interruption(&self) -> WorkerInterruption {
        worker_interruption(
            self.startup_timed_out.load(Ordering::Acquire),
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
            // Deliberate backoff waits count as startup progress.
            self.progress.touch();
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    fn build_and_run<F>(
        &self,
        models: &mut LoadedModels,
        recovery: &mut RecoveryCycle<ProductionDeviceSelector, ProductionCaptureFactory>,
        health: &mut HealthRegistry,
        mut on_healthy: F,
    ) -> Result<(), (SpeechFailure, String)>
    where
        F: FnMut(),
    {
        let (failure_tx, failure_rx, metrics) = failure_channel(4);
        let mut failure_metrics = self
            .failure_metrics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.failure_queue_dropped
            .fetch_add(failure_metrics.dropped(), Ordering::Relaxed);
        *failure_metrics = metrics;
        drop(failure_metrics);
        self.progress.mark(StartupStage::OpeningCapture);
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
        self.emit_state(SttPhaseDto::Listening, None);
        tracing::info!("speech pipeline listening");
        emit_health_events(
            &self.app,
            health.mark_pipeline_healthy(SystemClock.now_ms()),
        );
        on_healthy();

        let pipeline = SpeechPipeline::new(
            SpeechPipelineConfig::default(),
            &mut models.vad,
            &mut models.recognizer,
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

fn emit_state<R: Runtime>(app: &AppHandle<R>, payload: SttStateEventDto) {
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
        if crate::commands::safety::intercept_user_input(&self.app, text) {
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
    use std::time::Duration;

    use pw_application::speech::PipelineDiagnostics;
    use pw_audio::capture::CaptureError;
    use pw_audio::devices::InputDeviceInfo;
    use pw_audio::recovery::FailureQueueMetrics;
    use pw_platform::paths::AppDataLayout;

    use super::{
        CaptureFactory, CaptureLifecycle, DeviceSelector, HealthRegistry, RecoveryCycle,
        RecoveryCycleEvent, RunningPipeline, SpeechFailure, SpeechService, SpeechState,
        StartupProgress, StartupStage, StartupWatchdogOutcome, SttModelPaths, WorkerInterruption,
        aggregate_counter, resolve_device, wait_for_startup_watchdog, worker_interruption,
    };
    use pw_contracts::{HealthStatusDto, RuntimeFeatureDto, SttPhaseDto};

    #[test]
    fn speech_state_keeps_the_latest_phase_for_late_subscribers() {
        let state = SpeechState::default();
        assert_eq!(state.snapshot().phase, SttPhaseDto::Stopped);

        state.update_for_generation(
            1,
            SttPhaseDto::Starting,
            Some("initializing microphone".into()),
        );

        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, SttPhaseDto::Starting);
        assert_eq!(snapshot.message.as_deref(), Some("initializing microphone"));
    }

    #[test]
    fn startup_watchdog_cancels_a_pipeline_that_never_becomes_ready() {
        let state = SpeechState::default();
        state.update_for_generation(7, SttPhaseDto::Starting, None);
        let cancel = AtomicBool::new(false);
        let startup_timed_out = AtomicBool::new(false);
        let current_generation = AtomicU64::new(7);
        let progress = StartupProgress::new(StartupStage::LoadingVadModel);

        let outcome = wait_for_startup_watchdog(
            &state,
            &cancel,
            &startup_timed_out,
            7,
            &current_generation,
            &progress,
            Duration::ZERO,
        );

        assert_eq!(outcome, StartupWatchdogOutcome::TimedOut);
        assert!(cancel.load(Ordering::Acquire));
        assert!(startup_timed_out.load(Ordering::Acquire));
        assert_eq!(current_generation.load(Ordering::Acquire), 8);
    }

    #[test]
    fn startup_watchdog_finishes_when_listening_begins() {
        let state = SpeechState::default();
        state.update_for_generation(3, SttPhaseDto::Listening, None);
        let cancel = AtomicBool::new(false);
        let startup_timed_out = AtomicBool::new(false);
        let current_generation = AtomicU64::new(3);
        let progress = StartupProgress::new(StartupStage::LoadingVadModel);

        let outcome = wait_for_startup_watchdog(
            &state,
            &cancel,
            &startup_timed_out,
            3,
            &current_generation,
            &progress,
            Duration::from_secs(1),
        );

        assert_eq!(outcome, StartupWatchdogOutcome::Ready);
        assert!(!cancel.load(Ordering::Acquire));
        assert!(!startup_timed_out.load(Ordering::Acquire));
        assert_eq!(current_generation.load(Ordering::Acquire), 3);
    }

    #[test]
    fn slow_startup_stages_get_a_larger_no_progress_budget() {
        assert_eq!(StartupStage::WaitingPreviousShutdown.budget_multiplier(), 4);
        assert_eq!(StartupStage::LoadingVadModel.budget_multiplier(), 4);
        assert_eq!(StartupStage::LoadingRecognizerModel.budget_multiplier(), 4);
        assert_eq!(StartupStage::OpeningCapture.budget_multiplier(), 1);
        assert_eq!(StartupStage::RetryBackoff.budget_multiplier(), 1);
    }

    #[test]
    fn startup_watchdog_tolerates_slow_but_progressing_startup() {
        let state = Arc::new(SpeechState::default());
        state.update_for_generation(5, SttPhaseDto::Starting, None);
        let cancel = AtomicBool::new(false);
        let startup_timed_out = AtomicBool::new(false);
        let current_generation = AtomicU64::new(5);
        let progress = Arc::new(StartupProgress::new(StartupStage::LoadingRecognizerModel));

        // A worker that takes far longer than the no-progress budget
        // but keeps reporting progress must never be cut off.
        let worker_progress = Arc::clone(&progress);
        let worker_state = Arc::clone(&state);
        let worker = std::thread::spawn(move || {
            for _ in 0..30 {
                worker_progress.touch();
                std::thread::sleep(Duration::from_millis(10));
            }
            worker_state.update_for_generation(5, SttPhaseDto::Listening, None);
        });

        let outcome = wait_for_startup_watchdog(
            &state,
            &cancel,
            &startup_timed_out,
            5,
            &current_generation,
            &progress,
            Duration::from_millis(120),
        );

        assert_eq!(outcome, StartupWatchdogOutcome::Ready);
        assert!(!startup_timed_out.load(Ordering::Acquire));
        worker.join().unwrap();
    }

    #[test]
    fn stale_worker_cannot_overwrite_a_newer_timeout_state() {
        let state = SpeechState::default();
        assert!(
            state
                .update_for_generation(
                    8,
                    SttPhaseDto::Unavailable,
                    Some("startup timed out".into()),
                )
                .is_some()
        );

        assert!(
            state
                .update_for_generation(7, SttPhaseDto::Listening, None)
                .is_none()
        );
        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, SttPhaseDto::Unavailable);
        assert_eq!(snapshot.message.as_deref(), Some("startup timed out"));
    }

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

    /// Regression: the heartbeat calls `diagnostics()` every second, and
    /// locking `failure_metrics` twice inside one struct literal made the
    /// call self-deadlock as soon as a pipeline existed, which then froze
    /// speech startup at the metrics swap until the watchdog fired.
    #[test]
    fn diagnostics_with_a_running_pipeline_do_not_deadlock() {
        let service = Arc::new(SpeechService::default());
        let release = service.testing_install_blocked_stopping_worker();

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker_service = Arc::clone(&service);
        let probe = std::thread::spawn(move || {
            let _ = done_tx.send(worker_service.diagnostics());
        });

        let diagnostics = done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("diagnostics deadlocked with a running pipeline");
        assert!(!diagnostics.running, "installed pipeline is cancelled");
        release.send(()).unwrap();
        probe.join().unwrap();
    }

    #[test]
    fn stop_and_mute_are_noops_without_a_pipeline() {
        let service = SpeechService::default();
        service.stop();
        service.set_capture_enabled(false);
        assert!(!service.diagnostics().running);
    }

    #[test]
    fn stop_does_not_wait_for_an_unresponsive_worker() {
        let service = SpeechService::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = release_rx.recv();
        });
        *service.lock_active_cancel() = Some(Arc::clone(&cancel));
        *service.lock() = Some(RunningPipeline {
            active_device_id: Arc::new(Mutex::new("test-device".to_owned())),
            cancel: Arc::clone(&cancel),
            capture_enabled: Arc::new(AtomicBool::new(true)),
            diagnostics: Arc::new(PipelineDiagnostics::default()),
            dropped_samples: Arc::new(AtomicU64::new(0)),
            failure_metrics: Arc::new(Mutex::new(Arc::new(FailureQueueMetrics::default()))),
            failure_queue_dropped: Arc::new(AtomicU64::new(0)),
            worker: Some(worker),
        });
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            let _ = release_tx.send(());
        });

        let started = std::time::Instant::now();
        service.stop();
        let state = service.current_state();

        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(cancel.load(Ordering::Acquire));
        assert_eq!(state.phase, SttPhaseDto::Stopped);
        assert_eq!(service.active_device_id(), "inactive");
        let _ = releaser.join();
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
    fn startup_timeout_keeps_both_features_unavailable() {
        let mut health = HealthRegistry::new();
        let events = health.mark_startup_timeout(20);

        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .all(|event| event.status == HealthStatusDto::Recovering)
        );
        assert!(events.iter().all(|event| event.circuit_open));
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
        assert_eq!(
            worker_interruption(false, true, 1, 2),
            WorkerInterruption::Stopped
        );
    }

    #[test]
    fn startup_timeout_wins_over_cancelled_worker_cleanup() {
        assert_eq!(
            worker_interruption(true, true, 1, 2),
            WorkerInterruption::TimedOut
        );
    }

    #[test]
    fn stale_configuration_suppresses_old_worker_transitions() {
        assert_eq!(
            worker_interruption(false, false, 1, 2),
            WorkerInterruption::Stale
        );
        assert_eq!(
            worker_interruption(false, false, 2, 2),
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
