//! The configurable "100" — the soft, banded warehouse-threshold model, the
//! overflow-to-edge sizing, and the internalise-then-hedge netting decomposition.
//!
//! The threshold is a **soft, banded risk budget** per `(book × metric)`, reusing
//! the `celnet-limits` vocabulary (`LimitMetric` / `LimitSpec` soft bands /
//! `RagStatus` / `Utilization`) rather than a parallel one
//! (`docs/hedging/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §4). Green = warehouse,
//! amber = skew to attract offset, red = hedge the overflow.
//!
//! # Sizing — hedge to the band edge, not to flat (§4.3)
//!
//! When the red band fires, the default hedge size is the **overflow**: the amount
//! by which `|net_risk|` exceeds a configurable **target** (the band edge), clipped
//! to `[min_clip, max_clip]`:
//!
//! ```text
//! target      = target_fraction · cap        (defaults to the amber-band edge)
//! overflow    = max(0, |net_risk| − target)
//! hedge_size  = clamp(overflow, min_clip, max_clip)   (only when breached)
//! ```
//!
//! Hedging to the edge (not to zero) is the transaction-cost-optimal band policy:
//! because hedging is costly, the optimal control lets the risk drift inside a
//! no-trade region and trades only enough to return to the boundary
//! (Davis & Norman 1990, *Portfolio selection with transaction costs*;
//! Whalley & Wilmott 1997, *An asymptotic analysis of an optimal hedging model …
//! with transaction costs*, band half-width ∝ (cost·|Γ|/γ)^{1/3}). `min_clip`
//! respects the fixed-cost / minimum-ticket term so a trivially small back-to-back
//! never fires (Zakamouline 2006). `max_clip` bounds a single hedge; a larger
//! overflow is worked (Almgren & Chriss 2000). An optional **ramped fraction**
//! externalises gently for a small breach and aggressively for a large one,
//! approximating a continuous hedging *rate* (Barzykin, Bergault & Guéant 2021,
//! arXiv:2112.02269).
//!
//! # Netting — internalise before you externalise (§6.3)
//!
//! [`netting_split`] nets the sized amount against opposing internal flow first and
//! externalises only the residual — the internalisation-ratio result that paying
//! the street is the last resort (Butz & Oomen 2019, *Internalisation by electronic
//! FX spot dealers*, Quantitative Finance 19(1)).

use crate::graph::HedgeSize;
use celnet_limits::{Enforcement, LimitMetric, LimitSpec, RagStatus};

/// The conventional amber/red utilisation bands (`celnet-limits` illustrative
/// defaults, RH §5.2): warehouse below 80 %, skew 80–90 %, hedge at/over 90 %.
const DEFAULT_AMBER: f64 = 0.80;
const DEFAULT_RED: f64 = 0.90;

/// A soft, banded warehouse threshold for one `(book × metric)` scope — the
/// configurable "100".
///
/// Holds primitive fields (not a live [`LimitSpec`]) so it is `Copy` and free of a
/// serde dependency on `celnet-limits`; classification reconstructs the soft
/// [`LimitSpec`] on the fly, reusing that crate's `utilization` / `classify` logic
/// verbatim (one source of truth for the RAG bands).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WarehouseThreshold {
    /// Which exposure the budget caps (net DV01 for FI, net notional/delta for FX).
    pub metric: LimitMetric,
    /// The budget magnitude — the "100", in the metric's native units.
    pub cap: f64,
    /// Amber utilisation fraction (start skewing) in `[0, red]`.
    pub amber: f64,
    /// Red utilisation fraction (start hedging the overflow) in `[amber, 1]`.
    pub red: f64,
    /// The band-edge target as a fraction of `cap` (default = `amber`): hedge the
    /// overflow beyond this, not to flat.
    pub target_fraction: f64,
    /// Minimum hedge clip (fixed-cost / minimum-ticket floor).
    pub min_clip: f64,
    /// Maximum single hedge clip; a larger overflow is worked in tranches.
    pub max_clip: f64,
    /// Whether to ramp the hedged fraction with utilisation (dynamic soft externalisation)
    /// rather than hedging the whole overflow at once.
    pub ramped: bool,
    /// The ramp gain `k` in `hedge_fraction = clamp(k·(utilization − 1), 0, 1)`.
    pub ramp_k: f64,
}

