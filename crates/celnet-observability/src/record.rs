//! The hot-path telemetry vocabulary: a `Copy` POD sample, the operation kind
//! it describes, and the structured error taxonomy — all encoded as small
//! integers so a producer on the pinned core can publish a sample with **zero
//! allocation, no lock, and no formatting** (the formatting/aggregation happens
//! later on the drain core; see [`crate::channel`]).
//!
//! Nothing here allocates, calls a runtime, or reads a wall clock. Time is
//! supplied to the probe as an opaque, monotonic `u64` *tick* by the caller (the
//! engine reads the architectural cycle counter — `cntvct_el0` / `rdtsc` —
//! outside this crate); we never call `std::time` on the hot path. Elapsed ticks
//! are converted to nanoseconds only on the drain side via a calibrated
//! [`TickRate`].

use core::fmt;

use serde::{Deserialize, Serialize};

/// Which engine operation a [`HotSample`] measures.
///
/// `#[repr(u16)]` so it rides inside the POD sample as a plain integer — no
/// pointer, no string, no allocation on the hot path. The drain side maps it
/// back to a stable label for metrics/logs. It also derives serde so it can ride
/// in the (off-hot-path, allocation-allowed) audit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u16)]
pub enum OpKind {
    /// A vanilla (Garman-Kohlhagen) price + Greeks evaluation.
    VanillaPrice = 0,
    /// A volatility-surface point read / smile evaluation.
    SurfaceVol = 1,
    /// A barrier / touch / digital exotic evaluation.
    ExoticPrice = 2,
    /// A request-for-stream (RFS) quote tick produced for a subscriber.
    StreamQuote = 3,
    /// A request-for-quote (RFQ) priced at the edge.
    RfqQuote = 4,
    /// A market-state republication (blue-green hot-swap / surface mark).
    StatePublish = 5,
    // --- Async-edge stages (drain-side `LatencyByKind` producers, never the
    //     pinned ring; see `docs/LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md`
    //     §4.1/§4.2). They share the same aggregator shape as the pinned-core
    //     kinds above, but are recorded straight into a shared drain-side
    //     recorder from the already-non-critical async edge. ---
    /// Spread / tiering feature-pipeline application (mid-shift → tier → guard).
    TieringRun = 6,
    /// Aggregation / consolidation of an inbound book into a composite quote.
    Consolidate = 7,
    /// Quote publication on the streaming edge (snapshot/update assembled).
    StreamPublish = 8,
    /// RFQ receive → price → respond round trip on the quote edge.
    RfqRespond = 9,
    /// Quote → lift / acceptance handling on the quote/desk edge.
    QuoteAccept = 10,
    /// Booking commit (ack → fill → book) into a position store.
    Book = 11,
    /// Risk-routing decision-graph evaluation for an accepted trade.
    RiskRoute = 12,
    /// Auto-hedge fire (threshold breach → hedge action) on the booking tier.
    HedgeFire = 13,
}

impl OpKind {
    /// The number of distinct kinds; used to size per-kind aggregation arrays.
    pub const COUNT: usize = 14;

