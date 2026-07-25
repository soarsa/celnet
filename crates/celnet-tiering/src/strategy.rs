//! The pluggable strategy seam and the two phase-1 strategies.
//!
//! One trait, N strategies, composed additively by [`crate::quote`]. Phase 1
//! ships [`FlatMarkup`] and [`InventorySkew`]; the remaining four (volatility
//! scale, size ladder, toxicity widen, per-client tier base — see
//! `docs/FI-TIERING-RESEARCH.md` §5) slot in behind the same trait without
//! changing this interface.
//!
//! Provenance: the half-spread `h` (fill-frequency vs profit) and skew `s`
//! (inventory control, ~linear in `q`, clamped) are the two functionally
//! distinct knobs of the inventory-control market-making lineage
//! (Avellaneda–Stoikov / Guéant–Lehalle; Bergault et al., arXiv:1810.04383);
//! real desks use clamped linear heuristics rather than solving the HJB.

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
        // Linear-in-inventory skew, clamped in magnitude space, then converted
        // (conversion is sign-preserving so the clamp bounds hold in price space).
        let cap = self.s_max.abs();
        let skew_magnitude = (self.kappa * ctx.inventory).clamp(-cap, cap);
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
