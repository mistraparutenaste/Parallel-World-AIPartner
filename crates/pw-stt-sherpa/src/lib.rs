//! sherpa-onnx adapters for the speech ports.
//!
//! Uses the official `sherpa-onnx` crate (safe wrapper over the C
//! API), so this crate contains no `unsafe` of its own. See
//! `docs/adr/2026-07-12-sherpa-onnx-binding.md` for the binding
//! decision and the VAD design note.

mod recognizer;
mod vad;

pub use recognizer::{ReazonSpeechRecognizer, RecognizerModelPaths};
pub use vad::SileroVad;

/// Errors constructing sherpa-onnx adapters.
#[derive(Debug, thiserror::Error)]
pub enum SherpaError {
    #[error("model file not found: {0}")]
    ModelMissing(std::path::PathBuf),
    #[error("failed to initialize sherpa-onnx {0}")]
    Init(&'static str),
}
