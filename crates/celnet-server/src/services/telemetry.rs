//! The latency/ops telemetry hub — the drain-side aggregation store that wires
//! the previously-built-but-unwired `celnet-observability` capture primitives
//! into the running server (`docs/LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md`
//! §0/§5). It owns:
//!
//! * a shared drain-side [`LatencyByKind`] — one HdrHistogram recorder per
//!   [`OpKind`], recorded into by **two producer tiers**, both off the pinned
//!   pricing hot core (guardrail 11):
//!   1. the **pinned engine core** publishes a 32-byte POD [`HotSample`] through
//!      the lossy wait-free SPSC ring; the hub [`drain`](TelemetryHub::drain_hot)s
//!      it on a non-critical core, converts opaque ticks → ns via a calibrated
//!      [`TickRate`], and records it (this is the L1/L2 tick-to-quote price
//!      stage);
//!   2. **async-edge stages** (tiering, consolidation, quote-publish, RFQ
//!      respond, booking, …) record straight into the same aggregator via
//!      [`record_edge`](TelemetryHub::record_edge) — the exact `stream.rs:647`
//!      pattern generalised, no ring hop needed because they are already off the
//!      pricing thread.
//!
//! All HdrHistogram bookkeeping, tick→ns conversion and the metrics-facade
//! (`record_op`) emission run here on the drain tier; **the pinned core only ever
//! reads the cycle counter and pushes a POD** (see `core_link::drain_timed`).

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_observability::{
    HotProbe, LatencyByKind, LatencySnapshot, OpKind, TelemetryDrain, TickRate, metrics_facade,
    telemetry_channel,
};

/// The number of samples the pinned-core → drain SPSC telemetry ring can hold
/// before it drops (lossy by design). Sized to absorb a drain-scheduling jitter
/// window at IB-scale price rates; a full ring drops-and-counts, never blocks.
const RING_CAPACITY: usize = 8192;

/// A `Copy` per-stage latency readout, projected onto the wire by the RPC.
#[derive(Debug, Clone, Copy)]
pub struct StageStat {
    /// The operation kind (stable metric label via [`OpKind::label`]).
    pub kind: OpKind,
    /// The HdrHistogram snapshot (count + p50/p99/p999/p9999/min/max/mean).
    pub snapshot: LatencySnapshot,
}

/// Ring / drain health, surfaced beside the per-stage stats so an operator can
/// see telemetry loss (a full ring dropping samples) at a glance.
#[derive(Debug, Clone, Copy, Default)]
pub struct TelemetryHealth {
    /// Total POD samples drained from the SPSC ring over the process lifetime.
    pub drained_total: u64,
    /// Total samples the pinned-core producer dropped on a full ring.
    pub dropped_total: u64,
    /// Sequence-gaps the drain observed (holes from producer drops).
    pub observed_gaps: u64,
    /// The calibrated cycle-counter frequency (ticks/sec) used for tick→ns.
    pub tick_hz: u64,
}

/// The shared telemetry aggregation hub (behind an `Arc`).
#[derive(Debug)]
pub struct TelemetryHub {
    /// Drain-side per-`OpKind` HdrHistogram recorders (both producer tiers feed
    /// this). A short-critical-section `Mutex`: every access is a single record
    /// or a snapshot, never held across `.await`.
    by_kind: Mutex<LatencyByKind>,
    /// The consumer end of the pinned-core → drain ring, installed once by
    /// `CoreLink` at startup. `None` until installed (edge-only capture still
    /// works before/without it).
    drain: Mutex<Option<TelemetryDrain>>,
    /// Calibrated ticks → ns conversion for the pinned-core samples.
    tick_rate: TickRate,
    /// The cycle-counter frequency (Hz) — for the health readout.
    tick_hz: u64,
    /// Lifetime samples drained from the ring.
    drained_total: AtomicU64,
    /// Absolute producer drop count (mirrors the ring's cache-padded counter).
    dropped_total: AtomicU64,
    /// Observed sequence gaps.
    observed_gaps: AtomicU64,
}

impl TelemetryHub {
    /// Build a hub calibrated to a cycle-counter frequency (from
    /// `celnet_engine::tick_hz()`). A zero / invalid frequency degrades to a 1 GHz
    /// rate rather than an ill-defined conversion.
    #[must_use]
    pub fn new(tick_hz: u64) -> Self {
        let tick_rate = TickRate::from_hz(tick_hz)
            .or_else(|| TickRate::from_hz(1_000_000_000))
            .expect("1 GHz is always a valid tick rate");
        Self {
            by_kind: Mutex::new(LatencyByKind::new()),
            drain: Mutex::new(None),
            tick_rate,
            tick_hz: tick_rate.hz(),
            drained_total: AtomicU64::new(0),
            dropped_total: AtomicU64::new(0),
            observed_gaps: AtomicU64::new(0),
        }
    }

    /// Create the pinned-core → drain SPSC ring, install the drain end into the
    /// hub, and return the [`HotProbe`] producer for the caller to move onto the
    /// pinned pricing core thread. Called once by `CoreLink::start`.
    #[must_use]
    pub fn install_hot_ring(&self) -> HotProbe {
        let (probe, drain) = telemetry_channel(RING_CAPACITY);
        *self
            .drain
            .lock()
            .expect("telemetry drain mutex not poisoned") = Some(drain);
        probe
    }

