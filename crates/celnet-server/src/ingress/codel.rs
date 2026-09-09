//! Controlled Delay (CoDel RFC 8289) & Anti-Coordinated Omission Queue (SOTA Scalability Phase 5).
//!
//! Eliminates bufferbloat and surfaces true end-to-end residence latency (sojourn time).
//!
//! # Problem Statement & Coordinated Omission (Gil Tene)
//! Standard latency benchmarks measure service time: from when an item is dequeued to when its
//! processing finishes. When inbound bursts cause bufferbloat, items sit in queue for milliseconds
//! before being processed in microseconds. Traditional metrics report sub-10 µs latency, completely
//! hiding the multi-millisecond queue latency (Coordinated Omission).
//!
//! # CoDel Mechanism (Nichols & Jacobson, RFC 8289)
//! 1. Every incoming packet is stamped with its entry timestamp.
//! 2. When dequeued, the queue calculates the item's sojourn time:
//!    $$T_{\text{sojourn}} = T_{\text{dequeue}} - T_{\text{enqueue}}$$
//! 3. If minimum sojourn time persistently exceeds `target_delay` throughout an entire `interval`,
//!    CoDel enters dropping state to drain the standing queue bufferbloat.
//! 4. Comprehensive sojourn latency percentiles (p50, p95, p99, max) are recorded accurately.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Configuration parameters for Controlled Delay (CoDel).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoDelConfig {
    /// Target acceptable sojourn delay (e.g. 5 ms or 500 µs).
    pub target_delay: Duration,
    /// Sliding observation interval (e.g. 100 ms or 10 ms).
    pub interval: Duration,
    /// Maximum physical capacity of the queue buffer.
    pub max_capacity: usize,
}

impl Default for CoDelConfig {
    fn default() -> Self {
        Self {
            target_delay: Duration::from_micros(500),
            interval: Duration::from_millis(10),
            max_capacity: 10_000,
        }
    }
}

/// A wrapped item carrying its ingress timestamp for sojourn tracking.
#[derive(Debug)]
struct StampedItem<T> {
    item: T,
    enqueued_at: Instant,
}

/// Point-in-time percentile statistics of queue sojourn times.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SojournLatencyStats {
    /// Total items that traversed the queue.
    pub total_samples: u64,
    /// Total items dropped to eliminate bufferbloat.
    pub codel_dropped: u64,
    /// Minimum sojourn time observed.
    pub min: Duration,
    /// Median sojourn time (50th percentile).
    pub p50: Duration,
    /// 95th percentile sojourn time.
    pub p95: Duration,
    /// 99th percentile sojourn time.
    pub p99: Duration,
    /// Maximum sojourn time observed.
    pub max: Duration,
}

/// Internal state tracking CoDel drop schedule and sojourn samples.
#[derive(Debug)]
struct CoDelState<T> {
    buffer: VecDeque<StampedItem<T>>,
    first_above_time: Option<Instant>,
    drop_next: Instant,
    count: u32,
    dropping: bool,
    codel_dropped: u64,
    sojourn_history: VecDeque<Duration>,
}

/// An ingress queue managed by Controlled Delay (CoDel) and sojourn tracking.
#[derive(Debug)]
pub struct CoDelQueue<T> {
    config: CoDelConfig,
    state: Mutex<CoDelState<T>>,
}

impl<T> CoDelQueue<T> {
    /// Create a new CoDel queue with the given configuration.
    #[must_use]
    pub fn new(config: CoDelConfig) -> Self {
        Self {
            config,
            state: Mutex::new(CoDelState {
                buffer: VecDeque::with_capacity(config.max_capacity),
                first_above_time: None,
                drop_next: Instant::now(),
                count: 0,
                dropping: false,
                codel_dropped: 0,
                sojourn_history: VecDeque::with_capacity(2048),
            }),
        }
    }

    /// Enqueue an item, stamping it with current time.
    /// Returns `true` if enqueued, or `false` if rejected due to reaching `max_capacity`.
    pub fn enqueue(&self, item: T) -> bool {
        let mut guard = self.state.lock().unwrap();
        if guard.buffer.len() >= self.config.max_capacity {
            return false;
        }
        guard.buffer.push_back(StampedItem {
            item,
            enqueued_at: Instant::now(),
        });
        true
    }

    /// Dequeue the next item, executing CoDel drop decisions to eliminate bufferbloat.
    pub fn dequeue(&self) -> Option<T> {
        let now = Instant::now();
        let mut guard = self.state.lock().unwrap();

        loop {
            let stamped = guard.buffer.pop_front()?;
            let sojourn = now.duration_since(stamped.enqueued_at);

            // Record sojourn sample in O(1) (retaining up to 100,000 recent samples)
            if guard.sojourn_history.len() >= 100_000 {
                guard.sojourn_history.pop_front();
            }
            guard.sojourn_history.push_back(sojourn);

            let is_above = sojourn > self.config.target_delay;

            if is_above {
                if guard.first_above_time.is_none() {
                    guard.first_above_time = Some(now + self.config.interval);
                } else if let Some(fat) = guard.first_above_time {
                    if now >= fat && !guard.dropping {
                        guard.dropping = true;
                        guard.count = 1;
                        guard.drop_next = self.control_law(now, guard.count);
                    }
                }
            } else {
                guard.first_above_time = None;
                guard.dropping = false;
            }

            // If in dropping state and drop deadline passed, shed this packet and continue!
            if guard.dropping && now >= guard.drop_next {
                guard.codel_dropped += 1;
                guard.count += 1;
                guard.drop_next = self.control_law(now, guard.count);
                // Drop this packet and loop to inspect next packet
                continue;
            }

            return Some(stamped.item);
        }
    }

    /// CoDel control law for inverse square-root pacing:
    /// $$t_{\text{next}} = t + \frac{\text{interval}}{\sqrt{\text{count}}}$$
    fn control_law(&self, now: Instant, count: u32) -> Instant {
        let count_f64 = (count as f64).max(1.0);
        let factor = self.config.interval.as_secs_f64() / count_f64.sqrt();
        now + Duration::from_secs_f64(factor)
    }

    /// Calculate sojourn latency percentiles to expose true queue residence without
    /// Coordinated Omission.
    #[must_use]
    pub fn latency_stats(&self) -> SojournLatencyStats {
        let guard = self.state.lock().unwrap();
        let total_samples = guard.sojourn_history.len() as u64;
        let codel_dropped = guard.codel_dropped;

        if guard.sojourn_history.is_empty() {
            return SojournLatencyStats {
                total_samples: 0,
                codel_dropped,
                min: Duration::ZERO,
                p50: Duration::ZERO,
                p95: Duration::ZERO,
                p99: Duration::ZERO,
                max: Duration::ZERO,
            };
        }

        let mut sorted: Vec<Duration> = guard.sojourn_history.iter().copied().collect();
        sorted.sort_unstable();

        let len = sorted.len();
        let min = sorted[0];
        let max = sorted[len - 1];
        let p50 = sorted[(len * 50) / 100];
        let p95 = sorted[(len * 95) / 100];
        let p99 = sorted[(len * 99) / 100];

        SojournLatencyStats {
            total_samples,
            codel_dropped,
            min,
            p50,
            p95,
            p99,
            max,
        }
    }

    /// Current queue length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.lock().unwrap().buffer.len()
    }

    /// Returns `true` if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
