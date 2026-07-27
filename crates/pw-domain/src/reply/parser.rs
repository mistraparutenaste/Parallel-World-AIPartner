//! Splits an assistant reply stream into an optional control prelude
//! and spoken text.

use serde::Deserialize;

/// Control prelude of a reply: never synthesized, only mapped to
/// character expression / motion.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
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

/// Field names accepted in a control prelude.
const CONTROL_KEYS: [&str; 3] = ["emotion", "intensity", "motion"];

/// Upper bound on the text buffered while deciding control vs speech.
/// Small local models pad the prelude with markdown, so the decision
/// needs lookahead; the bound keeps a rambling first line streaming.
const MAX_PRELUDE_BYTES: usize = 512;

enum State {
    /// Buffering the first line to decide control vs speech.
    Prelude,
    /// A control prelude was accepted; the blank line and any closing
    /// fence that follow it are still being skipped.
    AfterControl,
    /// The prelude was resolved; everything else is speech.
    Speech,
}

/// Streams an assistant reply, separating the optional leading
/// control fields from spoken text.
///
/// The prelude is recognized leniently: small models emit it wrapped
/// in markdown (`* …`, ```` ```json ````), with single quotes, or with
/// the braces missing entirely. Anything that is not a control field
/// is replayed verbatim as speech, so ordinary replies are untouched.
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
            State::AfterControl => {
                let speech = speech_after_control(chunk);
                if speech.is_empty() {
                    Vec::new()
                } else {
                    self.state = State::Speech;
                    vec![ReplyEvent::Speech(speech.to_owned())]
                }
            }
            State::Prelude => {
                self.buffer.push_str(chunk);
                // Past the lookahead bound the prelude can no longer
                // be completed, so decide with what has arrived.
                let final_chunk = self.buffer.len() >= MAX_PRELUDE_BYTES;
                self.resolve(final_chunk)
            }
        }
    }

    /// Resolves a prelude that never saw its terminator (short replies).
    pub fn finish(&mut self) -> Vec<ReplyEvent> {
        if !matches!(self.state, State::Prelude) || self.buffer.is_empty() {
            return Vec::new();
        }
        self.resolve(true)
    }

    fn resolve(&mut self, final_chunk: bool) -> Vec<ReplyEvent> {
        match classify_prelude(&self.buffer, final_chunk) {
            Prelude::Pending => Vec::new(),
            Prelude::Speech => {
                self.state = State::Speech;
                let original = std::mem::take(&mut self.buffer);
                if original.is_empty() {
                    Vec::new()
                } else {
                    vec![ReplyEvent::Speech(original)]
                }
            }
            Prelude::Control(control, speech_start) => {
                let mut events = vec![ReplyEvent::Control(control)];
                let speech = speech_after_control(&self.buffer[speech_start..]).to_owned();
                self.buffer.clear();
                if speech.is_empty() {
                    self.state = State::AfterControl;
                } else {
                    self.state = State::Speech;
                    events.push(ReplyEvent::Speech(speech));
                }
                events
            }
        }
    }
}

/// Outcome of inspecting the buffered prelude.
enum Prelude {
    /// More text is needed before the prelude can be classified.
    Pending,
    /// Control fields, plus the byte offset where speech resumes.
    Control(ReplyControl, usize),
    /// The buffer is ordinary speech and must be replayed verbatim.
    Speech,
}

/// One step of lenient token scanning.
enum Token<T> {
    /// The token may still be growing at the end of the buffer.
    Need,
    /// The text cannot be part of a control prelude.
    Invalid,
    /// A token, plus the byte offset just past it.
    Found(T, usize),
}

fn classify_prelude(buffer: &str, final_chunk: bool) -> Prelude {
    let Some(mut cursor) = skip_decoration(buffer, final_chunk) else {
        return if final_chunk {
            Prelude::Speech
        } else {
            Prelude::Pending
        };
    };
    let braced = buffer[cursor..].starts_with('{');
    if braced {
        cursor += 1;
    }
    scan_control_fields(buffer, cursor, braced, final_chunk)
}

