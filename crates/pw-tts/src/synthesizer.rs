//! [`TtsSynthesizer`] port implementation: engine + WAV cache.

use std::io::Cursor;
use std::path::PathBuf;

use pw_application::PortError;
use pw_application::speech_synthesis::TtsSynthesizer;

use crate::TtsError;
use crate::aivis::{AivisSpeechClient, SynthesisParams};
use crate::cache::{WavCache, cache_key};
use crate::irodori::IrodoriTtsClient;

/// Finite set of local TTS engines supported by the cached synthesizer.
#[derive(Debug)]
pub enum EngineClient {
    Aivis(AivisSpeechClient),
    Irodori(IrodoriTtsClient),
}

impl From<AivisSpeechClient> for EngineClient {
    fn from(client: AivisSpeechClient) -> Self {
        Self::Aivis(client)
    }
}

impl From<IrodoriTtsClient> for EngineClient {
    fn from(client: IrodoriTtsClient) -> Self {
        Self::Irodori(client)
    }
}

impl EngineClient {
    fn cache_namespace(&self) -> String {
        match self {
            Self::Aivis(_) => "aivis".to_owned(),
            Self::Irodori(client) => client.cache_namespace(),
        }
    }

    fn synthesize(
        &self,
        text: &str,
        voice_id: &str,
        params: SynthesisParams,
    ) -> Result<Vec<u8>, TtsError> {
        match self {
            Self::Aivis(client) => {
                let style_id = voice_id.parse::<u32>().map_err(|_| {
                    TtsError::Protocol(format!("invalid Aivis style id: {voice_id}"))
                })?;
                client.synthesize(text, style_id, &params)
            }
            Self::Irodori(client) => {
                let wav = client.synthesize(text, voice_id, params.speed)?;
                apply_wav_gain(&wav, params.volume)
            }
        }
    }
}

/// Synthesizes through the selected local engine with a disk cache in front.
#[derive(Debug)]
pub struct CachedSpeechSynthesizer {
    client: EngineClient,
    cache: WavCache,
    voice_id: String,
    params: SynthesisParams,
}

impl CachedSpeechSynthesizer {
    #[must_use]
    // Taking the selector by value preserves the existing numeric Aivis call sites
    // while accepting owned string IDs for Irodori.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        client: impl Into<EngineClient>,
        cache: WavCache,
        voice_id: impl ToString,
        params: SynthesisParams,
    ) -> Self {
        Self {
            client: client.into(),
            cache,
            voice_id: voice_id.to_string(),
            params,
        }
    }
}

impl TtsSynthesizer for CachedSpeechSynthesizer {
    fn synthesize(&self, text: &str) -> Result<PathBuf, PortError> {
        let key = cache_key(
            &self.client.cache_namespace(),
            &self.voice_id,
            text,
            &self.params,
        );
        if let Some(path) = self.cache.lookup(&key) {
            return Ok(path);
        }
        let wav = self
            .client
            .synthesize(text, &self.voice_id, self.params)
            .map_err(|error| PortError(error.to_string()))?;
        self.cache
            .store(&key, &wav)
            .map_err(|error| PortError(format!("failed to cache wav: {error}")))
    }
}

fn apply_wav_gain(wav: &[u8], volume: f32) -> Result<Vec<u8>, TtsError> {
    if volume.to_bits() == 1.0_f32.to_bits() || !volume.is_finite() {
        return Ok(wav.to_vec());
    }

    let mut reader = hound::WavReader::new(Cursor::new(wav))
        .map_err(|error| TtsError::Protocol(format!("invalid WAV: {error}")))?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int {
        return Err(TtsError::Protocol(
            "Irodori WAV must use integer PCM samples".to_owned(),
        ));
    }

    let mut gained = Vec::with_capacity(wav.len());
    {
        let cursor = Cursor::new(&mut gained);
        let mut writer = hound::WavWriter::new(cursor, spec)
            .map_err(|error| TtsError::Protocol(format!("failed to encode WAV: {error}")))?;
        match spec.bits_per_sample {
            1..=8 => write_i8_samples(&mut reader, &mut writer, volume)?,
            9..=16 => write_i16_samples(&mut reader, &mut writer, volume)?,
            17..=32 => write_i32_samples(&mut reader, &mut writer, volume, spec.bits_per_sample)?,
            bits => {
                return Err(TtsError::Protocol(format!(
                    "unsupported WAV sample width: {bits}"
                )));
            }
        }
        writer
            .finalize()
            .map_err(|error| TtsError::Protocol(format!("failed to encode WAV: {error}")))?;
    }
    Ok(gained)
}

fn write_i8_samples<W: std::io::Write + std::io::Seek>(
    reader: &mut hound::WavReader<Cursor<&[u8]>>,
    writer: &mut hound::WavWriter<W>,
    volume: f32,
) -> Result<(), TtsError> {
    for sample in reader.samples::<i8>() {
        let sample = sample.map_err(|error| wav_decode_error(&error))?;
        writer
            .write_sample(scale_i8(sample, volume))
            .map_err(|error| wav_encode_error(&error))?;
    }
    Ok(())
}

