//! Composition pipeline and guardrails.
//!
//! Sums the enabled strategies' contributions, applies the guardrail clamps in
//! the fixed order of `docs/FI-TIERING-RESEARCH.md` §4, forms the two-way, and
//! enforces the anti-cross invariant last. Guardrails are engineering
//! invariants (min/max half-spread, max skew, min tradeable spread, stale-input
//! handling) — the safety net that keeps the outbound book strictly two-sided
//! regardless of how strategies are configured.

use crate::{QuoteCtx, TieringStrategy, TwoWay};
use serde::{Deserialize, Serialize};

/// Guardrail bounds, expressed as absolute **price offsets** (points), so they
/// are independent of any strategy's spread unit and act as pure price-space
/// engineering invariants.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guardrails {
    /// Minimum half-spread `h_min ≥ 0` (price points).
    pub h_min: f64,
    /// Maximum half-spread `h_max ≥ h_min` (price points). Also the width the
    /// [`StalePolicy::WidenToMax`] fallback quotes at.
    pub h_max: f64,
    /// Maximum absolute skew `s_max ≥ 0` (price points): `|s| ≤ s_max`.
    pub s_max: f64,
    /// Minimum tradeable spread `spread_floor > 0` (price points):
    /// `offer − bid ≥ spread_floor`. Enforced by flooring `h ≥ spread_floor/2`,
    /// which — since `offer − bid = 2h` is skew-invariant — also guarantees
    /// `bid < offer`.
    pub spread_floor: f64,
}

impl Guardrails {
    /// Construct guardrails.
    #[must_use]
    pub fn new(h_min: f64, h_max: f64, s_max: f64, spread_floor: f64) -> Self {
        Self {
            h_min,
            h_max,
            s_max,
            spread_floor,
        }
    }

    /// Validate the bounds are internally consistent and finite. A valid config
    /// requires `0 ≤ h_min ≤ h_max`, `s_max ≥ 0`, `spread_floor > 0`, and
    /// `h_max ≥ spread_floor/2` (so the min-spread floor never contradicts the
    /// max half-spread cap).
    pub fn validate(&self) -> Result<(), Suppressed> {
        let finite = self.h_min.is_finite()
            && self.h_max.is_finite()
            && self.s_max.is_finite()
            && self.spread_floor.is_finite();
        let consistent = self.h_min >= 0.0
            && self.h_max >= self.h_min
            && self.s_max >= 0.0
            && self.spread_floor > 0.0
            && self.h_max >= self.spread_floor / 2.0;
        if finite && consistent {
            Ok(())
        } else {
            Err(Suppressed::new(SuppressReason::InvalidConfig))
        }
    }

    /// Enforce the price-space invariants on an **already-formed** two-way,
    /// preserving its centre. Clamps the half-spread to `[h_min, h_max]` then
    /// floors it to `spread_floor/2`, so `offer − bid = 2h ≥ spread_floor > 0`
    /// and hence `bid < offer` — the anti-cross safety net, in the exact clamp
    /// order [`quote`]'s [`finalize`] uses.
    ///
    /// The free [`quote`] path builds the two-way from a mid and its `(h, s)`
    /// contributions and guards them there; this method guards an **arbitrary
    /// running two-way** produced by a composed chain of pricing features (see
    /// [`crate::FeaturePipeline`]), where the skew is already baked into the
    /// centre and only the width needs bounding. Panic-safe against an
    /// inconsistent `h_min > h_max` (orders the clamp bounds); assumes a finite
    /// input two-way (the upstream composite is validated before pricing).
    #[must_use]
    pub fn enforce(&self, tw: TwoWay) -> TwoWay {
        let center = 0.5 * (tw.bid + tw.offer);
        let half = 0.5 * (tw.offer - tw.bid);
        let lo = self.h_min.min(self.h_max);
        let hi = self.h_min.max(self.h_max);
        let half = half.clamp(lo, hi).max(self.spread_floor / 2.0);
        TwoWay {
            bid: center - half,
            offer: center + half,
        }
    }
}

/// What to do when the upstream composite is stale / the LP quorum is lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StalePolicy {
    /// Publish no quote — return [`Suppressed`]. The safe default.
    #[default]
    Suppress,
    /// Keep a market but widen to `h_max` with zero skew.
    WidenToMax,
}

