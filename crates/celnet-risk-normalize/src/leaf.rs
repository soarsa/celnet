//! Convention canonicalization (`docs/RISK-HIERARCHY.md` §2.2), generalized to the
//! cross-asset carry seam (ADR-0008).
//!
//! A position arrives carrying its own quoted conventions — which delta
//! definition it was marked under, and the currency/units its premium is paid in.
//! Two positions under *different* delta conventions cannot be added: "0.25Δ"
//! under spot-premium-adjusted is a different hedge ratio than "0.25Δ" under
//! forward-unadjusted. Before anything is aggregated, this module re-derives each
//! position's risk into the **canonical internal convention**:
//!
//! - **delta**: spot-unadjusted (`Δ = e^{-r_f T}·φ·N(φd₁)` for FX; `∂V/∂S`
//!   premium-unadjusted for every carry asset class), premium **excluded**;
//! - **premium**: carried as a *separate* monetary line (numeraire/quote PV), never
//!   folded into delta — so a premium-adjusted *view* is reconstructable by
//!   subtracting the premium-in-base delta at presentation time.
//!
//! The transform re-uses the asset's own pricing leaf **through the agnostic
//! [`CarryPricer`] seam** ([`crate::AssetPricer`] by default), so the
//! canonicalization is exact and deterministic, never a re-implementation, and
//! never a silent FX proxy for a non-FX position. The discriminant on the asset
//! class is read *only inside the leaf adapter's* own guard (ADR-0008 sanctioned);
//! the canonicalization loop itself never matches on the underlying.
//!
//! The quoted [`DeltaConvention`] / [`PremiumStyle`] are FX **provenance**: they
//! are `Some(..)` for an FX position (and let downstream views re-derive any
//! non-canonical convention) and `None` for the other asset classes, whose delta
//! has a single canonical spot definition and no premium-adjusted view.

use celnet_core::carry::{CarryGreeks, CarryInputs, CarryPriceError, CarryPricer};
use celnet_types::{
    Carry, Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, Underlying, VanillaInputs,
};
use celnet_vanilla::{adjoint_greeks, convention_delta};

/// Which differentiation engine produces the per-position Greek set inside
/// [`canonicalize_with`].
///
/// The canonical leaf carries the **full** Greek strip; how those Greeks are
/// computed is an *internal acceleration* choice that does not change the
/// contract — both engines target the same mathematical sensitivities and are
/// gated equal to numerical tolerance in this crate's test suite.
///
/// - [`Adjoint`](GreekEngine::Adjoint) — **reverse-mode algorithmic
///   differentiation** (`celnet_vanilla::adjoint_greeks`). One reverse sweep of
///   the recorded Garman-Kohlhagen graph yields the whole first-order block at a
///   small constant multiple of one price, *independent of the number of risk
///   factors* — versus bump-and-revalue's O(n) repricings. This is the
///   `docs/RISK-HIERARCHY.md` §3.3 scale path for portfolio risk and is the
///   **default**. The adjoint specialization is an **FX acceleration**: it is only
///   taken for an FX/metal underlying (the only leaf that records the GK graph);
///   for every other asset class the engine transparently falls back to the leaf's
///   own closed-form analytic strip via the seam (the leaves emit analytic Greeks
///   directly, which are themselves the scale path). The equivalence is gated by
///   `adjoint_leaf_matches_analytic_leaf` (FX-scoped).
/// - [`Analytic`](GreekEngine::Analytic) — the leaf's closed-form Greeks via the
///   seam. Retained as the **validation oracle / fallback**: it is the reference
///   the adjoint path is checked against, and the only path for non-FX asset
///   classes.
///
/// Provenance lives here in the doc comment only; the identifier is purpose-named
/// (`Adjoint`, not a person/method name) per guardrail #8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GreekEngine {
    /// Reverse-mode adjoint AD — the scale path, and the default. FX-only fast
    /// path; non-FX transparently uses the analytic seam.
    #[default]
    Adjoint,
    /// Closed-form analytic Greeks (via the seam) — the validation oracle / the
    /// universal cross-asset path.
    Analytic,
}

