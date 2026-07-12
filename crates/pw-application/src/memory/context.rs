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
}
