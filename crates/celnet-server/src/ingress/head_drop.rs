//! Strict Head-Drop Ingress Ring Buffer (SOTA Scalability Phase 5).
//!
//! Eliminates bufferbloat and stale quote consumption during Pareto volume bursts.
//!
//! # Problem Statement
//! Traditional bounded FIFO queues employ tail-drop when saturated: incoming fresh quotes
//! are dropped while stale, buffered quotes continue to be processed by downstream engines.
//! In financial trading, this delivers toxic, out-of-date prices to market makers and consumers.
//!
//! # SOTA Head-Drop Semantics
//! When this ring buffer reaches capacity, incoming items displace the *oldest* unconsumed
//! items at the head of the buffer. Downstream consumers are guaranteed to consume only the
//! freshest market state.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Telemetry counters for queue throughput and shedding rates.
#[derive(Debug, Default)]
pub struct HeadDropStats {
    /// Total elements pushed to the queue.
    pub enqueued: AtomicU64,
    /// Total elements successfully popped by consumers.
    pub dequeued: AtomicU64,
    /// Total stale elements evicted from the head due to capacity overflow.
    pub head_dropped: AtomicU64,
}

impl HeadDropStats {
    /// Snapshot of the telemetry counters.
    #[must_use]
    pub fn snapshot(&self) -> HeadDropStatsSnapshot {
        HeadDropStatsSnapshot {
            enqueued: self.enqueued.load(Ordering::Relaxed),
            dequeued: self.dequeued.load(Ordering::Relaxed),
            head_dropped: self.head_dropped.load(Ordering::Relaxed),
        }
    }
}

/// Point-in-time snapshot of head-drop queue statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadDropStatsSnapshot {
    /// Total elements pushed to the queue.
    pub enqueued: u64,
    /// Total elements successfully popped by consumers.
    pub dequeued: u64,
    /// Total stale elements evicted from the head due to capacity overflow.
    pub head_dropped: u64,
}

/// A bounded ingress queue enforcing strict head-drop eviction on overflow.
#[derive(Debug)]
pub struct HeadDropQueue<T> {
    capacity: usize,
    buffer: Mutex<VecDeque<T>>,
    stats: HeadDropStats,
}

impl<T> HeadDropQueue<T> {
    /// Create a new head-drop queue with bounded capacity.
    ///
    /// # Panics
    /// Panics if `capacity == 0`.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be greater than zero");
        Self {
            capacity,
            buffer: Mutex::new(VecDeque::with_capacity(capacity)),
            stats: HeadDropStats::default(),
        }
    }

    /// Maximum capacity of the queue.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Push an item into the queue.
    ///
    /// If the queue is at capacity, the oldest item at the head is evicted and returned,
    /// ensuring that only fresh items remain buffered.
    pub fn push(&self, item: T) -> Option<T> {
        let mut guard = self.buffer.lock().unwrap();
        self.stats.enqueued.fetch_add(1, Ordering::Relaxed);

        let evicted = if guard.len() >= self.capacity {
            self.stats.head_dropped.fetch_add(1, Ordering::Relaxed);
            guard.pop_front()
        } else {
            None
        };

        guard.push_back(item);
        evicted
    }

    /// Dequeue the oldest available item from the head of the queue.
    pub fn pop(&self) -> Option<T> {
        let mut guard = self.buffer.lock().unwrap();
        let item = guard.pop_front();
        if item.is_some() {
            self.stats.dequeued.fetch_add(1, Ordering::Relaxed);
        }
        item
    }

    /// Current number of elements in the queue.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.lock().unwrap().len()
    }

    /// Returns `true` if the queue currently contains no items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Access queue telemetry counters.
    #[must_use]
    pub fn stats(&self) -> &HeadDropStats {
        &self.stats
    }
}
