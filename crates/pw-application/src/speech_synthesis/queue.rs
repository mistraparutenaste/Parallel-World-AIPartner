//! Serial synthesis queue with turn invalidation.

use pw_domain::reply::{TurnId, is_speakable, is_terminator};

use super::ports::{SpeechAudioSink, TtsSynthesizer};

/// How the sentences of one turn are grouped into synthesis requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthesisBatching {
    /// One request per sentence. Audio starts as soon as the first
    /// sentence completes, so this is the low-latency default for
    /// engines whose output is deterministic (`AivisSpeech`).
    PerSentence,
    /// One request per turn, capped at `max_chars` characters.
    ///
    /// Flow-matching engines sample timbre and prosody afresh for every
    /// request, so a per-sentence split makes the voice audibly change
    /// between sentences and inserts a synthesis gap at each boundary.
    /// Buffering the whole turn removes those boundaries at the cost of
    /// waiting for the reply to finish streaming (Irodori).
    ///
    /// `max_chars` only bounds one request; a single sentence longer
    /// than the cap is still sent whole because it cannot be split
    /// further.
    WholeTurn { max_chars: usize },
}

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
/// Under [`SynthesisBatching::WholeTurn`] sentences accumulate instead
/// of being synthesized on arrival, and the caller must signal the end
/// of the reply with [`Self::finish_turn`] (or drop the buffer with
/// [`Self::discard_pending`]) or nothing is spoken.
///
/// Failure rule: the first synthesis error of a turn is reported and
/// the rest of that turn is skipped (text stays visible; TTS障害 →
/// テキスト表示, 基本設計 Phase 6縮退表). A later turn tries again.
pub struct SpeechSynthesisQueue<S, K> {
    synth: S,
    sink: K,
    batching: SynthesisBatching,
    current: Option<TurnId>,
    /// Turns at or below this id are invalidated (stop / cancel).
    invalidated_up_to: Option<TurnId>,
    /// Sentences of the current turn awaiting a batched request.
    pending: String,
    seq: u32,
    failed: bool,
}

