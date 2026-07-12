//! Speech text normalization: emoji removal.

/// True for characters that render as emoji / pictographs and must
/// not reach the chat display or the TTS queue.
fn is_emoji_component(ch: char) -> bool {
    matches!(ch,
        // Emoticons, symbols & pictographs, transport, supplemental,
        // extended pictographs, regional indicators, game tiles.
        '\u{1F000}'..='\u{1FAFF}'
        // Misc symbols and dingbats (☀..➿).
        | '\u{2600}'..='\u{27BF}'
        // Variation selectors, zero-width joiner, combining keycap.
        | '\u{FE0E}' | '\u{FE0F}' | '\u{200D}' | '\u{20E3}'
        // Arrows-as-emoji and geometric shapes commonly emojified.
        | '\u{2B00}'..='\u{2BFF}'
    )
}

/// Removes emoji (including ZWJ sequences and skin-tone modifiers)
/// while leaving Japanese text and normal punctuation intact.
#[must_use]
pub fn strip_emoji(text: &str) -> String {
    text.chars().filter(|ch| !is_emoji_component(*ch)).collect()
}

#[cfg(test)]
mod tests {
    use super::strip_emoji;

    #[test]
    fn removes_common_emoji() {
        assert_eq!(
            strip_emoji("こんにちは😊今日もいい天気ですね☀️"),
            "こんにちは今日もいい天気ですね"
        );
    }

    #[test]
    fn removes_zwj_sequences_and_modifiers() {
        assert_eq!(strip_emoji("がんばって👍🏻👨‍👩‍👧!"), "がんばって!");
    }

    #[test]
    fn keeps_japanese_punctuation_and_symbols() {
        let text = "はい、そうです。全角！？「引用」…（括弧）100%・ー〜";
        assert_eq!(strip_emoji(text), text);
    }

    #[test]
    fn keeps_ascii_and_numbers() {
        let text = "OpenAI互換APIはport 1234で動作中です。";
        assert_eq!(strip_emoji(text), text);
    }
}
