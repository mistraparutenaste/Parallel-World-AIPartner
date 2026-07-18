//! Speech capture service wiring (audio, VAD, STT adapters).

mod service;

pub use service::{STATE_EVENT, SpeechService, SttModelPaths};
