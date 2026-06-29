//! The **regulatory** FRTB-SbM parameter set + cross-asset bucket/correlation
//! assignment (BCBS **MAR21**), re-derived from the published Basel text.
//!
//! # What this module adds (and the anti-circularity rule it obeys)
//!
//! [`crate::frtb`] is the SbM *aggregation* machinery — the within-bucket `K_b`, the
//! cross-bucket roll-up, the three-correlation-scenario maximum — and it is
//! **asset-agnostic given buckets + weights as caller data**. What was missing is the
//! **bucket/correlation *assignment*** that maps a cross-asset node's net
//! sensitivities to [`RiskBucket`](crate::frtb::RiskBucket)/[`CurvatureBucket`](crate::frtb::CurvatureBucket)
//! with the **regulatory** risk weights and correlations. This module supplies those
//! constants.
//!
//! ## The 0.75ρ circular-oracle lesson (binding constraint)
//!
//! A prior FRTB bug reused a constant from an in-repo source that was itself
//! unverified, making the oracle **circular**. The binding rule here: **every** numeric
//! FRTB parameter ([`StandardFrtbParams`] risk weights, intra-bucket ρ, the vega
//! maturity-correlation `α`) is re-derived **in a doc comment from the cited BCBS MAR
//! paragraph**, computed independently from first principles, and the test
//! (`standard_frtb_params_match_cited_text`) asserts each constant against a
//! **separately hand-typed value from the paragraph** — never against another in-repo
//! constant. The aggregation in [`crate::frtb`] consumes these as *data*
//! ([`crate::frtb::SbmParams`], the `gamma` closure), so a recalibration is data, not a
//! recompile (guardrail §2.11).
//!
//! # The cited regulatory constants (re-derived from MAR21)
//!
//! - **FX delta risk weight — MAR21.88.** The prescribed delta risk weight for an FX
//!   spot risk factor is **15%** (`RW_fx = 0.15`). MAR21.88 grants a **relief** for a
//!   regulator-specified set of liquid currency pairs: the risk weight **may be divided
//!   by √2**, giving `RW_fx_liquid = 0.15 / √2 = 0.106066…`. Re-derivation: `0.15` is
//!   the verbatim RW; `√2 = 1.414213562373095…`; `0.15 / √2 = 0.10606601717798212…`.
//! - **FX vega risk weight — MAR21.93 / MAR21.94.** The vega risk weight is built from
//!   the simplified `RW_σ = 0.55` (MAR21.94) and the regulatory liquidity horizon for
//!   the FX risk class `LH_FX = 40` (MAR21.93, the per-class `LH` table): the vega RW is
//!   `RW_vega = min( RW_σ · √(LH/10), 1.0 )`. Re-derivation: `√(40/10) = √4 = 2`;
//!   `0.55 · 2 = 1.10`; capped at `1.0` ⇒ `RW_vega_fx = 1.0`. (The cap is the
//!   MAR21.94 ceiling on a risk weight.)
//! - **FX delta intra/cross correlation.** FX delta has a **single risk factor per
//!   pair** (the spot), so the intra-bucket ρ is **degenerate** — one factor, no
//!   off-diagonal term. MAR does **not** prescribe a single FX-delta cross-bucket γ:
//!   each currency is its own bucket and the correlation enters only through the shared
//!   USD leg, which the currency-node netting (`celnet-risk-normalize`'s numeraire
//!   vector) already handles — *not* a γ. So FX-delta cross-bucket correlation is
//!   **structurally absent**; this module does not invent one.
//! - **FX vega maturity correlation — MAR21.94.** Within a vega bucket the correlation
//!   between two option-maturity vertices `T_k`, `T_l` (in years) is
//!   `ρ(T_k,T_l) = exp( −α · |T_k − T_l| / min(T_k,T_l) )` with **α = 0.01**
//!   (MAR21.94). Re-derivation: the kernel is `e^{−α·Δ/min}`; `α = 0.01` is the verbatim
//!   decay; e.g. `ρ(1,2) = exp(−0.01·1/1) = exp(−0.01) = 0.990049834…`.
//! - **Curvature — MAR21.98.** The curvature risk weight for a risk class is the
//!   **largest prescribed delta risk weight** of the bucket's risk factors. For FX the
//!   single spot factor's delta RW is the (relief-adjusted) `RW_fx`, so the curvature
//!   shift uses that RW. The curvature cross-bucket correlation is `(δ-γ)²` (already in
//!   [`crate::frtb::curvature_class`]). MAR21.98 is the cited basis.
//! - **LOW-correlation scenario floor — MAR21.6(2).** The decorrelation scenario sets
//!   `ρ_low = max(2ρ − 1, 0.75·ρ)` — the `0.75ρ` floor that caused the historical bug.
//!   `0.75` is re-typed verbatim from MAR21.6(2) in
//!   `low_scenario_075_floor_rederived`; the production transform lives in
//!   [`crate::frtb::CorrelationScenario::Low`].
//!
//! No person/method/vendor/regulator name appears in any identifier (guardrail #8);
//! the MAR paragraph numbers live in doc comments only. Every constant accessor is
//! `const` so a misuse is a compile-time, not a runtime, surprise.