/// One position's risk **as reported**, in its own (possibly bespoke) pricing
/// conventions — the heterogeneous, cross-asset input to canonicalization.
///
/// The pricing `inputs` are the single source of truth: the canonical Greek set
/// is re-derived from them through the asset's own leaf, so the canonicalization is
/// exact regardless of which convention the book happened to quote in. The
/// `quoted_delta` / `premium_style` fields are FX **provenance** (and reconstruct
/// non-canonical views) — `Some(..)` for FX, `None` for the other asset classes.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionRisk {
    /// The underlying asset (FX pair / metal / equity / commodity / digital asset).
    pub underlying: Underlying,
    /// Option type (call/put on the base / asset leg).
    pub option: OptionType,
    /// Signed base-currency / asset notional, in units of the **base** leg
    /// (positive = long the option). Per-unit Greeks are scaled by this.
    pub notional_base: f64,
    /// The generalized, carry-tagged pricing inputs (spot, strike, vol, time, the
    /// underlying, and the cost-of-carry model). The canonical risk is re-derived
    /// from these through the seam.
    pub inputs: CarryInputs,
    /// FX delta-convention provenance; `None` for non-FX asset classes (whose
    /// delta has a single canonical spot definition and no premium-adjusted view).
    pub quoted_delta: Option<DeltaConvention>,
    /// FX premium-style provenance; `None` for non-FX (the premium currency is the
    /// numeraire/quote leg and is never paid in the asset leg).
    pub premium_style: Option<PremiumStyle>,
}

impl PositionRisk {
    /// FX-ergonomic constructor: lift an FX [`VanillaInputs`] (plus the quoted FX
    /// conventions) into the generalized carry-tagged position. The carry is the FX
    /// two-rate carry, so the canonical numbers are byte-identical to the FX path.
    #[must_use]
    pub fn fx(
        pair: CcyPair,
        option: OptionType,
        notional_base: f64,
        vanilla: VanillaInputs,
        quoted_delta: DeltaConvention,
        premium_style: PremiumStyle,
    ) -> Self {
        Self {
            underlying: Underlying::Fx(pair),
            option,
            notional_base,
            inputs: CarryInputs::new(
                vanilla.spot,
                vanilla.strike,
                vanilla.vol,
                vanilla.t,
                Underlying::Fx(pair),
                Carry::FxRates {
                    r_dom: vanilla.r_dom,
                    r_for: vanilla.r_for,
                },
            ),
            quoted_delta: Some(quoted_delta),
            premium_style: Some(premium_style),
        }
    }

    /// Cross-asset constructor for a non-FX position: the conventions are inert
    /// (`None`), the canonical delta is the leaf's spot-unadjusted `delta_spot`.
    #[must_use]
    pub const fn carry(
        underlying: Underlying,
        option: OptionType,
        notional_base: f64,
        inputs: CarryInputs,
    ) -> Self {
        Self {
            underlying,
            option,
            notional_base,
            inputs,
            quoted_delta: None,
            premium_style: None,
        }
    }

    /// The numeraire (premium/quote-leg) currency for the position, when the
    /// underlying projects to a fiat-quoted [`CcyPair`] (FX, metals). The
    /// cross-asset arms whose quote leg is not an ISO-4217 currency
    /// (equity/commodity quote ccy is on the ref; crypto quote may be a coin) return
    /// the projected fiat numeraire where available, else the caller supplies it.
    #[must_use]
    pub fn numeraire_ccy(&self) -> Option<Ccy> {
        match &self.underlying {
            u @ (Underlying::Fx(_) | Underlying::Metal(_)) => u.as_ccy_pair().map(|p| p.quote),
            Underlying::Equity(e) => Some(e.currency),
            Underlying::Commodity(c) => Some(c.currency),
            // A digital-asset quote leg is a free-form string (fiat or coin); only a
            // fiat quote has a `Ccy` projection — parse it, else `None` (the inverse
            // coin-leg numeraire is the deferred case, see the crate docs).
            Underlying::DigitalAsset(p) => Ccy::parse(&p.quote),
        }
    }
}