impl<S, K> SpeechSynthesisQueue<S, K>
where
    S: TtsSynthesizer,
    K: SpeechAudioSink,
{
    pub fn new(synth: S, sink: K) -> Self {
        Self::with_batching(synth, sink, SynthesisBatching::PerSentence)
    }

    pub fn with_batching(synth: S, sink: K, batching: SynthesisBatching) -> Self {
        Self {
            synth,
            sink,
            batching,
            current: None,
            invalidated_up_to: None,
            pending: String::new(),
            seq: 0,
            failed: false,
        }
    }

    /// Accepts one sentence, applying the turn and failure rules above.
    ///
    /// Synthesizes immediately under [`SynthesisBatching::PerSentence`];
    /// otherwise buffers until the character cap is reached or
    /// [`Self::finish_turn`] is called.
    pub fn push_sentence(&mut self, turn: TurnId, text: &str) {
        if !self.adopt_turn(turn) {
            self.sink.on_unspoken(turn, text);
            return;
        }
        if self.failed {
            self.sink.on_unspoken(turn, text);
            return;
        }
        match self.batching {
            SynthesisBatching::PerSentence => {
                if is_speakable(text) {
                    self.synthesize(turn, text);
                } else {
                    self.sink.on_unspoken(turn, text);
                }
            }
            // An unspeakable fragment stays in the batch rather than
            // being released early: it keeps the reply's text in order,
            // and the flush judges the batch as a whole.
            SynthesisBatching::WholeTurn { max_chars } => {
                // Keep one request under the cap, but never split a
                // sentence that already exceeds it on its own.
                if !self.pending.is_empty()
                    && self.pending.chars().count() + text.chars().count() > max_chars
                {
                    self.flush(turn);
                    if self.failed {
                        self.sink.on_unspoken(turn, text);
                        return;
                    }
                }
                self.buffer_sentence(text);
            }
        }
    }

    /// The turn whose text is buffered, if any. The worker polls this
    /// to decide when a completed reply is ready to be flushed.
    pub fn pending_turn(&self) -> Option<TurnId> {
        if self.pending.is_empty() {
            None
        } else {
            self.current
        }
    }

    /// Signals that the reply for `turn` is complete and synthesizes
    /// whatever is still buffered. No-op for a stale or invalidated
    /// turn, and for [`SynthesisBatching::PerSentence`].
    pub fn finish_turn(&mut self, turn: TurnId) {
        if !self.adopt_turn(turn) || self.failed {
            self.abandon_pending();
            return;
        }
        self.flush(turn);
    }

    /// Drops buffered text without synthesizing it (cancelled turn or
    /// worker shutdown), reporting it so the caller can still show it.
    pub fn discard_pending(&mut self) {
        self.abandon_pending();
    }

    /// Reports text the caller accepted but never handed to the queue,
    /// so a shutdown that strands queued sentences still releases them.
    pub fn release_unspoken(&self, turn: TurnId, text: &str) {
        self.sink.on_unspoken(turn, text);
    }

    /// Stops playback immediately and drops the rest of the current
    /// turn (発話割り込み).
    pub fn stop(&mut self) {
        self.abandon_pending();
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

    /// Applies the turn rules, returning false when `turn` is stale.
    /// A newer turn supersedes anything still buffered.
    fn adopt_turn(&mut self, turn: TurnId) -> bool {
        if self
            .invalidated_up_to
            .is_some_and(|invalid| turn <= invalid)
        {
            return false;
        }
        match self.current {
            Some(current) if turn < current => return false,
            Some(current) if turn == current => {}
            _ => {
                self.abandon_pending();
                self.current = Some(turn);
                self.seq = 0;
                self.failed = false;
            }
        }
        true
    }

    /// Releases buffered text that will not be synthesized.
    fn abandon_pending(&mut self) {
        let text = std::mem::take(&mut self.pending);
        if let (false, Some(turn)) = (text.is_empty(), self.current) {
            self.sink.on_unspoken(turn, &text);
        }
    }

    /// Appends a sentence, restoring the boundary the splitter consumed
    /// so the engine still pauses where the reply had a line break.
    fn buffer_sentence(&mut self, text: &str) {
        if !self.pending.is_empty() && !self.pending.ends_with(is_terminator) {
            self.pending.push('\n');
        }
        self.pending.push_str(text);
    }

    fn flush(&mut self, turn: TurnId) {
        let text = std::mem::take(&mut self.pending);
        if text.is_empty() {
            return;
        }
        if is_speakable(&text) {
            self.synthesize(turn, &text);
        } else {
            self.sink.on_unspoken(turn, &text);
        }
    }

    fn synthesize(&mut self, turn: TurnId, text: &str) {
        match self.synth.synthesize(text) {
            Ok(path) => {
                self.sink.on_audio(turn, self.seq, &path, text);
                self.seq += 1;
            }
            Err(error) => {
                self.failed = true;
                self.sink.on_error(turn, &error.to_string());
                self.sink.on_unspoken(turn, text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use pw_domain::reply::{TurnId, TurnTracker};

    use super::{SpeechSynthesisQueue, SynthesisBatching};
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
        unspoken: Mutex<Vec<(TurnId, String)>>,
        /// `on_audio` and `on_unspoken` text, interleaved in emission order.
        released: Mutex<Vec<String>>,
        stops: Mutex<u32>,
        errors: Mutex<Vec<String>>,
    }

    impl Recording {
        /// Text handed back to the caller in emission order, whether it
        /// was spoken or not. A caller that withholds display until the
        /// sink releases the text sees exactly this.
        fn released_text(&self) -> String {
            self.released.lock().unwrap().concat().replace('\n', "")
        }
    }

    struct Sink(Arc<Recording>);

    impl SpeechAudioSink for Sink {
        fn on_audio(&self, turn: TurnId, seq: u32, wav_path: &Path, text: &str) {
            self.0
                .audio
                .lock()
                .unwrap()
                .push((turn, seq, wav_path.to_owned(), text.to_owned()));
            self.0.released.lock().unwrap().push(text.to_owned());
        }
        fn on_unspoken(&self, turn: TurnId, text: &str) {
            self.0
                .unspoken
                .lock()
                .unwrap()
                .push((turn, text.to_owned()));
            self.0.released.lock().unwrap().push(text.to_owned());
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
        batched_queue(fail_on, SynthesisBatching::PerSentence)
    }

    fn batched_queue(fail_on: Option<&'static str>, batching: SynthesisBatching) -> Fixture {
        let recording = Arc::new(Recording::default());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let queue = SpeechSynthesisQueue::with_batching(
            FakeSynth {
                fail_on,
                calls: Arc::clone(&calls),
            },
            Sink(Arc::clone(&recording)),
            batching,
        );
        (queue, recording, calls)
    }

    fn whole_turn(max_chars: usize) -> SynthesisBatching {
        SynthesisBatching::WholeTurn { max_chars }
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
    fn whole_turn_batching_synthesizes_the_reply_as_one_request() {
        let (mut queue, recording, calls) = batched_queue(None, whole_turn(200));
        let turn = turns(1)[0];

        queue.push_sentence(turn, "おはよう。");
        queue.push_sentence(turn, "今日は晴れです。");
        assert!(
            calls.lock().unwrap().is_empty(),
            "batched sentences must not be synthesized before the turn ends"
        );

        queue.finish_turn(turn);

        assert_eq!(*calls.lock().unwrap(), ["おはよう。今日は晴れです。"]);
        let audio = recording.audio.lock().unwrap();
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].1, 0);
    }

    #[test]
    fn batched_sentences_without_punctuation_keep_their_line_break() {
        let (mut queue, _, calls) = batched_queue(None, whole_turn(200));
        let turn = turns(1)[0];

        queue.push_sentence(turn, "一行目");
        queue.push_sentence(turn, "二行目");
        queue.finish_turn(turn);

        assert_eq!(*calls.lock().unwrap(), ["一行目\n二行目"]);
    }

    #[test]
    fn the_character_cap_splits_a_long_reply_without_splitting_one_sentence() {
        let (mut queue, _, calls) = batched_queue(None, whole_turn(8));
        let turn = turns(1)[0];

        queue.push_sentence(turn, "あいうえ。");
        queue.push_sentence(turn, "かきくけ。");
        queue.push_sentence(turn, "とてもながいいちぶん。");
        queue.finish_turn(turn);

        assert_eq!(
            *calls.lock().unwrap(),
            ["あいうえ。", "かきくけ。", "とてもながいいちぶん。"]
        );
    }

    #[test]
    fn finishing_a_turn_twice_does_not_speak_it_twice() {
        let (mut queue, _, calls) = batched_queue(None, whole_turn(200));
        let turn = turns(1)[0];

        queue.push_sentence(turn, "こんにちは。");
        queue.finish_turn(turn);
        queue.finish_turn(turn);

        assert_eq!(*calls.lock().unwrap(), ["こんにちは。"]);
    }

    #[test]
    fn a_newer_turn_drops_the_previous_batch_before_it_is_spoken() {
        let (mut queue, _, calls) = batched_queue(None, whole_turn(200));
        let ids = turns(2);

        queue.push_sentence(ids[0], "古い一。");
        queue.push_sentence(ids[1], "新しい一。");
        queue.finish_turn(ids[1]);
        // The interrupted turn's late completion must not resurrect it.
        queue.finish_turn(ids[0]);

        assert_eq!(*calls.lock().unwrap(), ["新しい一。"]);
    }

    #[test]
    fn stopping_drops_the_batch_and_a_late_completion_stays_silent() {
        let (mut queue, recording, calls) = batched_queue(None, whole_turn(200));
        let turn = turns(1)[0];

        queue.push_sentence(turn, "話しかけ。");
        queue.stop();
        queue.finish_turn(turn);

        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(*recording.stops.lock().unwrap(), 1);
    }

    #[test]
    fn discarding_pending_text_leaves_nothing_to_speak() {
        let (mut queue, _, calls) = batched_queue(None, whole_turn(200));
        let turn = turns(1)[0];

        queue.push_sentence(turn, "取り消される。");
        queue.discard_pending();
        queue.finish_turn(turn);

        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn every_accepted_sentence_is_released_once_however_the_turn_ends() {
        type Ending = fn(&mut SpeechSynthesisQueue<FakeSynth, Sink>, TurnId);

        let sentences = ["こんにちは。", "…！？", "元気ですか？", "またね。"];
        let endings: [(&str, Ending); 4] = [
            ("finished", SpeechSynthesisQueue::finish_turn),
            ("stopped", |queue, _| queue.stop()),
            ("discarded", |queue, _| queue.discard_pending()),
            ("superseded", |queue, turn| {
                let mut tracker = TurnTracker::after(turn.value());
                queue.push_sentence(tracker.begin_turn(), "割り込み。");
            }),
        ];

        for (label, end) in endings {
            for batching in [SynthesisBatching::PerSentence, whole_turn(200)] {
                for failing in [None, Some("こんにちは。")] {
                    let (mut queue, recording, _) = batched_queue(failing, batching);
                    let turn = turns(1)[0];

                    for sentence in sentences {
                        queue.push_sentence(turn, sentence);
                    }
                    end(&mut queue, turn);
                    // Drain whatever the ending left buffered, so the
                    // comparison covers every sentence in every case.
                    queue.discard_pending();

                    let expected = if label == "superseded" {
                        format!("{}割り込み。", sentences.concat())
                    } else {
                        sentences.concat()
                    };
                    assert_eq!(
                        recording.released_text(),
                        expected,
                        "{label} / {batching:?} / failing={failing:?} lost, reordered or duplicated text"
                    );
                }
            }
        }
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