use celnet_core::ExoticLegPricer;
use celnet_core::carry::CarryPricer;
use celnet_core::math::{exp, sqrt};

use crate::cube::NodeAggregate;
use crate::frtb::{CurvatureBucket, RiskBucket, curvature_legs};

/// The prescribed FRTB-SbM regulatory parameter set, re-derived from BCBS MAR21
/// (see the module docs for the per-constant paragraph derivations).
///
/// This is the **provider** of the cited constants that [`crate::frtb`]'s aggregation
/// consumes as data. It holds no node state; it is a pure constant table whose values
/// are gated against hand-typed paragraph values (never against another in-repo
/// constant — the anti-circularity rule).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StandardFrtbParams {
    /// Whether the FX pair is one of the regulator-specified **liquid** pairs that
    /// receive the MAR21.88 `√2` risk-weight relief on the delta RW.
    liquid_fx: bool,
}

impl StandardFrtbParams {
    /// The FX delta base risk weight `RW_fx = 0.15` (MAR21.88). Re-typed verbatim.
    pub const FX_DELTA_RW: f64 = 0.15;

    /// The FX vega simplified sigma risk weight `RW_σ = 0.55` (MAR21.94).
    pub const FX_VEGA_RW_SIGMA: f64 = 0.55;

    /// The FX risk-class liquidity horizon `LH_FX = 40` (MAR21.93 table), in the
    /// `√(LH/10)` vega-RW scaling.
    pub const FX_LIQUIDITY_HORIZON: f64 = 40.0;

    /// The FX vega maturity-correlation decay `α = 0.01` (MAR21.94).
    pub const VEGA_CORR_ALPHA: f64 = 0.01;

    /// The cited regulatory parameter set for a **non-liquid** (full-weight) FX pair.
    #[must_use]
    pub const fn new() -> Self {
        Self { liquid_fx: false }
    }

    /// The cited parameter set for a regulator-specified **liquid** FX pair (the
    /// MAR21.88 `√2` delta-RW relief applies).
    #[must_use]
    pub const fn liquid_fx() -> Self {
        Self { liquid_fx: true }
    }

    /// The FX **delta** risk weight applicable to this parameter set: `0.15` for a
    /// standard pair, or `0.15 / √2` for a regulator-specified liquid pair (MAR21.88).
    #[must_use]
    pub fn fx_delta_rw(&self) -> f64 {
        if self.liquid_fx {
            Self::FX_DELTA_RW / sqrt(2.0)
        } else {
            Self::FX_DELTA_RW
        }
    }

    /// The MAR21.88 liquid-pair relieved FX delta risk weight `0.15 / √2`, irrespective
    /// of this set's `liquid_fx` flag (exposed for the gate).
    #[must_use]
    pub fn fx_delta_rw_liquid() -> f64 {
        Self::FX_DELTA_RW / sqrt(2.0)
    }

