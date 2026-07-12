//! Blocking client for the `AivisSpeech` Engine HTTP API.
//!
//! `AivisSpeech` Engine exposes the VOICEVOX-compatible API surface:
//! `/speakers`, `/audio_query`, `/synthesis` and the user dictionary
//! endpoints. The engine runs on loopback only (基本設計 4.3章), so
//! remote hosts are always rejected.

use std::time::Duration;

use pw_platform::net::validate_base_url;
use serde::Deserialize;

/// Failure modes of the TTS adapter.
#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    #[error("invalid tts endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("tts request failed: {0}")]
    Transport(String),
    #[error("tts engine returned {status}: {detail}")]
    Api { status: u16, detail: String },
    #[error("unexpected tts response: {0}")]
    Protocol(String),
}

/// Connection settings for one `AivisSpeech` Engine instance.
#[derive(Debug, Clone)]
pub struct TtsClientConfig {
    /// e.g. `http://127.0.0.1:10101`
    pub base_url: String,
    /// Overall request timeout (synthesis of one sentence).
    pub timeout: Duration,
}

impl Default for TtsClientConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:10101".to_owned(),
            timeout: Duration::from_secs(30),
        }
    }
}

/// One selectable style of a speaker.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpeakerStyle {
    pub name: String,
    pub id: u32,
}

/// One installed voice with its styles.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Speaker {
    pub name: String,
    #[serde(default)]
    pub styles: Vec<SpeakerStyle>,
}

/// Per-request synthesis tuning applied onto the audio query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SynthesisParams {
    /// `volumeScale`, 1.0 = unchanged.
    pub volume: f32,
    /// `speedScale`, 1.0 = unchanged.
    pub speed: f32,
}

impl Default for SynthesisParams {
    fn default() -> Self {
        Self {
            volume: 1.0,
            speed: 1.0,
        }
    }
}

/// One user dictionary entry (pronunciation override).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserDictWord {
    pub uuid: String,
    pub surface: String,
    pub pronunciation: String,
    pub accent_type: u32,
}

#[derive(Deserialize)]
struct RawDictWord {
    surface: String,
    pronunciation: String,
    accent_type: u32,
}

#[derive(Debug)]
pub struct AivisSpeechClient {
    http: reqwest::blocking::Client,
    base: String,
}

