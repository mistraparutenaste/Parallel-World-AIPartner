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
    /// (JSON payloads carry plain numbers, so the TS type is number.)
    #[ts(type = "number")]
    pub turn_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    #[ts(type = "number")]
    pub message_id: Option<i64>,
    pub role: ChatRoleDto,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ConversationMessageDto.ts")]
pub struct ConversationMessageDto {
    pub schema_version: u16,
    #[ts(type = "number")]
    pub message_id: i64,
    #[ts(type = "number")]
    pub turn_id: Option<u64>,
    pub role: ChatRoleDto,
    pub text: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ConversationLogPageDto.ts")]
pub struct ConversationLogPageDto {
    pub schema_version: u16,
    pub messages: Vec<ConversationMessageDto>,
    #[ts(type = "number | null")]
    pub next_before_message_id: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "ConversationHistoryDeletedEventDto.ts")]
pub struct ConversationHistoryDeletedEventDto {
    pub schema_version: u16,
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

/// LLM endpoint presets supported by the desktop settings UI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "LlmProviderKind.ts")]
pub enum LlmProviderKind {
    #[default]
    Local,
    Openai,
    Gemini,
    OpencodeZen,
    Custom,
}

/// Persisted LLM connection and prompt settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[allow(clippy::struct_excessive_bools)]
#[ts(export_to = "LlmSettingsDto.ts")]
pub struct LlmSettingsDto {
    pub schema_version: u16,
    /// Provider preset. Missing legacy values remain local.
    #[serde(default)]
    pub provider: LlmProviderKind,
    /// OpenAI-compatible base URL, e.g. `http://127.0.0.1:8080/v1`.
    pub base_url: String,
    pub model: String,
    /// A replacement API key supplied by the UI. It is never persisted or
    /// returned by the backend.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    /// Whether a key is currently stored in the operating-system credential store.
    #[serde(default)]
    pub api_key_configured: bool,
    /// Requests deletion of the stored key. This command-only flag is not persisted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear_api_key: bool,
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
            message_id: None,
            role: ChatRoleDto::Assistant,
            text: "こんにちは。".into(),
        };
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["turn_id"], 3);
        assert_eq!(json["role"], "assistant");
    }
}