/// The canonical, **convention-free** Greek set for a position, scaled by the
/// position's base notional.
///
/// All sensitivities are in the canonical convention: **delta is spot-unadjusted
/// and premium-excluded**; the premium is carried separately on the
/// [`CanonicalLeaf`]. Delta is dimensionless × notional → a **base-leg amount** (the
/// spot-hedge quantity in the asset/base unit). Vega/volga are per `1.0` absolute
/// vol (divide by 100 for "per vol point"); theta is per year; gamma is `∂²V/∂S²`.
/// Because these are the *additive* leaf measures (§2.5), the cube sums them
/// directly across the hierarchy without any further convention work.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanonicalGreeks {
    /// Spot-unadjusted, premium-excluded delta × notional — a **base-leg** hedge
    /// amount (units of the asset/base leg). This is the leg that nets at the base
    /// node; the matching quote-leg is derived at numeraire time.
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
/// premium split out — the convention-free, cross-asset leaf the cube aggregates
/// over.
///
/// Invariant: a [`CanonicalLeaf`] carries **no** convention choice. Its delta is
/// always spot-unadjusted and premium-excluded; its premium is a standalone
/// monetary line in the numeraire/quote currency. Any presentation convention (e.g.
/// a premium-adjusted delta) is reconstructed *from* the leaf at view time, never
/// stored back into it.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalLeaf {
    /// The underlying this leaf came from (its delta nets at this underlying's
    /// base/quote legs). For FX/metals this projects to a [`CcyPair`]; the
    /// cross-asset arms net their base leg by asset id and their quote leg in the
    /// numeraire currency.
    pub underlying: Underlying,
    /// Spot used to derive the leaf (the rate at which the base delta converts to a
    /// quote-leg amount). Carried so numeraire conversion is self-contained.
    pub spot: f64,
    /// The canonical, premium-excluded Greek set, scaled by notional.
    pub greeks: CanonicalGreeks,
    /// The premium (option PV) as a **separate** monetary line, in the **quote /
    /// numeraire currency**, scaled by notional. Positive for a long option.
    /// Excluding it from delta is exactly what makes the delta convention-free;
    /// keeping it lets a premium-adjusted view be reconstructed.
    pub premium_quote: f64,
    /// The currency vega is measured in (the **premium / numeraire** currency). Vega
    /// P&L is in premium-currency terms, so cross-book vega netting must convert
    /// through this currency, not blindly sum (§2.3).
    pub vega_premium_ccy: Ccy,
    /// Whether the position's *quoted* delta was premium-adjusted. FX-only
    /// provenance: it does not change the canonical (always-unadjusted) delta, but
    /// it tells a reconstructed-view consumer that the original book's number
    /// differed from canonical by the premium-in-base delta. Always `false` for a
    /// non-FX position (no premium-adjusted view exists).
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
    /// carry FX risk (always for a non-FX position).
    #[must_use]
    pub fn premium_base_delta_adjustment(&self) -> f64 {
        if self.quoted_was_premium_adjusted {
            self.premium_quote / self.spot
        } else {
            0.0
        }
    }
}

/// Re-derive a position's risk into the canonical convention (§2.2) through the
/// **default** cross-asset dispatcher ([`crate::AssetPricer`]) and the **default**
/// Greek engine ([`GreekEngine::Adjoint`]).
///
/// # Errors
/// Returns [`CarryPriceError`] if no leaf in the dispatcher prices the position's
/// underlying/carry (never a silent FX proxy).
pub fn canonicalize(pos: &PositionRisk) -> Result<CanonicalLeaf, CarryPriceError> {
    canonicalize_with(&crate::AssetPricer, GreekEngine::default(), pos)
}

