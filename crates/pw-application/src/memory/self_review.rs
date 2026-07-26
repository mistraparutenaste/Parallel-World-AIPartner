use crate::conversation::{ChatMessage, ChatRole};

/// Builds a human-facing reflection request. This output is intentionally
/// separate from `MemoryContext` and must never be fed back into chat prompts.
pub struct SelfReviewGenerator;

impl SelfReviewGenerator {
    #[must_use]
    pub fn prompt(transcript: &str) -> Vec<ChatMessage> {
        vec![
            ChatMessage::new(
                ChatRole::System,
                "Write a short, compassionate Japanese reflection addressed to the user. Summarize recurring interests, concerns, and progress visible in the supplied conversation. Do not diagnose, invent facts, expose secrets, or give instructions to the assistant. Return plain text only.",
            ),
            ChatMessage::new(
                ChatRole::User,
                format!(
                    "会話記録:\n{transcript}\n\n「あなたについて」として200〜500文字で振り返ってください。"
                ),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_human_facing_and_plain_text_only() {
        let prompt = SelfReviewGenerator::prompt("user: 紅茶が好き");
        assert_eq!(prompt.len(), 2);
        assert!(prompt[0].content.contains("plain text"));
        assert!(prompt[1].content.contains("紅茶が好き"));
    }
}
