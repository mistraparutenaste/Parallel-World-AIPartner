//! End-to-end acceptance tests with the real VAD and STT models.
//!
//! Run manually (models required):
//! ```text
//! PW_VAD_MODEL=<path to silero_vad.onnx> \
//! PW_STT_MODEL_DIR=<extracted reazonspeech dir> \
//! cargo test -p pw-stt-sherpa --test e2e_pipeline -- --ignored --nocapture
//! ```

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use pw_application::speech::{
    PipelineDiagnostics, SpeechEvents, SpeechPipeline, SpeechPipelineConfig,
};
use pw_domain::speech::RejectionReason;
use pw_stt_sherpa::{ReazonSpeechRecognizer, RecognizerModelPaths, SileroVad};

const FRAME_LEN: usize = 512;

#[derive(Default)]
struct Collector {
    transcripts: Mutex<Vec<String>>,
    rejections: Mutex<Vec<RejectionReason>>,
}

struct Events(Arc<Collector>);

impl SpeechEvents for Events {
    fn on_level(&self, _rms: f32) {}
    fn on_speech_started(&self) {}
    fn on_transcript(&self, text: &str) {
        self.0.transcripts.lock().unwrap().push(text.to_owned());
    }
    fn on_rejected(&self, reason: RejectionReason) {
        self.0.rejections.lock().unwrap().push(reason);
    }
    fn on_error(&self, message: &str) {
        panic!("pipeline error: {message}");
    }
}

struct Setup {
    pipeline: SpeechPipeline<SileroVad, ReazonSpeechRecognizer, Events>,
    collector: Arc<Collector>,
}

fn setup() -> Setup {
    let vad_model = std::env::var("PW_VAD_MODEL").expect("set PW_VAD_MODEL");
    let stt_dir = std::env::var("PW_STT_MODEL_DIR").expect("set PW_STT_MODEL_DIR");
    let vad = SileroVad::new(std::path::Path::new(&vad_model), 0.5).unwrap();
    let recognizer = ReazonSpeechRecognizer::new(&RecognizerModelPaths::in_directory(
        std::path::Path::new(&stt_dir),
    ))
    .unwrap();
    let collector = Arc::new(Collector::default());
    let pipeline = SpeechPipeline::new(
        SpeechPipelineConfig::default(),
        vad,
        recognizer,
        Events(Arc::clone(&collector)),
        Arc::new(AtomicBool::new(true)),
        Arc::new(PipelineDiagnostics::default()),
    );
    Setup {
        pipeline,
        collector,
    }
}

/// Acceptance: 無音10分でLLM送信0件。
#[test]
#[ignore = "requires downloaded models"]
fn ten_minutes_of_silence_produce_zero_transcripts() {
    let mut s = setup();
    let frame = vec![0.0_f32; FRAME_LEN];
    // 10 minutes at 32 ms frames.
    for _ in 0..(600_000 / 32) {
        s.pipeline.push_frame(&frame);
    }
    let transcripts = s.collector.transcripts.lock().unwrap();
    assert!(
        transcripts.is_empty(),
        "unexpected transcripts: {transcripts:?}"
    );
}

/// Acceptance: 通常の短文を安定して認識できる。
#[test]
#[ignore = "requires downloaded models"]
fn bundled_japanese_sample_is_recognized_through_the_full_pipeline() {
    let stt_dir = std::env::var("PW_STT_MODEL_DIR").expect("set PW_STT_MODEL_DIR");
    let wave = sherpa_onnx::Wave::read(
        &std::path::Path::new(&stt_dir)
            .join("test_wavs/3.wav")
            .to_string_lossy(),
    )
    .expect("read bundled test wav");
    assert_eq!(wave.sample_rate(), 16_000);

    let mut s = setup();
    for chunk in wave.samples().chunks(FRAME_LEN) {
        let mut frame = chunk.to_vec();
        frame.resize(FRAME_LEN, 0.0);
        s.pipeline.push_frame(&frame);
    }
    // Trailing silence so the hang time elapses and the segment closes.
    let silence = vec![0.0_f32; FRAME_LEN];
    for _ in 0..64 {
        s.pipeline.push_frame(&silence);
    }

    let transcripts = s.collector.transcripts.lock().unwrap();
    assert_eq!(transcripts.len(), 1, "transcripts: {transcripts:?}");
    assert!(
        transcripts[0].contains("ヤンバルクイナ"),
        "unexpected transcription: {}",
        transcripts[0]
    );
}
