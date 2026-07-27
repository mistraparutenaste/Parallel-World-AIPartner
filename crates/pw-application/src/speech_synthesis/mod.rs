//! Sentence-level TTS queue (基本設計 8章): synthesize ahead of
//! playback, drop stale turns, degrade without losing the text.

mod ports;
mod queue;

pub use ports::{SpeechAudioSink, TtsSynthesizer};
pub use queue::{SpeechSynthesisQueue, SynthesisBatching};
