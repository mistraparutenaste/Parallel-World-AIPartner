//! Conversation state machine states.

use serde::{Deserialize, Serialize};

/// Observable state of a single conversation loop.
///
/// `*Unavailable` states mean the named subsystem failed and the
/// current operation cannot continue until recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationState {
    Starting,
    Idle,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
    Muted,
    Interrupting,
    Cancelled,
    Recovering,
    SttUnavailable,
    LlmUnavailable,
    TtsUnavailable,
    RendererUnavailable,
}

impl ConversationState {
    #[must_use]
    pub const fn is_unavailable(self) -> bool {
        matches!(
            self,
            Self::SttUnavailable
                | Self::LlmUnavailable
                | Self::TtsUnavailable
                | Self::RendererUnavailable
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ConversationState;

    #[test]
    fn serializes_idle_as_snake_case() {
        let json = serde_json::to_string(&ConversationState::Idle).unwrap();
        assert_eq!(json, "\"idle\"");
    }

    #[test]
    fn unavailable_states_are_terminal_for_the_current_operation() {
        assert!(ConversationState::SttUnavailable.is_unavailable());
        assert!(ConversationState::LlmUnavailable.is_unavailable());
        assert!(ConversationState::TtsUnavailable.is_unavailable());
        assert!(ConversationState::RendererUnavailable.is_unavailable());
        assert!(!ConversationState::Idle.is_unavailable());
    }
}