/// Skips whitespace and the markdown decoration small models put in
/// front of the control JSON, returning where the fields may start.
/// `None` means the decoration itself is still incomplete.
///
/// Skipping is for classification only: when what follows is not a
/// control field the caller replays the original bytes as speech.
fn skip_decoration(buffer: &str, final_chunk: bool) -> Option<usize> {
    let mut cursor = 0;
    loop {
        let trimmed = buffer[cursor..].trim_start();
        cursor = buffer.len() - trimmed.len();
        if trimmed.is_empty() {
            return if final_chunk { Some(cursor) } else { None };
        }
        if trimmed.starts_with("```") {
            // A fence opener (```json) occupies the rest of its line.
            let newline = trimmed.find('\n')?;
            cursor += newline + 1;
            continue;
        }
        let first = trimmed.chars().next().expect("checked non-empty");
        if matches!(first, '*' | '-' | '>' | '#' | '`' | '・' | '•') {
            cursor += first.len_utf8();
            continue;
        }
        return Some(cursor);
    }
}

/// Reads `key: value` pairs until something that is not a control
/// field appears; that position is where speech begins.
fn scan_control_fields(buffer: &str, start: usize, braced: bool, final_chunk: bool) -> Prelude {
    let mut control = ReplyControl::default();
    let mut found = false;
    let mut cursor = start;
    loop {
        cursor += leading_len(&buffer[cursor..], |ch| ch.is_whitespace() || ch == ',');
        let rest = &buffer[cursor..];
        if rest.is_empty() {
            return exhausted(control, found, cursor, final_chunk);
        }
        if rest.starts_with('}') {
            return if found {
                Prelude::Control(control, cursor + 1)
            } else {
                Prelude::Speech
            };
        }
        let (key, after_key) = match read_key(rest) {
            Token::Found(key, offset) => (key, cursor + offset),
            // Anything else ends the prelude: text after a field is
            // speech, text instead of a field means there was none.
            Token::Invalid => {
                return unexpected(buffer, control, found, cursor, braced, final_chunk);
            }
            Token::Need => return exhausted(control, found, cursor, final_chunk),
        };
        match read_value(&buffer[after_key..]) {
            Token::Found(value, offset) => {
                apply_field(&mut control, key, &value);
                found = true;
                cursor = after_key + offset;
            }
            Token::Invalid => {
                return unexpected(buffer, control, found, cursor, braced, final_chunk);
            }
            Token::Need => return exhausted(control, found, cursor, final_chunk),
        }
    }
}

/// Resolves a scan that ran out of input: only a completed stream can
/// accept a prelude that never saw its terminator.
fn exhausted(control: ReplyControl, found: bool, cursor: usize, final_chunk: bool) -> Prelude {
    if !final_chunk {
        Prelude::Pending
    } else if found {
        Prelude::Control(control, cursor)
    } else {
        Prelude::Speech
    }
}

/// Resolves text that is not a control field. Inside braces the
/// prelude still runs to the closing brace, so unknown fields are
/// dropped rather than spoken; without braces the text is speech.
fn unexpected(
    buffer: &str,
    control: ReplyControl,
    found: bool,
    cursor: usize,
    braced: bool,
    final_chunk: bool,
) -> Prelude {
    if !found {
        return Prelude::Speech;
    }
    if !braced {
        return Prelude::Control(control, cursor);
    }
    match buffer[cursor..].find('}') {
        Some(offset) => Prelude::Control(control, cursor + offset + 1),
        None if final_chunk => Prelude::Control(control, buffer.len()),
        None => Prelude::Pending,
    }
}

/// Reads an optionally quoted control field name and its colon.
fn read_key(rest: &str) -> Token<&'static str> {
    let quote = rest.chars().next().filter(|ch| *ch == '"' || *ch == '\'');
    let name_start = quote.map_or(0, char::len_utf8);
    let name_len = leading_len(&rest[name_start..], |ch| {
        ch.is_ascii_alphabetic() || ch == '_'
    });
    let mut cursor = name_start + name_len;
    let name = &rest[name_start..cursor];
    if cursor == rest.len() {
        // The name may still be growing.
        return Token::Need;
    }
    let Some(key) = CONTROL_KEYS
        .iter()
        .copied()
        .find(|known| known.eq_ignore_ascii_case(name))
    else {
        return Token::Invalid;
    };
    if let Some(quote) = quote {
        if !rest[cursor..].starts_with(quote) {
            return Token::Invalid;
        }
        cursor += quote.len_utf8();
    }
    cursor += leading_len(&rest[cursor..], char::is_whitespace);
    if cursor == rest.len() {
        return Token::Need;
    }
    if !rest[cursor..].starts_with(':') {
        return Token::Invalid;
    }
    Token::Found(key, cursor + 1)
}

