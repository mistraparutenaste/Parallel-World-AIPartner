//! Frame-driven speech pipeline: VAD, segmentation, STT, filtering.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use pw_domain::speech::{
    FilterConfig, SegmentEvent, SegmenterConfig, SpeechSegmenter, TranscriptCandidate,
    TranscriptFilter,
};

use super::ports::{
    FrameRead, SpeechEvents, SpeechFrameSource, SpeechRecognizer, VoiceActivityDetector,
};

/// Tuning for [`SpeechPipeline`].
#[derive(Debug, Clone, Copy)]
pub struct SpeechPipelineConfig {
    pub segmenter: SegmenterConfig,
    pub filter: FilterConfig,
    /// Samples per VAD frame (512 at 16 kHz).
    pub frame_len: usize,
    /// Sample rate of incoming frames.
    pub sample_rate: u32,
}

impl Default for SpeechPipelineConfig {
    fn default() -> Self {
        Self {
            segmenter: SegmenterConfig {
                frame_ms: 32,
                speech_threshold: 0.5,
                pre_roll_ms: 224,
                min_speech_ms: 192,
                hang_ms: 640,
                max_segment_ms: 30_000,
            },
            filter: FilterConfig {
                min_speech_ms: 192,
                min_mean_probability: 0.55,
            },
            frame_len: 512,
            sample_rate: 16_000,
        }
    }
}

/// Shared counters observable from the diagnostics UI.
#[derive(Debug, Default)]
pub struct PipelineDiagnostics {
    pub frames_processed: AtomicU64,
    pub segments_completed: AtomicU64,
    pub transcripts_accepted: AtomicU64,
    pub transcripts_rejected: AtomicU64,
}

/// Synchronous, frame-driven pipeline core. Threading is applied by
/// [`run_pipeline`]; keeping the core synchronous makes the whole
/// accept/reject behaviour unit-testable.
pub struct SpeechPipeline<V, R, E> {
    config: SpeechPipelineConfig,
    vad: V,
    recognizer: R,
    events: E,
    segmenter: SpeechSegmenter,
    filter: TranscriptFilter,
    /// Rolling history for pre-roll while idle.
    history: VecDeque<f32>,
    /// Samples of the in-progress segment (pre-roll included).
    segment_samples: Vec<f32>,
    speaking: bool,
    capture_enabled: Arc<AtomicBool>,
    diagnostics: Arc<PipelineDiagnostics>,
}

impl<V, R, E> SpeechPipeline<V, R, E>
where
    V: VoiceActivityDetector,
    R: SpeechRecognizer,
    E: SpeechEvents,
{
    pub fn new(
        config: SpeechPipelineConfig,
        vad: V,
        recognizer: R,
        events: E,
        capture_enabled: Arc<AtomicBool>,
        diagnostics: Arc<PipelineDiagnostics>,
    ) -> Self {
        let history_len = config.pre_roll_samples();
        Self {
            segmenter: SpeechSegmenter::new(config.segmenter),
            filter: TranscriptFilter::new(config.filter),
            history: VecDeque::with_capacity(history_len),
            segment_samples: Vec::new(),
            speaking: false,
            config,
            vad,
            recognizer,
            events,
            capture_enabled,
            diagnostics,
        }
    }

    /// Processes one frame of `config.frame_len` samples.
    pub fn push_frame(&mut self, frame: &[f32]) {
        self.diagnostics
            .frames_processed
            .fetch_add(1, Ordering::Relaxed);
        self.events.on_level(rms(frame));

        if !self.capture_enabled.load(Ordering::Relaxed) {
            // Capture disabled (mute or TTS playback): drop any
            // in-progress segment and keep only pre-roll history.
            if self.speaking {
                self.speaking = false;
                self.segment_samples.clear();
                self.segmenter.reset();
                self.vad.reset();
            }
            self.push_history(frame);
            return;
        }

        let probability = match self.vad.probability(frame) {
            Ok(probability) => probability,
            Err(error) => {
                self.events.on_error(&error.to_string());
                return;
            }
        };

        let events = self.segmenter.push_frame(probability);
        if self.speaking {
            self.segment_samples.extend_from_slice(frame);
        }
        for event in events {
            match event {
                SegmentEvent::Started { .. } => {
                    self.speaking = true;
                    self.segment_samples.clear();
                    // Pre-roll plus the frame that triggered speech.
                    self.segment_samples.extend(self.history.iter().copied());
                    self.segment_samples.extend_from_slice(frame);
                    self.events.on_speech_started();
                }
                SegmentEvent::Completed(segment) => {
                    self.speaking = false;
                    self.diagnostics
                        .segments_completed
                        .fetch_add(1, Ordering::Relaxed);
                    let samples = std::mem::take(&mut self.segment_samples);
                    self.vad.reset();
                    match self.recognizer.transcribe(&samples) {
                        Ok(text) => {
                            let candidate = TranscriptCandidate {
                                text: &text,
                                speech_ms: segment.speech_ms,
                                mean_probability: segment.mean_probability,
                                capture_enabled: self.capture_enabled.load(Ordering::Relaxed),
                            };
                            match self.filter.evaluate(&candidate) {
                                Ok(()) => {
                                    self.diagnostics
                                        .transcripts_accepted
                                        .fetch_add(1, Ordering::Relaxed);
                                    self.events.on_transcript(text.trim());
                                }
                                Err(reason) => {
                                    self.diagnostics
                                        .transcripts_rejected
                                        .fetch_add(1, Ordering::Relaxed);
                                    self.events.on_rejected(reason);
                                }
                            }
                        }
                        Err(error) => {
                            self.events.on_error(&error.to_string());
                        }
                    }
                }
                SegmentEvent::DiscardedTooShort => {
                    self.speaking = false;
                    self.segment_samples.clear();
                    self.vad.reset();
                    self.diagnostics
                        .transcripts_rejected
                        .fetch_add(1, Ordering::Relaxed);
                    self.events
                        .on_rejected(pw_domain::speech::RejectionReason::TooShortSpeech);
                }
            }
        }

        if !self.speaking {
            self.push_history(frame);
        }
    }

    fn push_history(&mut self, frame: &[f32]) {
        let capacity = self.config.pre_roll_samples();
        for sample in frame {
            if self.history.len() == capacity {
                self.history.pop_front();
            }
            self.history.push_back(*sample);
        }
    }
}

