//! Turn identity used to discard stale generation output.

/// Monotonically increasing identifier of one conversation turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TurnId(u64);

impl TurnId {
    #[must_use]
    pub fn value(self) -> u64 {
        self.0
    }
}

/// Issues turn ids and knows which one is current. Output tagged
/// with a non-current id must be discarded (設計spec 3章).
#[derive(Debug, Default)]
pub struct TurnTracker {
    next: u64,
    current: Option<TurnId>,
}

impl TurnTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resumes allocation after a previously persisted turn id.
    #[must_use]
    pub const fn after(last_turn_id: u64) -> Self {
        Self {
            next: last_turn_id,
            current: None,
        }
    }

    /// Starts a new turn, invalidating the previous one.
    pub fn begin_turn(&mut self) -> TurnId {
        self.next += 1;
        let id = TurnId(self.next);
        self.current = Some(id);
        id
    }

    /// True when the id belongs to the active turn.
    #[must_use]
    pub fn is_current(&self, id: TurnId) -> bool {
        self.current == Some(id)
    }

    /// Ends the active turn without starting a new one (cancel).
    pub fn invalidate(&mut self) {
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::TurnTracker;

    #[test]
    fn issues_monotonically_increasing_turn_ids() {
        let mut tracker = TurnTracker::new();
        let first = tracker.begin_turn();
        let second = tracker.begin_turn();
        assert!(second > first);
    }

    #[test]
    fn only_the_latest_turn_is_current() {
        let mut tracker = TurnTracker::new();
        let first = tracker.begin_turn();
        assert!(tracker.is_current(first));
        let second = tracker.begin_turn();
        assert!(!tracker.is_current(first));
        assert!(tracker.is_current(second));
    }

    #[test]
    fn invalidate_makes_no_turn_current() {
        let mut tracker = TurnTracker::new();
        let turn = tracker.begin_turn();
        tracker.invalidate();
        assert!(!tracker.is_current(turn));
    }

    #[test]
    fn resumes_after_the_largest_persisted_turn_id() {
        let mut tracker = TurnTracker::after(41);
        assert_eq!(tracker.begin_turn().value(), 42);
    }
}