    /// The FX **vega** risk weight `min( RW_σ · √(LH_FX/10), 1.0 )` (MAR21.93/.94).
    /// Re-derivation: `0.55 · √(40/10) = 0.55 · 2 = 1.10`, capped at `1.0`.
    #[must_use]
    pub fn fx_vega_rw(&self) -> f64 {
        (Self::FX_VEGA_RW_SIGMA * sqrt(Self::FX_LIQUIDITY_HORIZON / 10.0)).min(1.0)
    }

    /// The FX **curvature** shift risk weight — the largest delta RW of the bucket's
    /// factors (MAR21.98); for FX's single spot factor that is the (relief-adjusted)
    /// delta RW.
    #[must_use]
    pub fn fx_curvature_rw(&self) -> f64 {
        self.fx_delta_rw()
    }

    /// The FX vega maturity-correlation decay `α = 0.01` (MAR21.94).
    #[must_use]
    pub const fn vega_corr_alpha(&self) -> f64 {
        Self::VEGA_CORR_ALPHA
    }

    /// The MAR21.94 vega maturity-correlation kernel between two option-maturity
    /// vertices `t_k`, `t_l` (in years): `ρ = exp(−α · |t_k − t_l| / min(t_k, t_l))`,
    /// clamped to `[0, 1]`. A zero/degenerate maturity (`min ≤ 0`) collapses to a
    /// perfectly-correlated `1.0` (the only well-defined limit). This is the kernel
    /// the [`standard_vega_buckets`] correlation closure evaluates.
    #[must_use]
    pub fn vega_maturity_corr(&self, t_k: f64, t_l: f64) -> f64 {
        let m = t_k.min(t_l);
        if m <= 0.0 {
            return 1.0;
        }
        let rho = exp(-self.vega_corr_alpha() * (t_k - t_l).abs() / m);
        rho.clamp(0.0, 1.0)
    }
}

impl Default for StandardFrtbParams {
    fn default() -> Self {
        Self::new()
    }
}

/// Assign a node's net **delta** sensitivity to FRTB delta [`RiskBucket`]s with the
/// MAR21.88-cited risk weight — **asset-agnostic**, driven off the canonical leaves'
/// currency/asset legs, never a match on the underlying.
///
/// # How the buckets are formed (cross-asset, no match-on-underlying)
///
/// Under FRTB FX-delta each **currency** is its own bucket (MAR21.88), and the
/// sensitivity is the net delta the firm carries against that currency's spot. The
/// node's canonical leaves were produced through the seam, so each leaf already knows
/// its own currency/asset legs (`underlying.as_ccy_pair()`): the **base** leg carries
/// `+delta_base` units of the base asset and the **quote** leg carries
/// `−delta_base·spot` of the numeraire. We net these per **currency leg** and weight
/// the net by `RW_fx`. The bucket id is the leg's currency code (its `Ccy` byte
/// packing), so a EURUSD delta, an XAUUSD delta and a fiat-quoted equity delta all
/// bucket by their **risk-factor leg** — the cross-asset generalization — and the
/// SbM aggregation in [`crate::frtb`] is unchanged.
///
/// A leg with no fiat-currency projection (a non-fiat asset leg) is skipped here (its
/// asset-unit bucketing is the deferred cross-asset numeraire-collapse case — see the
/// `celnet-risk-normalize` honest-scope note); its **numeraire** funding leg is still
/// netted.
#[must_use]
pub fn standard_delta_buckets(node: &NodeAggregate, rw: &StandardFrtbParams) -> Vec<RiskBucket> {
    // Net the signed currency-leg exposure per currency, preserving first-seen order.
    let mut legs: Vec<(u32, f64)> = Vec::new();
    let mut net = |id: u32, amount: f64| {
        if let Some(slot) = legs.iter_mut().find(|(k, _)| *k == id) {
            slot.1 += amount;
        } else {
            legs.push((id, amount));
        }
    };
    for leaf in &node.leaves {
        let Some(pair) = leaf.underlying.as_ccy_pair() else {
            continue;
        };
        // The base leg holds +delta_base (asset/base units); the quote leg holds
        // −delta_base·spot (the numeraire funding leg). Both are currency risk factors
        // for a fiat-quoted underlying.
        net(ccy_id(pair.base), leaf.greeks.delta_base);
        net(ccy_id(pair.quote), -leaf.greeks.delta_base * leaf.spot);
    }
    let w = rw.fx_delta_rw();
    // One single-factor bucket per currency: WS = RW · s; a single factor ⇒ degenerate
    // intra-bucket ρ (no off-diagonal). MAR21.88.
    legs.into_iter()
        .map(|(id, s)| RiskBucket::new(id, vec![w * s], 0.0))
        .collect()
}

