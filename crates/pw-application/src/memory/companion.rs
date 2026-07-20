//! Deterministic, bounded companion signals.
//!
//! Completed turns are reduced to small state deltas before they leave the
//! conversation worker. No transcript is retained in this module.

use sha2::{Digest, Sha256};

use super::{
    Commitment, CommitmentStatus, DialogueSignals, is_safe_persistent_content,
    redact_persistent_content,
};

pub const MAX_SIGNAL_CHARS: usize = 96;
pub const MAX_COMMITMENT_CHARS: usize = 160;
const SIGNAL_TTL_SECONDS: i64 = 86_400;

#[must_use]
pub fn derive_dialogue_signals(
    conversation_id: &str,
    user_text: &str,
    assistant_text: &str,
    now: i64,
) -> Option<DialogueSignals> {
    if conversation_id.trim().is_empty() || now < 0 {
        return None;
    }
    let normalized = user_text.trim();
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();
    let mood = if contains_any(
        &lower,
        &["ありがとう", "嬉しい", "happy", "great", "助かる"],
    ) {
        Some("positive".to_owned())
    } else if contains_any(&lower, &["悲しい", "つらい", "sad", "angry", "困った"]) {
        Some("strained".to_owned())
    } else {
        None
    };
    let reaction = if contains_any(
        &assistant_text.to_ascii_lowercase(),
        &["sorry", "ごめん", "了解", "sure"],
    ) {
        Some("acknowledged".to_owned())
    } else if !assistant_text.trim().is_empty() {
        Some("answered".to_owned())
    } else {
        None
    };
    let relationship_delta = match mood.as_deref() {
        Some("positive") => 1,
        Some("strained") => -1,
        _ => 0,
    };
    let signals = DialogueSignals {
        conversation_id: conversation_id
            .trim()
            .chars()
            .take(MAX_SIGNAL_CHARS)
            .collect(),
        mood,
        reaction,
        relationship_delta,
        reflection_cursor: Some(stable_cursor(normalized)),
        reflection_state: Some("eligible".to_owned()),
        commitment: detect_explicit_commitment(conversation_id, normalized, now),
        observed_at: now,
        expires_at: now.saturating_add(SIGNAL_TTL_SECONDS),
    };
    signals.validate().ok().map(|_| signals)
}

/// Conservative commitment detector. Only explicit promise/remember intent
/// creates a candidate; ordinary statements never become commitments.
#[must_use]
pub fn detect_explicit_commitment(
    conversation_id: &str,
    user_text: &str,
    now: i64,
) -> Option<Commitment> {
    if conversation_id.trim().is_empty() || now < 0 {
        return None;
    }
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let english_explicit = contains_any(
        &lower,
        &[
            "promise",
            "i will",
            "i'll",
            "remember that",
            "don't let me forget",
        ],
    );
    // Phrase-based Japanese matching prevents ordinary words such as
    // "やすみ" from becoming durable commitments.
    let japanese_explicit = contains_any(
        trimmed,
        &[
            "\u{7d04}\u{675f}\u{3057}\u{307e}\u{3059}", // 約束します
            "\u{7d04}\u{675f}\u{3059}\u{308b}",         // 約束する
            "\u{899a}\u{3048}\u{3066}\u{304a}\u{3044}\u{3066}", // 覚えておいて
            "\u{5fd8}\u{308c}\u{306a}\u{3044}\u{3067}", // 忘れないで
            "\u{5fc5}\u{305a}",                         // 必ず...
            "\u{6b21}\u{306f}",                         // 次は...
            "\u{3084}\u{308a}\u{307e}\u{3059}",         // やります
            "\u{3057}\u{3066}\u{304a}\u{304d}\u{307e}\u{3059}", // しておきます
            "\u{3057}\u{307e}\u{3059}\u{306d}",         // しますね
        ],
    );
    let action_phrase = contains_any(
        trimmed,
        &[
            "\u{3057}\u{307e}\u{3059}",
            "\u{3084}\u{308a}\u{307e}\u{3059}",
            "\u{3057}\u{3066}\u{304a}\u{304d}\u{307e}\u{3059}",
            "\u{78ba}\u{8a8d}",
            "\u{9023}\u{7d61}",
            "\u{884c}\u{304d}",
            "\u{4e88}\u{7d04}",
            "\u{5bfe}\u{5fdc}",
        ],
    );
    let prefix_requires_action = contains_any(trimmed, &["\u{5fc5}\u{305a}", "\u{6b21}\u{306f}"]);
    let japanese_explicit = japanese_explicit && (!prefix_requires_action || action_phrase);
    if !english_explicit && !japanese_explicit {
        return None;
    }
    let mut content: String = trimmed.chars().take(MAX_COMMITMENT_CHARS).collect();
    content = redact_persistent_content(&content);
    if content.trim().is_empty() || !is_safe_persistent_content(&content) {
        return None;
    }
    Some(Commitment {
        id: None,
        conversation_id: conversation_id.trim().to_owned(),
        content,
        status: CommitmentStatus::Open,
        due_at: None,
        next_check_at: None,
        expires_at: Some(now.saturating_add(7 * 86_400)),
        revision: 0,
    })
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn stable_cursor(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"parallel-world/reflection/v1\0");
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signals_are_bounded_and_never_include_assistant_text() {
        let signals = derive_dialogue_signals(
            "chat",
            &"x".repeat(500),
            &"secret transcript".repeat(50),
            10,
        )
        .unwrap();
        assert!(signals.mood.is_none());
        assert_eq!(signals.reaction.as_deref(), Some("answered"));
        assert!(signals.reflection_cursor.as_ref().unwrap().len() <= MAX_SIGNAL_CHARS);
        assert!(!format!("{signals:?}").contains("secret transcript"));
        assert!(signals.validate().is_ok());
    }

    #[test]
    fn commitment_matching_is_phrase_based_and_bounded() {
        for ordinary in ["今日はやすみです", "やめます", "必ず雨です", "次は晴れです"]
        {
            assert!(detect_explicit_commitment("chat", ordinary, 10).is_none());
        }
        assert!(detect_explicit_commitment("chat", "必ず資料を確認します", 10).is_some());
        let commitment = detect_explicit_commitment("chat", "約束します", 10).unwrap();
        assert_eq!(commitment.status, CommitmentStatus::Open);
        assert!(commitment.content.chars().count() <= MAX_COMMITMENT_CHARS);
    }
}
