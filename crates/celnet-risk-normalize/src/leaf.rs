//! Convention canonicalization (`docs/RISK-HIERARCHY.md` §2.2).
//!
//! A position arrives carrying its own quoted conventions — which delta
//! definition it was marked under, and the currency/units its premium is paid in.
//! Two positions under *different* delta conventions cannot be added: "0.25Δ"
//! under spot-premium-adjusted is a different hedge ratio than "0.25Δ" under
//! forward-unadjusted. Before anything is aggregated, this module re-derives each
//! position's risk into the **canonical internal convention**:
//!
//! - **delta**: spot-unadjusted (`Δ = e^{-r_f T}·φ·N(φd₁)`), premium **excluded**;
//! - **premium**: carried as a *separate* monetary line (domestic PV), never
//!   folded into delta — so a premium-adjusted *view* is reconstructable by
//!   subtracting the premium-in-base delta at presentation time.
//!
//! The transform re-uses `celnet-vanilla`'s closed-form Greeks, so the
//! canonicalization is exact and deterministic, not a re-implementation. The
//! quoted [`DeltaConvention`] / [`PremiumStyle`] on the input are retained as
//! provenance and to let downstream views re-derive any non-canonical convention.

use celnet_types::{
    Ccy, CcyPair, DeltaConvention, Greeks, OptionType, PremiumStyle, VanillaInputs,
};
use celnet_vanilla::{convention_delta, greeks as vanilla_greeks};

/// One position's risk **as reported**, in its own (possibly bespoke) pricing
/// conventions — the heterogeneous input to canonicalization.
///
/// The pricing `inputs` are the single source of truth: the canonical Greek set
/// is re-derived from them, so the canonicalization is exact regardless of which
/// convention the book happened to quote in. The `quoted_delta` / `premium_style`
/// fields are retained as **provenance** (and to reconstruct non-canonical views),
/// not as the basis of the canonical numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionRisk {
    /// The currency pair (BASE/QUOTE = CCY1/CCY2).
    pub pair: CcyPair,
    /// Option type (call/put on the base currency).
    pub option: OptionType,
    /// Signed base-currency notional, in units of the **base** currency
    /// (positive = long the option). Per-unit-base Greeks are scaled by this.
    pub notional_base: f64,
    /// The pricing inputs the position was marked under (spot, strike, vol, time,
    /// the two continuous rates). The canonical risk is re-derived from these.
    pub inputs: VanillaInputs,
    /// The delta convention the position was **quoted** under — provenance only.
    pub quoted_delta: DeltaConvention,
    /// The premium style the position was quoted under — provenance, and it
    /// determines whether the quoted delta was premium-adjusted (and therefore
    /// whether the premium carries FX risk).
    pub premium_style: PremiumStyle,
}

impl PositionRisk {
    /// Convenience constructor.
    #[must_use]
    pub const fn new(
        pair: CcyPair,
        option: OptionType,
        notional_base: f64,
        inputs: VanillaInputs,
        quoted_delta: DeltaConvention,
        premium_style: PremiumStyle,
    ) -> Self {
        Self {
            pair,
            option,
            notional_base,
            inputs,
            quoted_delta,
            premium_style,
        }
    }
}

/// The canonical, **convention-free** Greek set for a position, scaled by the
/// position's base notional.
///
/// All sensitivities are in the canonical convention: **delta is spot-unadjusted
/// and premium-excluded**; the premium is carried separately on the
/// [`CanonicalLeaf`]. Delta is dimensionless × notional → a **base-currency
/// amount** (the spot-hedge quantity in CCY1). Vega/volga are per `1.0` absolute
/// vol (divide by 100 for "per vol point"); theta is per year; gamma is `∂²V/∂S²`.
/// Because these are the *additive* leaf measures (§2.5), the cube sums them
/// directly across the hierarchy without any further convention work.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanonicalGreeks {
    /// Spot-unadjusted, premium-excluded delta × notional — a **base-currency**
    /// hedge amount (units of CCY1). This is the leg that nets at the base
    /// currency node; the matching quote-currency leg is derived at numeraire time.
    pub delta_base: f64,
    /// Gamma `∂²V/∂S²` × notional.
    pub gamma: f64,
    /// Vega `∂V/∂σ` (per `1.0` absolute vol) × notional, in **premium currency**
    /// per unit — see [`CanonicalLeaf::vega_premium_ccy`] for the currency.
    pub vega: f64,
    /// Theta `∂V/∂t` per year × notional.
    pub theta: f64,
    /// Vanna `∂²V/∂S∂σ` × notional.
    pub vanna: f64,
    /// Volga `∂²V/∂σ²` × notional.
    pub volga: f64,
    /// Charm `∂(delta_spot)/∂T` per year × notional.
    pub charm: f64,
    /// Speed `∂³V/∂S³` × notional.
    pub speed: f64,
    /// Zomma `∂gamma/∂σ` × notional.
    pub zomma: f64,
    /// Color `∂gamma/∂T` × notional.
    pub color: f64,
}

