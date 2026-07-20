use super::{MemoryAction, MemoryAtom, MemoryCandidate};
use crate::PortError;
use crate::conversation::{ChatMessage, ChatRole};
use serde::{Deserialize, Serialize};

pub const DEFAULT_MEMORY_LIMIT: usize = 5;
pub const MAX_USER_SETTINGS_CHARS: usize = 2_000;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceSource {
    pub conversation_id: String,
    pub turn_id: u64,
}

impl EvidenceSource {
    #[must_use]
    pub fn new(conversation_id: impl Into<String>, turn_id: u64) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            turn_id,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaintenanceReport {
    pub dormant: usize,
    pub deleted: usize,
    pub remaining: bool,
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
        self.user_settings = self
            .user_settings
            .and_then(|value| take_trimmed_chars(&value, MAX_USER_SETTINGS_CHARS));
        self.memories.truncate(DEFAULT_MEMORY_LIMIT);
        let mut remaining = MAX_MEMORY_CHARS;
        self.memories = self
            .memories
            .into_iter()
            .filter_map(|value| take_bounded(&value, &mut remaining))
            .collect();
        self.summary = self
            .summary
            .and_then(|value| take_bounded_summary(&value, MAX_SUMMARY_CHARS));
        self
    }
}

fn take_trimmed_chars(value: &str, max: usize) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bounded = trimmed.chars().take(max).collect::<String>();
    let bounded = bounded.trim();
    (!bounded.is_empty()).then(|| bounded.to_owned())
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

