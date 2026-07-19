//! Contract tests against a loopback mock of the Irodori TTS HTTP API.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pw_tts::{IrodoriTtsClient, TtsClientConfig, TtsError};

#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    url: String,
    body: String,
}

struct Scripted {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    delay: Duration,
}

fn response(status: u16, content_type: &'static str, body: impl Into<Vec<u8>>) -> Scripted {
    Scripted {
        status,
        content_type,
        body: body.into(),
        delay: Duration::ZERO,
    }
}

struct MockServer {
    port: u16,
    received: Arc<Mutex<Vec<Recorded>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

fn spawn_server(responses: Vec<Scripted>) -> MockServer {
    let server = tiny_http::Server::http("127.0.0.1:0").expect("bind mock server");
    let port = server.server_addr().to_ip().unwrap().port();
    let received = Arc::new(Mutex::new(Vec::new()));
    let received_in = Arc::clone(&received);
    let handle = std::thread::spawn(move || {
        for scripted in responses {
            let Ok(mut request) = server.recv() else {
                return;
            };
            let mut body = String::new();
            let _ = std::io::Read::read_to_string(request.as_reader(), &mut body);
            received_in.lock().unwrap().push(Recorded {
                method: request.method().to_string(),
                url: request.url().to_owned(),
                body,
            });
            std::thread::sleep(scripted.delay);
            let reply = tiny_http::Response::from_data(scripted.body)
                .with_status_code(scripted.status)
                .with_header(
                    tiny_http::Header::from_bytes(
                        &b"Content-Type"[..],
                        scripted.content_type.as_bytes(),
                    )
                    .unwrap(),
                );
            let _ = request.respond(reply);
        }
    });
    MockServer {
        port,
        received,
        handle: Some(handle),
    }
}

impl MockServer {
    fn requests(&self) -> Vec<Recorded> {
        self.received.lock().unwrap().clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn client_for(port: u16) -> IrodoriTtsClient {
    IrodoriTtsClient::new(&TtsClientConfig {
        base_url: format!("http://127.0.0.1:{port}"),
        timeout: Duration::from_secs(1),
    })
    .unwrap()
}

#[test]
fn synthesize_posts_the_upstream_speech_request_and_returns_wav() {
    let wav = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
    let server = spawn_server(vec![response(200, "audio/wav", wav.clone())]);
    let client = client_for(server.port);

    let bytes = client.synthesize("こんにちは", "sample", 1.5).unwrap();

    assert_eq!(bytes, wav);
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].url, "/v1/audio/speech");
    let body: serde_json::Value = serde_json::from_str(&requests[0].body).unwrap();
    assert_eq!(body["model"], "irodori-tts");
    assert_eq!(body["input"], "こんにちは");
    assert_eq!(body["voice"], "sample");
    assert_eq!(body["response_format"], "wav");
    assert_eq!(body["speed"], 1.5);
}

#[test]
fn synthesize_clamps_speed_to_the_supported_range() {
    let wav = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
    let server = spawn_server(vec![
        response(200, "audio/wav", wav.clone()),
        response(200, "audio/wav", wav),
    ]);
    let client = client_for(server.port);

    client.synthesize("slow", "sample", 0.1).unwrap();
    client.synthesize("fast", "sample", 9.0).unwrap();

    let requests = server.requests();
    let slow: serde_json::Value = serde_json::from_str(&requests[0].body).unwrap();
    let fast: serde_json::Value = serde_json::from_str(&requests[1].body).unwrap();
    assert_eq!(slow["speed"], 0.25);
    assert_eq!(fast["speed"], 4.0);
}

#[test]
fn voices_extracts_ids_from_the_upstream_list_envelope() {
    let server = spawn_server(vec![response(
        200,
        "application/json",
        br#"{"object":"list","data":[{"id":"sample","object":"voice","name":"Sample"},{"id":"other","object":"voice"}]}"#
            .to_vec(),
    )]);
    let client = client_for(server.port);

    assert_eq!(client.voices().unwrap(), vec!["sample", "other"]);
    let requests = server.requests();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].url, "/v1/audio/voices");
}