impl WarehouseThreshold {
    /// A threshold for `metric` with budget `cap`, the conventional 80 %/90 %
    /// amber/red bands, a target at the amber edge, no clip bounds, and no ramp.
    #[must_use]
    pub fn new(metric: LimitMetric, cap: f64) -> Self {
        Self {
            metric,
            cap,
            amber: DEFAULT_AMBER,
            red: DEFAULT_RED,
            target_fraction: DEFAULT_AMBER,
            min_clip: 0.0,
            max_clip: f64::INFINITY,
            ramped: false,
            ramp_k: 1.0,
        }
    }

    /// Override the amber/red bands (utilisation fractions), clamped to
    /// `0 ≤ amber ≤ red ≤ 1` so a malformed band can never invert the RAG ordering.
    #[must_use]
    pub fn with_bands(mut self, amber: f64, red: f64) -> Self {
        let amber = amber.clamp(0.0, 1.0);
        let red = red.clamp(amber, 1.0);
        self.amber = amber;
        self.red = red;
        // Keep the target at the (possibly new) amber edge unless set explicitly.
        self.target_fraction = self.target_fraction.clamp(0.0, 1.0);
        self
    }

    /// Set the band-edge target as a fraction of `cap` (clamped to `[0, 1]`). A
    /// `target_fraction` of `0.0` hedges to flat; the default `amber` hedges to the
    /// amber edge.
    #[must_use]
    pub fn with_target_fraction(mut self, f: f64) -> Self {
        self.target_fraction = f.clamp(0.0, 1.0);
        self
    }

    /// Set the min / max hedge clip (each clamped non-negative; `max` raised to at
    /// least `min` so the clamp is always well-formed).
    #[must_use]
    pub fn with_clips(mut self, min_clip: f64, max_clip: f64) -> Self {
        self.min_clip = min_clip.max(0.0);
        self.max_clip = max_clip.max(self.min_clip);
        self
    }

    /// Enable the utilisation-ramped hedge fraction with gain `k` (`k ≥ 0`).
    #[must_use]
    pub fn with_ramp(mut self, k: f64) -> Self {
        self.ramped = true;
        self.ramp_k = k.max(0.0);
        self
    }

    /// The soft [`LimitSpec`] this threshold classifies against — reconstructed so
    /// the RAG bands come from `celnet-limits`, not a parallel implementation.
    fn spec(&self) -> LimitSpec {
        LimitSpec {
            metric: self.metric,
            cap: self.cap,
            amber: self.amber,
            red: self.red,
            enforcement: Enforcement::Soft,
        }
    }

    /// The band-edge target in native units: `target_fraction · cap`.
    #[must_use]
    pub fn target(&self) -> f64 {
        self.target_fraction * self.cap
    }

    /// `|net_risk| / cap` — the RAG utilisation ratio.
    #[must_use]
    pub fn utilization(&self, net_risk: f64) -> f64 {
        self.spec().utilization(net_risk)
    }

    /// Classify a signed `net_risk` into a RAG band (green/amber/red/breach).
    #[must_use]
    pub fn classify(&self, net_risk: f64) -> RagStatus {
        self.spec().classify(net_risk).status
    }

    /// Whether the red band has fired — the hedge trigger. True for `Red` and
    /// `Breach` (at/over the red band, i.e. utilisation ≥ `red`).
    #[must_use]
    pub fn breached(&self, net_risk: f64) -> bool {
        matches!(self.classify(net_risk), RagStatus::Red | RagStatus::Breach)
    }

    /// The overflow beyond the band edge: `max(0, |net_risk| − target)`.
    #[must_use]
    pub fn overflow(&self, net_risk: f64) -> f64 {
        (net_risk.abs() - self.target()).max(0.0)
    }

    /// The full sizing decomposition for `net_risk` — band, utilisation, target,
    /// overflow, and the default (`Overflow`) hedge size.
    ///
    /// The hedge size is `0` unless the red band is breached; when breached it is
    /// the overflow (optionally ramped) clipped to `[min_clip, max_clip]`. Because
    /// `min_clip` is a floor, a breach whose overflow is below the minimum ticket
    /// still hedges exactly `min_clip` (the fixed-cost term), never a trivially
    /// small amount.
    #[must_use]
    pub fn sizing(&self, net_risk: f64) -> HedgeSizing {
        let band = self.classify(net_risk);
        let utilization = self.utilization(net_risk);
        let target = self.target();
        let overflow = self.overflow(net_risk);
        let breached = matches!(band, RagStatus::Red | RagStatus::Breach);

        let size = if breached {
            let ramped = if self.ramped {
                let fraction = (self.ramp_k * (utilization - 1.0)).clamp(0.0, 1.0);
                overflow * fraction
            } else {
                overflow
            };
            let lo = self.min_clip.max(0.0);
            let hi = self.max_clip.max(lo);
            ramped.clamp(lo, hi)
        } else {
            0.0
        };

        HedgeSizing {
            metric: self.metric,
            band,
            utilization,
            target,
            overflow,
            size,
            breached,
        }
    }