    /// Stable, vendor-neutral label for metrics and structured logs.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            OpKind::VanillaPrice => "vanilla_price",
            OpKind::SurfaceVol => "surface_vol",
            OpKind::ExoticPrice => "exotic_price",
            OpKind::StreamQuote => "stream_quote",
            OpKind::RfqQuote => "rfq_quote",
            OpKind::StatePublish => "state_publish",
            OpKind::TieringRun => "tiering_run",
            OpKind::Consolidate => "consolidate",
            OpKind::StreamPublish => "stream_publish",
            OpKind::RfqRespond => "rfq_respond",
            OpKind::QuoteAccept => "quote_accept",
            OpKind::Book => "book",
            OpKind::RiskRoute => "risk_route",
            OpKind::HedgeFire => "hedge_fire",
        }
    }

    /// A human-facing stage label for the Latency/Ops workspace (the tick-to-quote
    /// and tick-to-trade stage decomposition). Distinct from [`Self::label`] (the
    /// stable metric key): this is for display only.
    #[must_use]
    pub const fn stage_label(self) -> &'static str {
        match self {
            OpKind::VanillaPrice => "Price (pinned core)",
            OpKind::SurfaceVol => "Surface / curve rebuild",
            OpKind::ExoticPrice => "Exotic price",
            OpKind::StreamQuote => "Stream quote",
            OpKind::StatePublish => "State publish",
            OpKind::TieringRun => "Spread / tiering",
            OpKind::Consolidate => "Aggregation / consolidation",
            OpKind::StreamPublish => "Quote publish (tick→quote)",
            OpKind::RfqQuote => "RFQ price",
            OpKind::RfqRespond => "RFQ receive→respond",
            OpKind::QuoteAccept => "Quote→lift / accept",
            OpKind::Book => "Ack→fill→book",
            OpKind::RiskRoute => "Risk routing",
            OpKind::HedgeFire => "Auto-hedge fire",
        }
    }

    /// Reconstruct a kind from its discriminant, returning `None` for an
    /// out-of-range value (defensive: the drain never trusts a corrupt sample).
    #[must_use]
    pub const fn from_u16(v: u16) -> Option<Self> {
        match v {
            0 => Some(OpKind::VanillaPrice),
            1 => Some(OpKind::SurfaceVol),
            2 => Some(OpKind::ExoticPrice),
            3 => Some(OpKind::StreamQuote),
            4 => Some(OpKind::RfqQuote),
            5 => Some(OpKind::StatePublish),
            6 => Some(OpKind::TieringRun),
            7 => Some(OpKind::Consolidate),
            8 => Some(OpKind::StreamPublish),
            9 => Some(OpKind::RfqRespond),
            10 => Some(OpKind::QuoteAccept),
            11 => Some(OpKind::Book),
            12 => Some(OpKind::RiskRoute),
            13 => Some(OpKind::HedgeFire),
            _ => None,
        }
    }

    /// The kind as its raw discriminant.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self as u16
    }
}

impl fmt::Display for OpKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Structured outcome / error taxonomy carried as a discriminant in the POD
/// sample — so the hot path classifies an outcome with an integer, never a
/// `String` and never an allocation.
///
/// The drain side and the edge map this deterministically onto log fields and
/// (in `celnet-server`) gRPC status codes. The variants are *outcomes of a
/// pricing operation*, not transport errors; transport lives at the async edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ErrorClass {
    /// The operation completed successfully.
    Ok = 0,
    /// Inputs were individually valid but jointly inadmissible (e.g. negative
    /// time to expiry, non-positive forward) — a caller contract violation.
    InvalidInput = 1,
    /// The referenced surface/market state was stale or absent at evaluation
    /// time (e.g. a tick arrived before the first surface mark).
    StaleState = 2,
    /// A numerical method failed to converge within its iteration/tolerance
    /// budget (root solve, PDE/MC, implied-vol inversion).
    NoConvergence = 3,
    /// The requested point violated a no-arbitrage constraint on the surface.
    Arbitrage = 4,
    /// The telemetry ring was full and this (or a prior) sample was dropped —
    /// telemetry is lossy by design; this records the fact without blocking.
    TelemetryDropped = 5,
    /// An internal invariant was violated (should be unreachable in production;
    /// surfaced rather than panicking on the hot path).
    Internal = 6,
}

impl ErrorClass {
    /// The number of distinct classes.
    pub const COUNT: usize = 7;

    /// `true` for the success class only.
    #[must_use]
    pub const fn is_ok(self) -> bool {
        matches!(self, ErrorClass::Ok)
    }

