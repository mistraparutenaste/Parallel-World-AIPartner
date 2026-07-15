//! Bounded, fail-closed proactive decision evaluator.

use std::io::Read;
use std::time::Duration;

use pw_application::behavior::proactive::{CandidateKind, CategoryId};
use pw_platform::net::validate_base_url;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const EVALUATOR_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_RESPONSE_BYTES: u64 = 16 * 1024;
const MAX_DURATION_SECONDS: u64 = 7 * 24 * 60 * 60;
const MAX_MODEL_CHARS: usize = 128;

#[derive(Debug, Clone)]
pub struct EvaluatorConfig {
    pub normal_base_url: String,
    pub normal_model: String,
    pub evaluator_base_url: Option<String>,
    pub evaluator_model: Option<String>,
    pub allow_remote: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationDecision {
    Speak,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluatorContext {
    kind: CandidateKind,
    category: CategoryId,
    duration_seconds: u64,
}

impl EvaluatorContext {
    /// Creates typed, bounded evaluator context.
    ///
    /// # Errors
    /// Returns [`InvalidEvaluatorContext`] when duration exceeds seven days.
    pub fn new(
        kind: CandidateKind,
        category: CategoryId,
        duration_seconds: u64,
    ) -> Result<Self, InvalidEvaluatorContext> {
        if duration_seconds > MAX_DURATION_SECONDS {
            return Err(InvalidEvaluatorContext);
        }
        Ok(Self {
            kind,
            category,
            duration_seconds,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidEvaluatorContext;

struct SelectedConfig {
    completions_url: String,
    model: String,
}

pub struct OpenAiCompatEvaluator {
    selected: Option<SelectedConfig>,
    http: Option<reqwest::blocking::Client>,
}

impl OpenAiCompatEvaluator {
    #[must_use]
    pub fn new(config: &EvaluatorConfig) -> Self {
        Self::new_with_timeout(config, EVALUATOR_TIMEOUT)
    }

    fn new_with_timeout(config: &EvaluatorConfig, timeout: Duration) -> Self {
        let selected = select_config(config);
        let http = selected.as_ref().and_then(|_| {
            reqwest::blocking::Client::builder()
                .timeout(timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .ok()
        });
        Self { selected, http }
    }

    /// Returns `Skip` for every invalid configuration, transport, status,
    /// timeout, size, or response-contract failure.
    #[must_use]
    pub fn evaluate(&self, context: &EvaluatorContext) -> EvaluationDecision {
        self.evaluate_inner(context)
            .unwrap_or(EvaluationDecision::Skip)
    }

    fn evaluate_inner(&self, context: &EvaluatorContext) -> Option<EvaluationDecision> {
        let selected = self.selected.as_ref()?;
        let http = self.http.as_ref()?;
        let body = request_body(&selected.model, context);
        let response = http
            .post(&selected.completions_url)
            .json(&body)
            .send()
            .ok()?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|length| length > MAX_RESPONSE_BYTES)
        {
            return None;
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > usize::try_from(MAX_RESPONSE_BYTES).ok()? {
            return None;
        }
        parse_response(&bytes)
    }
}

fn select_config(config: &EvaluatorConfig) -> Option<SelectedConfig> {
    let (base_url, model) = match (&config.evaluator_base_url, &config.evaluator_model) {
        (None, None) => (&config.normal_base_url, &config.normal_model),
        (Some(base_url), Some(model)) => (base_url, model),
        _ => return None,
    };
    let model = model.trim();
    if !(1..=MAX_MODEL_CHARS).contains(&model.chars().count()) {
        return None;
    }
    let base = validate_base_url(base_url.trim(), config.allow_remote).ok()?;
    Some(SelectedConfig {
        completions_url: format!("{}/chat/completions", base.as_str().trim_end_matches('/')),
        model: model.to_owned(),
    })
}

fn request_body(model: &str, context: &EvaluatorContext) -> serde_json::Value {
    let kind = match context.kind {
        CandidateKind::Return => "return",
        CandidateKind::LongSession => "long_session",
        CandidateKind::CategoryChange => "category_change",
    };
    let typed_context = json!({
        "candidate_kind": kind,
        "category": context.category.as_str(),
        "duration_seconds": context.duration_seconds,
    });
    json!({
        "model": model,
        "stream": false,
        "temperature": 0,
        "max_tokens": 16,
        "messages": [
            {
                "role": "system",
                "content": "Decide whether the companion should speak now. Return only the required JSON decision."
            },
            { "role": "user", "content": typed_context.to_string() }
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "proactive_decision",
                "strict": true,
                "schema": {
                    "type": "object",
                    "properties": {
                        "decision": { "type": "string", "enum": ["speak", "skip"] }
                    },
                    "required": ["decision"],
                    "additionalProperties": false
                }
            }
        }
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionResponse {
    choices: Vec<CompletionChoice>,
    #[serde(default)]
    #[serde(rename = "id")]
    _id: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "object")]
    _object: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "created")]
    _created: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "model")]
    _model: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "service_tier")]
    _service_tier: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "system_fingerprint")]
    _system_fingerprint: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "usage")]
    _usage: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "timings")]
    _timings: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionChoice {
    finish_reason: String,
    message: CompletionMessage,
    #[serde(default)]
    #[serde(rename = "index")]
    _index: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "logprobs")]
    _logprobs: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionMessage {
    content: Option<String>,
    #[serde(default)]
    refusal: Option<String>,
    #[serde(default)]
    #[serde(rename = "role")]
    _role: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "annotations")]
    _annotations: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "audio")]
    _audio: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "function_call")]
    _function_call: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(rename = "tool_calls")]
    _tool_calls: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionPayload {
    decision: DecisionValue,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum DecisionValue {
    Speak,
    Skip,
}

fn parse_response(bytes: &[u8]) -> Option<EvaluationDecision> {
    let response: CompletionResponse = serde_json::from_slice(bytes).ok()?;
    let [choice] = response.choices.as_slice() else {
        return None;
    };
    if choice.finish_reason != "stop" || choice.message.refusal.is_some() {
        return None;
    }
    let payload: DecisionPayload = serde_json::from_str(choice.message.content.as_deref()?).ok()?;
    Some(match payload.decision {
        DecisionValue::Speak => EvaluationDecision::Speak,
        DecisionValue::Skip => EvaluationDecision::Skip,
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use pw_application::behavior::proactive::{CandidateKind, CategoryId};

    use super::{
        EVALUATOR_TIMEOUT, EvaluationDecision, EvaluatorConfig, EvaluatorContext,
        OpenAiCompatEvaluator, select_config,
    };

    #[test]
    fn evaluator_production_timeout_is_eight_seconds() {
        assert_eq!(EVALUATOR_TIMEOUT, Duration::from_secs(8));
    }

    #[test]
    fn evaluator_short_injected_timeout_bounds_header_wait() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request);
            std::thread::sleep(Duration::from_millis(200));
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        });
        let config = EvaluatorConfig {
            normal_base_url: format!("http://127.0.0.1:{port}/v1"),
            normal_model: "test".into(),
            evaluator_base_url: None,
            evaluator_model: None,
            allow_remote: false,
        };
        let evaluator = OpenAiCompatEvaluator::new_with_timeout(&config, Duration::from_millis(40));
        let context =
            EvaluatorContext::new(CandidateKind::Return, CategoryId::new("work").unwrap(), 10)
                .unwrap();
        let started = Instant::now();
        assert_eq!(evaluator.evaluate(&context), EvaluationDecision::Skip);
        assert!(started.elapsed() < Duration::from_millis(150));
        server.join().unwrap();
    }

    #[test]
    fn evaluator_short_injected_timeout_bounds_body_read() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request);
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{");
            std::thread::sleep(Duration::from_millis(200));
        });
        let config = EvaluatorConfig {
            normal_base_url: format!("http://127.0.0.1:{port}/v1"),
            normal_model: "test".into(),
            evaluator_base_url: None,
            evaluator_model: None,
            allow_remote: false,
        };
        let evaluator = OpenAiCompatEvaluator::new_with_timeout(&config, Duration::from_millis(40));
        let context =
            EvaluatorContext::new(CandidateKind::Return, CategoryId::new("work").unwrap(), 10)
                .unwrap();
        let started = Instant::now();
        assert_eq!(evaluator.evaluate(&context), EvaluationDecision::Skip);
        assert!(started.elapsed() < Duration::from_millis(150));
        server.join().unwrap();
    }

    #[test]
    fn evaluator_remote_policy_is_reused_without_weakening() {
        let mut config = EvaluatorConfig {
            normal_base_url: "https://api.example.com/v1".into(),
            normal_model: "test".into(),
            evaluator_base_url: None,
            evaluator_model: None,
            allow_remote: false,
        };
        assert!(select_config(&config).is_none());
        config.allow_remote = true;
        assert!(select_config(&config).is_some());
    }
}
