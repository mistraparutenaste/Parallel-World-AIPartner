//! Contract tests against a local mock OpenAI-compatible server.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pw_application::conversation::{ChatMessage, ChatRole, LlmClient};
use pw_llm::{LlmClientConfig, OpenAiCompatClient};

struct MockServer {
    port: u16,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

fn sse_body(deltas: &[&str]) -> String {
    use std::fmt::Write;
    let mut body = String::new();
    for delta in deltas {
        let chunk = serde_json::json!({
            "choices": [{ "delta": { "content": delta } }]
        });
        let _ = write!(body, "data: {chunk}\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn spawn_server(status: u16, body: String) -> MockServer {
    let server = tiny_http::Server::http("127.0.0.1:0").expect("bind mock server");
    let port = server.server_addr().to_ip().unwrap().port();
    let received = Arc::new(Mutex::new(Vec::new()));
    let received_in = Arc::clone(&received);
    let handle = std::thread::spawn(move || {
        if let Ok(mut request) = server.recv() {
            let mut content = String::new();
            let _ = std::io::Read::read_to_string(request.as_reader(), &mut content);
            if let Ok(json) = serde_json::from_str(&content) {
                received_in.lock().unwrap().push(json);
            }
            let response = tiny_http::Response::from_string(body)
                .with_status_code(status)
                .with_header(
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..])
                        .unwrap(),
                );
            let _ = request.respond(response);
        }
    });
    MockServer {
        port,
        received,
        handle: Some(handle),
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn client_for(port: u16) -> OpenAiCompatClient {
    OpenAiCompatClient::new(LlmClientConfig {
        base_url: format!("http://127.0.0.1:{port}/v1"),
        model: "test-model".into(),
        allow_remote: false,
        timeout: Duration::from_secs(5),
    })
    .unwrap()
}

fn messages() -> Vec<ChatMessage> {
    vec![
        ChatMessage::new(ChatRole::System, "規則"),
        ChatMessage::new(ChatRole::User, "こんにちは"),
    ]
}

#[test]
fn hung_server_is_bounded_by_the_production_client_timeout() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let _connection = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(500));
    });
    let mut client = OpenAiCompatClient::new(LlmClientConfig {
        base_url: format!("http://127.0.0.1:{port}/v1"),
        model: "test-model".into(),
        allow_remote: false,
        timeout: Duration::from_millis(100),
    })
    .unwrap();
    let started = std::time::Instant::now();
    let result = client.stream_chat(&messages(), &AtomicBool::new(false), &mut |_| {});
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_millis(400));
    server.join().unwrap();
}

#[test]
fn cancellation_interrupts_a_request_waiting_for_response_headers() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let _connection = listener.accept().unwrap();
        std::thread::sleep(Duration::from_secs(2));
    });
    let mut client = OpenAiCompatClient::new(LlmClientConfig {
        base_url: format!("http://127.0.0.1:{port}/v1"),
        model: "test-model".into(),
        allow_remote: false,
        timeout: Duration::from_secs(30),
    })
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_in = Arc::clone(&cancel);
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        cancel_in.store(true, Ordering::Relaxed);
    });

    let started = std::time::Instant::now();
    client
        .stream_chat(&messages(), &cancel, &mut |_| {})
        .unwrap();
    assert!(started.elapsed() < Duration::from_millis(500));
    canceller.join().unwrap();
    server.join().unwrap();
}

#[test]
fn sends_the_openai_chat_request_shape_and_joins_deltas() {
    let server = spawn_server(200, sse_body(&["こん", "にちは。"]));
    let mut client = client_for(server.port);
    let mut output = String::new();

    client
        .stream_chat(&messages(), &AtomicBool::new(false), &mut |delta| {
            output.push_str(delta);
        })
        .unwrap();

    assert_eq!(output, "こんにちは。");
    let received = server.received.lock().unwrap();
    assert_eq!(received[0]["model"], "test-model");
    assert_eq!(received[0]["stream"], true);
    assert_eq!(received[0]["messages"][0]["role"], "system");
    assert_eq!(received[0]["messages"][1]["role"], "user");
    assert_eq!(received[0]["messages"][1]["content"], "こんにちは");
}

#[test]
fn cancellation_stops_the_stream_without_error() {
    let server = spawn_server(200, sse_body(&["一", "二", "三"]));
    let mut client = client_for(server.port);
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();

    client
        .stream_chat(&messages(), &cancel, &mut |delta| {
            seen.push(delta.to_owned());
            cancel.store(true, Ordering::Relaxed);
        })
        .unwrap();

    assert_eq!(seen.len(), 1, "seen: {seen:?}");
}

#[test]
fn http_errors_are_reported_with_status() {
    let server = spawn_server(500, "internal".into());
    let mut client = client_for(server.port);

    let error = client
        .stream_chat(&messages(), &AtomicBool::new(false), &mut |_| {})
        .unwrap_err();

    assert!(error.to_string().contains("500"), "{error}");
}

#[test]
fn malformed_sse_chunks_are_skipped() {
    let body = format!("data: not-json\n\n{}", sse_body(&["有効なデルタ。"]));
    let server = spawn_server(200, body);
    let mut client = client_for(server.port);
    let mut output = String::new();

    client
        .stream_chat(&messages(), &AtomicBool::new(false), &mut |delta| {
            output.push_str(delta);
        })
        .unwrap();

    assert_eq!(output, "有効なデルタ。");
}
