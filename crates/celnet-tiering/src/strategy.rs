//! The pluggable strategy seam and the two phase-1 strategies.
//!
//! One trait, N strategies, composed additively by [`crate::quote`]. Phase 1
//! ships [`FlatMarkup`] and [`InventorySkew`]; the remaining four (volatility
//! scale, size ladder, toxicity widen, per-client tier base — see
//! `docs/fixed-income/FI-TIERING-RESEARCH.md` §5) slot in behind the same trait without
//! changing this interface.
//!
//! Architecture: the half-spread `h` (fill-frequency vs profit) and skew `s`
//! (inventory control, ~linear in `q`, clamped) are the two functionally
//! distinct knobs of the inventory-control market-making engine;
//! real desks use clamped linear heuristics rather than solving continuous stochastic controls.

use crate::{QuoteCtx, SpreadSkew, SpreadUnit};

/// Message for the pipeline-guaranteed conversion invariant. [`crate::quote`]
/// pre-validates every strategy's [`SpreadUnit`] against the context (see
/// `precheck`), so the conversion inside `adjust` cannot fail there. Calling
/// `adjust` directly on a [`SpreadUnit::YieldBps`] strategy without DV01/ModDur
/// in the context is a caller precondition violation.
const PRECHECKED: &str =
    "QuoteCtx must support the strategy's SpreadUnit; route quotes through celnet_tiering::quote";

/// A pure tiering strategy: given the pricing context, produce a spread/skew
/// contribution (as price offsets). Contributions from all enabled strategies
/// are summed by the pipeline.
pub trait TieringStrategy {
    /// Produce this strategy's `(half_spread, skew)` contribution as price
    /// offsets.
    ///
    /// # Precondition
    /// The context must support [`Self::unit`]'s conversion. [`crate::quote`]
    /// enforces this before calling; a direct call that violates it panics.
    fn adjust(&self, ctx: &QuoteCtx) -> SpreadSkew;

    /// The spread unit this strategy's magnitudes are expressed in — used by the
    /// pipeline to pre-validate the conversion against the context.
    fn unit(&self) -> SpreadUnit;
}

/// **Strategy 1 — Flat markup** (the always-on baseline).
///
/// A constant symmetric half-spread `H` with no skew: `h = H`, `s = 0`.
/// Simplest possible tier; ignores inventory and vol.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlatMarkup {
    /// The half-spread magnitude `H` (in [`Self::unit`]). Must be `≥ 0`.
    half_spread: f64,
    /// The unit `half_spread` is expressed in.
    unit: SpreadUnit,
}

impl FlatMarkup {
    /// Construct a flat markup of `half_spread` in `unit`.
    #[must_use]
    pub fn new(half_spread: f64, unit: SpreadUnit) -> Self {
        Self { half_spread, unit }
    }
}

impl TieringStrategy for FlatMarkup {
    fn adjust(&self, ctx: &QuoteCtx) -> SpreadSkew {
        let half_spread = self
            .unit
            .to_price_offset(self.half_spread, ctx)
            .expect(PRECHECKED);
        SpreadSkew {
            half_spread,
            skew: 0.0,
        }
    }

    fn unit(&self) -> SpreadUnit {
        self.unit
    }
}

/// **Strategy 2 — Inventory skew.**
///
/// A base half-spread `H` plus a skew linear in signed inventory, clamped:
/// `s = clamp(κ·q, ±s_max)`, `h = H`.
///
/// Sign convention (`bid = mid − h − s`, `offer = mid + h − s`): a **long**
/// position (`q > 0`, `κ > 0`) yields `s > 0`, shifting **both** sides **down**
/// — cheapen the offer to sell, lower the bid — to shed inventory. A **short**
/// position shifts both sides up. `κ` and `s_max` are magnitudes in
/// [`Self::unit`]; the `κ·q` product and the clamp are applied in magnitude
/// space and then sign-preservingly converted to a price offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InventorySkew {
    /// Base half-spread magnitude `H` (in [`Self::unit`]). Must be `≥ 0`.
    half_spread: f64,
    /// Skew gain `κ` — spread magnitude per unit of inventory.
    kappa: f64,
    /// Strategy-local skew cap `s_max` (magnitude, `≥ 0`): `|κ·q| ≤ s_max`.
    /// The pipeline additionally clamps the summed skew against the guardrail
    /// `s_max` (see [`crate::Guardrails`]).
    s_max: f64,
    /// The unit `half_spread`, `kappa·q`, and `s_max` are expressed in.
    unit: SpreadUnit,
}

