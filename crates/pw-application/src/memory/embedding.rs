//! Optional local embedding re-ranking for planned memory retrieval.
//!
//! The ordinary conversation path does not construct this adapter, so its
//! default remains the existing `SQLite` FTS/lexical search.  An explicitly
//! configured local implementation may re-rank an already bounded lexical
//! candidate set.  The adapter never owns persistence and receives no
//! transcript beyond the bounded query and candidate snippets.

use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError, sync_channel};
use std::thread;
use std::time::Duration;

use crate::PortError;

/// Maximum query payload sent to an optional local embedder.
pub const MAX_EMBEDDING_QUERY_CHARS: usize = 240;
/// Maximum number of lexical candidates sent to an optional local embedder.
pub const MAX_EMBEDDING_CANDIDATES: usize = 16;
/// Maximum snippet payload per lexical candidate.
pub const MAX_EMBEDDING_CANDIDATE_CHARS: usize = 512;
/// Retrieval work must not delay a planned response for more than this.
pub const MAX_EMBEDDING_TIMEOUT: Duration = Duration::from_millis(100);

/// A bounded score returned by a local embedding adapter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmbeddingHit {
    pub memory_id: i64,
    pub score: f32,
}

/// Optional adapter port. Implementations are expected to be local-only and
/// must not make network or paid-service calls. They are never called for a
/// simple turn; callers pass only bounded lexical candidates here.
#[allow(clippy::missing_errors_doc)]
pub trait MemoryEmbedder: Send + 'static {
    fn rank(
        &mut self,
        query: &str,
        candidates: &[super::MemoryRecord],
    ) -> Result<Vec<EmbeddingHit>, PortError>;
}

type EmbeddingResponse = (
    Box<dyn MemoryEmbedder>,
    Result<Vec<EmbeddingHit>, PortError>,
);

/// Explicit opt-in embedding re-ranker with lexical/FTS fail-open behavior.
///
/// `LexicalFallback::disabled()` is the production default. A timeout leaves
/// the adapter running in the background, but prevents it from blocking the
/// response; once it finishes, the adapter is reclaimed for a later planned
/// turn. This bounds both the call and the number of in-flight adapters.
pub struct LexicalFallback {
    adapter: Option<Box<dyn MemoryEmbedder>>,
    pending: Option<Receiver<EmbeddingResponse>>,
    timeout: Duration,
}