/// Why a quote was suppressed (no outbound two-way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuppressReason {
    /// Upstream inputs stale and [`StalePolicy::Suppress`] is configured.
    StaleInputs,
    /// The composite mid was NaN/±∞.
    NonFiniteMid,
    /// A strategy's spread unit could not be converted against this context
    /// (e.g. yield-bps without DV01/ModDur).
    MissingConversionInput,
    /// The summed or final price came out non-finite.
    NonFiniteOutput,
    /// The guardrail bounds were internally inconsistent.
    InvalidConfig,
}

/// A suppressed quote outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Suppressed {
    /// The reason no two-way was produced.
    pub reason: SuppressReason,
}

impl Suppressed {
    pub(crate) fn new(reason: SuppressReason) -> Self {
        Self { reason }
    }
}

/// Compose `strategies` over `ctx` and produce the guarded outbound two-way.
///
/// Pipeline (per `docs/FI-TIERING-RESEARCH.md` §4):
/// 1. Reject a non-finite mid; validate the guardrails.
/// 2. If stale, apply [`StalePolicy`] (suppress, or widen to `h_max`).
/// 3. Pre-validate every strategy's unit conversion against the context.
/// 4. Sum contributions → `(h, s)` price offsets.
/// 5. Clamp `h ∈ [h_min, h_max]`, then `|s| ≤ s_max`.
/// 6. Floor `h ≥ spread_floor/2` (min tradeable spread), then clamp skew **last**.
/// 7. Form `bid = mid − h − s`, `offer = mid + h − s`.
///
/// Guarantees on `Ok`: `offer − bid = 2h ≥ spread_floor > 0`, hence `bid < offer`
/// always — skew shifts both sides equally and can never invert the book.
pub fn quote(
    strategies: &[&dyn TieringStrategy],
    ctx: &QuoteCtx,
    guards: &Guardrails,
    stale_policy: StalePolicy,
) -> Result<TwoWay, Suppressed> {
    let mid = ctx.mid.0;
    if !mid.is_finite() {
        return Err(Suppressed::new(SuppressReason::NonFiniteMid));
    }
    guards.validate()?;

    if ctx.is_stale {
        return match stale_policy {
            StalePolicy::Suppress => Err(Suppressed::new(SuppressReason::StaleInputs)),
            StalePolicy::WidenToMax => finalize(mid, guards.h_max, 0.0, guards),
        };
    }

    // Pre-validate conversions so the (infallible) `adjust` calls below cannot
    // hit an unsupported unit.
    for strat in strategies {
        strat
            .unit()
            .to_price_offset(1.0, ctx)
            .map_err(|_| Suppressed::new(SuppressReason::MissingConversionInput))?;
    }

    let mut half_spread = 0.0_f64;
    let mut skew = 0.0_f64;
    for strat in strategies {
        let contribution = strat.adjust(ctx);
        half_spread += contribution.half_spread;
        skew += contribution.skew;
    }
    if !half_spread.is_finite() || !skew.is_finite() {
        return Err(Suppressed::new(SuppressReason::NonFiniteOutput));
    }

    // Guardrail clamps (order per §4).
    half_spread = half_spread.clamp(guards.h_min, guards.h_max);
    skew = skew.clamp(-guards.s_max, guards.s_max);
    finalize(mid, half_spread, skew, guards)
}

/// Apply the min-spread floor, the final anti-cross skew clamp, and form the
/// two-way. Shared by the normal and widen-to-max paths.
fn finalize(
    mid: f64,
    half_spread: f64,
    skew: f64,
    guards: &Guardrails,
) -> Result<TwoWay, Suppressed> {
    // Min tradeable spread: floor the half-spread so offer − bid = 2h ≥ spread_floor.
    let half_spread = half_spread.max(guards.spread_floor / 2.0);
    // Anti-cross LAST: re-clamp skew after the floor so extreme inventory cannot
    // push a side past the guardrail cap.
    let skew = skew.clamp(-guards.s_max, guards.s_max);

    let bid = mid - half_spread - skew;
    let offer = mid + half_spread - skew;
    if !bid.is_finite() || !offer.is_finite() {
        return Err(Suppressed::new(SuppressReason::NonFiniteOutput));
    }
    Ok(TwoWay { bid, offer })
}
