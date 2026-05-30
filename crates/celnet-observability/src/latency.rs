//! Latency recording over [`hdrhistogram`] with **coordinated-omission
//! correction** and a tail-percentile readout (p50/p99/p99.9/p99.99).
//!
//! Lives on the **drain / edge side**, never the hot path: the hot core only
//! pushes raw [`crate::record::HotSample`]s; the drain feeds their nanosecond
//! latencies into a [`LatencyRecorder`] here, where heap use and HdrHistogram
//! bookkeeping are perfectly fine.
//!
//! ## Coordinated omission
//!
//! A naive latency histogram under-reports the tail: when the system stalls, the
//! requests that *would have* been slow are never measured because the load
//! generator itself stalls. We correct for this with HdrHistogram's
//! `record_correct(value, expected_interval)` — when a sample exceeds the
//! expected inter-arrival interval, synthetic samples are back-filled down to
//! the interval, recovering the latency the stall hid. (Gil Tene, *How NOT to
//! Measure Latency*.)

use hdrhistogram::Histogram;

use crate::record::OpKind;

/// The canonical tail percentiles Celnet reports against its latency budgets.
pub const REPORTED_PERCENTILES: [f64; 5] = [50.0, 99.0, 99.9, 99.99, 100.0];

/// A latency recorder for one logical operation stream.
///
/// Wraps an `u64`-valued HdrHistogram (nanoseconds) configured for the FX-pricing
/// range with three significant figures of precision — accurate to 0.1 % at any
/// magnitude, which is finer than the budget margins in `docs/ARCHITECTURE.md`.
#[derive(Debug, Clone)]
pub struct LatencyRecorder {
    hist: Histogram<u64>,
    /// Expected inter-arrival interval (ns) used for coordinated-omission
    /// correction; `0` disables correction (free-running measurement).
    expected_interval_ns: u64,
}

impl LatencyRecorder {
    /// Highest trackable latency: 60 s in nanoseconds. Anything slower than a
    /// minute is pathological and clamps to this ceiling rather than panicking.
    const MAX_NS: u64 = 60_000_000_000;

    /// Create a recorder with no coordinated-omission correction (records exactly
    /// what it is given). Suitable for the in-core ticks→ns durations the drain
    /// already measured, where there is no separate arrival cadence to correct
    /// against.
    #[must_use]
    pub fn new() -> Self {
        Self::with_expected_interval(0)
    }

    /// Create a recorder that applies coordinated-omission correction against an
    /// `expected_interval_ns` arrival cadence (e.g. an RFS tick interval). A
    /// recorded sample larger than the interval back-fills the omitted samples.
    #[must_use]
    pub fn with_expected_interval(expected_interval_ns: u64) -> Self {
        // 3 sig-figs over [1 ns, 60 s]. `new_with_bounds` cannot fail for these
        // well-formed arguments, but we degrade gracefully rather than unwrap a
        // theoretical error in mission-critical code.
        let hist = Histogram::<u64>::new_with_bounds(1, Self::MAX_NS, 3)
            .unwrap_or_else(|_| Histogram::<u64>::new(3).expect("3 sig-figs is always valid"));
        Self {
            hist,
            expected_interval_ns,
        }
    }

    /// Record one latency observation in nanoseconds.
    ///
    /// When a coordinated-omission interval is configured, uses
    /// `record_correct` to back-fill omitted samples; otherwise records the raw
    /// value. Values above the ceiling are saturated to the ceiling (never
    /// panic on the hot tail).
    pub fn record_ns(&mut self, nanos: u64) {
        let v = nanos.clamp(1, Self::MAX_NS);
        if self.expected_interval_ns > 0 {
            // `record_correct` only errors on an out-of-range value, which the
            // clamp above prevents; ignore the impossible error path.
            let _ = self.hist.record_correct(v, self.expected_interval_ns);
        } else {
            let _ = self.hist.record(v);
        }
    }

