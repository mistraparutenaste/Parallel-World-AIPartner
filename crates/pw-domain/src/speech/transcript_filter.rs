//! Rejection rules for speech-to-text results.

/// Tuning for [`TranscriptFilter`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterConfig {
    /// Minimum accumulated speech duration.
    pub min_speech_ms: u32,
    /// Minimum mean VAD probability over the segment.
    pub min_mean_probability: f32,
}

/// A speech recognition result with its segment statistics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TranscriptCandidate<'a> {
    pub text: &'a str,
    pub speech_ms: u32,
    pub mean_probability: f32,
    /// False when the result arrived after capture was disabled
    /// (e.g. during TTS playback).
    pub capture_enabled: bool,
}

/// Why a transcript was rejected. Recorded in diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    EmptyText,
    TooShortSpeech,
    LowVadConfidence,
    AcousticTagOnly,
    CaptureDisabled,
}

/// Composite filter deciding whether a transcript may enter the
/// conversation.
#[derive(Debug, Clone, Copy)]
pub struct TranscriptFilter {
    config: FilterConfig,
}

impl TranscriptFilter {
    #[must_use]
    pub fn new(config: FilterConfig) -> Self {
        Self { config }
    }

    /// Accepts or rejects a candidate.
    ///
    /// # Errors
    ///
    /// Returns the first matching [`RejectionReason`].
    pub fn evaluate(&self, candidate: &TranscriptCandidate<'_>) -> Result<(), RejectionReason> {
        if !candidate.capture_enabled {
            return Err(RejectionReason::CaptureDisabled);
        }
        let trimmed = candidate.text.trim();
        if trimmed.is_empty() {
            return Err(RejectionReason::EmptyText);
        }
        if candidate.speech_ms < self.config.min_speech_ms {
            return Err(RejectionReason::TooShortSpeech);
        }
        if candidate.mean_probability < self.config.min_mean_probability {
            return Err(RejectionReason::LowVadConfidence);
        }
        if without_acoustic_tags(trimmed).is_empty() {
            return Err(RejectionReason::AcousticTagOnly);
        }
        Ok(())
    }
}

/// Removes bracketed acoustic tags (（笑）, (拍手), [音楽]) and
/// standalone music/punctuation symbols; what remains is the spoken
/// content.
fn without_acoustic_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '（' | '(' | '[' | '［' | '{' | '｛' => depth += 1,
            '）' | ')' | ']' | '］' | '}' | '｝' => depth = depth.saturating_sub(1),
            _ if depth == 0 => result.push(ch),
            _ => {}
        }
    }
    result
        .chars()
        .filter(|ch| !ch.is_whitespace() && !is_non_speech_symbol(*ch))
        .collect()
}

fn is_non_speech_symbol(ch: char) -> bool {
    matches!(
        ch,
        '♪' | '♬'
            | '♫'
            | '♩'
            | '・'
            | '。'
            | '、'
            | '.'
            | ','
            | '!'
            | '?'
            | '！'
            | '？'
            | '…'
            | 'ー'
            | '〜'
            | '~'
            | '-'
    )
}

#[cfg(test)]
mod tests {
    use super::{FilterConfig, RejectionReason, TranscriptCandidate, TranscriptFilter};

    fn filter() -> TranscriptFilter {
        TranscriptFilter::new(FilterConfig {
            min_speech_ms: 200,
            min_mean_probability: 0.6,
        })
    }

    fn candidate(text: &str) -> TranscriptCandidate<'_> {
        TranscriptCandidate {
            text,
            speech_ms: 800,
            mean_probability: 0.9,
            capture_enabled: true,
        }
    }

    #[test]
    fn accepts_a_normal_short_sentence() {
        assert_eq!(
            filter().evaluate(&candidate("こんにちは、元気ですか")),
            Ok(())
        );
    }

    #[test]
    fn rejects_empty_or_whitespace_text() {
        assert_eq!(
            filter().evaluate(&candidate("  \u{3000} ")),
            Err(RejectionReason::EmptyText)
        );
    }

    #[test]
    fn rejects_too_short_speech() {
        let mut c = candidate("はい");
        c.speech_ms = 120;
        assert_eq!(filter().evaluate(&c), Err(RejectionReason::TooShortSpeech));
    }

    #[test]
    fn rejects_low_vad_confidence() {
        let mut c = candidate("こんにちは");
        c.mean_probability = 0.4;
        assert_eq!(
            filter().evaluate(&c),
            Err(RejectionReason::LowVadConfidence)
        );
    }

    #[test]
    fn rejects_acoustic_tags_only() {
        for text in ["（笑）", "(拍手)", "[音楽]", "♪♪", "（笑）(拍手)"] {
            assert_eq!(
                filter().evaluate(&candidate(text)),
                Err(RejectionReason::AcousticTagOnly),
                "text: {text}"
            );
        }
    }

    #[test]
    fn accepts_text_that_contains_words_besides_tags() {
        assert_eq!(
            filter().evaluate(&candidate("（笑）それで本題ですが")),
            Ok(())
        );
    }

    #[test]
    fn rejects_results_produced_while_capture_is_disabled() {
        let mut c = candidate("テレビの音声です");
        c.capture_enabled = false;
        assert_eq!(filter().evaluate(&c), Err(RejectionReason::CaptureDisabled));
    }
}
