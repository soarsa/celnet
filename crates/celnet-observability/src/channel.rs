//! The hot-path-safe telemetry transport: a **bounded wait-free SPSC ring**
//! (`rtrb`) carrying [`HotSample`] POD records from a single producer on the
//! pinned hot core to a single consumer on a non-critical core.
//!
//! ## Hot-path contract
//!
//! The producer side ([`HotProbe`]) is the only thing the engine touches on the
//! pinned core. A publish is:
//!
//! * **lock-free** — `rtrb` is a wait-free single-producer/single-consumer ring;
//! * **allocation-free** — the ring's storage is pre-allocated once at
//!   construction; pushing a `Copy` POD record never touches the heap;
//! * **drop-on-full** — telemetry is *lossy by design*: if the consumer falls
//!   behind, the producer drops the sample and bumps a `Relaxed` dropped-counter
//!   rather than blocking the hot core. (The audit log uses a separate, lossless
//!   path — see [`crate::logging`].)
//!
//! The dropped counter lives in a [`CachePadded`] cell so the producer's
//! `Relaxed` store can never false-share with the consumer's reads.
//!
//! Provenance: the lossy-telemetry / busy-core-offload split is the
//! "disruptor"-style pattern (LMAX) adapted to a wait-free SPSC ring; the
//! cache-line isolation guards against false sharing à la Drepper, *What Every
//! Programmer Should Know About Memory* (2007).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_utils::CachePadded;

use crate::record::HotSample;

/// Counters shared between the producer and the drain, each on its own cache
/// line so a `Relaxed` producer store never false-shares with a consumer read.
#[derive(Debug)]
struct Shared {
    /// Total samples the producer *attempted* to publish.
    published: CachePadded<AtomicU64>,
    /// Samples dropped because the ring was full (lossy-telemetry accounting).
    dropped: CachePadded<AtomicU64>,
}

impl Shared {
    fn new() -> Self {
        Self {
            published: CachePadded::new(AtomicU64::new(0)),
            dropped: CachePadded::new(AtomicU64::new(0)),
        }
    }
}

/// The producer end of the telemetry ring — the **only** observability object
/// the engine calls on the pinned hot core.
///
/// Construct it with [`telemetry_channel`]; it is `Send` but **not** `Sync`
/// (single-producer), so it is moved onto the hot thread and never shared.
#[derive(Debug)]
pub struct HotProbe {
    tx: rtrb::Producer<HotSample>,
    shared: Arc<Shared>,
    /// Monotonic publish sequence, stamped into each sample so the drain can
    /// detect ring-overrun gaps. Local to the producer — no atomics needed.
    seq: u64,
}