    /// Stable, vendor-neutral label for metrics and structured logs.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            ErrorClass::Ok => "ok",
            ErrorClass::InvalidInput => "invalid_input",
            ErrorClass::StaleState => "stale_state",
            ErrorClass::NoConvergence => "no_convergence",
            ErrorClass::Arbitrage => "arbitrage",
            ErrorClass::TelemetryDropped => "telemetry_dropped",
            ErrorClass::Internal => "internal",
        }
    }

    /// Reconstruct from a discriminant; out-of-range maps to `None`.
    #[must_use]
    pub const fn from_u16(v: u16) -> Option<Self> {
        match v {
            0 => Some(ErrorClass::Ok),
            1 => Some(ErrorClass::InvalidInput),
            2 => Some(ErrorClass::StaleState),
            3 => Some(ErrorClass::NoConvergence),
            4 => Some(ErrorClass::Arbitrage),
            5 => Some(ErrorClass::TelemetryDropped),
            6 => Some(ErrorClass::Internal),
            _ => None,
        }
    }

    /// The class as its raw discriminant.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self as u16
    }
}

impl fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A single hot-path telemetry observation: **`Copy`, POD, 32 bytes, no
/// pointers** — safe to publish through a wait-free SPSC ring with zero
/// allocation and to copy cheaply on the drain side.
///
/// The `elapsed_ticks` field is a *duration* in opaque monotonic ticks (the
/// difference of two cycle-counter reads taken by the caller), converted to
/// nanoseconds only on the drain via a calibrated [`TickRate`]. The hot path
/// never divides, never formats, and never touches a wall clock to build one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct HotSample {
    /// The request id the sample pertains to (echoes the engine's request id),
    /// for correlation with the audit log and the wire response.
    pub request_id: u64,
    /// A monotonically-increasing publish sequence stamped by the producer, used
    /// by the drain to detect ring-overrun gaps without a wall clock.
    pub seq: u64,
    /// Elapsed duration of the measured operation, in opaque monotonic ticks.
    pub elapsed_ticks: u64,
    /// Which operation this measures.
    pub kind: OpKind,
    /// The operation outcome / error class.
    pub class: ErrorClass,
    /// The core id the producer was pinned to (for per-core aggregation); the
    /// engine supplies it once at setup, so reading it is free on the hot path.
    pub core_id: u16,
    /// Reserved padding to keep the record a tidy power-of-two size and POD with
    /// no implicit padding bytes (deterministic, `bytemuck`-able if ever needed).
    pub _pad: u16,
}

// Compile-time guarantees that the record stays small and POD-shaped.
const _: () = {
    assert!(core::mem::size_of::<HotSample>() == 32);
    assert!(core::mem::align_of::<HotSample>() == 8);
};

impl HotSample {
    /// Construct a successful-by-default sample. All fields are plain integers,
    /// so this is a pure stack write with no allocation.
    #[inline]
    #[must_use]
    pub const fn new(request_id: u64, kind: OpKind, elapsed_ticks: u64, core_id: u16) -> Self {
        Self {
            request_id,
            seq: 0,
            elapsed_ticks,
            kind,
            class: ErrorClass::Ok,
            core_id,
            _pad: 0,
        }
    }

    /// Builder-style setter for the outcome class (consumes and returns `self`).
    #[inline]
    #[must_use]
    pub const fn with_class(mut self, class: ErrorClass) -> Self {
        self.class = class;
        self
    }
}

impl Default for HotSample {
    fn default() -> Self {
        Self {
            request_id: 0,
            seq: 0,
            elapsed_ticks: 0,
            kind: OpKind::VanillaPrice,
            class: ErrorClass::Ok,
            core_id: 0,
            _pad: 0,
        }
    }
}

