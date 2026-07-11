//! Application status contract.

use pw_domain::conversation::ConversationState;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Version of the IPC contract schema carried by every DTO.
pub const SCHEMA_VERSION: u16 = 1;

/// Wire representation of [`ConversationState`].
///
/// Mirrors the domain enum variant-for-variant so the webview windows
/// never depend on domain types directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "ConversationStateDto.ts")]
pub enum ConversationStateDto {
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

impl From<ConversationState> for ConversationStateDto {
    fn from(state: ConversationState) -> Self {
        match state {
            ConversationState::Starting => Self::Starting,
            ConversationState::Idle => Self::Idle,
            ConversationState::Listening => Self::Listening,
            ConversationState::Transcribing => Self::Transcribing,
            ConversationState::Thinking => Self::Thinking,
            ConversationState::Speaking => Self::Speaking,
            ConversationState::Muted => Self::Muted,
            ConversationState::Interrupting => Self::Interrupting,
            ConversationState::Cancelled => Self::Cancelled,
            ConversationState::Recovering => Self::Recovering,
            ConversationState::SttUnavailable => Self::SttUnavailable,
            ConversationState::LlmUnavailable => Self::LlmUnavailable,
            ConversationState::TtsUnavailable => Self::TtsUnavailable,
            ConversationState::RendererUnavailable => Self::RendererUnavailable,
        }
    }
}

/// Snapshot of the application status exposed over IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "AppStatusDto.ts")]
pub struct AppStatusDto {
    pub schema_version: u16,
    pub conversation_state: ConversationStateDto,
}

#[cfg(test)]
mod tests {
    use super::{AppStatusDto, ConversationStateDto, SCHEMA_VERSION};

    #[test]
    fn serializes_versioned_status_contract() {
        let value = AppStatusDto {
            schema_version: SCHEMA_VERSION,
            conversation_state: ConversationStateDto::Idle,
        };
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["conversation_state"], "idle");
    }
}
