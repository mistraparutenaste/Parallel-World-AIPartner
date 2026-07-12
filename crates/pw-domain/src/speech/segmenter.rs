//! VAD probability stream to speech segment state machine.

/// Tuning for [`SpeechSegmenter`]. All durations in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmenterConfig {
    /// Duration of one VAD frame (512 samples @ 16 kHz = 32 ms).
    pub frame_ms: u32,
    /// Probabilities at or above this value count as speech.
    pub speech_threshold: f32,
    /// Audio kept before the first speech frame.
    pub pre_roll_ms: u32,
    /// Segments with less accumulated speech are discarded.
    pub min_speech_ms: u32,
    /// Continuous non-speech that ends a segment.
    pub hang_ms: u32,
    /// A segment is force-completed at this length.
    pub max_segment_ms: u32,
}

/// A completed stretch of speech, in stream time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechSegment {
    /// Segment start including pre-roll.
    pub start_ms: u64,
    /// Segment end (exclusive) at the last speech frame boundary.
    pub end_ms: u64,
    /// Accumulated speech frames duration (dips excluded).
    pub speech_ms: u32,
    /// Mean VAD probability over speech frames.
    pub mean_probability: f32,
}

/// Output of one segmenter step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SegmentEvent {
    Started { at_ms: u64 },
    Completed(SpeechSegment),
    DiscardedTooShort,
}

#[derive(Debug)]
enum State {
    Idle,
    Speaking {
        /// First speech frame time (without pre-roll).
        speech_start_ms: u64,
        /// End of the last frame classified as speech.
        last_speech_end_ms: u64,
        speech_ms: u32,
        probability_sum: f64,
        speech_frames: u32,
    },
}

/// Turns a stream of per-frame VAD probabilities into speech
/// segments with pre-roll, hang time and length limits.
#[derive(Debug)]
pub struct SpeechSegmenter {
    config: SegmenterConfig,
    clock_ms: u64,
    state: State,
}

impl SpeechSegmenter {
    #[must_use]
    pub fn new(config: SegmenterConfig) -> Self {
        Self {
            config,
            clock_ms: 0,
            state: State::Idle,
        }
    }

    /// Advances the stream by one frame and returns emitted events.
    pub fn push_frame(&mut self, probability: f32) -> Vec<SegmentEvent> {
        let frame_start = self.clock_ms;
        let frame_end = frame_start + u64::from(self.config.frame_ms);
        self.clock_ms = frame_end;
        let is_speech = probability >= self.config.speech_threshold;
        let mut events = Vec::new();

        match &mut self.state {
            State::Idle => {
                if is_speech {
                    events.push(SegmentEvent::Started { at_ms: frame_start });
                    self.state = State::Speaking {
                        speech_start_ms: frame_start,
                        last_speech_end_ms: frame_end,
                        speech_ms: self.config.frame_ms,
                        probability_sum: f64::from(probability),
                        speech_frames: 1,
                    };
                }
            }
            State::Speaking {
                speech_start_ms,
                last_speech_end_ms,
                speech_ms,
                probability_sum,
                speech_frames,
            } => {
                if is_speech {
                    *last_speech_end_ms = frame_end;
                    *speech_ms += self.config.frame_ms;
                    *probability_sum += f64::from(probability);
                    *speech_frames += 1;
                }

                let segment_started =
                    speech_start_ms.saturating_sub(u64::from(self.config.pre_roll_ms));
                let reached_max =
                    frame_end - segment_started >= u64::from(self.config.max_segment_ms);
                let hang_elapsed =
                    !is_speech && frame_end - *last_speech_end_ms >= u64::from(self.config.hang_ms);

                if reached_max || hang_elapsed {
                    let end_ms = if reached_max {
                        segment_started + u64::from(self.config.max_segment_ms)
                    } else {
                        *last_speech_end_ms
                    };
                    if *speech_ms >= self.config.min_speech_ms {
                        #[allow(clippy::cast_possible_truncation)]
                        let mean_probability =
                            (*probability_sum / f64::from(*speech_frames)) as f32;
                        events.push(SegmentEvent::Completed(SpeechSegment {
                            start_ms: segment_started,
                            end_ms,
                            speech_ms: *speech_ms,
                            mean_probability,
                        }));
                    } else {
                        events.push(SegmentEvent::DiscardedTooShort);
                    }
                    self.state = State::Idle;
                }
            }
        }
        events
    }

