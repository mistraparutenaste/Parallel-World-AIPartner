//! Blocking OpenAI-compatible streaming client.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::time::Duration;

use pw_application::PortError;
use pw_application::conversation::{ChatMessage, ChatRole, LlmClient};
use serde::Deserialize;
use serde_json::json;

use pw_platform::net::validate_base_url;

const MAX_STREAM_WORKERS: usize = 16;
static ACTIVE_STREAM_WORKERS: AtomicUsize = AtomicUsize::new(0);

struct StreamWorkerGuard;

impl StreamWorkerGuard {
    fn acquire() -> Result<Self, PortError> {
        ACTIVE_STREAM_WORKERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_STREAM_WORKERS).then_some(active + 1)
            })
            .map_err(|_| PortError("too many active llm stream workers".into()))?;
        Ok(Self)
    }
}

impl Drop for StreamWorkerGuard {
    fn drop(&mut self) {
        ACTIVE_STREAM_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Connection settings for one OpenAI-compatible server.
#[derive(Debug, Clone)]
pub struct LlmClientConfig {
    /// e.g. `http://127.0.0.1:8080/v1`
    pub base_url: String,
    pub model: String,
    /// Permit non-loopback hosts (cloud endpoints).
    pub allow_remote: bool,
    /// Overall request timeout.
    pub timeout: Duration,
}

impl Default for LlmClientConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8080/v1".to_owned(),
            model: "default".to_owned(),
            allow_remote: false,
            timeout: Duration::from_mins(2),
        }
    }
}

pub struct OpenAiCompatClient {
    config: LlmClientConfig,
    http: reqwest::blocking::Client,
    completions_url: String,
}

impl OpenAiCompatClient {
    /// # Errors
    ///
    /// Returns [`PortError`] when the base URL is invalid or remote
    /// without permission, or the HTTP client cannot be constructed.
    pub fn new(config: LlmClientConfig) -> Result<Self, PortError> {
        let base = validate_base_url(&config.base_url, config.allow_remote)
            .map_err(|error| PortError(error.to_string()))?;
        let completions_url = format!("{}/chat/completions", base.as_str().trim_end_matches('/'));
        let http = reqwest::blocking::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| PortError(format!("failed to build http client: {error}")))?;
        Ok(Self {
            config,
            http,
            completions_url,
        })
    }
}

fn role_str(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    }
}

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize, Default)]
struct StreamDelta {
    content: Option<String>,
}

impl LlmClient for OpenAiCompatClient {
    fn stream_chat(
        &mut self,
        messages: &[ChatMessage],
        cancel: &AtomicBool,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<(), PortError> {
        let body = json!({
            "model": self.config.model,
            "stream": true,
            "messages": messages
                .iter()
                .map(|message| json!({
                    "role": role_str(message.role),
                    "content": message.content,
                }))
                .collect::<Vec<_>>(),
        });

        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let worker_guard = StreamWorkerGuard::acquire()?;
        let (events, receiver) = sync_channel(16);
        let http = self.http.clone();
        let completions_url = self.completions_url.clone();
        std::thread::spawn(move || {
            let _worker_guard = worker_guard;
            let result = stream_request(&http, &completions_url, &body, &events);
            let _ = events.send(StreamEvent::Finished(result));
        });
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            match receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(StreamEvent::Delta(delta)) => on_delta(&delta),
                Ok(StreamEvent::Finished(result)) => return result,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(PortError("llm stream worker disconnected".into()));
                }
            }
        }
    }
}

enum StreamEvent {
    Delta(String),
    Finished(Result<(), PortError>),
}

fn stream_request(
    http: &reqwest::blocking::Client,
    completions_url: &str,
    body: &serde_json::Value,
    events: &std::sync::mpsc::SyncSender<StreamEvent>,
) -> Result<(), PortError> {
    let response = http
        .post(completions_url)
        .json(body)
        .send()
        .map_err(|error| PortError(format!("llm request failed: {error}")))?;
    let status = response.status();
    if !status.is_success() {
        let detail = response.text().unwrap_or_default();
        let detail = detail.chars().take(200).collect::<String>();
        return Err(PortError(format!("llm returned {status}: {detail}")));
    }
    let reader = BufReader::new(response);
    for line in reader.lines() {
        let line = line.map_err(|error| PortError(format!("llm stream error: {error}")))?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        match serde_json::from_str::<StreamChunk>(data) {
            Ok(chunk) => {
                if let Some(content) = chunk
                    .choices
                    .first()
                    .and_then(|choice| choice.delta.content.as_deref())
                    && !content.is_empty()
                    && events.send(StreamEvent::Delta(content.to_owned())).is_err()
                {
                    return Ok(());
                }
            }
            Err(error) => tracing::debug!(%error, "skipping unparsable sse chunk"),
        }
    }
    Ok(())
}
