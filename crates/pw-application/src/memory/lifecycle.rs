const SECONDS_PER_DAY: i64 = 86_400;
pub const DORMANT_DELETE_AFTER_SECONDS: i64 = 180 * SECONDS_PER_DAY;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryState {
    Active,
    Dormant,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind {
    UserMention,
    Recalled,
    Pinned,
    Imported,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryEvidence {
    pub id: i64,
    pub kind: EvidenceKind,
    pub occurred_at: i64,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryCandidate {
    pub id: i64,
    /// `None` is reserved for adapters that cannot prove a current row revision.
    pub revision: Option<i64>,
    pub content: String,
    pub state: MemoryState,
    pub pinned: bool,
    pub mention_count: u64,
    pub last_seen_at: i64,
    pub lexical_relevance: f64,
    pub strength: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryAction {
    Add {
        content: String,
        pinned: bool,
    },
    Reinforce {
        memory_id: i64,
        pin: bool,
    },
    Supersede {
        old_memory_id: i64,
        content: String,
        pin_replacement: bool,
    },
    Ignore,
}

#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn memory_strength(evidence: &[MemoryEvidence], now: i64) -> f64 {
    let contribution = |item: &MemoryEvidence| {
        let age_seconds = now.saturating_sub(item.occurred_at).max(SECONDS_PER_DAY);
        let age_days = age_seconds as f64 / SECONDS_PER_DAY as f64;
        item.weight * (30.0 / age_days).sqrt()
    };
    let user = evidence
        .iter()
        .filter(|item| {
            matches!(
                item.kind,
                EvidenceKind::UserMention | EvidenceKind::Imported
            )
        })
        .map(contribution)
        .sum::<f64>();
    // Prompt recall is audit-only: it must never extend retention or prevent dormancy.
    user
}

#[must_use]
pub fn should_become_dormant(evidence: &[MemoryEvidence], now: i64) -> bool {
    memory_strength(evidence, now) < 1.0
}

#[must_use]
pub fn prompt_rank(lexical_relevance: f64, strength: f64) -> f64 {
    lexical_relevance.clamp(0.0, 1.0) * 0.7 + strength.clamp(0.0, 1.0) * 0.3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days(value: i64) -> i64 {
        value * 86_400
    }

    #[test]
    fn repeated_mentions_extend_retention_with_power_law_decay() {
        let evidence = |count| {
            (0..count)
                .map(|id| MemoryEvidence {
                    id,
                    kind: EvidenceKind::UserMention,
                    occurred_at: 0,
                    weight: 1.0,
                })
                .collect::<Vec<_>>()
        };
        assert!(!should_become_dormant(&evidence(1), days(30)));
        assert!(should_become_dormant(&evidence(1), days(31)));
        assert!(!should_become_dormant(&evidence(2), days(120)));
        assert!(should_become_dormant(&evidence(2), days(121)));
        assert!(!should_become_dormant(&evidence(3), days(270)));
        assert!(should_become_dormant(&evidence(3), days(271)));
    }

    #[test]
    fn recalled_evidence_never_changes_strength_or_dormancy() {
        let mut evidence = vec![MemoryEvidence {
            id: 1,
            kind: EvidenceKind::UserMention,
            occurred_at: 0,
            weight: 1.0,
        }];
        evidence.extend((2..102).map(|id| MemoryEvidence {
            id,
            kind: EvidenceKind::Recalled,
            occurred_at: 0,
            weight: 0.15,
        }));
        let user_only = memory_strength(&evidence[..1], days(30));
        let total = memory_strength(&evidence, days(30));
        assert!((total - user_only).abs() < 1e-9);
        assert_eq!(
            should_become_dormant(&evidence[..1], days(31)),
            should_become_dormant(&evidence, days(31))
        );
    }

    #[test]
    fn lexical_relevance_dominates_prompt_rank() {
        assert!(prompt_rank(0.9, 0.1) > prompt_rank(0.2, 1.0));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn clock_rollback_is_clamped_to_one_day_of_age() {
        let evidence = [MemoryEvidence {
            id: 1,
            kind: EvidenceKind::UserMention,
            occurred_at: 1_000,
            weight: 1.0,
        }];
        assert_eq!(
            memory_strength(&evidence, 999),
            memory_strength(&evidence, 1_000)
        );
    }
}