/// Assign a node's bucketed **vega** to (currency × maturity) FRTB vega buckets with
/// the MAR21.94 maturity-correlation kernel, and return the within-bucket correlation
/// closure (over the returned buckets' maturity vertices).
///
/// Each [`RiskBucket`] groups the vega vertices sharing a currency (the leaf's
/// premium currency) into one bucket whose `ws` are `RW_vega · vega_k` per maturity
/// vertex `k`; the returned closure `corr(i, j)` is the MAR21.94 kernel evaluated on
/// the two vertices' maturities. The vega risk weight is the MAR21.93/.94
/// `min(0.55·√(LH/10), 1)`. The buckets are keyed by currency id; the maturity
/// vertices live in the parallel `maturities` vector the closure reads.
///
/// The vega per vertex is the asset's own `∂V/∂σ` (read through the seam for a vanilla
/// position, from the exotic leg's canonical Greeks for an exotic), at the position's
/// **own option maturity** `t` — the genuine maturity the MAR21.94 kernel needs (not a
/// fabricated vertex). Positions sharing a `(currency, maturity)` net into one vertex.
///
/// Returns `(buckets, maturities_by_bucket)`: `maturities_by_bucket[b][k]` is the
/// maturity (years) of bucket `b`'s `k`-th vega vertex, so a caller building the
/// within-bucket `K_b` supplies `|i, j| rw.vega_maturity_corr(mats[b][i], mats[b][j])`.
#[must_use]
pub fn standard_vega_buckets<P: CarryPricer>(
    pricer: &P,
    exotic_pricer: &dyn ExoticLegPricer,
    node: &NodeAggregate,
    rw: &StandardFrtbParams,
) -> (Vec<RiskBucket>, Vec<Vec<f64>>) {
    // Group vega vertices by premium currency, keeping (maturity-years, weighted-vega).
    let mut by_ccy: Vec<(u32, Vec<(f64, f64)>)> = Vec::new();
    let w = rw.fx_vega_rw();
    let push = |id: u32, t: f64, ws: f64, by_ccy: &mut Vec<(u32, Vec<(f64, f64)>)>| {
        if let Some(slot) = by_ccy.iter_mut().find(|(k, _)| *k == id) {
            // Net vertices at the same maturity into one vertex (additive vega).
            if let Some(v) = slot.1.iter_mut().find(|(tt, _)| (*tt - t).abs() < 1e-12) {
                v.1 += ws;
            } else {
                slot.1.push((t, ws));
            }
        } else {
            by_ccy.push((id, vec![(t, ws)]));
        }
    };
    // Vanilla legs: vega via the seam, maturity from the carry inputs (asset-agnostic).
    for pos in &node.positions {
        let Ok(g) = pricer.price_greeks(pos.option, &pos.inputs) else {
            continue;
        };
        let Some(ccy) = pos.numeraire_ccy() else {
            continue;
        };
        push(
            ccy_id(ccy),
            pos.inputs.t,
            w * g.vega * pos.notional_base,
            &mut by_ccy,
        );
    }
    // Exotic legs: vega from the exotic's canonical leaf, maturity from its inputs.
    for leg in &node.exotic_legs {
        let leaf = leg.canonical_leaf(exotic_pricer);
        push(
            ccy_id(leg.quote_ccy()),
            leg.inputs.t,
            w * leaf.greeks.vega,
            &mut by_ccy,
        );
    }
    let mut buckets = Vec::with_capacity(by_ccy.len());
    let mut mats = Vec::with_capacity(by_ccy.len());
    // The within-bucket maturity correlation is supplied per bucket by the caller via
    // `rw.vega_maturity_corr`; a single representative ρ on `RiskBucket::rho_intra`
    // would lose the term structure, so we expose the maturities for the exact kernel
    // and set `rho_intra = 0.0` (the bucket's K_b is then built by the caller with the
    // maturity closure, e.g. via `quadratic_form`). The buckets carry the WS for the
    // cross-bucket S_b sum.
    for (id, mut verts) in by_ccy {
        verts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
        let ws: Vec<f64> = verts.iter().map(|(_, w)| *w).collect();
        let m: Vec<f64> = verts.iter().map(|(t, _)| *t).collect();
        buckets.push(RiskBucket::new(id, ws, 0.0));
        mats.push(m);
    }
    (buckets, mats)
}

