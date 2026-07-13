use crate::PortError;
use crate::conversation::ChatMessage;

pub const DEFAULT_MEMORY_LIMIT: usize = 5;
pub const MAX_MEMORY_CHARS: usize = 2_000;
pub const MAX_SUMMARY_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecord {
    pub id: i64,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSummary {
    pub content: String,
    pub through_message_id: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryContext {
    pub user_settings: Option<String>,
    pub memories: Vec<String>,
    pub summary: Option<String>,
}

impl MemoryContext {
    #[must_use]
    pub fn bounded(mut self) -> Self {
        self.memories.truncate(DEFAULT_MEMORY_LIMIT);
        let mut remaining = MAX_MEMORY_CHARS;
        self.memories = self
            .memories
            .into_iter()
            .filter_map(|value| take_bounded(&value, &mut remaining))
            .collect();
        self.summary = self
            .summary
            .and_then(|value| take_chars(&value, MAX_SUMMARY_CHARS));
        self
    }
}

fn take_bounded(value: &str, remaining: &mut usize) -> Option<String> {
    if *remaining == 0 || value.trim().is_empty() {
        return None;
    }
    let taken: String = value.chars().take(*remaining).collect();
    *remaining = remaining.saturating_sub(taken.chars().count());
    Some(taken)
}
fn take_chars(value: &str, max: usize) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.chars().take(max).collect())
    }
}

#[allow(clippy::missing_errors_doc)]
pub trait MemoryStore {
    fn load_summary(&self, conversation_id: &str) -> Result<Option<StoredSummary>, PortError>;
    fn upsert_summary(
        &mut self,
        conversation_id: &str,
        content: &str,
        through_message_id: i64,
        updated_at: i64,
    ) -> Result<(), PortError>;
    fn upsert_memory(
        &mut self,
        source_conversation_id: Option<&str>,
        content: &str,
        updated_at: i64,
    ) -> Result<i64, PortError>;
    fn update_memory(&mut self, id: i64, content: &str, updated_at: i64) -> Result<(), PortError>;
    fn delete_memory(&mut self, id: i64) -> Result<(), PortError>;
    fn delete_summary(&mut self, conversation_id: &str) -> Result<(), PortError>;
    fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryRecord>, PortError>;
}

#[allow(clippy::missing_errors_doc)]
pub trait SummaryGenerator {
    fn summarize(&mut self, messages: &[ChatMessage]) -> Result<String, PortError>;
}

#[allow(clippy::missing_errors_doc)]
pub trait PersistentFactGenerator {
    fn extract(&mut self, user_message: &str) -> Result<Vec<String>, PortError>;
}

#[derive(Default)]
pub struct JapanesePersistentFactGenerator;
impl PersistentFactGenerator for JapanesePersistentFactGenerator {
    fn extract(&mut self, user_message: &str) -> Result<Vec<String>, PortError> {
        let questions = [
            "？",
            "?",
            "ですか",
            "ますか",
            "誰",
            "なぜ",
            "どうして",
            "どこ",
            "いつ",
            "何",
        ];
        if questions.iter().any(|word| user_message.contains(word)) {
            return Ok(Vec::new());
        }
        let durable = ["好き", "嫌い", "住んで", "名前は", "仕事は", "誕生日"];
        let rejected = [
            "ない",
            "ません",
            "もし",
            "なら",
            "たら",
            "と言",
            "って言",
            "？",
            "?",
        ];
        Ok(user_message
            .split(['。', '！', '？', '\n'])
            .map(str::trim)
            .filter(|sentence| {
                !sentence.is_empty()
                    && (sentence.starts_with("私は") || sentence.starts_with("私の"))
                    && durable.iter().any(|word| sentence.contains(word))
                    && !rejected.iter().any(|word| sentence.contains(word))
                    && !sentence.contains(['「', '」', '“', '”', '"'])
            })
            .map(str::to_owned)
            .collect())
    }
}

