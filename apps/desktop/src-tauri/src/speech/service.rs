//! Lifecycle owner of the speech pipeline worker.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use pw_application::speech::{
    PipelineDiagnostics, SpeechEvents, SpeechPipeline, SpeechPipelineConfig, run_pipeline,
};
use pw_audio::capture::start_capture;
use pw_audio::frame_source::CaptureFrameSource;
use pw_contracts::{
    AudioDiagnosticsDto, AudioLevelEventDto, SCHEMA_VERSION, SttPhaseDto, SttStateEventDto,
    TranscriptEventDto,
};
use pw_domain::speech::RejectionReason;
use pw_platform::paths::AppDataLayout;
use pw_stt_sherpa::{ReazonSpeechRecognizer, RecognizerModelPaths, SherpaError, SileroVad};
use tauri::{AppHandle, Emitter, EventTarget, Runtime};

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
        {
            *guard = None;
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
        std::thread::Builder::new()
            .name("pw-speech-pipeline".into())
            .spawn(move || worker.run())
            .map_err(|error| format!("failed to spawn speech worker: {error}"))?;

        *guard = Some(RunningPipeline {
            cancel,
            capture_enabled,
            diagnostics,
            dropped_samples,
        });
        Ok(())
    }

    /// Requests the running pipeline to stop.
    pub fn stop(&self) {
        if let Some(running) = self.lock().take() {
            running.cancel.store(true, Ordering::Relaxed);
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
        match self.build_and_run() {
            Ok(()) => emit_state(&self.app, SttPhaseDto::Stopped, None),
            Err(message) => {
                tracing::warn!(%message, "speech pipeline unavailable");
                emit_state(&self.app, SttPhaseDto::Unavailable, Some(message));
            }
        }
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn build_and_run(&self) -> Result<(), String> {
        let vad =
            SileroVad::new(&self.paths.vad_model, 0.5).map_err(|error| model_error(&error))?;
        let recognizer = ReazonSpeechRecognizer::new(&RecognizerModelPaths::in_directory(
            &self.paths.recognizer_dir,
        ))
        .map_err(|error| model_error(&error))?;

        let session =
            start_capture(self.device_id.as_deref()).map_err(|error| error.to_string())?;
        let dropped_counter = Arc::clone(&session.dropped_samples);
        let source = CaptureFrameSource::new(session).map_err(|error| error.to_string())?;
        // Mirror the capture drop counter into the service counter.
        self.mirror_drops(dropped_counter);

        let events = TauriSpeechEvents {
            app: self.app.clone(),
            frame_count: AtomicU64::new(0),
        };
        emit_state(&self.app, SttPhaseDto::Listening, None);

        let pipeline = SpeechPipeline::new(
            SpeechPipelineConfig::default(),
            vad,
            recognizer,
            events,
            Arc::clone(&self.capture_enabled),
            Arc::clone(&self.diagnostics),
        );
        run_pipeline(source, pipeline, &self.cancel);
        Ok(())
    }

    fn mirror_drops(&self, source: Arc<AtomicU64>) {
        let target = Arc::clone(&self.dropped_samples);
        let cancel = Arc::clone(&self.cancel);
        std::thread::spawn(move || {
            while !cancel.load(Ordering::Relaxed) {
                target.store(source.load(Ordering::Relaxed), Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
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

    use super::{SpeechService, SttModelPaths};

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