/// Assign a node's spot **curvature** to a single-factor FX [`CurvatureBucket`] with
/// the MAR21.98 largest-delta-RW shift, re-pricing through the seam.
///
/// The curvature shift is the (relief-adjusted) FX delta RW (MAR21.98: the curvature
/// RW is the largest delta RW of the bucket's factors; FX has one spot factor). The
/// up/down legs come from [`curvature_legs`] (the same arithmetic the existing
/// curvature lens uses), re-priced through the agnostic `pricer`. One bucket id is
/// supplied by the caller (the node's currency-pair group id).
#[must_use]
pub fn standard_curvature_buckets<P: CarryPricer>(
    pricer: &P,
    node: &NodeAggregate,
    id: u32,
    rw: &StandardFrtbParams,
) -> Vec<CurvatureBucket> {
    let shift = rw.fx_curvature_rw();
    let (cvr_up, cvr_down) = curvature_legs(pricer, &node.positions, shift);
    vec![CurvatureBucket {
        id,
        cvr_up,
        cvr_down,
        rho_intra: 0.0,
    }]
}

/// Pack a [`Ccy`](celnet_types::Ccy) into a stable `u32` bucket id (its three ISO
/// letters in the low 24 bits). Deterministic and collision-free over ISO-4217.
#[must_use]
fn ccy_id(ccy: celnet_types::Ccy) -> u32 {
    let b = ccy.as_str().as_bytes();
    let mut v: u32 = 0;
    for &x in b {
        v = (v << 8) | u32::from(x);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frtb::CorrelationScenario;

    /// G7: every cited [`StandardFrtbParams`] constant equals a value **hand-typed
    /// from the MAR21 paragraph** — never read from another in-repo constant. This is
    /// the anti-circular guard: the test's literals are re-derived from the published
    /// text in the comments, and the production accessors must reproduce them.
    ///
    /// `clippy::approx_constant` is intentionally allowed: the √2 literal is hand-typed
    /// from MAR21.88's "divide by √2" relief on purpose — substituting
    /// `std::f64::consts::SQRT_2` (what the lint suggests) would defeat the
    /// anti-circular guarantee that this test's value is independent of any in-repo
    /// constant, including the standard library's.
    #[test]
    #[allow(clippy::approx_constant)]
    fn standard_frtb_params_match_cited_text() {
        let p = StandardFrtbParams::new();

        // MAR21.88: FX delta RW = 15%. Hand-typed: 0.15.
        assert_eq!(p.fx_delta_rw().to_bits(), 0.15_f64.to_bits());

        // MAR21.88 relief: divide by √2. Hand-typed √2 = 1.4142135623730951.
        let sqrt2: f64 = 1.414_213_562_373_095_1;
        let want_liquid = 0.15_f64 / sqrt2;
        assert!(
            (StandardFrtbParams::fx_delta_rw_liquid() - want_liquid).abs() <= 1e-15,
            "liquid FX delta RW {} vs hand-typed 0.15/√2 {want_liquid}",
            StandardFrtbParams::fx_delta_rw_liquid()
        );
        // And the liquid-flagged set actually applies the relief.
        assert!((StandardFrtbParams::liquid_fx().fx_delta_rw() - want_liquid).abs() <= 1e-15);

        // MAR21.93/.94: FX vega RW = min(0.55·√(40/10), 1) = min(0.55·2, 1) = min(1.10, 1)
        // = 1.0. Hand-typed.
        let want_vega = (0.55_f64 * (40.0_f64 / 10.0).sqrt()).min(1.0);
        assert_eq!(want_vega.to_bits(), 1.0_f64.to_bits());
        assert_eq!(p.fx_vega_rw().to_bits(), 1.0_f64.to_bits());

        // MAR21.94: vega maturity-correlation decay α = 0.01. Hand-typed.
        assert_eq!(p.vega_corr_alpha().to_bits(), 0.01_f64.to_bits());

        // MAR21.98: FX curvature shift RW = the largest delta RW = the (relief-adjusted)
        // delta RW. For a standard pair that is 0.15.
        assert_eq!(p.fx_curvature_rw().to_bits(), 0.15_f64.to_bits());
    }

    /// G6 (anti-circular floor for the historical 0.75ρ bug): the LOW-correlation
    /// scenario floor `max(2ρ−1, 0.75ρ)` is re-derived with **0.75 hand-typed from
    /// MAR21.6(2)** and asserted against [`CorrelationScenario::Low::scale`] at several
    /// ρ. The `0.75` is NOT read from any in-repo constant — exactly the
    /// circular-oracle guard the FRTB lesson demands.
    #[test]
    fn low_scenario_075_floor_rederived() {
        // MAR21.6(2): LOW correlation = max(2ρ − 1, 0.75·ρ). 0.75 is the verbatim floor.
        let floor: f64 = 0.75; // hand-typed from MAR21.6(2).
        let hand = |rho: f64| -> f64 { (2.0 * rho - 1.0).max(floor * rho) };
        for &rho in &[0.0, 0.1, 0.25, 0.3, 0.5, 0.6, 0.75, 0.9, 1.0] {
            assert_eq!(
                CorrelationScenario::Low.scale(rho).to_bits(),
                hand(rho).to_bits(),
                "LOW scenario at ρ={rho} must equal hand-typed max(2ρ−1, 0.75ρ)"
            );
        }
        // The floor genuinely binds where 2ρ−1 < 0.75ρ (small ρ): at ρ=0.3,
        // max(−0.4, 0.225) = 0.225 = 0.75·0.3.
        assert!((CorrelationScenario::Low.scale(0.3) - 0.225).abs() <= 1e-15);
        // And the 2ρ−1 arm wins for large ρ: at ρ=0.9, max(0.8, 0.675) = 0.8.
        assert!((CorrelationScenario::Low.scale(0.9) - 0.8).abs() <= 1e-15);
    }

    /// The MAR21.94 vega maturity-correlation kernel `exp(−α·|Δ|/min)` evaluates
    /// correctly: ρ(t,t)=1, monotone-decreasing in the maturity gap, in [0,1], with α
    /// hand-typed. At (1,2): exp(−0.01·1/1) = exp(−0.01) = 0.9900498337...
    #[test]
    fn vega_maturity_kernel_matches_cited_formula() {
        let p = StandardFrtbParams::new();
        assert_eq!(p.vega_maturity_corr(1.0, 1.0).to_bits(), 1.0_f64.to_bits());
        let alpha: f64 = 0.01; // MAR21.94, hand-typed.
        let want_12 = (-alpha * 1.0_f64 / 1.0).exp();
        assert!((p.vega_maturity_corr(1.0, 2.0) - want_12).abs() <= 1e-15);
        assert!((p.vega_maturity_corr(2.0, 1.0) - want_12).abs() <= 1e-15);
        // A wider gap is less correlated.
        assert!(p.vega_maturity_corr(0.25, 5.0) < p.vega_maturity_corr(1.0, 2.0));
        // Always in [0, 1].
        for &(a, b) in &[(0.1, 10.0), (1.0, 1.0), (0.5, 0.5)] {
            let r = p.vega_maturity_corr(a, b);
            assert!((0.0..=1.0).contains(&r));
        }
    }
}