#[must_use]
pub fn is_safe_persistent_content(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    let labels = [
        "api_key",
        "apikey",
        "api key",
        "token",
        "password",
        "passwd",
        "secret",
        "authorization",
        "bearer ",
        "apiキー",
        "トークン",
        "パスワード",
        "秘密",
        "認証",
        "ベアラー",
    ];
    if labels.iter().any(|label| lower.contains(label)) {
        return false;
    }
    !content
        .split(|character: char| {
            character.is_whitespace() || matches!(character, '=' | ':' | ',' | ';')
        })
        .any(|part| {
            let len = part.chars().count();
            len >= 24
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
                && part.chars().any(|c| c.is_ascii_lowercase())
                && part
                    .chars()
                    .any(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        })
}

#[must_use]
pub fn redact_persistent_content(content: &str) -> String {
    if is_safe_persistent_content(content) {
        return content.to_owned();
    }
    let lower = content.to_ascii_lowercase();
    let labels = [
        "authorization",
        "bearer",
        "api_key",
        "apikey",
        "api key",
        "token",
        "password",
        "passwd",
        "secret",
        "apiキー",
        "トークン",
        "パスワード",
        "秘密",
        "認証",
        "ベアラー",
    ];
    let cut = labels.iter().filter_map(|label| lower.find(label)).min();
    if let Some(index) = cut {
        let prefix = content[..index].trim_end();
        return if prefix.is_empty() {
            "[REDACTED]".into()
        } else {
            format!("{prefix} [REDACTED]")
        };
    }
    content
        .split_whitespace()
        .map(|part| {
            if part.chars().count() >= 24
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
            {
                "[REDACTED]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Default)]
pub struct RollingSummaryGenerator;
impl SummaryGenerator for RollingSummaryGenerator {
    fn summarize(&mut self, messages: &[ChatMessage]) -> Result<String, PortError> {
        Ok(messages
            .iter()
            .map(|message| message.content.trim())
            .filter(|content| !content.is_empty())
            .collect::<Vec<_>>()
            .join(" / "))
    }
}

/// Explicit background-service boundary. The caller supplies only an old,
/// stable message window; the live conversation path never waits on it.
pub struct SummaryWorker<G> {
    generator: G,
    min_messages: usize,
    batch_messages: usize,
}
impl<G: SummaryGenerator> SummaryWorker<G> {
    #[must_use]
    pub const fn new(generator: G, min_messages: usize, batch_messages: usize) -> Self {
        Self {
            generator,
            min_messages,
            batch_messages,
        }
    }
    /// # Errors
    /// Returns an error when the configured summary generator fails.
    pub fn summarize_old_window(
        &mut self,
        history: &[ChatMessage],
        recent_messages: usize,
    ) -> Result<Option<String>, PortError> {
        let old_len = history.len().saturating_sub(recent_messages);
        if old_len < self.min_messages {
            return Ok(None);
        }
        let start = old_len.saturating_sub(self.batch_messages);
        self.generator.summarize(&history[start..old_len]).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::ChatRole;
    struct Generator(Vec<String>);
    impl SummaryGenerator for Generator {
        fn summarize(&mut self, messages: &[ChatMessage]) -> Result<String, PortError> {
            self.0 = messages.iter().map(|m| m.content.clone()).collect();
            Ok("summary".into())
        }
    }
    #[test]
    fn worker_only_summarizes_a_bounded_old_window() {
        let history = (0..10)
            .map(|i| ChatMessage::new(ChatRole::User, i.to_string()))
            .collect::<Vec<_>>();
        let mut worker = SummaryWorker::new(Generator(vec![]), 3, 4);
        assert_eq!(
            worker.summarize_old_window(&history, 3).unwrap(),
            Some("summary".into())
        );
        assert_eq!(worker.generator.0, ["3", "4", "5", "6"]);
    }
    #[test]
    fn fact_generator_accepts_only_explicit_first_person_affirmative_facts() {
        let mut generator = JapanesePersistentFactGenerator;
        assert_eq!(
            generator.extract("私は猫が好きです").unwrap(),
            ["私は猫が好きです"]
        );
        for rejected in [
            "猫が好きですか？",
            "私は猫が好きではない",
            "彼は猫が好き",
            "もし私は猫が好きなら",
            "私は「猫が好き」と言った",
            "私は\"猫が好き\"と引用した",
        ] {
            assert!(
                generator.extract(rejected).unwrap().is_empty(),
                "{rejected}"
            );
        }
    }
    #[test]
    fn secret_classifier_rejects_labels_and_credential_shapes() {
        for secret in [
            "API_KEY=abc",
            "password: hello",
            "Authorization: Bearer abc",
            "token=abc",
            "secret value",
            "AbCdEf0123456789AbCdEf012345",
            "APIキー=a",
            "トークン: x",
            "パスワードは短い",
            "秘密=a",
            "認証情報=x",
            "ベアラー a",
        ] {
            assert!(!is_safe_persistent_content(secret), "{secret}");
        }
        assert!(is_safe_persistent_content("私は猫が好きです"));
    }

    #[test]
    fn persistence_redaction_preserves_safe_text_and_removes_credentials() {
        for (input, safe) in [
            (
                "keep this Authorization: Bearer abc",
                "keep this [REDACTED]",
            ),
            ("覚えて APIキー=秘密値", "覚えて [REDACTED]"),
            ("hello AbCdEfGhIjKlMnOpQrStUvWx1234", "hello [REDACTED]"),
        ] {
            let redacted = redact_persistent_content(input);
            assert_eq!(redacted, safe);
            assert!(!redacted.contains("abc"));
            assert!(!redacted.contains("秘密値"));
            assert!(!redacted.contains("AbCdEf"));
        }
    }

    #[test]
    fn question_marker_or_question_word_rejects_the_whole_message_before_split() {
        let mut generator = JapanesePersistentFactGenerator;
        for question in [
            "私は猫が好きです。あなたは好きですか？",
            "私は猫が好きです。なぜ犬が好きなの",
        ] {
            assert!(generator.extract(question).unwrap().is_empty());
        }
    }
}
