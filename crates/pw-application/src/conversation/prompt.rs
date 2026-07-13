//! Prompt assembly in the order fixed by 基本設計 7章.

use super::ports::{ChatMessage, ChatRole};
use crate::memory::MemoryContext;

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
        self.build_with_context(history, current_utterance, &MemoryContext::default())
    }

    #[must_use]
    pub fn build_with_context(
        &self,
        history: &[ChatMessage],
        current_utterance: &str,
        context: &MemoryContext,
    ) -> Vec<ChatMessage> {
        let context = context.clone().bounded();
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
        for section in [
            context.user_settings,
            (!context.memories.is_empty()).then(|| context.memories.join("\n")),
            context.summary,
        ]
        .into_iter()
        .flatten()
        {
            if !section.trim().is_empty() {
                messages.push(ChatMessage::new(ChatRole::System, section));
            }
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
    use crate::memory::MemoryContext;

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

    #[test]
    fn context_sections_follow_the_design_order() {
        let builder = PromptBuilder {
            system_rules: "規則".into(),
            character_prompt: "キャラ".into(),
        };
        let context = MemoryContext {
            user_settings: Some("設定".into()),
            summary: Some("要約".into()),
            memories: vec!["記憶A".into(), "記憶B".into()],
        };
        let history = [ChatMessage::new(ChatRole::Assistant, "直近")];
        let messages = builder.build_with_context(&history, "現在", &context);
        assert_eq!(
            messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>(),
            [
                "規則",
                "キャラ",
                "設定",
                "記憶A\n記憶B",
                "要約",
                "直近",
                "現在"
            ]
        );
    }

    #[test]
    fn empty_context_sections_are_omitted() {
        let builder = PromptBuilder {
            system_rules: String::new(),
            character_prompt: String::new(),
        };
        assert_eq!(
            builder.build_with_context(&[], "現在", &MemoryContext::default()),
            [ChatMessage::new(ChatRole::User, "現在")]
        );
    }
}
