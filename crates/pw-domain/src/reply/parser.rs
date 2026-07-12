//! Splits an assistant reply stream into an optional control prelude
//! and spoken text.

use serde::Deserialize;

/// Control prelude of a reply: never synthesized, only mapped to
/// character expression / motion.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReplyControl {
    pub emotion: Option<String>,
    pub intensity: Option<f32>,
    pub motion: Option<String>,
}

/// Incremental output of [`ReplyParser`].
#[derive(Debug, Clone, PartialEq)]
pub enum ReplyEvent {
    Control(ReplyControl),
    Speech(String),
}

enum State {
    /// Buffering the first line to decide control vs speech.
    Prelude,
    /// The prelude was resolved; everything else is speech.
    Speech,
}

/// Streams an assistant reply, separating the optional first-line
/// control JSON from spoken text.
///
/// The first line is treated as control only when it parses as a
/// JSON object; otherwise it is replayed verbatim as speech.
pub struct ReplyParser {
    state: State,
    buffer: String,
}

impl Default for ReplyParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplyParser {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: State::Prelude,
            buffer: String::new(),
        }
    }

    /// Feeds a chunk and returns any resolved events.
    pub fn push(&mut self, chunk: &str) -> Vec<ReplyEvent> {
        match self.state {
            State::Speech => {
                if chunk.is_empty() {
                    Vec::new()
                } else {
                    vec![ReplyEvent::Speech(chunk.to_owned())]
                }
            }
            State::Prelude => {
                self.buffer.push_str(chunk);
                // Only a first line starting with `{` can be a control
                // prelude; anything else streams as speech right away.
                let trimmed = self.buffer.trim_start();
                if !trimmed.is_empty() && !trimmed.starts_with('{') {
                    self.state = State::Speech;
                    let original = std::mem::take(&mut self.buffer);
                    return vec![ReplyEvent::Speech(original)];
                }
                let Some(newline) = self.buffer.find('\n') else {
                    return Vec::new();
                };
                let first_line = self.buffer[..newline].trim();
                self.state = State::Speech;

                let control = if first_line.starts_with('{') {
                    serde_json::from_str::<ReplyControl>(first_line).ok()
                } else {
                    None
                };

                let mut events = Vec::new();
                if let Some(control) = control {
                    events.push(ReplyEvent::Control(control));
                    // Skip the conventional blank line after the prelude.
                    let rest = &self.buffer[newline + 1..];
                    let speech = rest.strip_prefix('\n').unwrap_or(rest);
                    if !speech.is_empty() {
                        events.push(ReplyEvent::Speech(speech.to_owned()));
                    }
                } else {
                    // Not a control prelude: replay the buffered text
                    // verbatim as speech.
                    let original = std::mem::take(&mut self.buffer);
                    events.push(ReplyEvent::Speech(original));
                }
                self.buffer.clear();
                events
            }
        }
    }

    /// Flushes a prelude that never saw a newline (short replies).
    pub fn finish(&mut self) -> Vec<ReplyEvent> {
        if matches!(self.state, State::Speech) || self.buffer.is_empty() {
            return Vec::new();
        }
        self.state = State::Speech;
        let text = std::mem::take(&mut self.buffer);
        vec![ReplyEvent::Speech(text)]
    }
}

#[cfg(test)]
mod tests {
    use super::{ReplyControl, ReplyEvent, ReplyParser};

    fn feed(parser: &mut ReplyParser, chunks: &[&str]) -> (Option<ReplyControl>, String) {
        let mut control = None;
        let mut speech = String::new();
        for chunk in chunks {
            for event in parser.push(chunk) {
                match event {
                    ReplyEvent::Control(parsed) => control = Some(parsed),
                    ReplyEvent::Speech(text) => speech.push_str(&text),
                }
            }
        }
        for event in parser.finish() {
            match event {
                ReplyEvent::Control(parsed) => control = Some(parsed),
                ReplyEvent::Speech(text) => speech.push_str(&text),
            }
        }
        (control, speech)
    }

    #[test]
    fn extracts_the_control_prelude_and_speech() {
        let mut parser = ReplyParser::new();
        let (control, speech) = feed(
            &mut parser,
            &["{\"emotion\":\"happy\",\"intensity\":0.7,\"motion\":\"nod\"}\n\nおかえりなさい。"],
        );
        let control = control.expect("control prelude");
        assert_eq!(control.emotion.as_deref(), Some("happy"));
        assert!((control.intensity.unwrap() - 0.7).abs() < 1e-6);
        assert_eq!(control.motion.as_deref(), Some("nod"));
        assert_eq!(speech, "おかえりなさい。");
    }

    #[test]
    fn handles_control_json_split_across_chunks() {
        let mut parser = ReplyParser::new();
        let (control, speech) = feed(
            &mut parser,
            &["{\"emo", "tion\":\"sad\"}", "\n", "ごめんなさい。"],
        );
        assert_eq!(control.unwrap().emotion.as_deref(), Some("sad"));
        assert_eq!(speech, "ごめんなさい。");
    }

    #[test]
    fn treats_a_non_json_first_line_as_speech() {
        let mut parser = ReplyParser::new();
        let (control, speech) = feed(&mut parser, &["こんにちは。\n元気ですか。"]);
        assert!(control.is_none());
        assert_eq!(speech, "こんにちは。\n元気ですか。");
    }

    #[test]
    fn treats_invalid_json_braces_as_speech() {
        let mut parser = ReplyParser::new();
        let (control, speech) = feed(&mut parser, &["{壊れたJSON}\n本文です。"]);
        assert!(control.is_none());
        assert_eq!(speech, "{壊れたJSON}\n本文です。");
    }

    #[test]
    fn a_reply_without_newline_is_all_speech_at_finish() {
        let mut parser = ReplyParser::new();
        let (control, speech) = feed(&mut parser, &["短い返事"]);
        assert!(control.is_none());
        assert_eq!(speech, "短い返事");
    }

    #[test]
    fn control_only_replies_produce_no_speech() {
        let mut parser = ReplyParser::new();
        let (control, speech) = feed(&mut parser, &["{\"motion\":\"wave\"}\n"]);
        assert_eq!(control.unwrap().motion.as_deref(), Some("wave"));
        assert_eq!(speech, "");
    }

    #[test]
    fn ignores_unknown_control_fields() {
        let mut parser = ReplyParser::new();
        let (control, _) = feed(
            &mut parser,
            &["{\"emotion\":\"calm\",\"unknown_field\":123}\nはい。"],
        );
        assert_eq!(control.unwrap().emotion.as_deref(), Some("calm"));
    }
}
