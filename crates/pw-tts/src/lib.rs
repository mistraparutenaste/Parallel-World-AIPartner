//! `AivisSpeech` Engine adapter (VOICEVOX-compatible local HTTP API).

mod aivis;

pub use aivis::{
    AivisSpeechClient, Speaker, SpeakerStyle, SynthesisParams, TtsClientConfig, TtsError,
    UserDictWord,
};