#[test]
fn voices_rejects_an_envelope_without_the_object_discriminator() {
    let server = spawn_server(vec![response(
        200,
        "application/json",
        br#"{"data":[{"id":"sample","object":"voice"}]}"#.to_vec(),
    )]);

    let error = client_for(server.port).voices().unwrap_err();

    assert!(matches!(error, TtsError::Protocol(_)), "{error:?}");
}

#[test]
fn voices_rejects_an_envelope_with_a_non_list_object_discriminator() {
    let server = spawn_server(vec![response(
        200,
        "application/json",
        br#"{"object":"voice","data":[{"id":"sample","object":"voice"}]}"#.to_vec(),
    )]);

    let error = client_for(server.port).voices().unwrap_err();

    assert!(matches!(error, TtsError::Protocol(_)), "{error:?}");
}

#[test]
fn api_failures_preserve_4xx_and_5xx_statuses() {
    let client_error_server = spawn_server(vec![response(
        422,
        "application/json",
        br#"{"detail":"invalid voice"}"#.to_vec(),
    )]);
    let client_error = client_for(client_error_server.port)
        .synthesize("hello", "missing", 1.0)
        .unwrap_err();
    assert!(matches!(
        client_error,
        TtsError::Api { status: 422, ref detail } if detail.contains("invalid voice")
    ));

    let server_error_server =
        spawn_server(vec![response(503, "text/plain", b"unavailable".to_vec())]);
    let server_error = client_for(server_error_server.port).voices().unwrap_err();
    assert!(matches!(
        server_error,
        TtsError::Api { status: 503, ref detail } if detail.contains("unavailable")
    ));
}

#[test]
fn non_wav_synthesis_body_is_a_protocol_error() {
    let server = spawn_server(vec![response(
        200,
        "application/json",
        br#"{"unexpected":"json"}"#.to_vec(),
    )]);
    let error = client_for(server.port)
        .synthesize("hello", "sample", 1.0)
        .unwrap_err();

    assert!(matches!(error, TtsError::Protocol(_)), "{error:?}");
}

#[test]
fn riff_without_wave_signature_is_a_protocol_error() {
    let server = spawn_server(vec![response(
        200,
        "audio/wav",
        b"RIFF\x24\x00\x00\x00NOPEfmt ".to_vec(),
    )]);
    let error = client_for(server.port)
        .synthesize("hello", "sample", 1.0)
        .unwrap_err();

    assert!(matches!(error, TtsError::Protocol(_)), "{error:?}");
}

#[test]
fn transport_failure_is_mapped() {
    let port = {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        server.server_addr().to_ip().unwrap().port()
    };
    let error = client_for(port).voices().unwrap_err();

    assert!(matches!(error, TtsError::Transport(_)), "{error:?}");
}

#[test]
fn request_timeout_is_bounded_by_the_configured_timeout() {
    let mut delayed = response(
        200,
        "application/json",
        br#"{"object":"list","data":[]}"#.to_vec(),
    );
    delayed.delay = Duration::from_millis(500);
    let server = spawn_server(vec![delayed]);
    let client = IrodoriTtsClient::new(&TtsClientConfig {
        base_url: format!("http://127.0.0.1:{}", server.port),
        timeout: Duration::from_millis(100),
    })
    .unwrap();
    let started = std::time::Instant::now();

    assert!(matches!(client.voices(), Err(TtsError::Transport(_))));
    assert!(started.elapsed() < Duration::from_millis(400));
}

#[test]
fn rejects_non_loopback_endpoints() {
    let error = IrodoriTtsClient::new(&TtsClientConfig {
        base_url: "http://example.com:8088".to_owned(),
        ..TtsClientConfig::default()
    })
    .unwrap_err();

    assert!(matches!(error, TtsError::InvalidEndpoint(_)));
}