/// Re-derive a position's risk into the canonical convention (§2.2) under an
/// explicit [`CarryPricer`] dispatcher and [`GreekEngine`].
///
/// The Greek set is produced **through the seam**: `pricer.price_greeks(opt, &ci)`
/// resolves the asset's own leaf (no match-on-underlying in this loop). The
/// canonical *delta* is the spot-unadjusted, premium-excluded delta — `g.delta_spot`
/// for every carry asset class; for FX it is re-derived through
/// [`convention_delta`] pinned to [`DeltaConvention::SpotUnadjusted`] (equal to
/// `g.delta_spot` by construction, gated by the test suite). The FX-only adjoint
/// acceleration replaces the seam's analytic strip for the carry-neutral Greeks
/// when the engine is [`GreekEngine::Adjoint`] *and* the underlying is FX/metal.
///
/// # Errors
/// Returns [`CarryPriceError`] if the dispatcher does not price this position.
pub fn canonicalize_with<P: CarryPricer>(
    pricer: &P,
    engine: GreekEngine,
    pos: &PositionRisk,
) -> Result<CanonicalLeaf, CarryPriceError> {
    let n = pos.notional_base;
    // The leaf's own Greek strip, resolved through the agnostic seam. This is the
    // single virtual call; the asset class is matched inside the leaf adapter, not
    // here (ADR-0008 no-hot-path-match).
    let g: CarryGreeks = pricer.price_greeks(pos.option, &pos.inputs)?;

    // Canonical carry-neutral Greeks. The adjoint engine is an FX-only acceleration
    // — it is taken only for an FX/metal underlying (the leaf that records the GK
    // graph); every other asset class uses the seam's analytic strip directly.
    let g = match (engine, fx_vanilla(pos)) {
        (GreekEngine::Adjoint, Some(vi)) => {
            // Fast reverse-mode sweep of the GK graph, lifted into the generalized
            // strip; the FX rhos are tagged Fx. Equal to the analytic seam to ~1e-9
            // (gated by `adjoint_leaf_matches_analytic_leaf`).
            celnet_core::carry::fx_carry_greeks(&adjoint_greeks(pos.option, &vi))
        }
        // Non-FX, or the analytic engine: the seam's closed-form strip is canonical.
        _ => g,
    };

    // Canonical delta: spot-unadjusted, premium-excluded. For FX, re-derive it
    // through the named convention so the canonical definition stays pinned (it is
    // equal to `g.delta_spot` by construction). The convention re-derivation runs
    // ONLY for an FX position (provenance present) — it never touches a non-FX leg.
    // This `is_some()` test is a provenance branch, not an asset-class match in a
    // numeric hot loop.
    let delta_unadj = match (pos.quoted_delta, fx_vanilla(pos)) {
        (Some(_), Some(vi)) => convention_delta(DeltaConvention::SpotUnadjusted, pos.option, &vi),
        _ => g.delta_spot,
    };

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

    // The premium / numeraire currency. For FX/metals it is the projected quote
    // leg; for the cross-asset arms it is the asset's quote/settlement currency.
    let vega_premium_ccy = pos
        .numeraire_ccy()
        .ok_or(CarryPriceError::UnsupportedUnderlying)?;

    Ok(CanonicalLeaf {
        underlying: pos.underlying.clone(),
        spot: pos.inputs.spot,
        greeks,
        // `price` is the numeraire (quote) PV per unit base; scale by notional.
        premium_quote: g.price * n,
        vega_premium_ccy,
        quoted_was_premium_adjusted: pos.premium_style.is_some_and(|s| s.is_premium_adjusted()),
    })
}