    /// Drops any in-progress segment and returns to idle.
    pub fn reset(&mut self) {
        self.state = State::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::{SegmentEvent, SegmenterConfig, SpeechSegmenter};

    fn config() -> SegmenterConfig {
        SegmenterConfig {
            frame_ms: 32,
            speech_threshold: 0.5,
            pre_roll_ms: 96,
            min_speech_ms: 96,
            hang_ms: 128,
            max_segment_ms: 1_600,
        }
    }

    fn push_frames(
        segmenter: &mut SpeechSegmenter,
        probability: f32,
        count: usize,
    ) -> Vec<SegmentEvent> {
        let mut events = Vec::new();
        for _ in 0..count {
            events.extend(segmenter.push_frame(probability));
        }
        events
    }

    #[test]
    fn silence_never_produces_events() {
        let mut segmenter = SpeechSegmenter::new(config());
        // 10 minutes of silence at 32ms frames.
        let events = push_frames(&mut segmenter, 0.05, 600_000 / 32);
        assert!(events.is_empty());
    }

    #[test]
    fn speech_followed_by_silence_completes_one_segment_with_pre_roll() {
        let mut segmenter = SpeechSegmenter::new(config());
        // 5 frames of leading silence = 160ms.
        assert!(push_frames(&mut segmenter, 0.1, 5).is_empty());
        // 10 frames of speech = 320ms starting at 160ms.
        let started = push_frames(&mut segmenter, 0.9, 10);
        assert_eq!(started, [SegmentEvent::Started { at_ms: 160 }]);
        // 4 frames of silence = 128ms hang -> completed.
        let completed = push_frames(&mut segmenter, 0.1, 4);
        assert_eq!(completed.len(), 1);
        let SegmentEvent::Completed(segment) = &completed[0] else {
            panic!("expected Completed, got {completed:?}");
        };
        // pre-roll of 96ms is included before the first speech frame.
        assert_eq!(segment.start_ms, 160 - 96);
        assert_eq!(segment.end_ms, 160 + 10 * 32);
        assert_eq!(segment.speech_ms, 10 * 32);
        assert!((segment.mean_probability - 0.9).abs() < 1e-6);
    }

    #[test]
    fn pre_roll_is_clamped_at_stream_start() {
        let mut segmenter = SpeechSegmenter::new(config());
        let started = push_frames(&mut segmenter, 0.9, 1);
        assert_eq!(started, [SegmentEvent::Started { at_ms: 0 }]);
        push_frames(&mut segmenter, 0.9, 9);
        let completed = push_frames(&mut segmenter, 0.1, 4);
        let SegmentEvent::Completed(segment) = &completed[0] else {
            panic!("expected Completed, got {completed:?}");
        };
        assert_eq!(segment.start_ms, 0);
    }

    #[test]
    fn too_short_speech_is_discarded() {
        let mut segmenter = SpeechSegmenter::new(config());
        // 2 frames = 64ms < min_speech 96ms.
        push_frames(&mut segmenter, 0.9, 2);
        let events = push_frames(&mut segmenter, 0.1, 4);
        assert_eq!(events, [SegmentEvent::DiscardedTooShort]);
    }

    #[test]
    fn brief_dips_below_threshold_within_hang_do_not_split_the_segment() {
        let mut segmenter = SpeechSegmenter::new(config());
        push_frames(&mut segmenter, 0.9, 5);
        // 2 frames of dip = 64ms < hang 128ms.
        assert!(push_frames(&mut segmenter, 0.2, 2).is_empty());
        assert!(push_frames(&mut segmenter, 0.9, 5).is_empty());
        let completed = push_frames(&mut segmenter, 0.1, 4);
        assert_eq!(completed.len(), 1);
        let SegmentEvent::Completed(segment) = &completed[0] else {
            panic!("expected Completed, got {completed:?}");
        };
        // one continuous segment covering both bursts.
        assert_eq!(segment.end_ms, (5 + 2 + 5) * 32);
    }

    #[test]
    fn overlong_speech_is_force_completed_at_max_segment() {
        let mut segmenter = SpeechSegmenter::new(config());
        // 1600ms / 32ms = 50 frames reaches max while still speaking.
        let events = push_frames(&mut segmenter, 0.9, 60);
        let completed: Vec<_> = events
            .iter()
            .filter(|event| matches!(event, SegmentEvent::Completed(_)))
            .collect();
        assert_eq!(completed.len(), 1);
        let SegmentEvent::Completed(segment) = completed[0] else {
            unreachable!();
        };
        assert_eq!(segment.end_ms - segment.start_ms, 1_600);
    }

    #[test]
    fn reset_returns_to_idle_without_emitting() {
        let mut segmenter = SpeechSegmenter::new(config());
        push_frames(&mut segmenter, 0.9, 10);
        segmenter.reset();
        // silence afterwards produces nothing: the segment was dropped.
        assert!(push_frames(&mut segmenter, 0.1, 10).is_empty());
    }
}
