//! Lifecycle owner of the speech pipeline worker.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use pw_application::speech::{
    PipelineDiagnostics, SpeechEvents, SpeechPipeline, SpeechPipelineConfig, run_pipeline,
};
use pw_audio::capture::start_capture_with_failures;
use pw_audio::frame_source::CaptureFrameSource;
use pw_audio::recovery::failure_channel;
use pw_contracts::{
    AudioDiagnosticsDto, AudioLevelEventDto, RUNTIME_HEALTH_EVENT, RuntimeHealthEventDto,
    SCHEMA_VERSION, SttPhaseDto, SttStateEventDto, TranscriptEventDto,
};
use pw_domain::runtime_health::{FailureCode, RuntimeFailure, RuntimeFeature, RuntimeHealth};
use pw_domain::speech::RejectionReason;
use pw_platform::paths::AppDataLayout;
use pw_stt_sherpa::{ReazonSpeechRecognizer, RecognizerModelPaths, SherpaError, SileroVad};
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime};

pub const TRANSCRIPT_EVENT: &str = "stt-transcript";
pub const LEVEL_EVENT: &str = "stt-level";
pub const STATE_EVENT: &str = "stt-state";

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
    cancel: Arc<AtomicBool>,
    capture_enabled: Arc<AtomicBool>,
    diagnostics: Arc<PipelineDiagnostics>,
    dropped_samples: Arc<AtomicU64>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryAction {
    Retry,
    OpenCircuit,
    Stop,
}

struct SpeechRetryPolicy;

impl SpeechRetryPolicy {
    const MAX_ATTEMPTS: u8 = 8;

    fn action(failure: SpeechFailure, attempts: u8) -> RetryAction {
        match failure {
            SpeechFailure::Stopped => RetryAction::Stop,
            SpeechFailure::VadModel | SpeechFailure::SttModel => RetryAction::OpenCircuit,
            _ if attempts >= Self::MAX_ATTEMPTS => RetryAction::OpenCircuit,
            _ => RetryAction::Retry,
        }
    }
}

/// Managed state: at most one running speech pipeline.
#[derive(Default)]
pub struct SpeechService {
    running: Mutex<Option<RunningPipeline>>,
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

        let worker = PipelineWorker {
            app,
            paths,
            device_id,
            cancel: Arc::clone(&cancel),
            capture_enabled: Arc::clone(&capture_enabled),
            diagnostics: Arc::clone(&diagnostics),
            dropped_samples: Arc::clone(&dropped_samples),
        };
        let worker = std::thread::Builder::new()
            .name("pw-speech-pipeline".into())
            .spawn(move || worker.run())
            .map_err(|error| format!("failed to spawn speech worker: {error}"))?;

        *guard = Some(RunningPipeline {
            cancel,
            capture_enabled,
            diagnostics,
            dropped_samples,
            worker: Some(worker),
        });
        Ok(())
    }

    /// Requests the running pipeline to stop.
    pub fn stop(&self) {
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
            },
        }
    }
}

struct PipelineWorker<R: Runtime> {
    app: AppHandle<R>,
    paths: SttModelPaths,
    device_id: Option<String>,
    cancel: Arc<AtomicBool>,
    capture_enabled: Arc<AtomicBool>,
    diagnostics: Arc<PipelineDiagnostics>,
    dropped_samples: Arc<AtomicU64>,
}

