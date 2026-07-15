use std::sync::{Arc, Mutex};
use std::time::Duration;

use pw_application::behavior::proactive::{CandidateKind, CategoryId};
use pw_llm::{EvaluationDecision, EvaluatorConfig, EvaluatorContext, OpenAiCompatEvaluator};

struct FakeServer {
    port: u16,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl FakeServer {
    fn spawn(status: u16, body: String) -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let received = Arc::new(Mutex::new(Vec::new()));
        let received_in = Arc::clone(&received);
        let handle = std::thread::spawn(move || {
            let Ok(Some(mut request)) = server.recv_timeout(Duration::from_millis(150)) else {
                return;
            };
            let mut content = String::new();
            let _ = std::io::Read::read_to_string(request.as_reader(), &mut content);
            if let Ok(value) = serde_json::from_str(&content) {
                received_in.lock().unwrap().push(value);
            }
            let response = tiny_http::Response::from_string(body)
                .with_status_code(status)
                .with_header(
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                        .unwrap(),
                );
            let _ = request.respond(response);
        });
        Self {
            port,
            received,
            handle: Some(handle),
        }
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn response(content: Option<&str>, finish_reason: &str, refusal: Option<&str>) -> String {
    serde_json::json!({
        "choices": [{
            "finish_reason": finish_reason,
            "message": { "content": content, "refusal": refusal }
        }]
    })
    .to_string()
}

fn config(port: u16) -> EvaluatorConfig {
    EvaluatorConfig {
        normal_base_url: format!("http://127.0.0.1:{port}/v1"),
        normal_model: "normal-model".into(),
        evaluator_base_url: None,
        evaluator_model: None,
        allow_remote: false,
    }
}

fn context() -> EvaluatorContext {
    EvaluatorContext::new(CandidateKind::Return, CategoryId::new("work").unwrap(), 600).unwrap()
}

#[test]
fn evaluator_accepts_only_exact_speak_or_skip() {
    for (value, expected) in [
        (r#"{"decision":"speak"}"#, EvaluationDecision::Speak),
        (" { \"decision\" : \"skip\" } \n", EvaluationDecision::Skip),
    ] {
        let server = FakeServer::spawn(200, response(Some(value), "stop", None));
        let evaluator = OpenAiCompatEvaluator::new(&config(server.port));
        assert_eq!(evaluator.evaluate(&context()), expected);
    }
}

#[test]
fn evaluator_accepts_known_openai_wrapper_metadata() {
    let body = serde_json::json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 123,
        "model": "test",
        "choices": [{
            "index": 0,
            "finish_reason": "stop",
            "logprobs": null,
            "message": {
                "role": "assistant",
                "content": "{\"decision\":\"speak\"}",
                "refusal": null,
                "annotations": []
            }
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
        "system_fingerprint": null,
        "service_tier": "default"
    })
    .to_string();
    let server = FakeServer::spawn(200, body);
    let evaluator = OpenAiCompatEvaluator::new(&config(server.port));
    assert_eq!(evaluator.evaluate(&context()), EvaluationDecision::Speak);
}

#[test]
fn evaluator_rejects_malformed_outer_and_inner_contracts() {
    let invalid = [
        response(Some(r#"{"decision":"speak","why":"x"}"#), "stop", None),
        serde_json::json!({
            "choices": [{"finish_reason":"stop","message":{"content":"{\"decision\":\"speak\"}"}}],
            "unexpected": true
        })
        .to_string(),
        response(Some("```json\n{\"decision\":\"speak\"}\n```"), "stop", None),
        response(Some("prefix {\"decision\":\"speak\"}"), "stop", None),
        response(
            Some("{\"decision\":\"speak\"}{\"decision\":\"skip\"}"),
            "stop",
            None,
        ),
        response(None, "stop", None),
        response(Some(r#"{"decision":"speak"}"#), "length", None),
        response(Some(r#"{"decision":"speak"}"#), "stop", Some("refused")),
        serde_json::json!({"choices": []}).to_string(),
        serde_json::json!({"choices": [
            {"finish_reason":"stop","message":{"content":"{\"decision\":\"speak\"}"}},
            {"finish_reason":"stop","message":{"content":"{\"decision\":\"speak\"}"}}
        ]})
        .to_string(),
    ];
    for body in invalid {
        let server = FakeServer::spawn(200, body);
        let evaluator = OpenAiCompatEvaluator::new(&config(server.port));
        assert_eq!(evaluator.evaluate(&context()), EvaluationDecision::Skip);
    }
}

#[test]
fn evaluator_sends_one_non_streaming_strict_schema_request_with_typed_context_only() {
    let server = FakeServer::spawn(200, response(Some(r#"{"decision":"skip"}"#), "stop", None));
    let evaluator = OpenAiCompatEvaluator::new(&config(server.port));
    assert_eq!(evaluator.evaluate(&context()), EvaluationDecision::Skip);
    let requests = server.received.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request["model"], "normal-model");
    assert_eq!(request["stream"], false);
    assert_eq!(request["temperature"], 0);
    assert_eq!(request["max_tokens"], 16);
    assert_eq!(request["response_format"]["type"], "json_schema");
    assert_eq!(request["response_format"]["json_schema"]["strict"], true);
    assert_eq!(
        request["response_format"]["json_schema"]["schema"]["additionalProperties"],
        false
    );
    let encoded = request.to_string();
    assert!(encoded.contains("work"));
    assert!(!encoded.contains("RAW_TITLE_SENTINEL"));
}

#[test]
fn evaluator_override_is_selected_without_validating_unused_normal_pair() {
    let server = FakeServer::spawn(200, response(Some(r#"{"decision":"speak"}"#), "stop", None));
    let evaluator = OpenAiCompatEvaluator::new(&EvaluatorConfig {
        normal_base_url: "not a url".into(),
        normal_model: String::new(),
        evaluator_base_url: Some(format!("http://127.0.0.1:{}/v1", server.port)),
        evaluator_model: Some(" evaluator-model ".into()),
        allow_remote: false,
    });
    assert_eq!(evaluator.evaluate(&context()), EvaluationDecision::Speak);
    assert_eq!(
        server.received.lock().unwrap()[0]["model"],
        "evaluator-model"
    );
}

#[test]
fn evaluator_partial_blank_invalid_or_denied_configuration_makes_zero_calls() {
    for mutate in 0..5 {
        let server =
            FakeServer::spawn(200, response(Some(r#"{"decision":"speak"}"#), "stop", None));
        let mut cfg = config(server.port);
        match mutate {
            0 => cfg.evaluator_base_url = Some(cfg.normal_base_url.clone()),
            1 => cfg.evaluator_model = Some("small".into()),
            2 => cfg.normal_model = "   ".into(),
            3 => cfg.normal_base_url = "not a url".into(),
            _ => cfg.normal_base_url = "https://example.com/v1".into(),
        }
        let evaluator = OpenAiCompatEvaluator::new(&cfg);
        assert_eq!(evaluator.evaluate(&context()), EvaluationDecision::Skip);
        assert!(server.received.lock().unwrap().is_empty());
    }
}

#[test]
fn evaluator_non_success_and_oversized_response_fail_closed() {
    for (status, body) in [
        (500, "error".to_owned()),
        (302, "redirect".to_owned()),
        (200, "x".repeat(16 * 1024 + 1)),
    ] {
        let server = FakeServer::spawn(status, body);
        let evaluator = OpenAiCompatEvaluator::new(&config(server.port));
        assert_eq!(evaluator.evaluate(&context()), EvaluationDecision::Skip);
    }
}

#[test]
fn evaluator_transport_failure_is_skip() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let evaluator = OpenAiCompatEvaluator::new(&config(port));
    assert_eq!(evaluator.evaluate(&context()), EvaluationDecision::Skip);
}

#[test]
fn evaluator_failed_response_is_sent_exactly_once() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let received_in = Arc::clone(&received);
    let handle = std::thread::spawn(move || {
        while let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(200)) {
            received_in.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let _ = request.respond(
                tiny_http::Response::from_string("retryable failure").with_status_code(503),
            );
        }
    });
    let evaluator = OpenAiCompatEvaluator::new(&config(port));
    assert_eq!(evaluator.evaluate(&context()), EvaluationDecision::Skip);
    handle.join().unwrap();
    assert_eq!(received.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn evaluator_does_not_follow_redirects() {
    use std::io::{Read, Write};

    let target = FakeServer::spawn(200, response(Some(r#"{"decision":"speak"}"#), "stop", None));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let location = format!("http://127.0.0.1:{}/v1/chat/completions", target.port);
    let handle = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        let _ = socket.read(&mut request);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = socket.write_all(response.as_bytes());
    });
    let evaluator = OpenAiCompatEvaluator::new(&config(port));
    assert_eq!(evaluator.evaluate(&context()), EvaluationDecision::Skip);
    handle.join().unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(target.received.lock().unwrap().is_empty());
}

#[test]
fn evaluator_bounds_chunked_body_without_content_length() {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        let _ = socket.read(&mut request);
        let oversized = "x".repeat(16 * 1024 + 1);
        let header = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
        let _ = socket.write_all(header.as_bytes());
        let _ = socket.write_all(format!("{:x}\r\n", oversized.len()).as_bytes());
        let _ = socket.write_all(oversized.as_bytes());
        let _ = socket.write_all(b"\r\n0\r\n\r\n");
    });
    let evaluator = OpenAiCompatEvaluator::new(&config(port));
    assert_eq!(evaluator.evaluate(&context()), EvaluationDecision::Skip);
    handle.join().unwrap();
}

#[test]
fn evaluator_context_is_bounded() {
    assert!(
        EvaluatorContext::new(
            CandidateKind::LongSession,
            CategoryId::new("work").unwrap(),
            604_801,
        )
        .is_err()
    );
}

#[test]
fn evaluator_model_limit_counts_characters_after_trimming() {
    let server = FakeServer::spawn(200, response(Some(r#"{"decision":"skip"}"#), "stop", None));
    let mut cfg = config(server.port);
    cfg.normal_model = format!(" {} ", "あ".repeat(128));
    let evaluator = OpenAiCompatEvaluator::new(&cfg);
    assert_eq!(evaluator.evaluate(&context()), EvaluationDecision::Skip);
    assert_eq!(server.received.lock().unwrap().len(), 1);
}
