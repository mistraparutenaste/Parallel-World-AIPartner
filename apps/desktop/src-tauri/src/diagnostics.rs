use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use pw_contracts::QueueMetricsDto;

pub struct QueueMetrics {
    name: &'static str,
    capacity: usize,
    depth: AtomicUsize,
    dropped: AtomicU64,
    busy: AtomicU64,
    coalesced: AtomicU64,
}

impl QueueMetrics {
    #[must_use]
    pub const fn new(name: &'static str, capacity: usize) -> Self {
        Self {
            name,
            capacity,
            depth: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
            busy: AtomicU64::new(0),
            coalesced: AtomicU64::new(0),
        }
    }
    pub fn enqueued(&self) {
        self.depth.fetch_add(1, Ordering::Relaxed);
    }
    pub fn dequeued(&self) {
        self.depth
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_sub(1))
            })
            .ok();
    }
    pub fn dropped(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
    pub fn busy(&self) {
        self.busy.fetch_add(1, Ordering::Relaxed);
    }
    pub fn coalesced(&self) {
        self.coalesced.fetch_add(1, Ordering::Relaxed);
    }
    pub fn snapshot(&self) -> QueueMetricsDto {
        QueueMetricsDto {
            name: self.name.to_owned(),
            depth: self.depth.load(Ordering::Relaxed),
            capacity: self.capacity,
            dropped: self.dropped.load(Ordering::Relaxed),
            busy: self.busy.load(Ordering::Relaxed),
            coalesced: self.coalesced.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::QueueMetrics;

    #[test]
    fn queue_counters_track_depth_and_saturate_on_extra_dequeue() {
        let metrics = QueueMetrics::new("test", 2);
        metrics.enqueued();
        metrics.enqueued();
        metrics.dequeued();
        metrics.dequeued();
        metrics.dequeued();
        metrics.busy();
        metrics.dropped();
        metrics.coalesced();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.depth, 0);
        assert_eq!(snapshot.capacity, 2);
        assert_eq!(snapshot.busy, 1);
        assert_eq!(snapshot.dropped, 1);
        assert_eq!(snapshot.coalesced, 1);
    }
}
