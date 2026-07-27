//! The composable **feature pipeline** engine.
//!
//! A [`FeaturePipeline`] is an ordered list of [`PricingFeature`]s plus the
//! price-space [`Guardrails`] applied last. [`FeaturePipeline::run`] threads a
//! **raw** two-way (the consolidated LP composite — the implicit RAW stage)
//! through each feature in order, records the two-way **after every stage** (the
//! provenance waterfall), applies the anti-cross guardrail, and returns a
//! [`PricedResult`].
//!
//! This is the outbound-price construction a pricing group uses per mode (ESP /
//! RFS-RFQ): the composite is consolidated once upstream; the pipeline is a pure
//! per-quote arithmetic transform, so it stays lean on the hot publish path.

use crate::{FeatureKind, Guardrails, PricingCtx, PricingFeature, TwoWay};
use serde::{Deserialize, Serialize};

/// One element of a [`FeaturePipeline`] — a [`PricingFeature`] carrying its `kind`
/// tag and params (`{ "kind": "...", ...params }` when serialized). Named for the
/// design's "feature spec" (the persisted / wire / GUI shape); it *is* the
/// pluggable transform, so there is a single source of truth (no separate
/// spec-vs-behaviour split).
pub type FeatureSpec = PricingFeature;

/// The result of running a [`FeaturePipeline`]: the outbound two-way plus the
/// full per-feature **provenance waterfall** so a caller can stamp exactly how the
/// price was constructed (design §7 — the analytics foundation).
///
/// The waterfall lets a caller reconstruct each stage exactly: with the sequence
/// `[raw] ++ after.map(.1) ++ [outbound]`, consecutive per-stage deltas telescope
/// to `outbound − raw`.
#[derive(Clone, Debug, PartialEq)]
pub struct PricedResult {
    /// The RAW input two-way (consolidated LP composite) before any feature.
    pub raw: TwoWay,
    /// The running two-way **after each feature**, in pipeline order, tagged with
    /// the feature's [`FeatureKind`].
    pub after: Vec<(FeatureKind, TwoWay)>,
    /// The final outbound two-way after the closing guardrail pass (`= what is
    /// sent to the client`).
    pub outbound: TwoWay,
    /// The per-side **margin** the [`FeatureKind::Tiering`] feature(s) contributed:
    /// the summed change in half-spread across the tiering stages (price offset).
    pub applied_margin: f64,
    /// The net **skew** the [`FeatureKind::Axe`] / [`FeatureKind::Position`] /
    /// [`FeatureKind::PanicSkew`] features contributed: the summed signed change in
    /// centre across those stages (price offset). A [`FeatureKind::MidShift`] is
    /// desk price *construction*, counted in neither.
    pub applied_skew: f64,
}

/// An ordered, serde-serializable pricing pipeline: the features a trader composed
/// plus the guardrails that bound the outbound width. This is the persisted / wire
/// / GUI shape a pricing group carries per mode (ESP and RFS/RFQ).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeaturePipeline {
    /// The ordered features (RAW → these → outbound).
    pub features: Vec<FeatureSpec>,
    /// The price-space guardrails applied last (anti-cross / min-spread floor).
    pub guardrails: Guardrails,
}

impl FeaturePipeline {
    /// Construct a pipeline from an ordered feature list and its guardrails.
    #[must_use]
    pub fn new(features: Vec<FeatureSpec>, guardrails: Guardrails) -> Self {
        Self {
            features,
            guardrails,
        }
    }

    /// Run the pipeline over `raw` in `ctx`: apply every feature in order, capture
    /// the provenance waterfall, apply the closing guardrail, and attribute the
    /// tiering margin and the skew-feature skew.
    #[must_use]
    pub fn run(&self, raw: TwoWay, ctx: &PricingCtx) -> PricedResult {
        let mut running = raw;
        let mut after = Vec::with_capacity(self.features.len());
        let mut applied_margin = 0.0_f64;
        let mut applied_skew = 0.0_f64;

        for feature in &self.features {
            let before = running;
            running = feature.apply(before, ctx);
            let kind = feature.kind();
            match kind {
                // Tiering contributes the margin: the change in half-spread.
                FeatureKind::Tiering => {
                    let before_half = 0.5 * (before.offer - before.bid);
                    let after_half = 0.5 * (running.offer - running.bid);
                    applied_margin += after_half - before_half;
                }
                // The skew features contribute the skew: the change in centre.
                FeatureKind::Axe | FeatureKind::Position | FeatureKind::PanicSkew => {
                    let before_center = 0.5 * (before.bid + before.offer);
                    let after_center = 0.5 * (running.bid + running.offer);
                    applied_skew += after_center - before_center;
                }
                // Desk price construction — neither margin nor skew.
                FeatureKind::MidShift => {}
            }
            after.push((kind, running));
        }

        let outbound = self.guardrails.enforce(running);
        PricedResult {
            raw,
            after,
            outbound,
            applied_margin,
            applied_skew,
        }
    }
}