    /// Record an async-edge stage latency (nanoseconds) under `kind` — the
    /// `stream.rs:647` pattern. Also emits the vendor-neutral `record_op` metric.
    /// Cheap and off the pricing thread; the caller brackets the stage with a
    /// monotonic `Instant`.
    pub fn record_edge(&self, kind: OpKind, nanos: u64) {
        self.by_kind
            .lock()
            .expect("telemetry by_kind mutex not poisoned")
            .record_ns(kind, nanos);
        metrics_facade::record_op(kind, celnet_observability::ErrorClass::Ok, nanos);
    }

    /// Drain the pinned-core ring (bounded, non-blocking): pop each POD sample,
    /// convert its opaque ticks → ns via the calibrated [`TickRate`], record it
    /// into the shared aggregator, and emit the `record_op` metric. Runs on a
    /// non-critical core (a periodic task), never on the pricing thread. Returns
    /// the number of samples drained this cycle.
    pub fn drain_hot(&self) -> usize {
        let mut drain_guard = self
            .drain
            .lock()
            .expect("telemetry drain mutex not poisoned");
        let Some(drain) = drain_guard.as_mut() else {
            return 0;
        };
        let rate = self.tick_rate;
        let n = {
            let mut by = self.by_kind.lock().expect("by_kind mutex not poisoned");
            drain.drain_all(|sample| {
                let ns = rate.ticks_to_nanos(sample.elapsed_ticks);
                by.record_ns(sample.kind, ns);
                metrics_facade::record_op(sample.kind, sample.class, ns);
            })
        };
        if n > 0 {
            self.drained_total.fetch_add(n as u64, Ordering::Relaxed);
            metrics_facade::record_drained(n as u64);
        }
        // Mirror the authoritative producer drop counter + observed gaps.
        let dropped = drain.producer_dropped();
        let prev = self.dropped_total.swap(dropped, Ordering::Relaxed);
        if dropped > prev {
            metrics_facade::record_dropped(dropped - prev);
        }
        self.observed_gaps
            .store(drain.observed_gaps(), Ordering::Relaxed);
        n
    }

    /// A snapshot of every instrumented stage that has recorded at least one
    /// sample, in `OpKind` discriminant order (stable). Empty stages are omitted
    /// so the workspace shows exactly what the running server captured.
    #[must_use]
    pub fn stage_stats(&self) -> Vec<StageStat> {
        let by = self.by_kind.lock().expect("by_kind mutex not poisoned");
        by.snapshots()
            .into_iter()
            .filter(|(_, snap)| snap.count > 0)
            .map(|(kind, snapshot)| StageStat { kind, snapshot })
            .collect()
    }

    /// The current ring / drain health readout.
    #[must_use]
    pub fn health(&self) -> TelemetryHealth {
        TelemetryHealth {
            drained_total: self.drained_total.load(Ordering::Relaxed),
            dropped_total: self.dropped_total.load(Ordering::Relaxed),
            observed_gaps: self.observed_gaps.load(Ordering::Relaxed),
            tick_hz: self.tick_hz,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_observability::{ErrorClass, HotSample};

    #[test]
    fn edge_record_folds_to_expected_percentiles() {
        let hub = TelemetryHub::new(1_000_000_000);
        // Record a known uniform 1..=1000 ns spread into the tiering stage.
        for v in 1..=1000u64 {
            hub.record_edge(OpKind::TieringRun, v);
        }
        let stats = hub.stage_stats();
        let tier = stats
            .iter()
            .find(|s| s.kind == OpKind::TieringRun)
            .expect("tiering stage present");
        assert_eq!(tier.snapshot.count, 1000);
        // p50 ≈ 500, p99 ≈ 990 (3 sig-fig HdrHistogram, ±3 tol).
        assert!(tier.snapshot.p50_ns.abs_diff(500) <= 3);
        assert!(tier.snapshot.p99_ns.abs_diff(990) <= 3);
        assert!(tier.snapshot.p50_ns <= tier.snapshot.p99_ns);
        assert!(tier.snapshot.p99_ns <= tier.snapshot.p999_ns);
        // Only the one instrumented stage shows up.
        assert_eq!(stats.len(), 1);
    }

    #[test]
    fn pinned_ring_drains_ticks_to_expected_ns() {
        // 1 GHz ⇒ 1 tick == 1 ns exactly, so injected ticks land as ns.
        let hub = TelemetryHub::new(1_000_000_000);
        let mut probe = hub.install_hot_ring();
        for _ in 0..500 {
            assert!(probe.publish(
                HotSample::new(1, OpKind::VanillaPrice, 800, 0).with_class(ErrorClass::Ok)
            ));
        }
        let drained = hub.drain_hot();
        assert_eq!(drained, 500);
        let stats = hub.stage_stats();
        let price = stats
            .iter()
            .find(|s| s.kind == OpKind::VanillaPrice)
            .expect("price stage present");
        assert_eq!(price.snapshot.count, 500);
        // Every sample was 800 ticks == 800 ns at 1 GHz.
        assert!(price.snapshot.p50_ns.abs_diff(800) <= 1);
        assert_eq!(hub.health().drained_total, 500);
        assert_eq!(hub.health().dropped_total, 0);
        assert_eq!(hub.health().tick_hz, 1_000_000_000);
    }

    #[test]
    fn drain_without_ring_is_a_noop() {
        let hub = TelemetryHub::new(24_000_000);
        assert_eq!(hub.drain_hot(), 0);
        assert!(hub.stage_stats().is_empty());
        assert_eq!(hub.health().tick_hz, 24_000_000);
    }
}
