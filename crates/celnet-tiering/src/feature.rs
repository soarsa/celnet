//! The composable **pricing-feature** library.
//!
//! Where [`crate::TieringStrategy`] is one margin/skew contribution summed into a
//! two-way *built from a mid*, a [`PricingFeature`] is a self-contained **transform
//! of a running two-way**: it takes the price the previous feature produced and
//! returns the next. A trader composes an ordered list of features (the
//! [`crate::FeaturePipeline`]) — RAW → …their chosen features… → outbound — and the
//! order *is* the pricing. Each feature is a pure, deterministic function; there is
//! no I/O, no wire, no server dependency (identical purity contract to the tiering
//! engine it extends).
//!
//! The initial library (all extensible behind this one enum):
//!
//! | Feature | Effect on the running two-way |
//! |---|---|
//! | [`PricingFeature::MidShift`]  | recentre: bias the mid and/or override the reference price (spread-invariant) |
//! | [`PricingFeature::Tiering`]   | **margin**: rebuild the two-way around the running centre using a shipped [`TieringConfig`] (Flat / Scaled-Smoothed-Spread / Inventory-skew) |
//! | [`PricingFeature::Axe`]       | skew the two-way toward a desk axe (spread-invariant centre shift) |
//! | [`PricingFeature::Position`]  | inventory skew — reuses [`crate::InventorySkew`] with the context's signed inventory (spread-invariant centre shift) |
//! | [`PricingFeature::PanicSkew`] | an emergency overlay skew, applied only when `triggered` (spread-invariant centre shift) |
//!
//! **RAW** is the implicit start of every pipeline (the consolidated LP composite),
//! not a feature.
//!
//! Provenance / sign conventions follow the shipped tiering engine
//! (`bid = mid − h − s`, `offer = mid + h − s`): a positive skew shifts the **whole**
//! two-way **down** (the direction a long dealer skews to shed inventory), and
//! `offer − bid = 2h` is invariant under any pure skew.

use crate::{InventorySkew, Mid, QuoteCtx, SpreadUnit, TieringConfig, TieringStrategy, TwoWay};
use serde::{Deserialize, Serialize};

/// Immutable pricing context threaded to every [`PricingFeature`].
///
/// Wraps the shipped [`QuoteCtx`] — the mid, signed inventory, DV01/modified
/// duration, realized vol, and the smoothed observed spread the tiering strategies
/// already read — and adds the extra live desk input the new features may consult:
/// a **reference price** (a fixing / desk reference a [`PricingFeature::MidShift`]
/// recentres on when its own `reference` param is `None`).
///
/// A desk **axe** is expressed as a self-contained [`PricingFeature::Axe`] param
/// (side + magnitude) rather than a context field, so a group's axe lean travels
/// with its pipeline config.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PricingCtx {
    quote_ctx: QuoteCtx,
    reference_price: Option<f64>,
}

impl PricingCtx {
    /// A context wrapping `quote_ctx` with no reference-price input.
    #[must_use]
    pub fn new(quote_ctx: QuoteCtx) -> Self {
        Self {
            quote_ctx,
            reference_price: None,
        }
    }

    /// Supply a live reference / fixing price (the fallback centre a
    /// [`PricingFeature::MidShift`] recentres on when it carries no `reference`).
    #[must_use]
    pub fn with_reference_price(mut self, reference_price: f64) -> Self {
        self.reference_price = Some(reference_price);
        self
    }

    /// The wrapped [`QuoteCtx`] (mid, inventory, DV01, vol, smoothed spread, …).
    #[must_use]
    pub fn quote_ctx(&self) -> &QuoteCtx {
        &self.quote_ctx
    }

    /// The live reference / fixing price input, if any.
    #[must_use]
    pub fn reference_price(&self) -> Option<f64> {
        self.reference_price
    }

    /// The wrapped [`QuoteCtx`] recentred on `mid` — the per-feature context used
    /// for unit conversion and for rebuilding the two-way around the running
    /// centre. All other fields (inventory, DV01, vol, smoothed spread, …) are
    /// preserved.
    fn ctx_at(&self, mid: f64) -> QuoteCtx {
        let mut q = self.quote_ctx;
        q.mid = Mid(mid);
        q
    }
}

