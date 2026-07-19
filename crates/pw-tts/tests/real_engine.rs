//! Acceptance test against a real `AivisSpeech` Engine instance.
//!
//! Requires the engine running on loopback (default port 10101):
//! `cargo test -p pw-tts --test real_engine -- --ignored --nocapture`
//! Override the endpoint with `PW_TTS_BASE_URL`.
//!
//! The Irodori acceptance test uses `PW_IRODORI_BASE_URL` (default port 8088)
//! and optionally `PW_IRODORI_VOICE`. It remains ignored unless explicitly run.

use std::time::{Duration, Instant};

use pw_application::speech_synthesis::TtsSynthesizer;
use pw_tts::{
    AivisSpeechClient, CachedSpeechSynthesizer, EngineClient, IrodoriTtsClient, SynthesisParams,
    TtsClientConfig, WavCache, cache_key,
};

fn base_url() -> String {
    std::env::var("PW_TTS_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:10101".to_owned())
}

fn client() -> AivisSpeechClient {
    AivisSpeechClient::new(&TtsClientConfig {
        base_url: base_url(),
        timeout: Duration::from_mins(1),
    })
    .expect("client")
}

#[test]
#[ignore = "requires a running AivisSpeech Engine"]
fn speakers_then_synthesis_produces_wav() {
    let client = client();

    let speakers = client.speakers().expect("GET /speakers");
    assert!(!speakers.is_empty(), "engine reports no speakers");
    let style = speakers[0].styles.first().expect("speaker without styles");
    println!("using {} / {} ({})", speakers[0].name, style.name, style.id);

    let wav = client
        .synthesize(
            "こんにちは。音声合成のテストです。",
            style.id,
            &SynthesisParams::default(),
        )
        .expect("synthesis");
    assert!(wav.starts_with(b"RIFF"));
    assert!(
        wav.len() > 44,
        "wav has no sample data: {} bytes",
        wav.len()
    );
    println!("synthesized {} bytes", wav.len());
}

#[test]
#[ignore = "requires a running AivisSpeech Engine"]
fn cached_synthesizer_reuses_the_wav_file() {
    let client = client();
    let style_id = client.speakers().expect("speakers")[0]
        .styles
        .first()
        .expect("styles")
        .id;

    let dir = std::env::temp_dir().join(format!("pw-tts-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let params = SynthesisParams::default();
    let voice_id = style_id.to_string();
    let key = cache_key("aivis", &voice_id, "キャッシュの検証です。", &params);
    let synthesizer = CachedSpeechSynthesizer::new(
        EngineClient::Aivis(client),
        WavCache::new(dir.clone(), 10),
        voice_id,
        params,
    );

    let first = synthesizer
        .synthesize("キャッシュの検証です。")
        .expect("first synthesis");
    let modified = first.metadata().and_then(|meta| meta.modified()).unwrap();
    assert_eq!(first, WavCache::new(dir, 10).path_for(&key));

    let second = synthesizer
        .synthesize("キャッシュの検証です。")
        .expect("second synthesis (cache hit)");
    assert_eq!(first, second);
    let modified_again = second.metadata().and_then(|meta| meta.modified()).unwrap();
    assert_eq!(
        modified, modified_again,
        "cache hit must not rewrite the file"
    );
}

fn irodori_client() -> IrodoriTtsClient {
    let base_url =
        std::env::var("PW_IRODORI_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8088".to_owned());
    IrodoriTtsClient::new(&TtsClientConfig {
        base_url,
        timeout: Duration::from_mins(1),
    })
    .expect("Irodori client")
}

fn selected_irodori_voice(voices: &[String]) -> String {
    let requested = std::env::var("PW_IRODORI_VOICE").ok();
    match requested {
        Some(voice) => {
            assert!(
                voices.contains(&voice),
                "PW_IRODORI_VOICE {voice:?} is not installed"
            );
            voice
        }
        None => voices[0].clone(),
    }
}

#[test]
#[ignore = "requires a running Irodori TTS server and an installed voice"]
fn irodori_voices_then_short_synthesis_produces_wav_and_records_latency() {
    let client = irodori_client();
    let voices = client.voices().expect("GET /v1/audio/voices");
    assert!(!voices.is_empty(), "server reports no Irodori voices");

    let voice = selected_irodori_voice(&voices);
    println!("using Irodori voice {voice}");

    let started = Instant::now();
    let wav = client
        .synthesize("こんにちは。", &voice, 1.0)
        .expect("POST /v1/audio/speech");
    let elapsed = started.elapsed();

    assert!(wav.starts_with(b"RIFF"), "response has no RIFF header");
    assert_eq!(wav.get(8..12), Some(&b"WAVE"[..]), "response is not WAVE");
    assert!(
        wav.len() > 44,
        "wav has no sample data: {} bytes",
        wav.len()
    );
    println!("synthesized {} bytes in {elapsed:?}", wav.len());
}
