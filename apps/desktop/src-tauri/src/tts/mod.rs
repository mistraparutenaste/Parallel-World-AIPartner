//! TTS wiring: settings persistence and the synthesis worker.

mod service;
mod settings;

pub use service::TtsService;
pub use settings::{default_tts_settings, load_tts_settings, save_tts_settings};