/// Which side a desk is axed on (which direction it wants to trade).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxeSide {
    /// The desk wants to **buy** — lean the two-way **up** (a keener bid).
    Buy,
    /// The desk wants to **sell** — lean the two-way **down** (a keener offer).
    Sell,
}

/// The palette / provenance identifier of a [`PricingFeature`] (kind only, no
/// params). Stamped alongside each stage in a [`crate::PricedResult`] waterfall.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeatureKind {
    /// [`PricingFeature::MidShift`].
    MidShift,
    /// [`PricingFeature::Tiering`].
    Tiering,
    /// [`PricingFeature::Axe`].
    Axe,
    /// [`PricingFeature::Position`].
    Position,
    /// [`PricingFeature::PanicSkew`].
    PanicSkew,
}

/// One pluggable pricing-feature transform + its params — the serde-serializable
/// unit a trader drags into a pipeline.
///
/// Internally tagged by `kind` (identical convention to
/// [`crate::StrategySpec`]): a serialized feature is `{ "kind": "...", ...params }`.
/// Materialized directly (no build step) and applied with [`Self::apply`].
///
/// An **unconvertible spread unit** for the current context (e.g.
/// [`SpreadUnit::YieldBps`] with no DV01/modified duration) degrades that feature
/// to a **no-op** rather than panicking — [`Self::apply`] is infallible by
/// contract, and a feature that cannot resolve its offset must not move the price.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PricingFeature {
    /// **Mid shift** — recentre the running two-way (spread-invariant). The new
    /// centre is `reference ?? ctx.reference_price ?? current_centre`, then biased
    /// by `shift` (a signed magnitude in `unit`). Pure desk price construction: it
    /// never changes the width. `reference = None` + a signed `shift` ⇒ a plain
    /// mid bias; `reference = Some(r)` + `shift = 0` ⇒ a pure reference override.
    MidShift {
        /// Signed mid bias magnitude, in `unit` (may be `0.0`).
        shift: f64,
        /// The unit `shift` is expressed in.
        unit: SpreadUnit,
        /// An explicit reference-price override for the centre; falls back to the
        /// context's reference price, then the running centre.
        #[serde(default)]
        reference: Option<f64>,
    },
    /// **Tiering** — apply margin/markup by rebuilding the two-way around the
    /// running centre with a shipped [`TieringConfig`] (Flat / Scaled-Smoothed-
    /// Spread / Inventory-skew). Reused **verbatim**: the feature sets the running
    /// mid to the current centre and calls [`TieringConfig::quote`]. A suppressed
    /// tier (stale / invalid inputs under [`crate::StalePolicy::Suppress`]) leaves
    /// the running two-way unchanged (no margin added).
    Tiering {
        /// The shipped tiering configuration applied as this feature's margin.
        config: TieringConfig,
    },
    /// **Axe** — skew the two-way toward the desk's axe (spread-invariant): a
    /// `Buy` axe shifts the whole two-way **up** by `magnitude` (a keener bid), a
    /// `Sell` axe shifts it **down**. `magnitude` is a non-negative lean expressed
    /// in `unit`; its absolute value is used.
    Axe {
        /// Which way the desk is axed.
        side: AxeSide,
        /// The lean magnitude (non-negative), in `unit`.
        magnitude: f64,
        /// The unit `magnitude` is expressed in.
        unit: SpreadUnit,
    },
    /// **Position** — inventory skew, reusing [`crate::InventorySkew`] over the
    /// context's signed inventory `q`: `s = clamp(κ·q, ±s_max)`, applied as a
    /// spread-invariant centre shift **down** by `s` (long ⇒ down, to shed the
    /// position). Skew only — margin is [`Self::Tiering`]'s job.
    Position {
        /// Skew gain `κ` (magnitude per unit of inventory), in `unit`.
        kappa: f64,
        /// Strategy-local skew cap `s_max` (magnitude ≥ 0), in `unit`.
        s_max: f64,
        /// The unit `kappa·q` and `s_max` are expressed in.
        unit: SpreadUnit,
    },
    /// **Panic skew** — an emergency overlay skew applied **only** when
    /// `triggered`. A positive `skew` shifts the whole two-way **down** by that
    /// signed magnitude (shipped skew sign convention); when not triggered it is a
    /// no-op. Separate from tiering — this is not part of the margin.
    PanicSkew {
        /// Signed skew magnitude, in `unit` (positive ⇒ shift down).
        skew: f64,
        /// The unit `skew` is expressed in.
        unit: SpreadUnit,
        /// Whether the overlay is currently armed; `false` ⇒ no-op.
        triggered: bool,
    },
}

