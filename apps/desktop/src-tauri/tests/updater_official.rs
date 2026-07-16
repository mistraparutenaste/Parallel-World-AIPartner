//! Exercises the official Tauri updater download and minisign verification path
//! against a loopback HTTPS server with an explicitly trusted fixture certificate.

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
};

use rcgen::{CertifiedKey, generate_simple_self_signed};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use tauri_plugin_updater::UpdaterExt;

const ARTIFACT: &[u8] = include_bytes!("../../../../tools/fixtures/updater/fixture-update.bin");
const SIGNATURE: &str = include_str!("../../../../tools/fixtures/updater/fixture-update.bin.sig");
const PUBLIC_KEY: &str = include_str!("../../../../tools/fixtures/updater/test-public.key");
const WRONG_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXkgRTc2MjBGMTg0MkI0RTgxRgpSV1FmNkxSQ0dBOWk1M21sWWVjTzRJelQ1MVRHUHB2V3VjTlNDaDFDQk0wUVRhTG43M1k3R0ZPMwo=";

struct HttpsFixture {
    endpoint: String,
    certificate: CertificateDer<'static>,
    server: thread::JoinHandle<()>,
}

impl HttpsFixture {
    fn start(artifact: Vec<u8>) -> Self {
        let CertifiedKey { cert, signing_key } =
            generate_simple_self_signed(vec!["localhost".to_owned()])
                .expect("fixture certificate should be generated");
        let certificate = cert.der().clone();
        let private_key =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(signing_key.serialize_der()));
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate.clone()], private_key)
            .expect("fixture TLS server config should be valid");
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("fixture listener should bind");
        let port = listener
            .local_addr()
            .expect("fixture listener should have an address")
            .port();
        let endpoint = format!("https://localhost:{port}/latest.json");
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                serve_request(
                    stream.expect("fixture connection should be accepted"),
                    Arc::new(config.clone()),
                    port,
                    &artifact,
                );
            }
        });
        Self {
            endpoint,
            certificate,
            server,
        }
    }

    fn join(self) {
        self.server
            .join()
            .expect("fixture HTTPS server should finish");
    }
}

fn serve_request(stream: TcpStream, config: Arc<ServerConfig>, port: u16, artifact: &[u8]) {
    let connection =
        ServerConnection::new(config).expect("fixture TLS connection should initialize");
    let mut stream = StreamOwned::new(connection, stream);
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream
            .read(&mut buffer)
            .expect("fixture request should be readable");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let request_line = String::from_utf8_lossy(&request);
    let path = request_line
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("fixture request should contain a path");
    let (content_type, body) = if path == "/latest.json" {
        let body = serde_json::json!({
            "version": "0.2.0",
            "notes": "signed updater fixture",
            "url": format!("https://localhost:{port}/fixture-update.bin"),
            "signature": SIGNATURE.trim(),
        })
        .to_string()
        .into_bytes();
        ("application/json", body)
    } else {
        ("application/octet-stream", artifact.to_vec())
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("fixture response headers should be writable");
    stream
        .write_all(&body)
        .expect("fixture response body should be writable");
    stream.flush().expect("fixture response should flush");
}

fn official_download(artifact: Vec<u8>, public_key: &str) -> tauri_plugin_updater::Result<Vec<u8>> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let fixture = HttpsFixture::start(artifact);
    let certificate = reqwest_updater::Certificate::from_der(fixture.certificate.as_ref())
        .expect("fixture certificate should be accepted by reqwest");
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().plugins.0.insert(
        "updater".to_owned(),
        serde_json::json!({ "pubkey": public_key }),
    );
    let app = tauri::test::mock_builder()
        .plugin(
            tauri_plugin_updater::Builder::new()
                .pubkey(public_key)
                .build(),
        )
        .build(context)
        .expect("mock Tauri app should build");
    let updater = app
        .handle()
        .updater_builder()
        .pubkey(public_key)
        .endpoints(vec![
            fixture
                .endpoint
                .parse()
                .expect("fixture endpoint should be a URL"),
        ])
        .expect("fixture endpoint should be accepted")
        .configure_client(move |builder| {
            builder.add_root_certificate(certificate.clone()).no_proxy()
        })
        .build()
        .expect("fixture updater should build");

    let result = tauri::async_runtime::block_on(async {
        let update = updater
            .check()
            .await?
            .expect("fixture should advertise an update");
        update.download(|_, _| {}, || {}).await
    });
    fixture.join();
    result
}

#[test]
fn official_download_accepts_valid_signed_artifact_over_https() {
    let downloaded = official_download(ARTIFACT.to_vec(), PUBLIC_KEY.trim())
        .expect("official updater download should verify the fixture signature");

    assert_eq!(downloaded, ARTIFACT);
}

#[test]
fn official_download_rejects_one_byte_artifact_tamper() {
    let mut tampered = ARTIFACT.to_vec();
    tampered[0] ^= 1;

    let error = official_download(tampered, PUBLIC_KEY.trim())
        .expect_err("official updater download must reject a modified artifact");

    assert!(
        error.to_string().to_ascii_lowercase().contains("signature"),
        "unexpected verification error: {error}"
    );
}

#[test]
fn official_download_rejects_signature_from_another_key() {
    let error = official_download(ARTIFACT.to_vec(), WRONG_PUBLIC_KEY)
        .expect_err("official updater download must reject the wrong public key");

    assert!(
        error.to_string().to_ascii_lowercase().contains("signature"),
        "unexpected verification error: {error}"
    );
}