/// A calibrated conversion from opaque monotonic ticks to nanoseconds, applied
/// only on the **drain side** (never on the hot path).
///
/// On Apple silicon the system counter (`cntvct_el0`) ticks at a fixed
/// frequency (`cntfrq_el0`, commonly 24 MHz); on x86 the invariant-TSC frequency
/// is measured once at startup. The caller supplies the measured frequency; we
/// store nanoseconds-per-tick as an exact rational to avoid drift, and round at
/// the point of use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickRate {
    ticks_per_sec: u64,
}

impl TickRate {
    /// Build from a measured tick frequency (ticks per second). Returns `None`
    /// for a zero frequency (which would make conversion ill-defined).
    #[must_use]
    pub const fn from_hz(ticks_per_sec: u64) -> Option<Self> {
        if ticks_per_sec == 0 {
            None
        } else {
            Some(Self { ticks_per_sec })
        }
    }

    /// The configured tick frequency in Hz.
    #[must_use]
    pub const fn hz(self) -> u64 {
        self.ticks_per_sec
    }

    /// Convert an elapsed tick count to nanoseconds, rounding to nearest.
    ///
    /// Uses 128-bit intermediate arithmetic so a multi-second duration cannot
    /// overflow; deterministic and allocation-free.
    #[must_use]
    pub const fn ticks_to_nanos(self, ticks: u64) -> u64 {
        let num = (ticks as u128) * 1_000_000_000u128;
        let den = self.ticks_per_sec as u128;
        // Round to nearest rather than truncating (sub-tick fairness for p99.9).
        let q = (num + den / 2) / den;
        q as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opkind_roundtrips() {
        for raw in 0..OpKind::COUNT as u16 {
            let k = OpKind::from_u16(raw).expect("in range");
            assert_eq!(k.as_u16(), raw);
            assert!(!k.label().is_empty());
        }
        assert_eq!(OpKind::from_u16(OpKind::COUNT as u16), None);
    }

    #[test]
    fn errorclass_roundtrips() {
        for raw in 0..ErrorClass::COUNT as u16 {
            let c = ErrorClass::from_u16(raw).expect("in range");
            assert_eq!(c.as_u16(), raw);
            assert!(!c.label().is_empty());
        }
        assert_eq!(ErrorClass::from_u16(ErrorClass::COUNT as u16), None);
        assert!(ErrorClass::Ok.is_ok());
        assert!(!ErrorClass::Internal.is_ok());
    }

    #[test]
    fn sample_is_pod_and_small() {
        let s = HotSample::new(42, OpKind::StreamQuote, 1234, 7).with_class(ErrorClass::StaleState);
        assert_eq!(s.request_id, 42);
        assert_eq!(s.kind, OpKind::StreamQuote);
        assert_eq!(s.class, ErrorClass::StaleState);
        assert_eq!(s.core_id, 7);
        // POD copy semantics.
        let t = s;
        assert_eq!(s, t);
    }

    #[test]
    fn tickrate_converts_and_rounds() {
        // 1 GHz: 1 tick == 1 ns exactly.
        let r = TickRate::from_hz(1_000_000_000).expect("nonzero");
        assert_eq!(r.ticks_to_nanos(1_000), 1_000);
        // 24 MHz (Apple cntfrq): 24 ticks ≈ 1000 ns, rounded to nearest.
        let r = TickRate::from_hz(24_000_000).expect("nonzero");
        assert_eq!(r.ticks_to_nanos(24), 1_000);
        // Rounding to nearest (not truncation): 1 tick @24MHz = 41.66.. ns -> 42.
        assert_eq!(r.ticks_to_nanos(1), 42);
        assert_eq!(TickRate::from_hz(0), None);
    }

    #[test]
    fn tickrate_no_overflow_on_long_durations() {
        // ~1 hour at 1 GHz must not overflow the 128-bit intermediate.
        let r = TickRate::from_hz(1_000_000_000).expect("nonzero");
        let one_hour_ticks = 3_600u64 * 1_000_000_000;
        assert_eq!(r.ticks_to_nanos(one_hour_ticks), one_hour_ticks);
    }
}
