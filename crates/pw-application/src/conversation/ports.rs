//! Ports implemented by LLM adapters and the UI event sink.

use std::sync::atomic::AtomicBool;

use pw_domain::conversation::ConversationState;
use pw_domain::reply::{ReplyControl, TurnId};

use crate::PortError;

/// Role of one chat message in the OpenAI-compatible contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// One prompt message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    #[must_use]
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

/// Streaming chat completion client.
pub trait LlmClient: Send {
    /// Streams a completion, invoking `on_delta` per content chunk.
    /// Implementations must poll `cancel` between chunks and return
    /// early (Ok) once it is set.
    ///
    /// # Errors
    ///
    /// Returns [`PortError`] on transport or protocol failure.
    fn stream_chat(
        &mut self,
        messages: &[ChatMessage],
        cancel: &AtomicBool,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<(), PortError>;
}

/// Sink for conversation lifecycle events. Every payload carries the
/// turn id so stale output can be dropped by consumers as well.
pub trait ConversationEvents: Send {
    fn on_state(&self, state: ConversationState);
    fn on_user_message(&self, turn: TurnId, text: &str);
    fn on_control(&self, turn: TurnId, control: &ReplyControl);
    fn on_sentence(&self, turn: TurnId, sentence: &str);
    fn on_reply_complete(&self, turn: TurnId, speech_text: &str);
    fn on_cancelled(&self, turn: TurnId);
    fn on_error(&self, turn: TurnId, message: &str);
}
