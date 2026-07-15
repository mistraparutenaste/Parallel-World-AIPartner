//! Persistence-facing conversation history types and port.

mod ports;

pub use ports::{
    ConversationHistory, MessageRole, PersistedProactiveAssistantMessage,
    ProactiveAssistantHistory, ProactiveAssistantHistoryError, ProactiveAssistantMessage,
    StoredConversation, StoredMessage, StoredTurn,
};