impl HotProbe {
    /// Publish one telemetry sample. **Hot-path safe**: wait-free, lock-free,
    /// allocation-free; on a full ring the sample is dropped and the drop
    /// counter bumped (`Relaxed`) rather than blocking.
    ///
    /// Returns `true` if the sample was enqueued, `false` if it was dropped.
    #[inline]
    pub fn publish(&mut self, mut sample: HotSample) -> bool {
        self.seq = self.seq.wrapping_add(1);
        sample.seq = self.seq;
        // `push` only fails when the ring is full; it never allocates.
        match self.tx.push(sample) {
            Ok(()) => {
                self.shared.published.store(self.seq, Ordering::Relaxed);
                true
            }
            Err(_full) => {
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Number of samples successfully published so far (the last stamped
    /// sequence). `Relaxed`; for diagnostics, not synchronization.
    #[must_use]
    pub fn published(&self) -> u64 {
        self.shared.published.load(Ordering::Relaxed)
    }

    /// Number of samples dropped due to a full ring (lossy-telemetry counter).
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }
}

// A producer is moved to the hot thread and used from there only.
// SPSC: not shared across threads, hence `!Sync` is appropriate and automatic
// (rtrb::Producer is `Send + !Sync`).

/// The consumer end of the telemetry ring, run on a **non-critical** core. It
/// drains POD samples and hands them to a sink for formatting/aggregation —
/// none of which ever touches the hot path.
#[derive(Debug)]
pub struct TelemetryDrain {
    rx: rtrb::Consumer<HotSample>,
    shared: Arc<Shared>,
    /// The last sequence the drain has seen, used to count overrun gaps.
    last_seq: u64,
    /// Total samples drained.
    drained: u64,
    /// Sequence-gap count (samples the producer dropped, observed as holes).
    gaps: u64,
}

impl TelemetryDrain {
    /// Drain at most `budget` samples, invoking `sink` for each in FIFO order.
    /// Returns the number drained. Bounded work — the drain never spins
    /// unboundedly, so it co-operates with whatever scheduler runs it.
    ///
    /// This performs the formatting/aggregation the hot path is forbidden from
    /// doing; allocation here is fine (non-critical core).
    pub fn drain<F: FnMut(HotSample)>(&mut self, budget: usize, mut sink: F) -> usize {
        let mut n = 0;
        while n < budget {
            match self.rx.pop() {
                Ok(sample) => {
                    self.account(sample.seq);
                    self.drained += 1;
                    sink(sample);
                    n += 1;
                }
                Err(_empty) => break,
            }
        }
        n
    }

    /// Drain everything currently available (formatting via `sink`), bounded by
    /// the ring capacity, returning the count drained.
    pub fn drain_all<F: FnMut(HotSample)>(&mut self, sink: F) -> usize {
        // `slots()` is an upper bound on currently-readable items; using it as
        // the budget keeps this bounded and prevents a live-lock against a
        // continuously-producing hot core.
        let budget = self.rx.slots();
        self.drain(budget, sink)
    }

    fn account(&mut self, seq: u64) {
        // Sequences are stamped 1,2,3,…; a jump implies the producer dropped
        // (ring was full) between our last observation and this one.
        let expected = self.last_seq.wrapping_add(1);
        if self.drained != 0 && seq != expected {
            self.gaps = self.gaps.saturating_add(seq.wrapping_sub(expected));
        }
        self.last_seq = seq;
    }

    /// Total samples drained over the lifetime of this drain.
    #[must_use]
    pub fn drained(&self) -> u64 {
        self.drained
    }

    /// Observed sequence-gap count (samples the producer dropped on a full
    /// ring, inferred from holes in the sequence).
    #[must_use]
    pub fn observed_gaps(&self) -> u64 {
        self.gaps
    }

    /// Samples the producer reports having dropped (the authoritative
    /// drop counter, read across the cache-padded boundary).
    #[must_use]
    pub fn producer_dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// `true` when no samples are currently waiting in the ring.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }
}

/// Create a bounded SPSC telemetry channel with room for `capacity` samples.
///
/// All ring storage is pre-allocated here, **once**, so neither end allocates
/// thereafter. `capacity` is rounded up internally by `rtrb` to fit its layout;
/// pick a power of two sized to absorb a drain-scheduling jitter window.
///
/// Returns the `(producer, drain)` pair. Move the [`HotProbe`] onto the pinned
/// hot thread and the [`TelemetryDrain`] onto a non-critical core.
#[must_use]
pub fn telemetry_channel(capacity: usize) -> (HotProbe, TelemetryDrain) {
    let cap = capacity.max(1);
    let (tx, rx) = rtrb::RingBuffer::<HotSample>::new(cap);
    let shared = Arc::new(Shared::new());
    let probe = HotProbe {
        tx,
        shared: Arc::clone(&shared),
        seq: 0,
    };
    let drain = TelemetryDrain {
        rx,
        shared,
        last_seq: 0,
        drained: 0,
        gaps: 0,
    };
    (probe, drain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{ErrorClass, OpKind};

    #[test]
    fn roundtrips_pod_records_in_order() {
        let (mut probe, mut drain) = telemetry_channel(64);
        for i in 0..10u64 {
            let s = HotSample::new(i, OpKind::VanillaPrice, i * 100, 3);
            assert!(probe.publish(s));
        }
        let mut seen = Vec::new();
        let n = drain.drain_all(|s| seen.push(s));
        assert_eq!(n, 10);
        assert_eq!(seen.len(), 10);
        for (i, s) in seen.iter().enumerate() {
            assert_eq!(s.request_id, i as u64);
            assert_eq!(s.elapsed_ticks, i as u64 * 100);
            assert_eq!(s.seq, i as u64 + 1);
            assert_eq!(s.kind, OpKind::VanillaPrice);
        }
        assert_eq!(drain.drained(), 10);
        assert_eq!(drain.observed_gaps(), 0);
        assert_eq!(probe.dropped(), 0);
    }

    #[test]
    fn drops_on_full_and_counts() {
        // Tiny ring so we overflow it. Burst-publish to overflow, then partially
        // drain, then burst again: this leaves the *accepted* stream with holes
        // (the producer advances `seq` even on a dropped push), which the drain
        // detects as gaps.
        let (mut probe, mut drain) = telemetry_channel(4);
        let mut accepted = 0u64;
        let mut rejected = 0u64;
        let mut drained_total = 0usize;

        for round in 0..8 {
            // Overflow the ring on each round.
            for i in 0..8u64 {
                if probe.publish(HotSample::new(round * 8 + i, OpKind::StreamQuote, 1, 0)) {
                    accepted += 1;
                } else {
                    rejected += 1;
                }
            }
            // Drain only part of it so subsequent rounds keep overflowing.
            drained_total += drain.drain(2, |_| {});
        }
        drained_total += drain.drain_all(|_| {});

        assert!(rejected > 0, "a size-4 ring must reject some pushes");
        assert_eq!(probe.dropped(), rejected);
        assert_eq!(drained_total as u64, accepted);
        assert_eq!(drain.producer_dropped(), rejected);
        // The accepted stream had holes (dropped seqs), so the drain observed
        // gaps.
        assert!(
            drain.observed_gaps() > 0,
            "interleaved drops ({rejected}) must leave observable sequence gaps"
        );
    }

    #[test]
    fn bounded_drain_respects_budget() {
        let (mut probe, mut drain) = telemetry_channel(64);
        for i in 0..20u64 {
            probe.publish(HotSample::new(i, OpKind::SurfaceVol, 1, 0));
        }
        let first = drain.drain(5, |_| {});
        assert_eq!(first, 5);
        let rest = drain.drain_all(|_| {});
        assert_eq!(rest, 15);
        assert!(drain.is_empty());
    }

    #[test]
    fn carries_error_class() {
        let (mut probe, mut drain) = telemetry_channel(8);
        probe.publish(
            HotSample::new(7, OpKind::ExoticPrice, 9, 1).with_class(ErrorClass::NoConvergence),
        );
        let mut got = None;
        drain.drain_all(|s| got = Some(s));
        assert_eq!(got.unwrap().class, ErrorClass::NoConvergence);
    }
}
