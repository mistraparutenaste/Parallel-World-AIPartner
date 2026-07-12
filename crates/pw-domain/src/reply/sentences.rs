//! Incremental sentence segmentation for streamed replies.

/// Characters that end a sentence (newline handled separately).
fn is_terminator(ch: char) -> bool {
    matches!(ch, '。' | '！' | '？' | '!' | '?' | '…')
}

/// Splits streamed text into sentences as soon as they complete, so
/// downstream consumers (TTS queue, chat display) can start early.
/// Consecutive terminators (！？) stay attached to their sentence.
#[derive(Debug, Default)]
pub struct SentenceSplitter {
    pending: String,
    /// The pending sentence ended with a terminator; flush before
    /// the next regular character.
    ready: bool,
}

impl SentenceSplitter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a chunk and returns completed sentences.
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        let mut sentences = Vec::new();
        for ch in chunk.chars() {
            if ch == '\n' {
                self.flush_into(&mut sentences);
            } else if is_terminator(ch) {
                self.pending.push(ch);
                if has_content(&self.pending) {
                    self.ready = true;
                }
            } else {
                if self.ready {
                    self.flush_into(&mut sentences);
                }
                if !self.pending.is_empty() || !ch.is_whitespace() {
                    self.pending.push(ch);
                }
            }
        }
        sentences
    }

    /// Flushes any trailing partial sentence.
    pub fn finish(&mut self) -> Vec<String> {
        let mut sentences = Vec::new();
        self.flush_into(&mut sentences);
        sentences
    }

    fn flush_into(&mut self, sentences: &mut Vec<String>) {
        self.ready = false;
        let pending = std::mem::take(&mut self.pending);
        if has_content(&pending) {
            sentences.push(pending.trim().to_owned());
        }
    }
}

/// True when the buffer contains something besides whitespace and
/// terminator punctuation.
fn has_content(pending: &str) -> bool {
    pending
        .chars()
        .any(|ch| !ch.is_whitespace() && !is_terminator(ch))
}

#[cfg(test)]
mod tests {
    use super::SentenceSplitter;

    fn collect(chunks: &[&str]) -> Vec<String> {
        let mut splitter = SentenceSplitter::new();
        let mut sentences = Vec::new();
        for chunk in chunks {
            sentences.extend(splitter.push(chunk));
        }
        sentences.extend(splitter.finish());
        sentences
    }

    #[test]
    fn splits_japanese_sentences_at_terminal_punctuation() {
        assert_eq!(
            collect(&["おはよう。今日は晴れです。散歩しますか？"]),
            ["おはよう。", "今日は晴れです。", "散歩しますか？"]
        );
    }

    #[test]
    fn emits_sentences_as_soon_as_they_complete_across_chunks() {
        let mut splitter = SentenceSplitter::new();
        assert!(splitter.push("今日はいい天").is_empty());
        assert_eq!(
            splitter.push("気ですね。明日は"),
            ["今日はいい天気ですね。"]
        );
        assert_eq!(splitter.finish(), ["明日は"]);
    }

    #[test]
    fn newlines_terminate_sentences() {
        assert_eq!(collect(&["一行目\n二行目"]), ["一行目", "二行目"]);
    }

    #[test]
    fn exclamation_and_ellipsis_terminate() {
        assert_eq!(
            collect(&["すごい！それで…どうなったの?"]),
            ["すごい！", "それで…", "どうなったの?"]
        );
    }

    #[test]
    fn consecutive_terminators_stay_on_one_sentence() {
        assert_eq!(
            collect(&["ほんと！？次いくよ。"]),
            ["ほんと！？", "次いくよ。"]
        );
    }

    #[test]
    fn whitespace_only_fragments_are_dropped() {
        assert_eq!(collect(&["  \n 。\nこんにちは。"]), ["こんにちは。"]);
    }
}
