//! Acceptance tests against real TTS engine instances.
//!
//! Requires the engine running on loopback (default port 10101):
//! `cargo test -p pw-tts --test real_engine -- --ignored --nocapture`
//! Override the endpoint with `PW_TTS_BASE_URL`.
//!
//! The Irodori acceptance test requires `PW_IRODORI_BASE_URL` and optionally
//! uses `PW_IRODORI_VOICE`. Without `PW_IRODORI_BASE_URL`, it reports an
//! explicit self-skip so the existing Aivis command above remains compatible.
//! Run each engine independently with an exact test-name filter:
//! `cargo test -p pw-tts --test real_engine speakers_then_synthesis_produces_wav -- --ignored --nocapture`
//! `cargo test -p pw-tts --test real_engine irodori_voices_then_short_synthesis_produces_wav_and_records_latency -- --ignored --nocapture`

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

fn explicit_irodori_base_url(value: Option<String>) -> Option<String> {
    value.filter(|base_url| !base_url.trim().is_empty())
}

fn irodori_client() -> Option<IrodoriTtsClient> {
    let base_url = explicit_irodori_base_url(std::env::var("PW_IRODORI_BASE_URL").ok())?;
    Some(
        IrodoriTtsClient::new(&TtsClientConfig {
            base_url,
            timeout: Duration::from_mins(1),
        })
        .expect("Irodori client"),
    )
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

fn wav_has_decodable_sample(wav: &[u8]) -> Result<bool, hound::Error> {
    let mut reader = hound::WavReader::new(std::io::Cursor::new(wav))?;
    match reader.spec().sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i32>()
            .next()
            .transpose()
            .map(|sample| sample.is_some()),
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .next()
            .transpose()
            .map(|sample| sample.is_some()),
    }
}

fn pcm16_wav(samples: &[i16]) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 24_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = Vec::new();
    {
        let mut writer = hound::WavWriter::new(std::io::Cursor::new(&mut wav), spec).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
    }
    wav
}

#[test]
fn wav_sample_check_rejects_metadata_only_wav() {
    assert!(!wav_has_decodable_sample(&pcm16_wav(&[])).unwrap());
}

#[test]
fn wav_sample_check_accepts_one_pcm_sample() {
    assert!(wav_has_decodable_sample(&pcm16_wav(&[1])).unwrap());
}

#[test]
fn irodori_opt_in_requires_a_non_empty_explicit_url() {
    assert_eq!(explicit_irodori_base_url(None), None);
    assert_eq!(explicit_irodori_base_url(Some("   ".to_owned())), None);
    assert_eq!(
        explicit_irodori_base_url(Some("http://127.0.0.1:8088".to_owned())),
        Some("http://127.0.0.1:8088".to_owned())
    );
}

#[test]
#[ignore = "requires a running Irodori TTS server and an installed voice"]
fn irodori_voices_then_short_synthesis_produces_wav_and_records_latency() {
    let Some(client) = irodori_client() else {
        eprintln!(
            "SKIPPED: PW_IRODORI_BASE_URL is not set; no Irodori server request was attempted"
        );
        return;
    };
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
        wav_has_decodable_sample(&wav).expect("decode Irodori WAV"),
        "wav contains no decodable PCM sample"
    );
    println!("synthesized {} bytes in {elapsed:?}", wav.len());
}