impl std::fmt::Debug for LexicalFallback {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LexicalFallback")
            .field("enabled", &self.adapter.is_some())
            .field("pending", &self.pending.is_some())
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl LexicalFallback {
    /// Creates the safe default: no adapter, unchanged lexical ordering.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            adapter: None,
            pending: None,
            timeout: MAX_EMBEDDING_TIMEOUT,
        }
    }

    /// Enables one explicitly supplied local adapter. The timeout is capped
    /// at [`MAX_EMBEDDING_TIMEOUT`] even if configuration is over-generous.
    #[must_use]
    pub fn enabled(adapter: impl MemoryEmbedder, timeout: Duration) -> Self {
        Self {
            adapter: Some(Box::new(adapter)),
            pending: None,
            timeout: timeout.min(MAX_EMBEDDING_TIMEOUT),
        }
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.adapter.is_some() || self.pending.is_some()
    }

    /// Re-ranks lexical candidates when the optional adapter succeeds.
    ///
    /// Every failure mode returns the original lexical candidates bounded by
    /// `limit`: disabled/missing, malformed scores, panic, timeout, and
    /// adapter errors. No error can reach the reply or TTS path.
    #[must_use]
    pub fn rerank(
        &mut self,
        query: &str,
        lexical_candidates: Vec<super::MemoryRecord>,
        limit: usize,
    ) -> Vec<super::MemoryRecord> {
        if limit == 0 || lexical_candidates.is_empty() {
            return lexical_fallback(lexical_candidates, limit);
        }
        self.reclaim_pending();
        let Some(adapter) = self.adapter.take() else {
            return lexical_fallback(lexical_candidates, limit);
        };

        let query = bounded(query, MAX_EMBEDDING_QUERY_CHARS);
        if query.is_empty() {
            self.adapter = Some(adapter);
            return lexical_fallback(lexical_candidates, limit);
        }
        let input = lexical_candidates
            .iter()
            .take(MAX_EMBEDDING_CANDIDATES)
            .map(|candidate| super::MemoryRecord {
                id: candidate.id,
                content: bounded(&candidate.content, MAX_EMBEDDING_CANDIDATE_CHARS),
            })
            .collect::<Vec<_>>();
        if input.is_empty() || input.iter().any(|candidate| candidate.content.is_empty()) {
            self.adapter = Some(adapter);
            return lexical_fallback(lexical_candidates, limit);
        }

        let (sender, receiver) = sync_channel(1);
        thread::spawn(move || {
            let mut adapter = adapter;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                adapter.rank(&query, &input)
            }))
            .map_err(|_| PortError("memory embedder panicked".into()))
            .and_then(std::convert::identity);
            let _ = sender.send((adapter, result));
        });

        match receiver.recv_timeout(self.timeout) {
            Ok((adapter, result)) => {
                self.adapter = Some(adapter);
                apply_hits(lexical_candidates, result.ok(), limit)
            }
            Err(RecvTimeoutError::Timeout) => {
                self.pending = Some(receiver);
                lexical_fallback(lexical_candidates, limit)
            }
            Err(RecvTimeoutError::Disconnected) => lexical_fallback(lexical_candidates, limit),
        }
    }

    fn reclaim_pending(&mut self) {
        let Some(receiver) = self.pending.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok((adapter, _)) => self.adapter = Some(adapter),
            Err(TryRecvError::Empty) => self.pending = Some(receiver),
            Err(TryRecvError::Disconnected) => {}
        }
    }
}

fn lexical_fallback(
    candidates: Vec<super::MemoryRecord>,
    limit: usize,
) -> Vec<super::MemoryRecord> {
    candidates.into_iter().take(limit).collect()
}

fn bounded(value: &str, max: usize) -> String {
    value.trim().chars().take(max).collect()
}

