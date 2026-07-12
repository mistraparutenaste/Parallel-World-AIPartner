//! `AivisSpeech` Engine adapter (VOICEVOX-compatible local HTTP API).

mod aivis;
mod cache;
mod synthesizer;

pub use aivis::{
    AivisSpeechClient, Speaker, SpeakerStyle, SynthesisParams, TtsClientConfig, TtsError,
    UserDictWord,
};
pub use cache::{DEFAULT_MAX_ENTRIES, WavCache, cache_key};
pub use synthesizer::CachedSpeechSynthesizer;