/// The FX [`VanillaInputs`] lowering of a position, iff it is an FX/metal underlying
/// under an FX two-rate carry; else `None`. This is the FX-acceleration / FX-
/// provenance gate — it is the leaf's own "is this FX?" guard reused for the two
/// FX-only specializations (adjoint, convention delta), not a hot-loop match.
fn fx_vanilla(pos: &PositionRisk) -> Option<VanillaInputs> {
    celnet_core::carry::fx_vanilla_inputs(&pos.inputs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AssetPricer;
    use celnet_core::is_close;
    use celnet_types::{Ccy, CommodityRef, EquityRef, Symbol};

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    fn usdjpy() -> CcyPair {
        CcyPair::new(Ccy::USD, Ccy::JPY)
    }

    fn fx_pos(
        pair: CcyPair,
        opt: OptionType,
        n: f64,
        vi: VanillaInputs,
        dc: DeltaConvention,
        ps: PremiumStyle,
    ) -> PositionRisk {
        PositionRisk::fx(pair, opt, n, vi, dc, ps)
    }

    /// G2: the canonical delta is exactly the spot-unadjusted closed-form delta
    /// times notional, independent of the convention the position was *quoted*
    /// under. Two positions identical except for `quoted_delta` / `premium_style`
    /// must canonicalize to the SAME delta — the whole point of §2.2.
    #[test]
    fn canonical_delta_is_convention_independent() {
        let inputs = VanillaInputs::new(1.10, 1.12, 0.10, 0.5, 0.04, 0.02);
        let mk = |dc, ps| fx_pos(eurusd(), OptionType::Call, 1_000_000.0, inputs, dc, ps);
        let leaves = [
            canonicalize(&mk(
                DeltaConvention::SpotUnadjusted,
                PremiumStyle::DomesticPips,
            ))
            .unwrap(),
            canonicalize(&mk(
                DeltaConvention::ForwardUnadjusted,
                PremiumStyle::PercentDomestic,
            ))
            .unwrap(),
            canonicalize(&mk(
                DeltaConvention::SpotPremiumAdjusted,
                PremiumStyle::PercentForeign,
            ))
            .unwrap(),
            canonicalize(&mk(
                DeltaConvention::ForwardPremiumAdjusted,
                PremiumStyle::ForeignPips,
            ))
            .unwrap(),
        ];
        let d0 = leaves[0].greeks.delta_base;
        for l in &leaves {
            assert!(
                is_close(l.greeks.delta_base, d0, 1e-12, 1e-9),
                "canonical delta varied with quoted convention: {} vs {d0}",
                l.greeks.delta_base
            );
        }
        let want =
            convention_delta(DeltaConvention::SpotUnadjusted, OptionType::Call, &inputs) * 1e6;
        assert!(is_close(d0, want, 1e-12, 1e-9));
    }

    /// EURUSD call delta is POSITIVE (long base = long EUR); the matching USDJPY
    /// put delta is NEGATIVE. Sign and scale (× notional) must survive
    /// canonicalization.
    #[test]
    fn eurusd_vs_usdjpy_delta_sign_and_scale() {
        let eur_call = canonicalize(&fx_pos(
            eurusd(),
            OptionType::Call,
            5_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.09, 1.0, 0.04, 0.02),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ))
        .unwrap();
        assert!(eur_call.greeks.delta_base > 0.0);
        assert!(eur_call.greeks.delta_base > 2.0e6 && eur_call.greeks.delta_base < 3.0e6);

        let jpy_put = canonicalize(&fx_pos(
            usdjpy(),
            OptionType::Put,
            5_000_000.0,
            VanillaInputs::new(156.0, 156.0, 0.10, 1.0, 0.01, 0.05),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ))
        .unwrap();
        assert!(jpy_put.greeks.delta_base < 0.0);
        assert_eq!(jpy_put.vega_premium_ccy, Ccy::JPY);
    }

    /// Premium is split out into the quote currency and excluded from delta; the
    /// premium-adjusted *view* is reconstructable.
    #[test]
    fn premium_excluded_and_reconstructable() {
        let inputs = VanillaInputs::new(1.10, 1.15, 0.11, 0.75, 0.035, 0.015);
        let adj = canonicalize(&fx_pos(
            eurusd(),
            OptionType::Call,
            1_000_000.0,
            inputs,
            DeltaConvention::SpotPremiumAdjusted,
            PremiumStyle::PercentForeign,
        ))
        .unwrap();
        let unadj = canonicalize(&fx_pos(
            eurusd(),
            OptionType::Call,
            1_000_000.0,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ))
        .unwrap();
        assert!(is_close(
            adj.greeks.delta_base,
            unadj.greeks.delta_base,
            1e-12,
            1e-9
        ));
        assert!(adj.premium_quote > 0.0);
        assert!(is_close(
            adj.premium_quote,
            unadj.premium_quote,
            1e-12,
            1e-6
        ));
        assert!(adj.quoted_was_premium_adjusted);
        assert!(!unadj.quoted_was_premium_adjusted);
        let pa_adjustment = adj.premium_base_delta_adjustment();
        assert!(is_close(
            pa_adjustment,
            adj.premium_quote / inputs.spot,
            1e-12,
            1e-9
        ));
        assert_eq!(unadj.premium_base_delta_adjustment(), 0.0);
        let reconstructed = adj.greeks.delta_base - pa_adjustment;
        let direct = convention_delta(
            DeltaConvention::SpotPremiumAdjusted,
            OptionType::Call,
            &inputs,
        ) * 1e6;
        assert!(is_close(reconstructed, direct, 1e-9, 1.0));
    }

    /// G4: the adjoint (default) FX leaf equals the analytic (oracle) FX leaf to
    /// numerical tolerance — the equivalence that licenses defaulting
    /// [`canonicalize`] to the fast reverse-mode path for FX.
    #[test]
    fn adjoint_leaf_matches_analytic_leaf() {
        let cases = [
            (
                eurusd(),
                OptionType::Call,
                10_000_000.0,
                VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            ),
            (
                eurusd(),
                OptionType::Put,
                -5_000_000.0,
                VanillaInputs::new(1.10, 1.05, 0.14, 0.25, 0.03, 0.05),
            ),
            (
                usdjpy(),
                OptionType::Call,
                8_000_000.0,
                VanillaInputs::new(156.0, 160.0, 0.09, 2.0, 0.05, 0.01),
            ),
            (
                usdjpy(),
                OptionType::Put,
                -3_000_000.0,
                VanillaInputs::new(156.0, 150.0, 0.18, 0.10, 0.01, 0.04),
            ),
            (
                CcyPair::new(Ccy::GBP, Ccy::USD),
                OptionType::Call,
                1_000_000.0,
                VanillaInputs::new(1.30, 1.30, 0.07, 0.5, 0.045, 0.005),
            ),
        ];
        for (pair, opt, notional, inputs) in cases {
            let pos = fx_pos(
                pair,
                opt,
                notional,
                inputs,
                DeltaConvention::SpotUnadjusted,
                PremiumStyle::DomesticPips,
            );
            let aad = canonicalize_with(&AssetPricer, GreekEngine::Adjoint, &pos).unwrap();
            let ana = canonicalize_with(&AssetPricer, GreekEngine::Analytic, &pos).unwrap();
            // The default `canonicalize` IS the adjoint path.
            assert_eq!(canonicalize(&pos).unwrap(), aad);
            // Delta is convention-pinned → bit-identical regardless of engine.
            assert_eq!(aad.greeks.delta_base, ana.greeks.delta_base);
            let pairs = [
                (aad.greeks.gamma, ana.greeks.gamma, "gamma"),
                (aad.greeks.vega, ana.greeks.vega, "vega"),
                (aad.greeks.theta, ana.greeks.theta, "theta"),
                (aad.greeks.vanna, ana.greeks.vanna, "vanna"),
                (aad.greeks.volga, ana.greeks.volga, "volga"),
                (aad.greeks.charm, ana.greeks.charm, "charm"),
                (aad.greeks.speed, ana.greeks.speed, "speed"),
                (aad.greeks.zomma, ana.greeks.zomma, "zomma"),
                (aad.greeks.color, ana.greeks.color, "color"),
                (aad.premium_quote, ana.premium_quote, "premium"),
            ];
            for (a, b, name) in pairs {
                assert!(
                    is_close(a, b, 1e-7, 1e-9 * notional.abs().max(1.0)),
                    "{name}: adjoint {a} vs analytic {b} on {pair:?} {opt:?}"
                );
            }
        }
    }

    /// Greeks scale linearly in notional and flip with its sign.
    #[test]
    fn notional_scaling_is_linear_and_signed() {
        let inputs = VanillaInputs::new(1.30, 1.28, 0.08, 0.5, 0.045, 0.005);
        let long = canonicalize(&fx_pos(
            CcyPair::new(Ccy::GBP, Ccy::USD),
            OptionType::Put,
            2_000_000.0,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ))
        .unwrap();
        let short = canonicalize(&fx_pos(
            CcyPair::new(Ccy::GBP, Ccy::USD),
            OptionType::Put,
            -2_000_000.0,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ))
        .unwrap();
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

    /// G3: for a non-FX position the canonical delta is exactly the leaf's own
    /// `delta_spot · notional` (NOT an FX proxy), and the conventions are inert. The
    /// canonical greeks equal the asset leaf's own greeks scaled by notional —
    /// proving the seam returns the correct leaf's sensitivities.
    #[test]
    fn non_fx_canonical_delta_is_leaf_delta_spot() {
        // Equity.
        let eq_under = Underlying::Equity(EquityRef::new(Symbol::new("ACME", ""), Ccy::USD));
        let eq_ci = CarryInputs::new(
            100.0,
            105.0,
            0.20,
            1.0,
            eq_under.clone(),
            Carry::CostOfCarry { r: 0.03, b: 0.01 },
        );
        let n = 1_000.0;
        let eq_pos = PositionRisk::carry(eq_under, OptionType::Call, n, eq_ci);
        let leaf = canonicalize(&eq_pos).unwrap();
        let direct = AssetPricer
            .price_greeks(OptionType::Call, &eq_pos.inputs)
            .unwrap();
        assert!(is_close(
            leaf.greeks.delta_base,
            direct.delta_spot * n,
            1e-12,
            1e-9
        ));
        assert!(is_close(leaf.greeks.vega, direct.vega * n, 1e-12, 1e-9));
        assert!(is_close(leaf.greeks.gamma, direct.gamma * n, 1e-12, 1e-12));
        assert!(is_close(leaf.premium_quote, direct.price * n, 1e-12, 1e-9));
        assert_eq!(leaf.vega_premium_ccy, Ccy::USD);
        // Conventions are inert for a non-FX leaf.
        assert!(!leaf.quoted_was_premium_adjusted);
        assert_eq!(leaf.premium_base_delta_adjustment(), 0.0);

        // Commodity.
        let com_under =
            Underlying::Commodity(CommodityRef::new(Symbol::new("BRENT", ""), Ccy::USD));
        let com_ci = CarryInputs::new(
            80.0,
            85.0,
            0.30,
            0.75,
            com_under.clone(),
            Carry::CostOfCarry { r: 0.04, b: 0.02 },
        );
        let com_pos = PositionRisk::carry(com_under, OptionType::Put, 500.0, com_ci);
        let com_leaf = canonicalize(&com_pos).unwrap();
        let com_direct = AssetPricer
            .price_greeks(OptionType::Put, &com_pos.inputs)
            .unwrap();
        assert!(is_close(
            com_leaf.greeks.delta_base,
            com_direct.delta_spot * 500.0,
            1e-12,
            1e-9
        ));
        assert!(
            com_leaf.greeks.delta_base < 0.0,
            "a commodity put has negative spot delta"
        );
    }

    /// The dispatcher refuses to price an underlying no leaf owns rather than
    /// silently mis-pricing — `canonicalize` surfaces the typed error.
    #[test]
    fn unpriceable_position_errors_not_silently_proxies() {
        // An FX underlying carrying a cost-of-carry is malformed for the FX leaf and
        // owned by no other leaf (it is not equity/commodity/crypto).
        let bad = PositionRisk::carry(
            Underlying::Fx(eurusd()),
            OptionType::Call,
            1.0,
            CarryInputs::new(
                1.1,
                1.1,
                0.1,
                1.0,
                Underlying::Fx(eurusd()),
                Carry::CostOfCarry { r: 0.03, b: 0.01 },
            ),
        );
        assert!(canonicalize(&bad).is_err());
    }
}