impl PricingFeature {
    /// This feature's [`FeatureKind`] (kind only, for the provenance waterfall).
    #[must_use]
    pub fn kind(&self) -> FeatureKind {
        match self {
            PricingFeature::MidShift { .. } => FeatureKind::MidShift,
            PricingFeature::Tiering { .. } => FeatureKind::Tiering,
            PricingFeature::Axe { .. } => FeatureKind::Axe,
            PricingFeature::Position { .. } => FeatureKind::Position,
            PricingFeature::PanicSkew { .. } => FeatureKind::PanicSkew,
        }
    }

    /// Apply this feature to the `running` two-way, returning the next running
    /// two-way. Pure and infallible (an unconvertible unit is a no-op — see the
    /// type-level note).
    #[must_use]
    pub fn apply(&self, running: TwoWay, ctx: &PricingCtx) -> TwoWay {
        let center = 0.5 * (running.bid + running.offer);
        let half = 0.5 * (running.offer - running.bid);
        match self {
            PricingFeature::MidShift {
                shift,
                unit,
                reference,
            } => {
                let base_center = (*reference).or(ctx.reference_price()).unwrap_or(center);
                let qctx = ctx.ctx_at(base_center);
                let offset = unit.to_price_offset(*shift, &qctx).unwrap_or(0.0);
                let new_center = base_center + offset;
                TwoWay {
                    bid: new_center - half,
                    offer: new_center + half,
                }
            }
            PricingFeature::Tiering { config } => {
                // Reuse the shipped engine verbatim: build margin around the
                // running centre. A suppressed tier leaves the price unchanged.
                let qctx = ctx.ctx_at(center);
                config.quote(&qctx).unwrap_or(running)
            }
            PricingFeature::Axe {
                side,
                magnitude,
                unit,
            } => {
                let qctx = ctx.ctx_at(center);
                let mag = unit.to_price_offset(magnitude.abs(), &qctx).unwrap_or(0.0);
                let signed = match side {
                    AxeSide::Buy => mag,
                    AxeSide::Sell => -mag,
                };
                TwoWay {
                    bid: running.bid + signed,
                    offer: running.offer + signed,
                }
            }
            PricingFeature::Position { kappa, s_max, unit } => {
                let qctx = ctx.ctx_at(center);
                // Guard the InventorySkew::adjust precondition (celnet_tiering::quote
                // would normally prevalidate the unit conversion): an unconvertible
                // unit is a no-op skew here rather than a panic.
                if unit.to_price_offset(1.0, &qctx).is_err() {
                    return running;
                }
                let skew = InventorySkew::new(0.0, *kappa, *s_max, *unit)
                    .adjust(&qctx)
                    .skew;
                // Shipped sign: skew s shifts BOTH sides down by s.
                TwoWay {
                    bid: running.bid - skew,
                    offer: running.offer - skew,
                }
            }
            PricingFeature::PanicSkew {
                skew,
                unit,
                triggered,
            } => {
                if !*triggered {
                    return running;
                }
                let qctx = ctx.ctx_at(center);
                let offset = unit.to_price_offset(*skew, &qctx).unwrap_or(0.0);
                // Positive skew shifts both sides down (shipped convention).
                TwoWay {
                    bid: running.bid - offset,
                    offer: running.offer - offset,
                }
            }
        }
    }
}