fn apply_hits(
    lexical_candidates: Vec<super::MemoryRecord>,
    result: Option<Vec<EmbeddingHit>>,
    limit: usize,
) -> Vec<super::MemoryRecord> {
    let Some(mut hits) = result else {
        return lexical_fallback(lexical_candidates, limit);
    };
    if hits.is_empty() || hits.len() > MAX_EMBEDDING_CANDIDATES {
        return lexical_fallback(lexical_candidates, limit);
    }
    let valid_ids = lexical_candidates
        .iter()
        .take(MAX_EMBEDDING_CANDIDATES)
        .map(|candidate| candidate.id)
        .collect::<std::collections::HashSet<_>>();
    let mut seen_ids = std::collections::HashSet::with_capacity(hits.len());
    let malformed = hits.iter().any(|hit| {
        !valid_ids.contains(&hit.memory_id)
            || !hit.score.is_finite()
            || !(0.0..=1.0).contains(&hit.score)
            || !seen_ids.insert(hit.memory_id)
    });
    if malformed {
        return lexical_fallback(lexical_candidates, limit);
    }
    hits.sort_by(|left, right| right.score.total_cmp(&left.score));
    let mut ranked = Vec::with_capacity(limit.min(lexical_candidates.len()));
    for hit in hits {
        if let Some(candidate) = lexical_candidates
            .iter()
            .find(|candidate| candidate.id == hit.memory_id)
        {
            ranked.push(candidate.clone());
        }
        if ranked.len() == limit {
            return ranked;
        }
    }
    for candidate in lexical_candidates {
        if ranked.len() == limit {
            break;
        }
        if !ranked.iter().any(|item| item.id == candidate.id) {
            ranked.push(candidate);
        }
    }
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records() -> Vec<super::super::MemoryRecord> {
        vec![
            super::super::MemoryRecord {
                id: 1,
                content: "lexical one".into(),
            },
            super::super::MemoryRecord {
                id: 2,
                content: "lexical two".into(),
            },
        ]
    }

    struct Reorder;
    impl MemoryEmbedder for Reorder {
        fn rank(
            &mut self,
            query: &str,
            candidates: &[super::super::MemoryRecord],
        ) -> Result<Vec<EmbeddingHit>, PortError> {
            assert_eq!(query, "tea");
            assert!(
                candidates
                    .iter()
                    .all(|item| item.content.chars().count() <= MAX_EMBEDDING_CANDIDATE_CHARS)
            );
            Ok(vec![
                EmbeddingHit {
                    memory_id: 2,
                    score: 0.9,
                },
                EmbeddingHit {
                    memory_id: 1,
                    score: 0.1,
                },
            ])
        }
    }

    #[test]
    fn disabled_adapter_preserves_lexical_fallback() {
        let mut fallback = LexicalFallback::disabled();
        assert_eq!(fallback.rerank("tea", records(), 1)[0].id, 1);
        assert!(!fallback.is_enabled());
    }

    #[test]
    fn successful_local_adapter_only_reorders_bounded_candidates() {
        let mut fallback = LexicalFallback::enabled(Reorder, Duration::from_millis(10));
        assert_eq!(
            fallback
                .rerank("tea", records(), 2)
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [2, 1]
        );
    }

    struct Slow;
    impl MemoryEmbedder for Slow {
        fn rank(
            &mut self,
            _: &str,
            _: &[super::super::MemoryRecord],
        ) -> Result<Vec<EmbeddingHit>, PortError> {
            thread::sleep(Duration::from_millis(50));
            Ok(Vec::new())
        }
    }

    #[test]
    fn slow_adapter_fails_open_without_blocking_and_is_reclaimed() {
        let mut fallback = LexicalFallback::enabled(Slow, Duration::from_millis(1));
        assert_eq!(fallback.rerank("tea", records(), 1)[0].id, 1);
        assert!(fallback.is_enabled());
        thread::sleep(Duration::from_millis(60));
        assert_eq!(fallback.rerank("tea", records(), 1)[0].id, 1);
    }

    struct Invalid;
    impl MemoryEmbedder for Invalid {
        fn rank(
            &mut self,
            _: &str,
            _: &[super::super::MemoryRecord],
        ) -> Result<Vec<EmbeddingHit>, PortError> {
            Ok(vec![EmbeddingHit {
                memory_id: 999,
                score: f32::NAN,
            }])
        }
    }

    #[test]
    fn malformed_hits_fall_back_and_payload_is_bounded() {
        let mut fallback = LexicalFallback::enabled(Invalid, Duration::from_millis(10));
        let candidates = vec![super::super::MemoryRecord {
            id: 1,
            content: "x".repeat(2_000),
        }];
        assert_eq!(fallback.rerank(&"q".repeat(1_000), candidates, 1)[0].id, 1);
        assert!(fallback.is_enabled());
    }

    struct MixedMalformed(Vec<EmbeddingHit>);
    impl MemoryEmbedder for MixedMalformed {
        fn rank(
            &mut self,
            _: &str,
            _: &[super::super::MemoryRecord],
        ) -> Result<Vec<EmbeddingHit>, PortError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn any_malformed_hit_falls_back_instead_of_partially_reranking() {
        for hits in [
            vec![
                EmbeddingHit {
                    memory_id: 2,
                    score: 0.9,
                },
                EmbeddingHit {
                    memory_id: 999,
                    score: 0.8,
                },
            ],
            vec![
                EmbeddingHit {
                    memory_id: 2,
                    score: 0.9,
                },
                EmbeddingHit {
                    memory_id: 1,
                    score: f32::NAN,
                },
            ],
            vec![
                EmbeddingHit {
                    memory_id: 2,
                    score: 0.9,
                },
                EmbeddingHit {
                    memory_id: 2,
                    score: 0.8,
                },
            ],
        ] {
            let mut fallback =
                LexicalFallback::enabled(MixedMalformed(hits), Duration::from_millis(10));
            let ranked = fallback.rerank("tea", records(), 2);
            assert_eq!(
                ranked.iter().map(|item| item.id).collect::<Vec<_>>(),
                [1, 2]
            );
        }
    }
}