impl AivisSpeechClient {
    /// # Errors
    ///
    /// Returns [`TtsError::InvalidEndpoint`] for unparsable or
    /// non-loopback URLs, [`TtsError::Transport`] when the HTTP client
    /// cannot be constructed.
    pub fn new(config: &TtsClientConfig) -> Result<Self, TtsError> {
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
        })
    }

    /// Lists installed speakers and their styles (`GET /speakers`).
    ///
    /// # Errors
    ///
    /// Returns [`TtsError`] on transport, API or decoding failure.
    pub fn speakers(&self) -> Result<Vec<Speaker>, TtsError> {
        let response = self
            .http
            .get(format!("{}/speakers", self.base))
            .send()
            .map_err(|error| transport(&error))?;
        let response = check_status(response)?;
        response
            .json::<Vec<Speaker>>()
            .map_err(|error| TtsError::Protocol(error.to_string()))
    }

    /// Synthesizes one sentence to WAV bytes.
    ///
    /// Runs `POST /audio_query` followed by `POST /synthesis`, applying
    /// the volume / speed scales onto the query in between.
    ///
    /// # Errors
    ///
    /// Returns [`TtsError`] on transport, API or decoding failure.
    pub fn synthesize(
        &self,
        text: &str,
        style_id: u32,
        params: &SynthesisParams,
    ) -> Result<Vec<u8>, TtsError> {
        let speaker = style_id.to_string();
        let response = self
            .http
            .post(format!("{}/audio_query", self.base))
            .query(&[("text", text), ("speaker", &speaker)])
            .send()
            .map_err(|error| transport(&error))?;
        let response = check_status(response)?;
        let mut query: serde_json::Value = response
            .json()
            .map_err(|error| TtsError::Protocol(error.to_string()))?;
        if let Some(map) = query.as_object_mut() {
            map.insert("volumeScale".into(), f32_json(params.volume));
            map.insert("speedScale".into(), f32_json(params.speed));
        }

        let response = self
            .http
            .post(format!("{}/synthesis", self.base))
            .query(&[("speaker", &speaker)])
            .json(&query)
            .send()
            .map_err(|error| transport(&error))?;
        let response = check_status(response)?;
        let bytes = response
            .bytes()
            .map_err(|error| TtsError::Protocol(error.to_string()))?;
        if !bytes.starts_with(b"RIFF") {
            return Err(TtsError::Protocol(
                "synthesis response is not a WAV file".to_owned(),
            ));
        }
        Ok(bytes.to_vec())
    }

    /// Lists the user dictionary (`GET /user_dict`).
    ///
    /// # Errors
    ///
    /// Returns [`TtsError`] on transport, API or decoding failure.
    pub fn user_dict(&self) -> Result<Vec<UserDictWord>, TtsError> {
        let response = self
            .http
            .get(format!("{}/user_dict", self.base))
            .send()
            .map_err(|error| transport(&error))?;
        let response = check_status(response)?;
        let raw: std::collections::BTreeMap<String, RawDictWord> = response
            .json()
            .map_err(|error| TtsError::Protocol(error.to_string()))?;
        Ok(raw
            .into_iter()
            .map(|(uuid, word)| UserDictWord {
                uuid,
                surface: word.surface,
                pronunciation: word.pronunciation,
                accent_type: word.accent_type,
            })
            .collect())
    }

    /// Adds a word (`POST /user_dict_word`), returning its UUID.
    ///
    /// # Errors
    ///
    /// Returns [`TtsError`] on transport, API or decoding failure.
    pub fn add_user_dict_word(
        &self,
        surface: &str,
        pronunciation: &str,
        accent_type: u32,
    ) -> Result<String, TtsError> {
        let accent = accent_type.to_string();
        let response = self
            .http
            .post(format!("{}/user_dict_word", self.base))
            .query(&[
                ("surface", surface),
                ("pronunciation", pronunciation),
                ("accent_type", &accent),
            ])
            .send()
            .map_err(|error| transport(&error))?;
        let response = check_status(response)?;
        response
            .json::<String>()
            .map_err(|error| TtsError::Protocol(error.to_string()))
    }

    /// Deletes a word (`DELETE /user_dict_word/{uuid}`).
    ///
    /// # Errors
    ///
    /// Returns [`TtsError`] on transport or API failure.
    pub fn delete_user_dict_word(&self, uuid: &str) -> Result<(), TtsError> {
        let response = self
            .http
            .delete(format!("{}/user_dict_word/{uuid}", self.base))
            .send()
            .map_err(|error| transport(&error))?;
        check_status(response).map(|_| ())
    }
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

/// JSON number for an `f32` scale (finite values only; NaN falls back
/// to 1.0 rather than emitting `null`).
fn f32_json(value: f32) -> serde_json::Value {
    serde_json::Number::from_f64(f64::from(value))
        .map_or_else(|| serde_json::json!(1.0), serde_json::Value::Number)
}

#[cfg(test)]
mod tests {
    use super::{AivisSpeechClient, TtsClientConfig, TtsError};

    fn client(base_url: &str) -> Result<AivisSpeechClient, TtsError> {
        AivisSpeechClient::new(&TtsClientConfig {
            base_url: base_url.to_owned(),
            ..TtsClientConfig::default()
        })
    }

    #[test]
    fn rejects_remote_endpoints() {
        let error = client("http://example.com:10101").unwrap_err();
        assert!(matches!(error, TtsError::InvalidEndpoint(_)));
    }

    #[test]
    fn accepts_loopback_endpoints() {
        assert!(client("http://127.0.0.1:10101").is_ok());
        assert!(client("http://localhost:10101/").is_ok());
    }
}