/// Reads a quoted string or a bare token (number, `null`).
fn read_value(rest: &str) -> Token<String> {
    let skipped = leading_len(rest, char::is_whitespace);
    let rest = &rest[skipped..];
    let Some(first) = rest.chars().next() else {
        return Token::Need;
    };
    if first == '"' || first == '\'' {
        let body_start = first.len_utf8();
        let mut value = String::new();
        let mut escaped = false;
        for (index, ch) in rest[body_start..].char_indices() {
            if escaped {
                value.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == first {
                let end = body_start + index + ch.len_utf8();
                return Token::Found(value, skipped + end);
            } else if ch == '\n' {
                // A quoted value never spans lines here; treat the
                // unterminated quote as ordinary text.
                return Token::Invalid;
            } else {
                value.push(ch);
            }
        }
        return Token::Need;
    }
    let len = leading_len(rest, |ch| {
        !(ch == ',' || ch == '}' || ch == '\n' || ch.is_whitespace())
    });
    if len == 0 {
        return Token::Invalid;
    }
    if len == rest.len() {
        // The token may still be growing.
        return Token::Need;
    }
    Token::Found(rest[..len].to_owned(), skipped + len)
}

fn apply_field(control: &mut ReplyControl, key: &str, value: &str) {
    let value = value.trim();
    let named =
        (!value.is_empty() && !value.eq_ignore_ascii_case("null")).then(|| value.to_owned());
    match key {
        "emotion" => control.emotion = named,
        "motion" => control.motion = named,
        "intensity" => control.intensity = value.parse().ok(),
        _ => {}
    }
}

/// Drops the blank line and any closing code fence between the
/// prelude and the spoken text.
fn speech_after_control(rest: &str) -> &str {
    let rest = rest.trim_start();
    rest.strip_prefix("```").map_or(rest, str::trim_start)
}

fn leading_len(text: &str, accept: impl Fn(char) -> bool) -> usize {
    text.char_indices()
        .find(|(_, ch)| !accept(*ch))
        .map_or(text.len(), |(index, _)| index)
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

    fn parse(chunks: &[&str]) -> (Option<ReplyControl>, String) {
        feed(&mut ReplyParser::new(), chunks)
    }

    /// Worst case for the incremental decision: one character per delta.
    fn parse_by_char(text: &str) -> (Option<ReplyControl>, String) {
        let chunks: Vec<String> = text.chars().map(String::from).collect();
        let borrowed: Vec<&str> = chunks.iter().map(String::as_str).collect();
        parse(&borrowed)
    }

    #[test]
    fn extracts_the_control_prelude_and_speech() {
        let (control, speech) = parse(&[
            "{\"emotion\":\"happy\",\"intensity\":0.7,\"motion\":\"nod\"}\n\nおかえりなさい。",
        ]);
        let control = control.expect("control prelude");
        assert_eq!(control.emotion.as_deref(), Some("happy"));
        assert!((control.intensity.unwrap() - 0.7).abs() < 1e-6);
        assert_eq!(control.motion.as_deref(), Some("nod"));
        assert_eq!(speech, "おかえりなさい。");
    }

    #[test]
    fn handles_control_json_split_across_chunks() {
        let (control, speech) = parse(&["{\"emo", "tion\":\"sad\"}", "\n", "ごめんなさい。"]);
        assert_eq!(control.unwrap().emotion.as_deref(), Some("sad"));
        assert_eq!(speech, "ごめんなさい。");
    }

    #[test]
    fn treats_a_non_json_first_line_as_speech() {
        let (control, speech) = parse(&["こんにちは。\n元気ですか。"]);
        assert!(control.is_none());
        assert_eq!(speech, "こんにちは。\n元気ですか。");
    }

    #[test]
    fn treats_invalid_json_braces_as_speech() {
        let (control, speech) = parse(&["{壊れたJSON}\n本文です。"]);
        assert!(control.is_none());
        assert_eq!(speech, "{壊れたJSON}\n本文です。");
    }

    #[test]
    fn a_reply_without_newline_is_all_speech_at_finish() {
        let (control, speech) = parse(&["短い返事"]);
        assert!(control.is_none());
        assert_eq!(speech, "短い返事");
    }

    #[test]
    fn control_only_replies_produce_no_speech() {
        let (control, speech) = parse(&["{\"motion\":\"wave\"}\n"]);
        assert_eq!(control.unwrap().motion.as_deref(), Some("wave"));
        assert_eq!(speech, "");
    }

    #[test]
    fn control_json_without_a_trailing_newline_is_not_spoken() {
        let (control, speech) = parse(&["{\"emotion\":\"happy\"}"]);
        assert_eq!(control.unwrap().emotion.as_deref(), Some("happy"));
        assert_eq!(speech, "");
    }

    #[test]
    fn ignores_unknown_control_fields() {
        let (control, speech) = parse(&["{\"emotion\":\"calm\",\"unknown_field\":123}\nはい。"]);
        assert_eq!(control.unwrap().emotion.as_deref(), Some("calm"));
        // The unknown field is dropped with the rest of the object.
        assert_eq!(speech, "はい。");
    }

    #[test]
    fn strips_a_bulleted_brace_less_prelude_from_the_same_line() {
        // Verbatim shape produced by a small local model.
        let (control, speech) = parse(&[
            "* \"emotion\": \"喜び\", \"intensity\": 1.0, \"motion\": \"手を合わせる\" \
             わたしは本当に嬉しいです。",
        ]);
        let control = control.expect("control prelude");
        assert_eq!(control.emotion.as_deref(), Some("喜び"));
        assert!((control.intensity.unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(control.motion.as_deref(), Some("手を合わせる"));
        assert_eq!(speech, "わたしは本当に嬉しいです。");
    }

    #[test]
    fn strips_a_fenced_prelude() {
        let (control, speech) = parse(&["```json\n{\"emotion\":\"happy\"}\n```\n本文です。"]);
        assert_eq!(control.unwrap().emotion.as_deref(), Some("happy"));
        assert_eq!(speech, "本文です。");
    }

    #[test]
    fn accepts_single_quotes_and_unquoted_keys() {
        let (control, speech) = parse(&["emotion: 'sad', intensity: 0.5\n泣きそうです。"]);
        let control = control.expect("control prelude");
        assert_eq!(control.emotion.as_deref(), Some("sad"));
        assert!((control.intensity.unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(speech, "泣きそうです。");
    }

    #[test]
    fn null_control_values_stay_unset() {
        let (control, speech) = parse(&["{\"emotion\":\"happy\",\"motion\":null}\nはい。"]);
        let control = control.expect("control prelude");
        assert_eq!(control.emotion.as_deref(), Some("happy"));
        assert!(control.motion.is_none());
        assert_eq!(speech, "はい。");
    }

    #[test]
    fn roleplay_emphasis_is_speech_not_a_prelude() {
        let (control, speech) = parse(&["*にっこり笑う* こんにちは。"]);
        assert!(control.is_none());
        assert_eq!(speech, "*にっこり笑う* こんにちは。");
    }

    #[test]
    fn a_long_first_line_without_a_prelude_still_streams() {
        let long = "あ".repeat(400);
        let (control, speech) = parse(&[&long]);
        assert!(control.is_none());
        assert_eq!(speech, long);
    }

    #[test]
    fn a_malformed_prelude_streamed_one_character_at_a_time_is_not_spoken() {
        let (control, speech) = parse_by_char(
            "* \"emotion\": \"喜び\", \"intensity\": 1.0, \"motion\": \"手を合わせる\" \
             わたしは本当に嬉しいです。",
        );
        assert_eq!(control.unwrap().emotion.as_deref(), Some("喜び"));
        assert_eq!(speech, "わたしは本当に嬉しいです。");
    }

    #[test]
    fn ordinary_speech_streamed_one_character_at_a_time_is_intact() {
        let (control, speech) = parse_by_char("こんにちは。今日はいい天気ですね。");
        assert!(control.is_none());
        assert_eq!(speech, "こんにちは。今日はいい天気ですね。");
    }

    #[test]
    fn brace_less_prelude_split_across_chunks_is_not_spoken() {
        let (control, speech) = parse(&["\"emo", "tion\": \"喜", "び\"\n", "うれしいです。"]);
        assert_eq!(control.unwrap().emotion.as_deref(), Some("喜び"));
        assert_eq!(speech, "うれしいです。");
    }
}