    /// Number of recorded samples.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.hist.len()
    }

    /// `true` when nothing has been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hist.is_empty()
    }

    /// The value at a given percentile (ns). `0.0..=100.0`.
    #[must_use]
    pub fn percentile_ns(&self, q: f64) -> u64 {
        self.hist.value_at_quantile(q / 100.0)
    }

    /// 50th percentile (median) latency, ns.
    #[must_use]
    pub fn p50_ns(&self) -> u64 {
        self.percentile_ns(50.0)
    }

    /// 99th percentile latency, ns.
    #[must_use]
    pub fn p99_ns(&self) -> u64 {
        self.percentile_ns(99.0)
    }

    /// 99.9th percentile latency, ns.
    #[must_use]
    pub fn p999_ns(&self) -> u64 {
        self.percentile_ns(99.9)
    }

    /// 99.99th percentile latency, ns.
    #[must_use]
    pub fn p9999_ns(&self) -> u64 {
        self.percentile_ns(99.99)
    }

    /// Maximum recorded latency, ns.
    #[must_use]
    pub fn max_ns(&self) -> u64 {
        self.hist.max()
    }

    /// Minimum recorded latency, ns.
    #[must_use]
    pub fn min_ns(&self) -> u64 {
        self.hist.min()
    }

    /// Arithmetic mean latency, ns.
    #[must_use]
    pub fn mean_ns(&self) -> f64 {
        self.hist.mean()
    }

    /// A POD snapshot of the reported percentiles, safe to log/serialize on the
    /// drain side.
    #[must_use]
    pub fn snapshot(&self) -> LatencySnapshot {
        LatencySnapshot {
            count: self.count(),
            min_ns: if self.is_empty() { 0 } else { self.min_ns() },
            p50_ns: self.p50_ns(),
            p99_ns: self.p99_ns(),
            p999_ns: self.p999_ns(),
            p9999_ns: self.p9999_ns(),
            max_ns: self.max_ns(),
            mean_ns: self.mean_ns(),
        }
    }

    /// Reset the recorder to empty (e.g. at a reporting-interval boundary).
    pub fn clear(&mut self) {
        self.hist.clear();
    }
}

impl Default for LatencyRecorder {
    fn default() -> Self {
        Self::new()
    }
}

/// A `Copy` snapshot of a [`LatencyRecorder`]'s headline statistics — the shape
/// emitted to metrics/logs at each reporting boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencySnapshot {
    /// Number of samples in the window.
    pub count: u64,
    /// Minimum latency, ns.
    pub min_ns: u64,
    /// Median latency, ns.
    pub p50_ns: u64,
    /// 99th percentile latency, ns.
    pub p99_ns: u64,
    /// 99.9th percentile latency, ns.
    pub p999_ns: u64,
    /// 99.99th percentile latency, ns.
    pub p9999_ns: u64,
    /// Maximum latency, ns.
    pub max_ns: u64,
    /// Mean latency, ns.
    pub mean_ns: f64,
}

/// A fixed set of per-[`OpKind`] latency recorders, so the drain can aggregate
/// each operation stream independently against its own budget without any
/// hashing on the (already non-critical) drain path.
#[derive(Debug)]
pub struct LatencyByKind {
    recorders: [LatencyRecorder; OpKind::COUNT],
}

impl LatencyByKind {
    /// Create per-kind recorders, all free-running (no coordinated-omission
    /// correction).
    #[must_use]
    pub fn new() -> Self {
        Self {
            recorders: core::array::from_fn(|_| LatencyRecorder::new()),
        }
    }

    /// Record a latency under the given operation kind.
    pub fn record_ns(&mut self, kind: OpKind, nanos: u64) {
        self.recorders[kind.as_u16() as usize].record_ns(nanos);
    }

    /// Borrow the recorder for a kind.
    #[must_use]
    pub fn recorder(&self, kind: OpKind) -> &LatencyRecorder {
        &self.recorders[kind.as_u16() as usize]
    }

    /// Snapshot every kind in discriminant order.
    #[must_use]
    pub fn snapshots(&self) -> [(OpKind, LatencySnapshot); OpKind::COUNT] {
        core::array::from_fn(|i| {
            let kind = OpKind::from_u16(i as u16).expect("index in range");
            (kind, self.recorders[i].snapshot())
        })
    }
}

