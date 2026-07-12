//! Serial synthesis queue with turn invalidation.

use pw_domain::reply::{TurnId, is_speakable};

use super::ports::{SpeechAudioSink, TtsSynthesizer};

/// Processes sentences in arrival order on the caller's thread (the
/// Tauri layer provides the worker, mirroring the conversation
/// orchestrator pattern).
///
/// Turn rules:
/// - a sentence of a newer turn adopts that turn and resets the
///   sequence number;
/// - a sentence of an older turn is stale and dropped silently;
/// - `stop()` invalidates the current turn (its remaining queued
///   sentences are dropped) and tells the sink to halt playback.
///
/// Failure rule: the first synthesis error of a turn is reported and
/// the rest of that turn is skipped (text stays visible; TTS障害 →
/// テキスト表示, 基本設計 Phase 6縮退表). A later turn tries again.
pub struct SpeechSynthesisQueue<S, K> {
    synth: S,
    sink: K,
    current: Option<TurnId>,
    /// Turns at or below this id are invalidated (stop / cancel).
    invalidated_up_to: Option<TurnId>,
    seq: u32,
    failed: bool,
}

impl<S, K> SpeechSynthesisQueue<S, K>
where
    S: TtsSynthesizer,
    K: SpeechAudioSink,
{
    pub fn new(synth: S, sink: K) -> Self {
        Self {
            synth,
            sink,
            current: None,
            invalidated_up_to: None,
            seq: 0,
            failed: false,
        }
    }

    /// Synthesizes one sentence and emits it to the sink, applying
    /// the turn and failure rules above.
    pub fn push_sentence(&mut self, turn: TurnId, text: &str) {
        if self
            .invalidated_up_to
            .is_some_and(|invalid| turn <= invalid)
        {
            return;
        }
        match self.current {
            Some(current) if turn < current => return,
            Some(current) if turn == current => {}
            _ => {
                self.current = Some(turn);
                self.seq = 0;
                self.failed = false;
            }
        }
        if self.failed || !is_speakable(text) {
            return;
        }
        match self.synth.synthesize(text) {
            Ok(path) => {
                self.sink.on_audio(turn, self.seq, &path, text);
                self.seq += 1;
            }
            Err(error) => {
                self.failed = true;
                self.sink.on_error(turn, &error.to_string());
            }
        }
    }

    /// Stops playback immediately and drops the rest of the current
    /// turn (発話割り込み).
    pub fn stop(&mut self) {
        if let Some(current) = self.current.take() {
            self.invalidated_up_to = Some(match self.invalidated_up_to {
                Some(invalid) if invalid > current => invalid,
                _ => current,
            });
        }
        self.seq = 0;
        self.failed = false;
        self.sink.on_stop();
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use pw_domain::reply::{TurnId, TurnTracker};

    use super::SpeechSynthesisQueue;
    use crate::PortError;
    use crate::speech_synthesis::{SpeechAudioSink, TtsSynthesizer};

    struct FakeSynth {
        fail_on: Option<&'static str>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl TtsSynthesizer for FakeSynth {
        fn synthesize(&self, text: &str) -> Result<PathBuf, PortError> {
            self.calls.lock().unwrap().push(text.to_owned());
            if self.fail_on == Some(text) {
                return Err(PortError("engine unreachable".into()));
            }
            Ok(PathBuf::from(format!("{text}.wav")))
        }
    }

    #[derive(Default)]
    struct Recording {
        audio: Mutex<Vec<(TurnId, u32, PathBuf, String)>>,
        stops: Mutex<u32>,
        errors: Mutex<Vec<String>>,
    }

    struct Sink(Arc<Recording>);

    impl SpeechAudioSink for Sink {
        fn on_audio(&self, turn: TurnId, seq: u32, wav_path: &Path, text: &str) {
            self.0
                .audio
                .lock()
                .unwrap()
                .push((turn, seq, wav_path.to_owned(), text.to_owned()));
        }
        fn on_stop(&self) {
            *self.0.stops.lock().unwrap() += 1;
        }
        fn on_error(&self, _turn: TurnId, message: &str) {
            self.0.errors.lock().unwrap().push(message.to_owned());
        }
    }

    type Fixture = (
        SpeechSynthesisQueue<FakeSynth, Sink>,
        Arc<Recording>,
        Arc<Mutex<Vec<String>>>,
    );

    fn queue(fail_on: Option<&'static str>) -> Fixture {
        let recording = Arc::new(Recording::default());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let queue = SpeechSynthesisQueue::new(
            FakeSynth {
                fail_on,
                calls: Arc::clone(&calls),
            },
            Sink(Arc::clone(&recording)),
        );
        (queue, recording, calls)
    }

    fn turns(count: usize) -> Vec<TurnId> {
        let mut tracker = TurnTracker::new();
        (0..count).map(|_| tracker.begin_turn()).collect()
    }

    #[test]
    fn sentences_are_emitted_in_order_with_sequence_numbers() {
        let (mut queue, recording, _) = queue(None);
        let turn = turns(1)[0];

        queue.push_sentence(turn, "一文目。");
        queue.push_sentence(turn, "二文目。");

        let audio = recording.audio.lock().unwrap();
        assert_eq!(audio.len(), 2);
        assert_eq!(audio[0].1, 0);
        assert_eq!(audio[1].1, 1);
        assert_eq!(audio[0].2, PathBuf::from("一文目。.wav"));
        assert_eq!(audio[1].3, "二文目。");
    }

    #[test]
    fn unspeakable_sentences_are_skipped_without_synthesis() {
        let (mut queue, recording, calls) = queue(None);
        let turn = turns(1)[0];

        queue.push_sentence(turn, "…！？");
        queue.push_sentence(turn, "こんにちは。");

        assert_eq!(*calls.lock().unwrap(), ["こんにちは。"]);
        let audio = recording.audio.lock().unwrap();
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].1, 0, "seq must not advance for skipped text");
    }

    #[test]
    fn a_newer_turn_resets_sequence_and_older_sentences_are_dropped() {
        let (mut queue, recording, _) = queue(None);
        let ids = turns(2);

        queue.push_sentence(ids[0], "古い一。");
        queue.push_sentence(ids[1], "新しい一。");
        // Late-arriving sentence of the old turn.
        queue.push_sentence(ids[0], "古い二。");

        let audio = recording.audio.lock().unwrap();
        let spoken: Vec<_> = audio.iter().map(|(_, _, _, text)| text.clone()).collect();
        assert_eq!(spoken, ["古い一。", "新しい一。"]);
        assert_eq!(audio[1].0, ids[1]);
        assert_eq!(audio[1].1, 0, "sequence restarts per turn");
    }

    #[test]
    fn stop_halts_playback_and_drops_the_rest_of_the_turn() {
        let (mut queue, recording, calls) = queue(None);
        let ids = turns(2);

        queue.push_sentence(ids[0], "一文目。");
        queue.stop();
        queue.push_sentence(ids[0], "二文目。");

        assert_eq!(*recording.stops.lock().unwrap(), 1);
        assert_eq!(*calls.lock().unwrap(), ["一文目。"]);

        // A new turn speaks again.
        queue.push_sentence(ids[1], "次の話。");
        assert_eq!(recording.audio.lock().unwrap().len(), 2);
    }

    #[test]
    fn synthesis_failure_reports_once_and_skips_the_rest_of_the_turn() {
        let (mut queue, recording, calls) = queue(Some("壊れる。"));
        let ids = turns(2);

        queue.push_sentence(ids[0], "壊れる。");
        queue.push_sentence(ids[0], "続き。");
        queue.push_sentence(ids[1], "再挑戦。");

        assert_eq!(recording.errors.lock().unwrap().len(), 1);
        assert_eq!(*calls.lock().unwrap(), ["壊れる。", "再挑戦。"]);
        let audio = recording.audio.lock().unwrap();
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].3, "再挑戦。");
    }
}
