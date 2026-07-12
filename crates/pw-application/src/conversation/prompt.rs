//! Prompt assembly in the order fixed by 基本設計 7章.

use super::ports::{ChatMessage, ChatRole};

/// Builds the message list: system rules, character settings,
/// (user settings / memory / summary arrive in Phase 5), recent
/// conversation, current utterance.
#[derive(Debug, Clone)]
pub struct PromptBuilder {
    pub system_rules: String,
    pub character_prompt: String,
}

impl PromptBuilder {
    #[must_use]
    pub fn build(&self, history: &[ChatMessage], current_utterance: &str) -> Vec<ChatMessage> {
        let mut messages = Vec::with_capacity(history.len() + 3);
        if !self.system_rules.trim().is_empty() {
            messages.push(ChatMessage::new(
                ChatRole::System,
                self.system_rules.clone(),
            ));
        }
        if !self.character_prompt.trim().is_empty() {
            messages.push(ChatMessage::new(
                ChatRole::System,
                self.character_prompt.clone(),
            ));
        }
        messages.extend(history.iter().cloned());
        messages.push(ChatMessage::new(ChatRole::User, current_utterance));
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::super::ports::{ChatMessage, ChatRole};
    use super::PromptBuilder;

    #[test]
    fn orders_rules_character_history_and_utterance() {
        let builder = PromptBuilder {
            system_rules: "規則".into(),
            character_prompt: "キャラ設定".into(),
        };
        let history = [
            ChatMessage::new(ChatRole::User, "前の質問"),
            ChatMessage::new(ChatRole::Assistant, "前の答え"),
        ];
        let messages = builder.build(&history, "今の質問");
        let contents: Vec<_> = messages.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(
            contents,
            ["規則", "キャラ設定", "前の質問", "前の答え", "今の質問"]
        );
        assert_eq!(messages[0].role, ChatRole::System);
        assert_eq!(messages[1].role, ChatRole::System);
        assert_eq!(messages[4].role, ChatRole::User);
    }

    #[test]
    fn skips_empty_sections() {
        let builder = PromptBuilder {
            system_rules: String::new(),
            character_prompt: "キャラ".into(),
        };
        let messages = builder.build(&[], "こんにちは");
        assert_eq!(messages.len(), 2);
    }
}