impl Default for LatencyByKind {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_on_known_uniform_sample() {
        // Record 1..=1000 ns. With 3 sig-figs the percentile values are exact to
        // 0.1%, so we assert within a small tolerance band.
        let mut r = LatencyRecorder::new();
        for v in 1..=1000u64 {
            r.record_ns(v);
        }
        assert_eq!(r.count(), 1000);
        // p50 ≈ 500, p99 ≈ 990, p99.9 ≈ 999.
        let near = |got: u64, want: u64, tol: u64| {
            assert!(
                got.abs_diff(want) <= tol,
                "p got {got}, want ~{want} (±{tol})"
            );
        };
        near(r.p50_ns(), 500, 3);
        near(r.p99_ns(), 990, 3);
        near(r.p999_ns(), 999, 3);
        assert_eq!(r.max_ns(), 1000);
        near(r.min_ns(), 1, 0);
        near(r.mean_ns() as u64, 500, 2);
    }

    #[test]
    fn monotone_percentiles() {
        let mut r = LatencyRecorder::new();
        for v in [10, 20, 30, 40, 50, 1000, 2000, 5000] {
            r.record_ns(v);
        }
        assert!(r.p50_ns() <= r.p99_ns());
        assert!(r.p99_ns() <= r.p999_ns());
        assert!(r.p999_ns() <= r.max_ns());
    }

    #[test]
    fn coordinated_omission_inflates_the_tail() {
        // A free-running recorder vs one with a 100 ns expected interval. Feed a
        // single huge 10_000 ns stall; the corrected recorder back-fills the
        // omitted samples, so its mean must exceed the naive one's.
        let mut naive = LatencyRecorder::new();
        let mut corrected = LatencyRecorder::with_expected_interval(100);
        for _ in 0..99 {
            naive.record_ns(100);
            corrected.record_ns(100);
        }
        naive.record_ns(10_000);
        corrected.record_ns(10_000);
        assert!(
            corrected.mean_ns() > naive.mean_ns(),
            "coordinated-omission correction must raise the apparent latency: \
             corrected={} naive={}",
            corrected.mean_ns(),
            naive.mean_ns()
        );
        // The correction also raises the count (back-filled synthetic samples).
        assert!(corrected.count() > naive.count());
    }

    #[test]
    fn clamps_extreme_values_without_panic() {
        let mut r = LatencyRecorder::new();
        r.record_ns(0); // clamped up to 1
        r.record_ns(u64::MAX); // clamped down to ceiling
        assert_eq!(r.count(), 2);
        // HdrHistogram reports the *highest value equivalent* to the recorded
        // bucket, which can sit a fraction above the clamp ceiling (bucket upper
        // bound); it must never exceed that bucket's highest-equivalent value.
        let ceiling_bucket_top = Histogram::<u64>::new_with_bounds(1, LatencyRecorder::MAX_NS, 3)
            .expect("valid bounds")
            .highest_equivalent(LatencyRecorder::MAX_NS);
        assert!(
            r.max_ns() <= ceiling_bucket_top,
            "max {} must be within the ceiling bucket top {ceiling_bucket_top}",
            r.max_ns()
        );
    }

    #[test]
    fn snapshot_matches_accessors() {
        let mut r = LatencyRecorder::new();
        for v in 1..=100u64 {
            r.record_ns(v);
        }
        let s = r.snapshot();
        assert_eq!(s.count, 100);
        assert_eq!(s.p50_ns, r.p50_ns());
        assert_eq!(s.p99_ns, r.p99_ns());
        assert_eq!(s.max_ns, r.max_ns());
    }

    #[test]
    fn per_kind_isolation() {
        let mut by = LatencyByKind::new();
        for _ in 0..10 {
            by.record_ns(OpKind::VanillaPrice, 100);
            by.record_ns(OpKind::ExoticPrice, 5000);
        }
        assert_eq!(by.recorder(OpKind::VanillaPrice).count(), 10);
        assert_eq!(by.recorder(OpKind::ExoticPrice).count(), 10);
        assert_eq!(by.recorder(OpKind::SurfaceVol).count(), 0);
        assert!(
            by.recorder(OpKind::VanillaPrice).p50_ns() < by.recorder(OpKind::ExoticPrice).p50_ns()
        );
        let snaps = by.snapshots();
        assert_eq!(snaps.len(), OpKind::COUNT);
        assert_eq!(snaps[OpKind::VanillaPrice.as_u16() as usize].1.count, 10);
    }
}