impl InventorySkew {
    /// Construct an inventory-skew strategy.
    ///
    /// * `half_spread` — base `H` (magnitude, `≥ 0`)
    /// * `kappa` — skew gain `κ` (magnitude per unit inventory)
    /// * `s_max` — strategy-local skew cap (magnitude, `≥ 0`)
    /// * `unit` — the unit the above magnitudes are in
    #[must_use]
    pub fn new(half_spread: f64, kappa: f64, s_max: f64, unit: SpreadUnit) -> Self {
        Self {
            half_spread,
            kappa,
            s_max,
            unit,
        }
    }
}

impl TieringStrategy for InventorySkew {
    fn adjust(&self, ctx: &QuoteCtx) -> SpreadSkew {
        let half_spread = self
            .unit
            .to_price_offset(self.half_spread, ctx)
            .expect(PRECHECKED);
        // Linear-in-inventory (or limit-utilization when present) skew, clamped in magnitude space, then converted
        // (conversion is sign-preserving so the clamp bounds hold in price space).
        let cap = self.s_max.abs();
        let input = ctx.inventory_utilization.unwrap_or(ctx.inventory);
        let skew_magnitude = (self.kappa * input).clamp(-cap, cap);
        let skew = self
            .unit
            .to_price_offset(skew_magnitude, ctx)
            .expect(PRECHECKED);
        SpreadSkew { half_spread, skew }
    }

    fn unit(&self) -> SpreadUnit {
        self.unit
    }
}

/// Advance the fading-memory (EWMA) smoothed spread one step — the pure, stateful
/// updater the aggregation layer folds over each instrument's raw observed spread.
///
/// `Sₙ = w·Rₙ + (1−w)·Sₙ₋₁`, seeded `S₀ = R₀` (`prev == None`). The **Smoothing
/// Weight** `w ∈ (0, 1]` governs the fade: `w = 1` disables smoothing (`Sₙ = Rₙ`);
/// smaller `w` fades slower, so a step in `R` decays geometrically at rate `(1−w)`.
///
/// Purely arithmetic: the caller owns the per-(book, instrument) `Sₙ₋₁` state and
/// feeds the returned `Sₙ` into [`QuoteCtx::with_smoothed_spread`]. Provenance: a
/// standard exponentially-weighted moving average / first-order IIR low-pass filter
/// (see `docs/fixed-income/FI-TIERING-RESEARCH.md` — Scaled Smoothed Spread).
#[must_use]
pub fn smooth(prev: Option<f64>, raw: f64, w: f64) -> f64 {
    match prev {
        None => raw,
        Some(p) => w * raw + (1.0 - w) * p,
    }
}