/// A position's risk re-expressed in the single canonical convention, with the
/// premium split out — the convention-free leaf the cube aggregates over.
///
/// Invariant: a [`CanonicalLeaf`] carries **no** convention choice. Its delta is
/// always spot-unadjusted and premium-excluded; its premium is a standalone
/// monetary line in the quote currency. Any presentation convention (e.g. a
/// premium-adjusted delta) is reconstructed *from* the leaf at view time, never
/// stored back into it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanonicalLeaf {
    /// The pair this leaf came from (its delta nets at this pair's currency legs).
    pub pair: CcyPair,
    /// Spot used to derive the leaf (the rate at which the base delta converts to
    /// a quote-currency leg). Carried so numeraire conversion is self-contained.
    pub spot: f64,
    /// The canonical, premium-excluded Greek set, scaled by notional.
    pub greeks: CanonicalGreeks,
    /// The premium (option PV) as a **separate** monetary line, in the **quote
    /// currency** (domestic), scaled by notional. Positive for a long option.
    /// Excluding it from delta is exactly what makes the delta convention-free;
    /// keeping it lets a premium-adjusted view be reconstructed.
    pub premium_quote: f64,
    /// The currency vega is measured in (the **premium** currency). Vega P&L is in
    /// premium-currency terms, so cross-book vega netting must convert through this
    /// currency, not blindly sum (§2.3). For the canonical leaf the premium PV is
    /// the quote currency, so vega is in the quote currency too.
    pub vega_premium_ccy: Ccy,
    /// Whether the position's *quoted* delta was premium-adjusted. Retained as
    /// provenance: it does not change the canonical (always-unadjusted) delta, but
    /// it tells a reconstructed-view consumer that the original book's number
    /// differed from canonical by the premium-in-base delta.
    pub quoted_was_premium_adjusted: bool,
}

impl CanonicalLeaf {
    /// The premium-in-base delta adjustment: the amount by which a
    /// premium-adjusted (base-paid-premium) delta differs from the canonical
    /// spot-unadjusted delta, as a **base-currency** amount.
    ///
    /// When the premium is paid in the base (foreign) currency it itself carries
    /// FX risk: the premium-adjusted delta is `Δ_unadj − premium_in_base/notional`,
    /// so per the position the base-leg adjustment is `premium_in_base` =
    /// `premium_quote / spot`. A view that wants the premium-adjusted delta
    /// subtracts this from [`CanonicalGreeks::delta_base`]; a quote-paid-premium
    /// position has no such adjustment. Returns `0.0` when the premium does not
    /// carry FX risk.
    #[must_use]
    pub fn premium_base_delta_adjustment(&self) -> f64 {
        if self.quoted_was_premium_adjusted {
            self.premium_quote / self.spot
        } else {
            0.0
        }
    }
}

