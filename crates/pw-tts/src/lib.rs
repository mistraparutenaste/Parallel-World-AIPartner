//! `AivisSpeech` Engine adapter (VOICEVOX-compatible local HTTP API).

mod aivis;
mod cache;
mod irodori;
mod synthesizer;

pub use aivis::{
    AivisSpeechClient, Speaker, SpeakerStyle, SynthesisParams, TtsClientConfig, TtsError,
    UserDictWord,
};
pub use cache::{DEFAULT_MAX_ENTRIES, WavCache, WavCacheClearError, WavCacheStats, cache_key};
pub use irodori::IrodoriTtsClient;
pub use synthesizer::{CachedSpeechSynthesizer, EngineClient};
