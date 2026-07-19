//! Blocking client for the Irodori TTS HTTP API.

use pw_platform::net::validate_base_url;
use serde::Deserialize;

use crate::{TtsClientConfig, TtsError};

const MIN_SPEED: f32 = 0.25;
const MAX_SPEED: f32 = 4.0;

/// One Irodori voice returned by `GET /v1/audio/voices`.
#[derive(Deserialize)]
struct Voice {
    id: String,
}

#[derive(Deserialize)]
struct VoiceList {
    object: String,
    data: Vec<Voice>,
}

/// Loopback-only client for an Irodori TTS server.
#[derive(Debug)]
pub struct IrodoriTtsClient {
    http: reqwest::blocking::Client,
    base: String,
    lora_adapter: Option<String>,
}

impl IrodoriTtsClient {
    /// Creates a client using the shared TTS endpoint and timeout settings.
    ///
    /// # Errors
    ///
    /// Returns [`TtsError::InvalidEndpoint`] for unparsable or non-loopback
    /// URLs, and [`TtsError::Transport`] if the HTTP client cannot be built.
    pub fn new(config: &TtsClientConfig) -> Result<Self, TtsError> {
        Self::build(config, None)
    }

    /// Creates a client that selects one dynamic PEFT `LoRA` adapter for every
    /// synthesis request. The path is resolved by the Irodori server process.
    ///
    /// # Errors
    ///
    /// Returns the same endpoint and transport errors as [`Self::new`].
    pub fn with_lora_adapter(
        config: &TtsClientConfig,
        lora_adapter: &str,
    ) -> Result<Self, TtsError> {
        Self::build(config, Some(lora_adapter))
    }

    fn build(config: &TtsClientConfig, lora_adapter: Option<&str>) -> Result<Self, TtsError> {
        let base = validate_base_url(&config.base_url, false)
            .map_err(|error| TtsError::InvalidEndpoint(error.to_string()))?;
        let http = reqwest::blocking::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| {
                TtsError::Transport(format!("failed to build http client: {error}"))
            })?;
        Ok(Self {
            http,
            base: base.as_str().trim_end_matches('/').to_owned(),
            lora_adapter: lora_adapter
                .map(str::trim)
                .filter(|adapter| !adapter.is_empty())
                .map(str::to_owned),
        })
    }

    pub(crate) fn cache_namespace(&self) -> String {
        match &self.lora_adapter {
            Some(adapter) => format!("irodori:lora:{adapter}"),
            None => "irodori:base".to_owned(),
        }
    }

    /// Lists installed voice identifiers (`GET /v1/audio/voices`).
    ///
    /// # Errors
    ///
    /// Returns [`TtsError`] on transport, API, or response decoding failure.
    pub fn voices(&self) -> Result<Vec<String>, TtsError> {
        let response = self
            .http
            .get(format!("{}/v1/audio/voices", self.base))
            .send()
            .map_err(|error| transport(&error))?;
        let response = check_status(response)?;
        let voices = response
            .json::<VoiceList>()
            .map_err(|error| TtsError::Protocol(error.to_string()))?;
        if voices.object != "list" {
            return Err(TtsError::Protocol(
                "voice list response object is not list".to_owned(),
            ));
        }
        Ok(voices.data.into_iter().map(|voice| voice.id).collect())
    }

    /// Synthesizes one sentence to WAV bytes (`POST /v1/audio/speech`).
    ///
    /// The upstream API accepts speed values from 0.25 through 4.0. Values
    /// outside that range are clamped before the request is encoded.
    ///
    /// # Errors
    ///
    /// Returns [`TtsError`] on transport, API, or response validation failure.
    pub fn synthesize(&self, text: &str, voice_id: &str, speed: f32) -> Result<Vec<u8>, TtsError> {
        let mut body = serde_json::json!({
            "model": "irodori-tts",
            "input": text,
            "voice": voice_id,
            "response_format": "wav",
            "speed": clamp_speed(speed),
        });
        if let Some(lora_adapter) = &self.lora_adapter {
            body["irodori"] = serde_json::json!({ "lora_adapter": lora_adapter });
        }
        let response = self
            .http
            .post(format!("{}/v1/audio/speech", self.base))
            .json(&body)
            .send()
            .map_err(|error| transport(&error))?;
        let response = check_status(response)?;
        let bytes = response
            .bytes()
            .map_err(|error| TtsError::Protocol(error.to_string()))?;
        if !is_wav(&bytes) {
            return Err(TtsError::Protocol(
                "synthesis response is not a WAV file".to_owned(),
            ));
        }
        Ok(bytes.to_vec())
    }
}

fn clamp_speed(speed: f32) -> f32 {
    if speed.is_finite() {
        speed.clamp(MIN_SPEED, MAX_SPEED)
    } else {
        1.0
    }
}

fn is_wav(bytes: &[u8]) -> bool {
    bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE")
}

fn transport(error: &reqwest::Error) -> TtsError {
    TtsError::Transport(error.to_string())
}

fn check_status(
    response: reqwest::blocking::Response,
) -> Result<reqwest::blocking::Response, TtsError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let detail = response.text().unwrap_or_default();
    let detail = detail.chars().take(200).collect::<String>();
    Err(TtsError::Api {
        status: status.as_u16(),
        detail,
    })
}