impl<R: Runtime> PipelineWorker<R> {
    fn run(self) {
        emit_state(&self.app, SttPhaseDto::Starting, None);
        emit_health(&self.app, None, 0, false);
        let mut attempts = 0_u8;
        loop {
            let result = self.build_and_run();
            let failure = match result {
                Ok(()) if self.cancel.load(Ordering::Acquire) => SpeechFailure::Stopped,
                Ok(()) => SpeechFailure::Audio,
                Err((failure, message)) => {
                    tracing::warn!(%message, ?failure, "speech pipeline unavailable");
                    failure
                }
            };
            match SpeechRetryPolicy::action(failure, attempts) {
                RetryAction::Stop => {
                    emit_state(&self.app, SttPhaseDto::Stopped, None);
                    emit_health(&self.app, Some(SpeechFailure::Stopped), attempts, false);
                    break;
                }
                RetryAction::OpenCircuit => {
                    emit_state(&self.app, SttPhaseDto::Unavailable, Some(failure.message()));
                    emit_health(&self.app, Some(failure), attempts, true);
                    break;
                }
                RetryAction::Retry => {
                    attempts += 1;
                    emit_health(&self.app, Some(failure), attempts, false);
                    emit_state(
                        &self.app,
                        SttPhaseDto::Starting,
                        Some("音声認識を再初期化しています".into()),
                    );
                    if self.wait_retry(attempts) {
                        continue;
                    }
                    emit_state(&self.app, SttPhaseDto::Stopped, None);
                    break;
                }
            }
        }
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn wait_retry(&self, attempt: u8) -> bool {
        let delay = Duration::from_millis(250_u64.saturating_mul(1_u64 << attempt.min(7)));
        let deadline = std::time::Instant::now() + delay.min(Duration::from_secs(30));
        while std::time::Instant::now() < deadline {
            if self.cancel.load(Ordering::Acquire) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    fn build_and_run(&self) -> Result<(), (SpeechFailure, String)> {
        let vad = SileroVad::new(&self.paths.vad_model, 0.5)
            .map_err(|error| classify_model_error(&error, true))?;
        let recognizer = ReazonSpeechRecognizer::new(&RecognizerModelPaths::in_directory(
            &self.paths.recognizer_dir,
        ))
        .map_err(|error| classify_model_error(&error, false))?;

        let (failure_tx, failure_rx, _) = failure_channel(4);
        let session = start_capture_with_failures(self.device_id.as_deref(), Some(failure_tx))
            .map_err(|error| (SpeechFailure::Audio, error.to_string()))?;
        let dropped_counter = Arc::clone(&session.dropped_samples);
        let source = CaptureFrameSource::new(session)
            .map_err(|error| (SpeechFailure::Audio, error.to_string()))?;
        let session_cancel = Arc::new(AtomicBool::new(false));
        // Mirror the capture drop counter into the service counter.
        let mirror = self.mirror_drops(dropped_counter, Arc::clone(&session_cancel));

        let events = TauriSpeechEvents {
            app: self.app.clone(),
            frame_count: AtomicU64::new(0),
        };
        emit_state(&self.app, SttPhaseDto::Listening, None);
        emit_health(&self.app, None, 0, false);

        let pipeline = SpeechPipeline::new(
            SpeechPipelineConfig::default(),
            vad,
            recognizer,
            events,
            Arc::clone(&self.capture_enabled),
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
        let cancel = Arc::clone(&self.cancel);
        std::thread::Builder::new()
            .name("pw-speech-drop-mirror".into())
            .spawn(move || {
                while !cancel.load(Ordering::Relaxed) && !session_cancel.load(Ordering::Relaxed) {
                    target.store(source.load(Ordering::Relaxed), Ordering::Relaxed);
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            })
            .expect("failed to spawn speech diagnostics mirror")
    }
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

fn emit_health<R: Runtime>(
    app: &AppHandle<R>,
    failure: Option<SpeechFailure>,
    attempts: u8,
    circuit_open: bool,
) {
    let feature = if failure == Some(SpeechFailure::Audio) {
        RuntimeFeature::AudioInput
    } else {
        RuntimeFeature::SpeechToText
    };
    let mut health = RuntimeHealth::new(feature);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        });
    match failure {
        None => health.mark_healthy(now),
        Some(SpeechFailure::Stopped) => health.mark_stopped(now),
        Some(SpeechFailure::VadModel | SpeechFailure::SttModel) => {
            health.mark_failed(&RuntimeFailure::permanent(FailureCode::MissingModel), now);
        }
        Some(SpeechFailure::Audio) => {
            health.mark_failed(&RuntimeFailure::transient(FailureCode::Unavailable), now);
        }
        Some(SpeechFailure::VadRuntime | SpeechFailure::SttRuntime) => {
            health.mark_failed(&RuntimeFailure::transient(FailureCode::Internal), now);
        }
    }
    let mut payload = RuntimeHealthEventDto::from((&health, attempts));
    payload.circuit_open = circuit_open;
    let _ = app.emit(RUNTIME_HEALTH_EVENT, payload);
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
}

impl<R: Runtime> SpeechEvents for TauriSpeechEvents<R> {
    fn on_level(&self, rms: f32) {
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
    use pw_platform::paths::AppDataLayout;

    use super::{RetryAction, SpeechFailure, SpeechRetryPolicy, SpeechService, SttModelPaths};

    #[test]
    fn transient_pipeline_failures_retry_with_a_bound() {
        assert_eq!(
            SpeechRetryPolicy::action(SpeechFailure::Audio, 0),
            RetryAction::Retry
        );
        assert_eq!(
            SpeechRetryPolicy::action(SpeechFailure::VadRuntime, 7),
            RetryAction::Retry
        );
        assert_eq!(
            SpeechRetryPolicy::action(SpeechFailure::SttRuntime, 8),
            RetryAction::OpenCircuit
        );
    }

    #[test]
    fn missing_models_open_the_circuit_without_retry() {
        assert_eq!(
            SpeechRetryPolicy::action(SpeechFailure::VadModel, 0),
            RetryAction::OpenCircuit
        );
        assert_eq!(
            SpeechRetryPolicy::action(SpeechFailure::SttModel, 0),
            RetryAction::OpenCircuit
        );
    }

    #[test]
    fn explicit_stop_never_retries() {
        assert_eq!(
            SpeechRetryPolicy::action(SpeechFailure::Stopped, 0),
            RetryAction::Stop
        );
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
}