/// **Strategy 3 — Scaled Smoothed Spread** (a spread *source*, not an additive
/// markup).
///
/// Damps spread volatility while tiering: from the fading-memory smoothed observed
/// spread `Sₙ` (advanced upstream by [`smooth`] and read from
/// [`QuoteCtx::smoothed_spread`]) it derives one **absolute** output spread `Oₙ`,
/// centred symmetrically on mid (`h = Oₙ/2`, `s = 0` — no skew):
///
/// ```text
/// Dₙ = |Sₙ − e|                          (divergence from the Expected Spread e)
/// Pₙ = 0                     if Dₙ ≤ d    (within the Max Divergence dead-band d)
/// Pₙ = f · Dₙ / e            if Dₙ > d    (Spread Scale Factor f, relative to e)
/// Oₙ = min(m, c · (1 + Pₙ))              (Core spread c, capped at Max Output m)
/// bid = M − Oₙ/2,  offer = M + Oₙ/2
/// ```
///
/// All of `e, d, c, m` are **absolute price spreads** (price offsets) and `w, f`
/// are dimensionless, so the strategy is asset-agnostic and works directly in the
/// mid's price space — hence [`Self::unit`] is [`SpreadUnit::PricePoints`] (identity
/// conversion), independent of the config's shared `unit` (which governs the other
/// strategies). Because it *sets* the half-spread absolutely, use it **instead of**
/// [`FlatMarkup`], not on top of it; [`InventorySkew`] may still layer skew.
///
/// **Observed level unavailable** (`smoothed_spread == None`) ⇒ quote at the Max
/// Output Spread `m` (`h = m/2`); the layer that detected the missing observed level
/// marks that line **indicative**.
///
/// Provenance: `docs/fixed-income/FI-TIERING-RESEARCH.md` — Scaled Smoothed Spread (source spec
/// `CTMAINDOC-Scaled Smoothed Spread Tiering`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaledSmoothedSpread {
    /// Expected Spread `e` — the target the divergence `Dₙ = |Sₙ − e|` is measured
    /// against (absolute price spread). Must be `> 0` (it is the `Pₙ` denominator).
    expected_spread: f64,
    /// Max Divergence `d` — the dead-band half-width; `Dₙ ≤ d ⇒ Pₙ = 0` (no widen).
    max_divergence: f64,
    /// Core spread `c` — the minimum/base output spread when `Pₙ = 0` (`Oₙ = c`).
    core_spread: f64,
    /// Max Output Spread `m` — the hard cap on `Oₙ`, and the indicative fallback
    /// width when the observed level is unavailable.
    max_output_spread: f64,
    /// Spread Scale Factor `f` — the widening gain applied to the relative
    /// divergence `Dₙ/e` beyond the dead-band.
    spread_scale_factor: f64,
}

impl ScaledSmoothedSpread {
    /// Construct a Scaled-Smoothed-Spread strategy from its `(e, d, c, m, f)` params
    /// (the Smoothing Weight `w` lives with the stateful [`smooth`] step in the
    /// aggregation layer, not on this pure strategy). All spread params are absolute
    /// price offsets; validate them at the config boundary.
    #[must_use]
    pub fn new(
        expected_spread: f64,
        max_divergence: f64,
        core_spread: f64,
        max_output_spread: f64,
        spread_scale_factor: f64,
    ) -> Self {
        Self {
            expected_spread,
            max_divergence,
            core_spread,
            max_output_spread,
            spread_scale_factor,
        }
    }

    /// The absolute output spread `Oₙ` for a given smoothed observed spread `Sₙ`
    /// (the core methodology, exposed for the oracle tests). `None` for `Sₙ` ⇒ the
    /// indicative fallback `Oₙ = m`.
    #[must_use]
    pub fn output_spread(&self, smoothed_spread: Option<f64>) -> f64 {
        let Some(s) = smoothed_spread else {
            // Observed level unavailable ⇒ quote at the Max Output Spread.
            return self.max_output_spread;
        };
        let divergence = (s - self.expected_spread).abs();
        let percent = if divergence <= self.max_divergence {
            0.0
        } else {
            self.spread_scale_factor * divergence / self.expected_spread
        };
        (self.core_spread * (1.0 + percent)).min(self.max_output_spread)
    }
}

impl TieringStrategy for ScaledSmoothedSpread {
    fn adjust(&self, ctx: &QuoteCtx) -> SpreadSkew {
        // Absolute output spread centred on mid — half on each side, no skew.
        SpreadSkew {
            half_spread: 0.5 * self.output_spread(ctx.smoothed_spread),
            skew: 0.0,
        }
    }

    fn unit(&self) -> SpreadUnit {
        // The methodology is defined in absolute price-offset space; the magnitude
        // IS the price offset (identity conversion), so no DV01/duration is needed.
        SpreadUnit::PricePoints
    }
}