/// Re-derive a position's risk into the canonical convention (§2.2).
///
/// Pure and deterministic: computes the spot-unadjusted, premium-excluded Greek
/// set from the position's pricing inputs via `celnet-vanilla`'s closed forms,
/// scales by notional, and splits the premium out as a separate quote-currency
/// monetary line. The quoted conventions on the input are consulted only to record
/// provenance (`quoted_was_premium_adjusted`) — they never alter the canonical
/// numbers, which is precisely what makes the result safe to aggregate.
#[must_use]
pub fn canonicalize(pos: &PositionRisk) -> CanonicalLeaf {
    let n = pos.notional_base;
    // Closed-form Greeks under the model (spot/forward delta both reported by the
    // vanilla engine; we take the SPOT, premium-UNADJUSTED leg as canonical).
    let g: Greeks = vanilla_greeks(pos.option, &pos.inputs);
    // The canonical delta is the spot-unadjusted delta. The vanilla `Greeks`
    // already carries `delta_spot` as the premium-unadjusted spot delta, but we
    // re-derive it through `convention_delta` so the canonical definition is
    // pinned to the named convention (and stays correct if the DTO's `delta_spot`
    // semantics ever shift). They are equal by construction; the test suite gates
    // that equality.
    let delta_unadj = convention_delta(DeltaConvention::SpotUnadjusted, pos.option, &pos.inputs);

    let greeks = CanonicalGreeks {
        delta_base: delta_unadj * n,
        gamma: g.gamma * n,
        vega: g.vega * n,
        theta: g.theta * n,
        vanna: g.vanna * n,
        volga: g.volga * n,
        charm: g.charm * n,
        speed: g.speed * n,
        zomma: g.zomma * n,
        color: g.color * n,
    };

    CanonicalLeaf {
        pair: pos.pair,
        spot: pos.inputs.spot,
        greeks,
        // `Greeks.price` is the domestic (quote) PV per unit base; scale by notional.
        premium_quote: g.price * n,
        // Vega P&L is in premium-currency terms; the canonical premium line is the
        // quote currency, so canonical vega is in the quote currency.
        vega_premium_ccy: pos.pair.quote,
        quoted_was_premium_adjusted: pos.premium_style.is_premium_adjusted(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;
    use celnet_types::Ccy;

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    fn usdjpy() -> CcyPair {
        CcyPair::new(Ccy::USD, Ccy::JPY)
    }

    /// The canonical delta is exactly the spot-unadjusted closed-form delta times
    /// notional, independent of the convention the position was *quoted* under.
    /// Two positions identical except for `quoted_delta` / `premium_style` must
    /// canonicalize to the SAME delta — that is the whole point of §2.2.
    #[test]
    fn canonical_delta_is_convention_independent() {
        let inputs = VanillaInputs::new(1.10, 1.12, 0.10, 0.5, 0.04, 0.02);
        let mk =
            |dc, ps| PositionRisk::new(eurusd(), OptionType::Call, 1_000_000.0, inputs, dc, ps);
        let leaves = [
            canonicalize(&mk(
                DeltaConvention::SpotUnadjusted,
                PremiumStyle::DomesticPips,
            )),
            canonicalize(&mk(
                DeltaConvention::ForwardUnadjusted,
                PremiumStyle::PercentDomestic,
            )),
            canonicalize(&mk(
                DeltaConvention::SpotPremiumAdjusted,
                PremiumStyle::PercentForeign,
            )),
            canonicalize(&mk(
                DeltaConvention::ForwardPremiumAdjusted,
                PremiumStyle::ForeignPips,
            )),
        ];
        let d0 = leaves[0].greeks.delta_base;
        for l in &leaves {
            assert!(
                is_close(l.greeks.delta_base, d0, 1e-12, 1e-9),
                "canonical delta varied with quoted convention: {} vs {d0}",
                l.greeks.delta_base
            );
        }
        // And it equals e^{-r_f T}·N(d1)·notional exactly.
        let want =
            convention_delta(DeltaConvention::SpotUnadjusted, OptionType::Call, &inputs) * 1e6;
        assert!(is_close(d0, want, 1e-12, 1e-9));
    }

    /// EURUSD call delta is POSITIVE (long base = long EUR); the matching USDJPY
    /// put delta is NEGATIVE. Sign and scale (× notional) must survive
    /// canonicalization — a sign flip here corrupts every roll-up above.
    #[test]
    fn eurusd_vs_usdjpy_delta_sign_and_scale() {
        let eur_call = canonicalize(&PositionRisk::new(
            eurusd(),
            OptionType::Call,
            5_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.09, 1.0, 0.04, 0.02),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ));
        assert!(
            eur_call.greeks.delta_base > 0.0,
            "EURUSD call must be long base (EUR), positive delta"
        );
        // A 5mm notional ATM call: spot delta ~0.5·e^{-r_f T} → ~2.4mm EUR.
        assert!(
            eur_call.greeks.delta_base > 2.0e6 && eur_call.greeks.delta_base < 3.0e6,
            "ATM 5mm call base delta out of range: {}",
            eur_call.greeks.delta_base
        );

        let jpy_put = canonicalize(&PositionRisk::new(
            usdjpy(),
            OptionType::Put,
            5_000_000.0,
            VanillaInputs::new(156.0, 156.0, 0.10, 1.0, 0.01, 0.05),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ));
        assert!(
            jpy_put.greeks.delta_base < 0.0,
            "USDJPY put must be short base (USD), negative delta"
        );
        // Vega currency for USDJPY is JPY (the premium/quote currency).
        assert_eq!(jpy_put.vega_premium_ccy, Ccy::JPY);
    }

    /// Premium is split out into the quote currency and excluded from delta; the
    /// premium-adjusted *view* is reconstructable. A premium-adjusted-quoted
    /// position records the provenance flag and yields a non-zero base-delta
    /// adjustment; an unadjusted one yields zero.
    #[test]
    fn premium_excluded_and_reconstructable() {
        let inputs = VanillaInputs::new(1.10, 1.15, 0.11, 0.75, 0.035, 0.015);
        // Same economics, premium paid in base (PercentForeign → premium-adjusted)
        // vs in quote (DomesticPips → unadjusted).
        let adj = canonicalize(&PositionRisk::new(
            eurusd(),
            OptionType::Call,
            1_000_000.0,
            inputs,
            DeltaConvention::SpotPremiumAdjusted,
            PremiumStyle::PercentForeign,
        ));
        let unadj = canonicalize(&PositionRisk::new(
            eurusd(),
            OptionType::Call,
            1_000_000.0,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ));
        // Canonical delta is identical regardless of premium style.
        assert!(is_close(
            adj.greeks.delta_base,
            unadj.greeks.delta_base,
            1e-12,
            1e-9
        ));
        // Premium line is identical (same economics) and positive (long option).
        assert!(adj.premium_quote > 0.0);
        assert!(is_close(
            adj.premium_quote,
            unadj.premium_quote,
            1e-12,
            1e-6
        ));
        // Provenance + reconstruction.
        assert!(adj.quoted_was_premium_adjusted);
        assert!(!unadj.quoted_was_premium_adjusted);
        let pa_adjustment = adj.premium_base_delta_adjustment();
        assert!(
            is_close(pa_adjustment, adj.premium_quote / inputs.spot, 1e-12, 1e-9),
            "premium-in-base delta adjustment must be premium_quote/spot"
        );
        assert_eq!(unadj.premium_base_delta_adjustment(), 0.0);
        // Reconstructed premium-adjusted delta = canonical − premium_in_base.
        let reconstructed = adj.greeks.delta_base - pa_adjustment;
        // It must match the directly-computed premium-adjusted delta × notional.
        let direct = convention_delta(
            DeltaConvention::SpotPremiumAdjusted,
            OptionType::Call,
            &inputs,
        ) * 1e6;
        assert!(
            is_close(reconstructed, direct, 1e-9, 1.0),
            "reconstructed premium-adjusted delta {reconstructed} != direct {direct}"
        );
    }

    /// Greeks scale linearly in notional and flip with its sign (a short position
    /// negates every Greek and the premium line).
    #[test]
    fn notional_scaling_is_linear_and_signed() {
        let inputs = VanillaInputs::new(1.30, 1.28, 0.08, 0.5, 0.045, 0.005);
        let long = canonicalize(&PositionRisk::new(
            CcyPair::new(Ccy::GBP, Ccy::USD),
            OptionType::Put,
            2_000_000.0,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ));
        let short = canonicalize(&PositionRisk::new(
            CcyPair::new(Ccy::GBP, Ccy::USD),
            OptionType::Put,
            -2_000_000.0,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ));
        assert!(is_close(
            short.greeks.delta_base,
            -long.greeks.delta_base,
            1e-12,
            1e-9
        ));
        assert!(is_close(short.greeks.vega, -long.greeks.vega, 1e-12, 1e-9));
        assert!(is_close(
            short.premium_quote,
            -long.premium_quote,
            1e-12,
            1e-9
        ));
    }
}
