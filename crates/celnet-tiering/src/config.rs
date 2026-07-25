//! Serde-serializable tiering configuration.
//!
//! What a server persists on a book and a GUI edits: the spread unit, the
//! enabled strategies + their params, the guardrail bounds, and the stale
//! policy. Forward-compatible for the remaining four strategies — adding a
//! [`StrategySpec`] variant is an additive, single-contract change (no API
//! versioning per the project contract rule).

use crate::{
    FlatMarkup, Guardrails, InventorySkew, QuoteCtx, SpreadUnit, StalePolicy, Suppressed,
    TieringStrategy, TwoWay, quote,
};
use serde::{Deserialize, Serialize};

/// A single enabled strategy and its parameters. Magnitudes are in the parent
/// [`TieringConfig::unit`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum StrategySpec {
    /// [`FlatMarkup`]: constant symmetric half-spread, no skew.
    FlatMarkup {
        /// Half-spread magnitude `H`.
        half_spread: f64,
    },
    /// [`InventorySkew`]: base half-spread plus `clamp(κ·q, ±s_max)` skew.
    InventorySkew {
        /// Base half-spread magnitude `H`.
        half_spread: f64,
        /// Skew gain `κ` (magnitude per unit inventory).
        kappa: f64,
        /// Strategy-local skew cap `s_max` (magnitude).
        s_max: f64,
    },
    // Phase 3+ (docs/FI-TIERING-RESEARCH.md §5): VolatilityScale, SizeLadder,
    // ToxicityWiden, ClientTierBase — added here as additional variants behind
    // the same TieringStrategy seam without breaking this contract.
}

impl StrategySpec {
    /// Materialize this spec into a boxed [`TieringStrategy`] carrying the
    /// config's `unit`.
    #[must_use]
    fn build(&self, unit: SpreadUnit) -> Box<dyn TieringStrategy> {
        match *self {
            StrategySpec::FlatMarkup { half_spread } => {
                Box::new(FlatMarkup::new(half_spread, unit))
            }
            StrategySpec::InventorySkew {
                half_spread,
                kappa,
                s_max,
            } => Box::new(InventorySkew::new(half_spread, kappa, s_max, unit)),
        }
    }
}

/// A book's complete tiering configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TieringConfig {
    /// The unit all strategy magnitudes are expressed in.
    pub unit: SpreadUnit,
    /// The enabled strategies (composed additively, in order).
    pub strategies: Vec<StrategySpec>,
    /// Price-space guardrail bounds.
    pub guardrails: Guardrails,
    /// What to do on stale/absent inputs.
    pub stale_policy: StalePolicy,
}

impl TieringConfig {
    /// Build the configured strategies and produce the guarded outbound two-way
    /// for `ctx`. Convenience over the free [`quote`] function.
    pub fn quote(&self, ctx: &QuoteCtx) -> Result<TwoWay, Suppressed> {
        let built: Vec<Box<dyn TieringStrategy>> =
            self.strategies.iter().map(|s| s.build(self.unit)).collect();
        let refs: Vec<&dyn TieringStrategy> = built.iter().map(AsRef::as_ref).collect();
        quote(&refs, ctx, &self.guardrails, self.stale_policy)
    }
}
