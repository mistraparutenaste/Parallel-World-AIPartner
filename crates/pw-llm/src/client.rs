//! Blocking OpenAI-compatible streaming client.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::time::{Duration, Instant};

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
        tracing::info!(message_count = messages.len(), "llm stream worker queued");
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
                Ok(event) => {
                    if let Some(result) = handle_stream_event(event, cancel, on_delta) {
                        return result;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    if cancel.load(Ordering::Relaxed) {
                        return Ok(());
                    }
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

fn handle_stream_event(
    event: StreamEvent,
    cancel: &AtomicBool,
    on_delta: &mut dyn FnMut(&str),
) -> Option<Result<(), PortError>> {
    if cancel.load(Ordering::Relaxed) {
        return Some(Ok(()));
    }
    match event {
        StreamEvent::Delta(delta) => {
            on_delta(&delta);
            None
        }
        StreamEvent::Finished(result) => Some(result),
    }
}

fn stream_request(
    http: &reqwest::blocking::Client,
    completions_url: &str,
    body: &serde_json::Value,
    events: &std::sync::mpsc::SyncSender<StreamEvent>,
) -> Result<(), PortError> {
    let started = Instant::now();
    let message_count = body["messages"].as_array().map_or(0, Vec::len);
    tracing::info!(message_count, "llm request started");
    let response = http
        .post(completions_url)
        .json(body)
        .send()
        .map_err(|error| PortError(format!("llm request failed: {error}")))?;
    let status = response.status();
    tracing::info!(
        elapsed_ms = started.elapsed().as_millis(),
        status = %status,
        "llm response headers received"
    );
    if !status.is_success() {
        let detail = response.text().unwrap_or_default();
        let detail = detail.chars().take(200).collect::<String>();
        return Err(PortError(format!("llm returned {status}: {detail}")));
    }
    let reader = BufReader::new(response);
    let mut first_content_seen = false;
    for line in reader.lines() {
        let line = line.map_err(|error| PortError(format!("llm stream error: {error}")))?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            tracing::info!(
                elapsed_ms = started.elapsed().as_millis(),
                "llm sse done received"
            );
            return Ok(());
        }
        match serde_json::from_str::<StreamChunk>(data) {
            Ok(chunk) => {
                if let Some(content) = chunk
                    .choices
                    .first()
                    .and_then(|choice| choice.delta.content.as_deref())
                    && !content.is_empty()
                {
                    if !first_content_seen {
                        first_content_seen = true;
                        tracing::info!(
                            elapsed_ms = started.elapsed().as_millis(),
                            "llm first content delta received"
                        );
                    }
                    if events.send(StreamEvent::Delta(content.to_owned())).is_err() {
                        return Ok(());
                    }
                }
            }
            Err(error) => tracing::debug!(%error, "skipping unparsable sse chunk"),
        }
    }
    tracing::info!(
        elapsed_ms = started.elapsed().as_millis(),
        "llm transport eof received without done marker"
    );
    Err(PortError(
        "llm stream ended before the [DONE] marker".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_wins_over_an_already_received_delta_or_error() {
        let cancel = AtomicBool::new(true);
        let mut deltas = Vec::new();
        {
            let mut on_delta = |delta: &str| deltas.push(delta.to_owned());

            assert!(matches!(
                handle_stream_event(StreamEvent::Delta("late".into()), &cancel, &mut on_delta),
                Some(Ok(()))
            ));
            assert!(matches!(
                handle_stream_event(
                    StreamEvent::Finished(Err(PortError("late error".into()))),
                    &cancel,
                    &mut on_delta
                ),
                Some(Ok(()))
            ));
        }
        assert!(deltas.is_empty());
    }
}
