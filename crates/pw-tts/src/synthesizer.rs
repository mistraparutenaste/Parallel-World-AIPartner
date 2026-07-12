//! [`TtsSynthesizer`] port implementation: engine + WAV cache.

use std::path::PathBuf;

use pw_application::PortError;
use pw_application::speech_synthesis::TtsSynthesizer;

use crate::aivis::{AivisSpeechClient, SynthesisParams};
use crate::cache::{WavCache, cache_key};

/// Synthesizes through the `AivisSpeech` engine with a disk cache in
/// front (基本設計 8章: WAVキャッシュ).
#[derive(Debug)]
pub struct CachedSpeechSynthesizer {
    client: AivisSpeechClient,
    cache: WavCache,
    style_id: u32,
    params: SynthesisParams,
}

impl CachedSpeechSynthesizer {
    #[must_use]
    pub fn new(
        client: AivisSpeechClient,
        cache: WavCache,
        style_id: u32,
        params: SynthesisParams,
    ) -> Self {
        Self {
            client,
            cache,
            style_id,
            params,
        }
    }
}

impl TtsSynthesizer for CachedSpeechSynthesizer {
    fn synthesize(&self, text: &str) -> Result<PathBuf, PortError> {
        let key = cache_key(text, self.style_id, &self.params);
        if let Some(path) = self.cache.lookup(&key) {
            return Ok(path);
        }
        let wav = self
            .client
            .synthesize(text, self.style_id, &self.params)
            .map_err(|error| PortError(error.to_string()))?;
        self.cache
            .store(&key, &wav)
            .map_err(|error| PortError(format!("failed to cache wav: {error}")))
    }
}