impl SpeechPipelineConfig {
    fn pre_roll_samples(&self) -> usize {
        (self.sample_rate as usize / 1000) * self.segmenter.pre_roll_ms as usize
    }
}

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let mean_square = frame
        .iter()
        .map(|s| f64::from(*s) * f64::from(*s))
        .sum::<f64>()
        / frame.len() as f64;
    #[allow(clippy::cast_possible_truncation)]
    let value = mean_square.sqrt() as f32;
    value
}

/// Drives the pipeline from a frame source until cancelled or the
/// source ends. Intended to run on a worker thread.
pub fn run_pipeline<S, V, R, E>(
    mut source: S,
    mut pipeline: SpeechPipeline<V, R, E>,
    cancel: &AtomicBool,
) where
    S: SpeechFrameSource,
    V: VoiceActivityDetector,
    R: SpeechRecognizer,
    E: SpeechEvents,
{
    let frame_len = pipeline.config.frame_len;
    let mut frame = vec![0.0_f32; frame_len];
    while !cancel.load(Ordering::Relaxed) {
        match source.read_frame(&mut frame) {
            Ok(FrameRead::Frame) => pipeline.push_frame(&frame),
            Ok(FrameRead::Pending) => {
                std::thread::sleep(Duration::from_millis(8));
            }
            Ok(FrameRead::Ended) => break,
            Err(error) => {
                pipeline.events.on_error(&error.to_string());
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use pw_domain::speech::RejectionReason;

    use super::super::ports::{PortError, SpeechEvents, SpeechRecognizer, VoiceActivityDetector};
    use super::{PipelineDiagnostics, SpeechPipeline, SpeechPipelineConfig};

    /// Fake VAD: frames whose first sample is >= 0.5 count as speech.
    struct MarkerVad;
    impl VoiceActivityDetector for MarkerVad {
        fn probability(&mut self, frame: &[f32]) -> Result<f32, PortError> {
            Ok(if frame.first().copied().unwrap_or(0.0) >= 0.5 {
                0.95
            } else {
                0.05
            })
        }
        fn reset(&mut self) {}
    }

    struct FixedRecognizer {
        text: &'static str,
        received_lens: Arc<Mutex<Vec<usize>>>,
    }
    impl SpeechRecognizer for FixedRecognizer {
        fn transcribe(&mut self, samples: &[f32]) -> Result<String, PortError> {
            self.received_lens.lock().unwrap().push(samples.len());
            Ok(self.text.to_owned())
        }
    }

    #[derive(Default)]
    struct RecordingEvents {
        transcripts: Mutex<Vec<String>>,
        rejections: Mutex<Vec<RejectionReason>>,
        started: Mutex<u32>,
        errors: Mutex<Vec<String>>,
    }
    impl SpeechEvents for Arc<RecordingEvents> {
        fn on_level(&self, _rms: f32) {}
        fn on_speech_started(&self) {
            *self.started.lock().unwrap() += 1;
        }
        fn on_transcript(&self, text: &str) {
            self.transcripts.lock().unwrap().push(text.to_owned());
        }
        fn on_rejected(&self, reason: RejectionReason) {
            self.rejections.lock().unwrap().push(reason);
        }
        fn on_error(&self, message: &str) {
            self.errors.lock().unwrap().push(message.to_owned());
        }
    }

    struct Harness {
        pipeline: SpeechPipeline<MarkerVad, FixedRecognizer, Arc<RecordingEvents>>,
        events: Arc<RecordingEvents>,
        received_lens: Arc<Mutex<Vec<usize>>>,
        capture_enabled: Arc<AtomicBool>,
        diagnostics: Arc<PipelineDiagnostics>,
        config: SpeechPipelineConfig,
    }

    fn harness(text: &'static str) -> Harness {
        let config = SpeechPipelineConfig::default();
        let events = Arc::new(RecordingEvents::default());
        let received_lens = Arc::new(Mutex::new(Vec::new()));
        let capture_enabled = Arc::new(AtomicBool::new(true));
        let diagnostics = Arc::new(PipelineDiagnostics::default());
        let pipeline = SpeechPipeline::new(
            config,
            MarkerVad,
            FixedRecognizer {
                text,
                received_lens: Arc::clone(&received_lens),
            },
            Arc::clone(&events),
            Arc::clone(&capture_enabled),
            Arc::clone(&diagnostics),
        );
        Harness {
            pipeline,
            events,
            received_lens,
            capture_enabled,
            diagnostics,
            config,
        }
    }

    fn speech_frame(len: usize) -> Vec<f32> {
        let mut frame = vec![0.6_f32; len];
        frame[0] = 0.6;
        frame
    }

    fn silence_frame(len: usize) -> Vec<f32> {
        vec![0.0_f32; len]
    }

    fn push(harness: &mut Harness, frame: &[f32], count: usize) {
        for _ in 0..count {
            harness.pipeline.push_frame(frame);
        }
    }

    #[test]
    fn ten_minutes_of_silence_emit_no_transcripts() {
        let mut h = harness("こんにちは");
        let frame = silence_frame(h.config.frame_len);
        // 10 minutes at 32ms frames = 18750 frames.
        push(&mut h, &frame, 18_750);
        assert!(h.events.transcripts.lock().unwrap().is_empty());
        assert!(h.events.rejections.lock().unwrap().is_empty());
        assert_eq!(
            h.diagnostics.frames_processed.load(Ordering::Relaxed),
            18_750
        );
    }

    #[test]
    fn a_short_sentence_is_transcribed_once_with_pre_roll() {
        let mut h = harness("こんにちは");
        let speech = speech_frame(h.config.frame_len);
        let silence = silence_frame(h.config.frame_len);
        push(&mut h, &silence, 20);
        push(&mut h, &speech, 25); // 800ms speech
        push(&mut h, &silence, 21); // > hang 640ms
        let transcripts = h.events.transcripts.lock().unwrap();
        assert_eq!(transcripts.as_slice(), ["こんにちは"]);
        assert_eq!(*h.events.started.lock().unwrap(), 1);
        // recognizer received pre-roll (224ms = 3584 samples) + 25
        // speech frames (12800) + hang frames until completion.
        let lens = h.received_lens.lock().unwrap();
        assert_eq!(lens.len(), 1);
        let pre_roll = 224 * 16; // samples
        let speech_samples = 25 * h.config.frame_len;
        assert!(lens[0] >= pre_roll + speech_samples);
    }

    #[test]
    fn rejected_text_never_reaches_transcripts() {
        let mut h = harness("（笑）");
        let speech = speech_frame(h.config.frame_len);
        let silence = silence_frame(h.config.frame_len);
        push(&mut h, &speech, 25);
        push(&mut h, &silence, 21);
        assert!(h.events.transcripts.lock().unwrap().is_empty());
        assert_eq!(
            h.events.rejections.lock().unwrap().as_slice(),
            [RejectionReason::AcousticTagOnly]
        );
        assert_eq!(
            h.diagnostics.transcripts_rejected.load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn disabling_capture_drops_the_segment_in_progress() {
        let mut h = harness("こんにちは");
        let speech = speech_frame(h.config.frame_len);
        let silence = silence_frame(h.config.frame_len);
        push(&mut h, &speech, 10);
        h.capture_enabled.store(false, Ordering::Relaxed);
        push(&mut h, &speech, 15);
        push(&mut h, &silence, 21);
        assert!(h.events.transcripts.lock().unwrap().is_empty());
    }

    #[test]
    fn capture_can_resume_after_mute() {
        let mut h = harness("こんにちは");
        let speech = speech_frame(h.config.frame_len);
        let silence = silence_frame(h.config.frame_len);
        h.capture_enabled.store(false, Ordering::Relaxed);
        push(&mut h, &speech, 25);
        h.capture_enabled.store(true, Ordering::Relaxed);
        push(&mut h, &speech, 25);
        push(&mut h, &silence, 21);
        assert_eq!(h.events.transcripts.lock().unwrap().len(), 1);
    }

    #[test]
    fn too_short_bursts_are_rejected_not_transcribed() {
        let mut h = harness("はい");
        let speech = speech_frame(h.config.frame_len);
        let silence = silence_frame(h.config.frame_len);
        push(&mut h, &speech, 3); // 96ms < min_speech 192ms
        push(&mut h, &silence, 21);
        assert!(h.events.transcripts.lock().unwrap().is_empty());
        assert_eq!(
            h.events.rejections.lock().unwrap().as_slice(),
            [RejectionReason::TooShortSpeech]
        );
    }
}
