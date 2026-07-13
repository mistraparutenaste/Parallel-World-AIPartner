//! Contract tests against a local mock `AivisSpeech` (VOICEVOX API)
//! server: request shapes, WAV delivery, error mapping and the user
//! dictionary CRUD surface.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pw_tts::{AivisSpeechClient, SynthesisParams, TtsClientConfig, TtsError};

/// One recorded request: method, url (path + query) and body.
#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    url: String,
    body: String,
}

/// One scripted response.
struct Scripted {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

fn json_response(body: &str) -> Scripted {
    Scripted {
        status: 200,
        content_type: "application/json",
        body: body.as_bytes().to_vec(),
    }
}

struct MockServer {
    port: u16,
    received: Arc<Mutex<Vec<Recorded>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// Serves the scripted responses in order, recording every request.
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
            let response = tiny_http::Response::from_data(scripted.body)
                .with_status_code(scripted.status)
                .with_header(
                    tiny_http::Header::from_bytes(
                        &b"Content-Type"[..],
                        scripted.content_type.as_bytes(),
                    )
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

fn client_for(port: u16) -> AivisSpeechClient {
    AivisSpeechClient::new(&TtsClientConfig {
        base_url: format!("http://127.0.0.1:{port}"),
        timeout: Duration::from_secs(5),
    })
    .unwrap()
}

#[test]
fn hung_server_is_bounded_by_the_production_client_timeout() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let _connection = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(500));
    });
    let client = AivisSpeechClient::new(&TtsClientConfig {
        base_url: format!("http://127.0.0.1:{port}"),
        timeout: Duration::from_millis(100),
    })
    .unwrap();
    let started = std::time::Instant::now();
    assert!(client.speakers().is_err());
    assert!(started.elapsed() < Duration::from_millis(400));
    server.join().unwrap();
}

#[test]
fn speakers_parses_styles() {
    let server = spawn_server(vec![json_response(
        r#"[
            {"name":"Anneli","speaker_uuid":"u1","version":"1.0",
             "styles":[{"name":"ノーマル","id":888753760},{"name":"通常","id":888753761}]},
            {"name":"White","speaker_uuid":"u2","version":"1.0",
             "styles":[{"name":"ノーマル","id":706073888}]}
        ]"#,
    )]);
    let client = client_for(server.port);

    let speakers = client.speakers().unwrap();

    assert_eq!(speakers.len(), 2);
    assert_eq!(speakers[0].name, "Anneli");
    assert_eq!(speakers[0].styles[0].id, 888_753_760);
    assert_eq!(speakers[1].styles[0].name, "ノーマル");
    let requests = server.requests();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].url, "/speakers");
}

#[test]
fn synthesize_runs_audio_query_then_synthesis_with_scales() {
    let wav = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
    let server = spawn_server(vec![
        json_response(r#"{"accent_phrases":[],"speedScale":1.0,"volumeScale":1.0}"#),
        Scripted {
            status: 200,
            content_type: "audio/wav",
            body: wav.clone(),
        },
    ]);
    let client = client_for(server.port);

    let bytes = client
        .synthesize(
            "こんにちは。",
            888_753_760,
            &SynthesisParams {
                volume: 0.8,
                speed: 1.2,
            },
        )
        .unwrap();

    assert_eq!(bytes, wav);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "POST");
    assert!(
        requests[0].url.starts_with("/audio_query?"),
        "{}",
        requests[0].url
    );
    assert!(
        requests[0]
            .url
            .contains("text=%E3%81%93%E3%82%93%E3%81%AB%E3%81%A1%E3%81%AF%E3%80%82"),
        "{}",
        requests[0].url
    );
    assert!(requests[0].url.contains("speaker=888753760"));
    assert_eq!(requests[1].method, "POST");
    assert!(requests[1].url.starts_with("/synthesis?"));
    assert!(requests[1].url.contains("speaker=888753760"));
    let query: serde_json::Value = serde_json::from_str(&requests[1].body).unwrap();
    assert!((query["volumeScale"].as_f64().unwrap() - 0.8).abs() < 1e-6);
    assert!((query["speedScale"].as_f64().unwrap() - 1.2).abs() < 1e-6);
}

#[test]
fn synthesis_error_is_mapped_to_api_error() {
    let server = spawn_server(vec![Scripted {
        status: 422,
        content_type: "application/json",
        body: br#"{"detail":"invalid speaker"}"#.to_vec(),
    }]);
    let client = client_for(server.port);

    let error = client
        .synthesize("やあ", 1, &SynthesisParams::default())
        .unwrap_err();

    match error {
        TtsError::Api { status, detail } => {
            assert_eq!(status, 422);
            assert!(detail.contains("invalid speaker"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn non_wav_synthesis_body_is_a_protocol_error() {
    let server = spawn_server(vec![
        json_response("{}"),
        json_response(r#"{"unexpected":"json"}"#),
    ]);
    let client = client_for(server.port);

    let error = client
        .synthesize("やあ", 1, &SynthesisParams::default())
        .unwrap_err();

    assert!(matches!(error, TtsError::Protocol(_)), "{error:?}");
}

#[test]
fn user_dict_crud_round_trip() {
    let server = spawn_server(vec![
        // add → uuid
        json_response(r#""aaaa-bbbb""#),
        // list → map keyed by uuid
        json_response(
            r#"{"aaaa-bbbb":{"surface":"ＬＬＭ","priority":5,"context_id":1348,
                "part_of_speech":"名詞","part_of_speech_detail_1":"固有名詞",
                "part_of_speech_detail_2":"一般","part_of_speech_detail_3":"*",
                "inflectional_type":"*","inflectional_form":"*","stem":"*",
                "yomi":"エルエルエム","pronunciation":"エルエルエム",
                "accent_type":1,"mora_count":6,"accent_associative_rule":"*"}}"#,
        ),
        // delete → 204
        Scripted {
            status: 204,
            content_type: "application/json",
            body: Vec::new(),
        },
    ]);
    let client = client_for(server.port);

    let uuid = client
        .add_user_dict_word("ＬＬＭ", "エルエルエム", 1)
        .unwrap();
    assert_eq!(uuid, "aaaa-bbbb");

    let words = client.user_dict().unwrap();
    assert_eq!(words.len(), 1);
    assert_eq!(words[0].uuid, "aaaa-bbbb");
    assert_eq!(words[0].pronunciation, "エルエルエム");
    assert_eq!(words[0].accent_type, 1);

    client.delete_user_dict_word("aaaa-bbbb").unwrap();

    let requests = server.requests();
    assert_eq!(requests[0].method, "POST");
    assert!(requests[0].url.starts_with("/user_dict_word?"));
    assert!(
        requests[0].url.contains("accent_type=1"),
        "{}",
        requests[0].url
    );
    assert_eq!(requests[1].method, "GET");
    assert_eq!(requests[1].url, "/user_dict");
    assert_eq!(requests[2].method, "DELETE");
    assert_eq!(requests[2].url, "/user_dict_word/aaaa-bbbb");
}

#[test]
fn transport_failure_is_mapped() {
    // Nothing listens on this port (bound then dropped immediately).
    let port = {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        server.server_addr().to_ip().unwrap().port()
    };
    let client = client_for(port);
    let error = client.speakers().unwrap_err();
    assert!(matches!(error, TtsError::Transport(_)), "{error:?}");
}