    /// Resolve a policy leaf's [`HedgeSize`] choice into a concrete magnitude for
    /// `net_risk`. `Overflow` uses [`Self::sizing`]; `Full` flattens `|net_risk|`;
    /// `Fixed(x)` takes `|x|`, never exceeding `|net_risk|` (you cannot hedge more
    /// than you hold).
    #[must_use]
    pub fn resolve_size(&self, net_risk: f64, size: HedgeSize) -> f64 {
        match size {
            HedgeSize::Overflow => self.sizing(net_risk).size,
            HedgeSize::Full => net_risk.abs(),
            HedgeSize::Fixed(x) => x.abs().min(net_risk.abs()),
        }
    }
}

/// The sizing decomposition of one risk state against a [`WarehouseThreshold`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HedgeSizing {
    /// The budget metric.
    pub metric: LimitMetric,
    /// The RAG band the risk falls in.
    pub band: RagStatus,
    /// `|net_risk| / cap`.
    pub utilization: f64,
    /// The band-edge target in native units.
    pub target: f64,
    /// `max(0, |net_risk| − target)`.
    pub overflow: f64,
    /// The default (`Overflow`) hedge size — `0` unless breached.
    pub size: f64,
    /// Whether the red band fired.
    pub breached: bool,
}

/// The internalise-then-hedge decomposition of a sized hedge (§6.3).
///
/// `want` is the total magnitude to shed; `internal_offset` is the opposing
/// internal flow the aggregator could cross now. When `internal_first`, cross the
/// smaller of `want` and the available offset internally (at the consolidated mid,
/// saving impact + leakage) and externalise the residual; otherwise externalise the
/// whole amount (the desk's chosen ordering — the graph may prefer straight
/// externalisation for toxic flow).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NettingSplit {
    /// Crossed against opposing internal flow (the Agg Book).
    pub internal_crossed: f64,
    /// Externalised onto the RFQ / FIX panel.
    pub external_hedged: f64,
}

