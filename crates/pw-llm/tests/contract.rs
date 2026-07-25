//! Contract tests against a local mock OpenAI-compatible server.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pw_application::conversation::{ChatMessage, ChatRole, LlmClient};
use pw_llm::{LlmClientConfig, OpenAiCompatClient};

struct MockServer {
    port: u16,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
    authorization: Arc<Mutex<Option<String>>>,
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
    let authorization = Arc::new(Mutex::new(None));
    let authorization_in = Arc::clone(&authorization);
    let handle = std::thread::spawn(move || {
        if let Ok(mut request) = server.recv() {
            *authorization_in.lock().unwrap() = request
                .headers()
                .iter()
                .find(|header| header.field.equiv("Authorization"))
                .map(|header| header.value.as_str().to_owned());
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
        authorization,
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
        api_key: None,
        allow_remote: false,
        timeout: Duration::from_secs(5),
        ..LlmClientConfig::default()
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
        api_key: None,
        allow_remote: false,
        timeout: Duration::from_millis(100),
        ..LlmClientConfig::default()
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
        api_key: None,
        allow_remote: false,
        timeout: Duration::from_secs(30),
        ..LlmClientConfig::default()
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

#[test]
fn sends_a_bearer_token_when_configured() {
    let server = spawn_server(200, sse_body(&["ok"]));
    let mut client = OpenAiCompatClient::new(LlmClientConfig {
        base_url: format!("http://127.0.0.1:{}/v1", server.port),
        model: "test-model".into(),
        api_key: Some("provider-secret".into()),
        allow_remote: false,
        timeout: Duration::from_secs(5),
        ..LlmClientConfig::default()
    })
    .unwrap();

    client
        .stream_chat(&messages(), &AtomicBool::new(false), &mut |_| {})
        .unwrap();

    assert_eq!(
        server.authorization.lock().unwrap().as_deref(),
        Some("Bearer provider-secret")
    );
}

#[test]
fn eof_after_a_delta_without_done_marker_is_an_error() {
    let chunk = serde_json::json!({
        "choices": [{ "delta": { "content": "partial" } }]
    });
    let server = spawn_server(200, format!("data: {chunk}\n\n"));
    let mut client = client_for(server.port);
    let mut output = String::new();

    let error = client
        .stream_chat(&messages(), &AtomicBool::new(false), &mut |delta| {
            output.push_str(delta);
        })
        .unwrap_err();

    assert_eq!(output, "partial");
    assert!(error.to_string().contains("[DONE]"), "{error}");
}

#[test]
fn done_marker_finishes_without_waiting_for_transport_eof() {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (done_sent_tx, done_sent_rx) = std::sync::mpsc::channel();
    let (release_eof_tx, release_eof_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = [0_u8; 8_192];
        let _ = stream.read(&mut request);

        let event = sse_body(&["完了。"]);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:X}\r\n{}\r\n",
            event.len(),
            event
        )
        .unwrap();
        stream.flush().unwrap();
        done_sent_tx.send(()).unwrap();

        // LM Studio can keep the HTTP stream alive briefly after [DONE].
        // The client must treat the protocol marker as completion instead
        // of waiting for the transport-level EOF.
        let _ = release_eof_rx.recv_timeout(Duration::from_secs(3));
        let _ = stream.write_all(b"0\r\n\r\n");
    });

    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let client_thread = std::thread::spawn(move || {
        let mut client = client_for(port);
        let mut output = String::new();
        let result = client.stream_chat(&messages(), &AtomicBool::new(false), &mut |delta| {
            output.push_str(delta);
        });
        result_tx.send((result, output)).unwrap();
    });
    done_sent_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let early_result = result_rx.recv_timeout(Duration::from_secs(1));
    let returned_before_eof = early_result.is_ok();
    release_eof_tx.send(()).unwrap();
    let (result, output) = early_result.unwrap_or_else(|_| {
        result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("client did not return after transport EOF")
    });
    result.unwrap();
    client_thread.join().unwrap();
    server.join().unwrap();

    assert_eq!(output, "完了。");
    assert!(returned_before_eof, "client waited for transport EOF");
}
