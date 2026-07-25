//! Serde-serializable tiering configuration.
//!
//! What a server persists on a book and a GUI edits: the spread unit, the
//! enabled strategies + their params, the guardrail bounds, and the stale
//! policy. Forward-compatible for the remaining four strategies — adding a
//! [`StrategySpec`] variant is an additive, single-contract change (no API
//! versioning per the project contract rule).

use crate::{
    FlatMarkup, Guardrails, InventorySkew, QuoteCtx, ScaledSmoothedSpread, SpreadUnit, StalePolicy,
    Suppressed, TieringStrategy, TwoWay, quote,
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
    /// [`ScaledSmoothedSpread`]: a spread *source* that damps spread volatility.
    /// All spread params (`e, d, c, m`) are **absolute price offsets**; `w, f` are
    /// dimensionless. The Smoothing Weight `w` drives the stateful EWMA step the
    /// aggregation layer runs ([`crate::smooth`]); `e, d, c, m, f` parameterise the
    /// pure per-quote output-spread map. Use instead of [`FlatMarkup::new`], not on
    /// top of it (it *sets* the half-spread absolutely).
    ScaledSmoothedSpread {
        /// Smoothing Weight `w ∈ (0, 1]` (`1` ⇒ smoothing off). Consumed by the
        /// aggregation layer's [`crate::smooth`] step, not the pure strategy.
        smoothing_weight: f64,
        /// Expected Spread `e` (`> 0`) — the divergence reference.
        expected_spread: f64,
        /// Max Divergence `d` (`≥ 0`) — the dead-band half-width.
        max_divergence: f64,
        /// Core spread `c` (`≥ 0`) — the base output spread at zero widen.
        core_spread: f64,
        /// Max Output Spread `m` (`≥ c`) — the output cap and indicative fallback.
        max_output_spread: f64,
        /// Spread Scale Factor `f` (`≥ 0`) — the widening gain on `Dₙ/e`.
        spread_scale_factor: f64,
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
            StrategySpec::ScaledSmoothedSpread {
                // `smoothing_weight` drives the upstream EWMA step, not the pure
                // strategy; the strategy prices off the already-smoothed `Sₙ` in the
                // context. It works in absolute price-offset space, so the config
                // `unit` does not apply to it.
                smoothing_weight: _,
                expected_spread,
                max_divergence,
                core_spread,
                max_output_spread,
                spread_scale_factor,
            } => Box::new(ScaledSmoothedSpread::new(
                expected_spread,
                max_divergence,
                core_spread,
                max_output_spread,
                spread_scale_factor,
            )),
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
    /// The Smoothing Weight `w` of the first [`StrategySpec::ScaledSmoothedSpread`]
    /// this config enables, or `None` when no such strategy is present.
    ///
    /// The seam the aggregation layer uses to decide whether to run the stateful
    /// EWMA smoothing ([`crate::smooth`]) for a book and with what weight: `Some(w)`
    /// ⇒ maintain the per-instrument smoothed spread and feed it via
    /// [`QuoteCtx::with_smoothed_spread`]; `None` ⇒ no smoothing state (every other
    /// config path is byte-identical to before).
    #[must_use]
    pub fn smoothing_weight(&self) -> Option<f64> {
        self.strategies.iter().find_map(|s| match *s {
            StrategySpec::ScaledSmoothedSpread {
                smoothing_weight, ..
            } => Some(smoothing_weight),
            _ => None,
        })
    }

    /// Build the configured strategies and produce the guarded outbound two-way
    /// for `ctx`. Convenience over the free [`quote`] function.
    pub fn quote(&self, ctx: &QuoteCtx) -> Result<TwoWay, Suppressed> {
        let built: Vec<Box<dyn TieringStrategy>> =
            self.strategies.iter().map(|s| s.build(self.unit)).collect();
        let refs: Vec<&dyn TieringStrategy> = built.iter().map(AsRef::as_ref).collect();
        quote(&refs, ctx, &self.guardrails, self.stale_policy)
    }
}