/// Decompose a sized hedge into an internal cross + an external residual (§6.3).
#[must_use]
pub fn netting_split(want: f64, internal_offset: f64, internal_first: bool) -> NettingSplit {
    let want = want.max(0.0);
    if internal_first {
        let internal_crossed = want.min(internal_offset.max(0.0));
        NettingSplit {
            internal_crossed,
            external_hedged: want - internal_crossed,
        }
    } else {
        NettingSplit {
            internal_crossed: 0.0,
            external_hedged: want,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thr() -> WarehouseThreshold {
        // cap 100k DV01, amber 80k, red 90k, target = amber edge (80k).
        WarehouseThreshold::new(LimitMetric::Dv01, 100_000.0)
    }

    #[test]
    fn classify_bands() {
        let t = thr();
        assert_eq!(t.classify(50_000.0), RagStatus::Green);
        assert_eq!(t.classify(80_000.0), RagStatus::Amber); // == amber edge
        assert_eq!(t.classify(85_000.0), RagStatus::Amber);
        assert_eq!(t.classify(90_000.0), RagStatus::Red); // == red edge
        assert_eq!(t.classify(100_000.0), RagStatus::Red); // == cap
        assert_eq!(t.classify(120_000.0), RagStatus::Breach); // over cap
    }

    #[test]
    fn classify_is_sign_agnostic() {
        let t = thr();
        assert_eq!(t.classify(-95_000.0), RagStatus::Red);
        assert_eq!(t.classify(-50_000.0), RagStatus::Green);
    }

    #[test]
    fn overflow_is_to_the_amber_edge() {
        let t = thr(); // target = 80k
        assert_eq!(t.overflow(50_000.0), 0.0); // below target
        assert_eq!(t.overflow(80_000.0), 0.0); // at target
        assert_eq!(t.overflow(95_000.0), 15_000.0); // 95k - 80k
        assert_eq!(t.overflow(-95_000.0), 15_000.0); // sign-agnostic
    }

    #[test]
    fn size_zero_unless_breached() {
        let t = thr();
        // Amber (85k): overflow 5k exists but red band not fired → no hedge.
        let s = t.sizing(85_000.0);
        assert!(!s.breached);
        assert_eq!(s.size, 0.0);
        assert_eq!(s.overflow, 5_000.0);
    }

    #[test]
    fn size_is_overflow_when_breached() {
        let t = thr();
        let s = t.sizing(95_000.0); // red, overflow 15k
        assert!(s.breached);
        assert_eq!(s.size, 15_000.0);
    }

    #[test]
    fn min_clip_is_a_floor() {
        // Breach with a tiny overflow bumps up to the minimum ticket.
        let t = thr().with_clips(10_000.0, f64::INFINITY);
        let s = t.sizing(92_000.0); // red, overflow 12k -> above min, stays 12k
        assert_eq!(s.size, 12_000.0);
        let s2 = t.sizing(90_500.0); // red, overflow 10.5k... target 80k -> 10.5k
        assert_eq!(s2.overflow, 10_500.0);
        // Force a sub-min overflow via a higher target.
        let t2 = WarehouseThreshold::new(LimitMetric::Dv01, 100_000.0)
            .with_target_fraction(0.9)
            .with_clips(10_000.0, f64::INFINITY);
        // target = 90k, net 95k -> overflow 5k < min 10k -> clamped up to 10k.
        let s3 = t2.sizing(95_000.0);
        assert!(s3.breached);
        assert_eq!(s3.overflow, 5_000.0);
        assert_eq!(s3.size, 10_000.0);
    }

    #[test]
    fn max_clip_caps_a_single_hedge() {
        let t = thr().with_clips(0.0, 20_000.0);
        // net 150k -> breach, overflow 70k, capped to 20k (rest is worked later).
        let s = t.sizing(150_000.0);
        assert!(s.breached);
        assert_eq!(s.overflow, 70_000.0);
        assert_eq!(s.size, 20_000.0);
    }

    #[test]
    fn ramped_fraction_scales_with_utilization() {
        // ramp k=2: at util 1.1 -> fraction clamp(2*0.1,0,1)=0.2.
        let t = WarehouseThreshold::new(LimitMetric::Dv01, 100_000.0)
            .with_target_fraction(0.0) // hedge to flat so overflow == |risk|
            .with_ramp(2.0);
        let s = t.sizing(110_000.0); // util 1.1, overflow 110k, fraction 0.2
        assert!(s.breached);
        assert!((s.size - 22_000.0).abs() < 1e-6, "got {}", s.size);
        // Large breach: util 1.6 -> fraction clamp(2*0.6,0,1)=1.0 -> full overflow.
        let s2 = t.sizing(160_000.0);
        assert!((s2.size - 160_000.0).abs() < 1e-6, "got {}", s2.size);
    }

    #[test]
    fn resolve_size_variants() {
        let t = thr();
        assert_eq!(t.resolve_size(95_000.0, HedgeSize::Overflow), 15_000.0);
        assert_eq!(t.resolve_size(95_000.0, HedgeSize::Full), 95_000.0);
        assert_eq!(t.resolve_size(95_000.0, HedgeSize::Fixed(5_000.0)), 5_000.0);
        // Fixed cannot exceed what you hold.
        assert_eq!(t.resolve_size(3_000.0, HedgeSize::Fixed(5_000.0)), 3_000.0);
    }

    #[test]
    fn netting_splits_internal_first() {
        // want 15k, offset 20k -> all crossed internally, nothing external.
        assert_eq!(
            netting_split(15_000.0, 20_000.0, true),
            NettingSplit {
                internal_crossed: 15_000.0,
                external_hedged: 0.0
            }
        );
        // want 15k, offset 6k -> 6k internal, 9k external.
        assert_eq!(
            netting_split(15_000.0, 6_000.0, true),
            NettingSplit {
                internal_crossed: 6_000.0,
                external_hedged: 9_000.0
            }
        );
        // externalise-first policy: nothing crossed.
        assert_eq!(
            netting_split(15_000.0, 20_000.0, false),
            NettingSplit {
                internal_crossed: 0.0,
                external_hedged: 15_000.0
            }
        );
        // negative offset treated as zero.
        assert_eq!(
            netting_split(15_000.0, -5.0, true),
            NettingSplit {
                internal_crossed: 0.0,
                external_hedged: 15_000.0
            }
        );
    }
}