fn write_i16_samples<W: std::io::Write + std::io::Seek>(
    reader: &mut hound::WavReader<Cursor<&[u8]>>,
    writer: &mut hound::WavWriter<W>,
    volume: f32,
) -> Result<(), TtsError> {
    for sample in reader.samples::<i16>() {
        let sample = sample.map_err(|error| wav_decode_error(&error))?;
        writer
            .write_sample(scale_i16(sample, volume))
            .map_err(|error| wav_encode_error(&error))?;
    }
    Ok(())
}

fn write_i32_samples<W: std::io::Write + std::io::Seek>(
    reader: &mut hound::WavReader<Cursor<&[u8]>>,
    writer: &mut hound::WavWriter<W>,
    volume: f32,
    bits_per_sample: u16,
) -> Result<(), TtsError> {
    for sample in reader.samples::<i32>() {
        let sample = sample.map_err(|error| wav_decode_error(&error))?;
        writer
            .write_sample(scale_i32(sample, volume, bits_per_sample))
            .map_err(|error| wav_encode_error(&error))?;
    }
    Ok(())
}

fn wav_decode_error(error: &hound::Error) -> TtsError {
    TtsError::Protocol(format!("invalid WAV: {error}"))
}

fn wav_encode_error(error: &hound::Error) -> TtsError {
    TtsError::Protocol(format!("failed to encode WAV: {error}"))
}

#[allow(clippy::cast_possible_truncation)]
fn scale_i8(sample: i8, volume: f32) -> i8 {
    (f64::from(sample) * f64::from(volume))
        .round()
        .clamp(f64::from(i8::MIN), f64::from(i8::MAX)) as i8
}

#[allow(clippy::cast_possible_truncation)]
fn scale_i16(sample: i16, volume: f32) -> i16 {
    (f64::from(sample) * f64::from(volume))
        .round()
        .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

#[allow(clippy::cast_possible_truncation)]
fn scale_i32(sample: i32, volume: f32, bits_per_sample: u16) -> i32 {
    let (min, max) = if bits_per_sample == 32 {
        (i32::MIN, i32::MAX)
    } else {
        let magnitude = 1_i32 << (bits_per_sample - 1);
        (-magnitude, magnitude - 1)
    };
    (f64::from(sample) * f64::from(volume))
        .round()
        .clamp(f64::from(min), f64::from(max)) as i32
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::time::Duration;

    use super::{EngineClient, apply_wav_gain};
    use crate::aivis::SynthesisParams;
    use crate::cache::cache_key;
    use crate::{IrodoriTtsClient, TtsClientConfig};

    fn pcm16_wav(samples: &[i16]) -> Vec<u8> {
        let mut wav = Vec::new();
        {
            let cursor = Cursor::new(&mut wav);
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 24_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::new(cursor, spec).unwrap();
            for sample in samples {
                writer.write_sample(*sample).unwrap();
            }
            writer.finalize().unwrap();
        }
        wav
    }

    fn pcm16_samples(wav: &[u8]) -> Vec<i16> {
        hound::WavReader::new(Cursor::new(wav))
            .unwrap()
            .samples::<i16>()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn unity_gain_returns_the_original_wav_bytes() {
        let wav = pcm16_wav(&[-1_234, 0, 5_678]);

        assert_eq!(apply_wav_gain(&wav, 1.0).unwrap(), wav);
    }

    #[test]
    fn pcm_gain_scales_and_saturates_samples() {
        let wav = pcm16_wav(&[-20_000, -1_000, 1_000, 20_000]);

        let gained = apply_wav_gain(&wav, 2.0).unwrap();

        assert_eq!(
            pcm16_samples(&gained),
            vec![i16::MIN, -2_000, 2_000, i16::MAX]
        );
    }

    #[test]
    fn irodori_base_and_lora_adapters_use_distinct_cache_keys() {
        let config = TtsClientConfig {
            base_url: "http://127.0.0.1:8088".to_owned(),
            timeout: Duration::from_secs(1),
        };
        let base = EngineClient::Irodori(IrodoriTtsClient::new(&config).unwrap());
        let lora = EngineClient::Irodori(
            IrodoriTtsClient::with_lora_adapter(&config, "adapters/character-a").unwrap(),
        );
        let other_lora = EngineClient::Irodori(
            IrodoriTtsClient::with_lora_adapter(&config, "adapters/character-b").unwrap(),
        );

        let key = |client: &EngineClient| {
            cache_key(
                &client.cache_namespace(),
                "voice-a",
                "hello",
                &SynthesisParams::default(),
            )
        };

        assert_ne!(key(&base), key(&lora));
        assert_ne!(key(&lora), key(&other_lora));
    }
}
