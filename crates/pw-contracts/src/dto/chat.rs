//! Chat conversation contracts.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::app_status::ConversationStateDto;

/// Author of one chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "ChatRoleDto.ts")]
pub enum ChatRoleDto {
    User,
    Assistant,
}

/// `chat-message` event payload. Assistant messages arrive one
/// sentence at a time while the reply streams.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ChatMessageEventDto.ts")]
pub struct ChatMessageEventDto {
    pub schema_version: u16,
    /// Turn the message belongs to; stale turns can be dropped.
    pub turn_id: u64,
    pub role: ChatRoleDto,
    pub text: String,
}

/// `conversation-state` event payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ConversationStateEventDto.ts")]
pub struct ConversationStateEventDto {
    pub schema_version: u16,
    pub state: ConversationStateDto,
    /// Human-readable detail for degraded states (no secrets).
    pub message: Option<String>,
}

/// Persisted LLM connection and prompt settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "LlmSettingsDto.ts")]
pub struct LlmSettingsDto {
    pub schema_version: u16,
    /// OpenAI-compatible base URL, e.g. `http://127.0.0.1:8080/v1`.
    pub base_url: String,
    pub model: String,
    /// Permit non-loopback endpoints.
    pub allow_remote: bool,
    pub system_prompt: String,
    pub character_prompt: String,
    /// Remove emoji from replies (display and TTS safety).
    #[serde(default = "default_strip_emoji")]
    #[ts(optional = false)]
    pub strip_emoji: bool,
}

fn default_strip_emoji() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{ChatMessageEventDto, ChatRoleDto};
    use crate::SCHEMA_VERSION;

    #[test]
    fn serializes_chat_message_with_snake_case_role() {
        let value = ChatMessageEventDto {
            schema_version: SCHEMA_VERSION,
            turn_id: 3,
            role: ChatRoleDto::Assistant,
            text: "こんにちは。".into(),
        };
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["turn_id"], 3);
        assert_eq!(json["role"], "assistant");
    }
}
