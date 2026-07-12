//! Persistence-facing conversation history types and port.

mod ports;

pub use ports::{ConversationHistory, MessageRole, StoredConversation, StoredMessage, StoredTurn};
