use crate::PortError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredConversation {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    pub id: Option<i64>,
    pub conversation_id: String,
    pub turn_id: Option<u64>,
    pub role: MessageRole,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTurn {
    pub conversation_id: String,
    pub turn_id: u64,
    pub user_content: String,
    pub assistant_content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProactiveAssistantMessage {
    pub conversation_id: String,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistedProactiveAssistantMessage {
    pub turn_id: u64,
    pub message_id: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProactiveAssistantHistoryError;

impl std::fmt::Debug for ProactiveAssistantHistoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProactiveAssistantHistoryError")
    }
}

impl std::fmt::Display for ProactiveAssistantHistoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("proactive assistant history unavailable")
    }
}

impl std::error::Error for ProactiveAssistantHistoryError {}

/// Minimal persistence port for an assistant-only proactive message.
pub trait ProactiveAssistantHistory: Send {
    /// Atomically reserves a detached turn id and appends one assistant row.
    ///
    /// # Errors
    /// Returns an opaque error and writes nothing when validation or storage fails.
    fn append_proactive_assistant(
        &mut self,
        message: &ProactiveAssistantMessage,
    ) -> Result<PersistedProactiveAssistantMessage, ProactiveAssistantHistoryError>;
}

pub trait ConversationHistory: Send {
    /// Atomically stores one completed user/assistant turn. Repeating the same
    /// conversation and turn id is idempotent.
    ///
    /// # Errors
    /// Returns an adapter error and stores none of the turn on failure.
    fn store_completed_turn(&mut self, turn: &StoredTurn) -> Result<(), PortError>;

    /// Returns the largest persisted turn id for resuming allocation.
    ///
    /// # Errors
    /// Returns an adapter error when history cannot be queried.
    fn max_turn_id(&self, conversation_id: &str) -> Result<Option<u64>, PortError>;

    /// Transactionally reserves and returns a never-reused turn id.
    ///
    /// # Errors
    /// Returns an adapter error when the sequence cannot be advanced.
    fn reserve_turn_id(&mut self, conversation_id: &str, created_at: i64)
    -> Result<u64, PortError>;
    /// Creates a conversation or updates its latest activity timestamp.
    ///
    /// # Errors
    /// Returns an adapter error when persistence fails.
    fn upsert_conversation(&mut self, conversation: &StoredConversation) -> Result<(), PortError>;

    /// Appends one completed message to a conversation.
    ///
    /// # Errors
    /// Returns an adapter error when persistence fails.
    fn append_message(&mut self, message: &StoredMessage) -> Result<i64, PortError>;

    /// Lists conversations with the most recently updated first.
    ///
    /// # Errors
    /// Returns an adapter error when persistence fails.
    fn list_conversations(&self) -> Result<Vec<StoredConversation>, PortError>;

    /// Lists messages in stable chronological order.
    ///
    /// # Errors
    /// Returns an adapter error when persistence fails.
    fn list_messages(&self, conversation_id: &str) -> Result<Vec<StoredMessage>, PortError>;

    /// Deletes a conversation and its dependent history atomically.
    ///
    /// # Errors
    /// Returns an adapter error when persistence fails.
    fn delete_conversation(&mut self, conversation_id: &str) -> Result<bool, PortError>;
}
