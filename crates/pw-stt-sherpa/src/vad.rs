//! Silero VAD adapter.
//!
//! The sherpa-onnx VAD API is segment-oriented and does not expose
//! raw per-frame probabilities, so this adapter uses it as a binary
//! speech detector: `detected()` maps to probability 1.0 / 0.0 and
//! the domain [`SpeechSegmenter`] remains the single source of truth
//! for segmentation (pre-roll, hang, length limits).
//!
//! [`SpeechSegmenter`]: pw_domain::speech::SpeechSegmenter

use std::path::Path;

use pw_application::speech::{PortError, VoiceActivityDetector};
use sherpa_onnx::{SileroVadModelConfig, VadModelConfig};

use crate::SherpaError;

/// Frame length the Silero v5 model expects at 16 kHz.
pub const SILERO_WINDOW_SIZE: i32 = 512;

pub struct SileroVad {
    inner: sherpa_onnx::VoiceActivityDetector,
}

impl SileroVad {
    /// Loads the Silero VAD model from the given onnx file.
    ///
    /// # Errors
    ///
    /// Returns [`SherpaError`] when the model file is missing or the
    /// detector cannot be constructed.
    pub fn new(model_path: &Path, threshold: f32) -> Result<Self, SherpaError> {
        if !model_path.is_file() {
            return Err(SherpaError::ModelMissing(model_path.to_path_buf()));
        }
        let config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(model_path.to_string_lossy().into_owned()),
                threshold,
                // Keep sherpa's own gating minimal: the domain
                // segmenter owns hang/min-speech behaviour.
                min_silence_duration: 0.1,
                min_speech_duration: 0.032,
                window_size: SILERO_WINDOW_SIZE,
                max_speech_duration: 30.0,
            },
            sample_rate: 16_000,
            num_threads: 1,
            provider: Some("cpu".to_owned()),
            ..VadModelConfig::default()
        };
        let inner = sherpa_onnx::VoiceActivityDetector::create(&config, 4.0)
            .ok_or(SherpaError::Init("voice activity detector"))?;
        Ok(Self { inner })
    }
}

impl VoiceActivityDetector for SileroVad {
    fn probability(&mut self, frame: &[f32]) -> Result<f32, PortError> {
        self.inner.accept_waveform(frame);
        // Segments queued internally are not consumed here; drop them
        // so the buffer never grows.
        while !self.inner.is_empty() {
            self.inner.pop();
        }
        Ok(if self.inner.detected() { 1.0 } else { 0.0 })
    }

    fn reset(&mut self) {
        self.inner.reset();
        self.inner.clear();
    }
}
