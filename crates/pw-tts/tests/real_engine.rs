//! Acceptance test against a real `AivisSpeech` Engine instance.
//!
//! Requires the engine running on loopback (default port 10101):
//! `cargo test -p pw-tts --test real_engine -- --ignored --nocapture`
//! Override the endpoint with `PW_TTS_BASE_URL`.

use std::time::Duration;

use pw_application::speech_synthesis::TtsSynthesizer;
use pw_tts::{
    AivisSpeechClient, CachedSpeechSynthesizer, SynthesisParams, TtsClientConfig, WavCache,
    cache_key,
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
    let key = cache_key("キャッシュの検証です。", style_id, &params);
    let synthesizer =
        CachedSpeechSynthesizer::new(client, WavCache::new(dir.clone(), 10), style_id, params);

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