fn take_bounded_summary(value: &str, max: usize) -> Option<String> {
    if is_role_preserving_summary(value) {
        merge_rolling_summaries(None, value, max).ok()
    } else {
        take_chars(value, max)
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
    /// Loads the typed projection without treating legacy rows as user-attributed facts.
    fn load_memory_atom(&self, _id: i64) -> Result<Option<MemoryAtom>, PortError> {
        Ok(None)
    }
    /// Applies semantic typed fields only if its observed revision is still current.
    ///
    /// This CAS deliberately cannot change lifecycle state or its companion
    /// columns (`pinned`, `state_changed_at`, `superseded_by`). Versioned
    /// lifecycle mutations must use the dedicated action/transition boundary.
    fn update_memory_atom_cas(
        &mut self,
        _atom: &MemoryAtom,
        _expected_revision: i64,
        _updated_at: i64,
    ) -> Result<MemoryAtom, PortError> {
        Err(PortError("typed memory updates unsupported".into()))
    }
    fn delete_memory(&mut self, id: i64) -> Result<(), PortError>;
    fn delete_summary(&mut self, conversation_id: &str) -> Result<(), PortError>;
    fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryRecord>, PortError>;
    fn find_consolidation_candidates(
        &self,
        _query: &str,
        _limit: usize,
        _now: i64,
    ) -> Result<Vec<MemoryCandidate>, PortError> {
        Ok(Vec::new())
    }
    fn apply_action(
        &mut self,
        _action: &MemoryAction,
        _source: &EvidenceSource,
        _now: i64,
    ) -> Result<Option<i64>, PortError> {
        Err(PortError("memory lifecycle mutation unsupported".into()))
    }
    fn record_recalled(
        &mut self,
        _ids: &[i64],
        _source: &EvidenceSource,
        _now: i64,
    ) -> Result<(), PortError> {
        Ok(())
    }
    fn search_active_for_prompt(
        &self,
        _query: &str,
        _limit: usize,
        _now: i64,
    ) -> Result<Vec<MemoryCandidate>, PortError> {
        Ok(Vec::new())
    }
    fn run_maintenance(
        &mut self,
        _now: i64,
        _limit: usize,
    ) -> Result<MaintenanceReport, PortError> {
        Ok(MaintenanceReport::default())
    }
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
    redact_persistent_content(content) == content
}

#[must_use]
pub fn redact_persistent_content(content: &str) -> String {
    pw_domain::runtime_health::redact_persistent_content(content)
}

#[derive(Default)]
pub struct RollingSummaryGenerator;

const SUMMARY_SCHEMA: &str = "role_summary_v1";
const SUMMARY_CONTRACT: &str = "Preserve speaker role, modal wording, uncertainty, quotation, and negation. This summary is conversational context only and is not an observation or promotion source. The current user utterance takes precedence over this summary.";
const SUMMARY_DISCOURSE_HINT: &str = "preserve_exact_modality_and_polarity";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryEntry {
    pub role: String,
    pub content: String,
    pub discourse_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredSummary {
    schema: String,
    contract: String,
    entries: Vec<SummaryEntry>,
}

impl SummaryGenerator for RollingSummaryGenerator {
    fn summarize(&mut self, messages: &[ChatMessage]) -> Result<String, PortError> {
        let entries = messages
            .iter()
            .filter_map(|message| {
                let content = message.content.trim();
                (!content.is_empty()).then(|| SummaryEntry {
                    role: summary_role(message.role).to_owned(),
                    content: if content.contains("[REDACTED]") {
                        "[REDACTED]".to_owned()
                    } else {
                        content.to_owned()
                    },
                    discourse_hint: SUMMARY_DISCOURSE_HINT.to_owned(),
                })
            })
            .collect();
        serialize_summary(entries)
    }
}

fn summary_role(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    }
}

fn serialize_summary(entries: Vec<SummaryEntry>) -> Result<String, PortError> {
    serde_json::to_string(&StructuredSummary {
        schema: SUMMARY_SCHEMA.to_owned(),
        contract: SUMMARY_CONTRACT.to_owned(),
        entries,
    })
    .map_err(|error| PortError(format!("summary serialization failed: {error}")))
}

#[must_use]
pub fn is_role_preserving_summary(summary: &str) -> bool {
    parse_role_preserving_summary(summary).is_some()
}

#[allow(clippy::missing_errors_doc)]
pub fn merge_rolling_summaries(
    existing: Option<&str>,
    delta: &str,
    max_chars: usize,
) -> Result<String, PortError> {
    let mut entries = existing.map_or_else(Vec::new, summary_entries);
    entries.extend(summary_entries(delta));
    loop {
        let serialized = serialize_summary(entries.clone())?;
        let serialized_chars = serialized.chars().count();
        if serialized_chars <= max_chars {
            return Ok(serialized);
        }
        if entries.is_empty() {
            return Err(PortError(
                "summary character limit is too small for the structured envelope".into(),
            ));
        }
        if entries.len() > 1 {
            entries.remove(0);
            continue;
        }

        let content_chars = entries[0].content.chars().count();
        if content_chars <= 1 {
            return Err(PortError(
                "summary character limit is too small to preserve an entry".into(),
            ));
        }
        let remove_chars = serialized_chars
            .saturating_sub(max_chars)
            .max(1)
            .min(content_chars - 1);
        entries[0].content = entries[0].content.chars().skip(remove_chars).collect();
    }
}

fn summary_entries(summary: &str) -> Vec<SummaryEntry> {
    parse_role_preserving_summary(summary).map_or_else(Vec::new, |document| document.entries)
}

fn parse_role_preserving_summary(summary: &str) -> Option<StructuredSummary> {
    let document = serde_json::from_str::<StructuredSummary>(summary).ok()?;
    (document.schema == SUMMARY_SCHEMA
        && document.contract == SUMMARY_CONTRACT
        && document.entries.iter().all(|entry| {
            matches!(entry.role.as_str(), "system" | "user" | "assistant")
                && entry.discourse_hint == SUMMARY_DISCOURSE_HINT
        }))
    .then_some(document)
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

    #[test]
    fn bounded_user_settings_are_trimmed_capped_and_whitespace_is_omitted() {
        let whitespace = MemoryContext {
            user_settings: Some(" \n\t ".into()),
            memories: Vec::new(),
            summary: None,
        }
        .bounded();
        assert!(whitespace.user_settings.is_none());

        let bounded = MemoryContext {
            user_settings: Some(format!(
                " \n{} {} \t",
                "a".repeat(MAX_USER_SETTINGS_CHARS - 1),
                "b".repeat(50)
            )),
            memories: Vec::new(),
            summary: None,
        }
        .bounded();
        let settings = bounded.user_settings.unwrap();
        assert!(settings.chars().count() <= MAX_USER_SETTINGS_CHARS);
        assert_eq!(settings, settings.trim());
    }

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
    fn rolling_summary_preserves_roles_modal_words_and_negation() {
        let messages = [
            ChatMessage::new(ChatRole::User, "I think A may fail"),
            ChatMessage::new(ChatRole::Assistant, "A will fail"),
            ChatMessage::new(ChatRole::User, "A did not fail"),
        ];

        let summary = RollingSummaryGenerator.summarize(&messages).unwrap();
        let document: serde_json::Value =
            serde_json::from_str(&summary).expect("role-preserving summary JSON");

        assert_eq!(document["schema"], "role_summary_v1");
        assert!(
            document["contract"]
                .as_str()
                .is_some_and(|contract| contract.contains("not an observation"))
        );
        assert_eq!(
            document["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| (
                    entry["role"].as_str().unwrap(),
                    entry["content"].as_str().unwrap()
                ))
                .collect::<Vec<_>>(),
            [
                ("user", "I think A may fail"),
                ("assistant", "A will fail"),
                ("user", "A did not fail"),
            ]
        );
    }

    #[test]
    fn legacy_flat_summary_is_not_role_preserving() {
        assert!(!is_role_preserving_summary("user / assistant"));

        let delta = RollingSummaryGenerator
            .summarize(&[ChatMessage::new(ChatRole::User, "new statement")])
            .unwrap();
        assert!(is_role_preserving_summary(&delta));
    }

    #[test]
    fn merged_summary_stays_bounded_structured_json() {
        let delta = RollingSummaryGenerator
            .summarize(&[ChatMessage::new(ChatRole::User, "new statement")])
            .unwrap();
        let merged = merge_rolling_summaries(Some("legacy / flat"), &delta, 700).unwrap();

        assert!(merged.chars().count() <= 700);
        assert!(is_role_preserving_summary(&merged));
        assert!(!merged.contains("legacy / flat"));
    }

    #[test]
    fn bounded_context_keeps_an_oversized_structured_summary_valid() {
        let summary = RollingSummaryGenerator
            .summarize(&[ChatMessage::new(
                ChatRole::Assistant,
                "long role-preserving content ".repeat(200),
            )])
            .unwrap();
        assert!(summary.chars().count() > MAX_SUMMARY_CHARS);

        let bounded = MemoryContext {
            user_settings: None,
            memories: Vec::new(),
            summary: Some(summary),
        }
        .bounded()
        .summary
        .unwrap();

        assert!(bounded.chars().count() <= MAX_SUMMARY_CHARS);
        assert!(is_role_preserving_summary(&bounded));
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
            "password is hunter2",
            "token abc123",
            "API key my-secret",
            "AbCdEf0123456789AbCdEf012345",
            "[ABCDEF234567ABCDEF234567]",
            "APIキー=a",
            "トークン: x",
            "秘密=a",
            "認証情報=x",
        ] {
            assert!(!is_safe_persistent_content(secret), "{secret}");
        }
        for ordinary in [
            "私は猫が好きです",
            "token budget",
            "password policy",
            "password management",
            "authorization policy",
            "password is required",
            "パスワードは必須です",
        ] {
            assert!(is_safe_persistent_content(ordinary), "{ordinary}");
        }
    }

    #[test]
    fn persistence_redaction_preserves_safe_text_and_removes_credentials() {
        for (input, safe) in [
            (
                "keep this Authorization: Bearer abc",
                "keep this Authorization: Bearer [REDACTED]",
            ),
            ("覚えて APIキー=秘密値", "覚えて APIキー=[REDACTED]"),
            ("hello AbCdEfGhIjKlMnOpQrStUvWx1234", "hello [REDACTED]"),
        ] {
            let redacted = redact_persistent_content(input);
            assert_eq!(redacted, safe);
            assert!(!redacted.contains("abc"));
            assert!(!redacted.contains("秘密値"));
            assert!(!redacted.contains("AbCdEf"));
        }
        for ordinary in [
            "token economy",
            "password management",
            "パスワード管理方法",
            "authorization policy",
            "password is required",
            "パスワードは必須です",
        ] {
            assert_eq!(redact_persistent_content(ordinary), ordinary);
        }
        for (input, expected) in [
            ("APIキーは abc123", "APIキーは [REDACTED]"),
            ("トークン が 'abc def'", "トークン が [REDACTED]"),
            ("パスワード：\"abc def\"", "パスワード：[REDACTED]"),
            ("秘密の値: abc", "秘密の値: [REDACTED]"),
            ("認証情報 = xyz", "認証情報 = [REDACTED]"),
            ("認証は xyz123", "認証は [REDACTED]"),
        ] {
            assert_eq!(redact_persistent_content(input), expected, "{input}");
        }
        assert_eq!(
            redact_persistent_content("APIキー管理方法"),
            "APIキー管理方法"
        );
        assert_eq!(
            redact_persistent_content("token=abc, next"),
            "token=[REDACTED], next"
        );
        assert_eq!(
            redact_persistent_content("secret:xyz; next"),
            "secret:[REDACTED]; next"
        );
        for (input, expected) in [
            (r#"token = "abc def"; next"#, "token = [REDACTED]; next"),
            (r"secret='ab\' cd'", "secret=[REDACTED]"),
            (
                "Authorization: Basic abc123, next",
                "Authorization: Basic [REDACTED], next",
            ),
            (
                "Authorization Digest xyz",
                "Authorization Digest [REDACTED]",
            ),
            ("secret value abc; next", "secret value [REDACTED]; next"),
            ("パスワードは xyz123, 次", "パスワードは [REDACTED], 次"),
        ] {
            assert_eq!(redact_persistent_content(input), expected, "{input}");
        }
    }

    #[test]
    fn persistence_redaction_scans_mixed_long_content_without_truncation() {
        let input = format!("{} token economy token=secret tail", "safe ".repeat(200));
        let output = redact_persistent_content(&input);
        assert!(output.len() > 900);
        assert!(output.contains("token economy token=[REDACTED] tail"));
        assert!(!output.contains("token=secret"));
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
