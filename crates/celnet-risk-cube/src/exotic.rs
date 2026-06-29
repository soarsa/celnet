//! Exotic-leg risk measure (`docs/RISK-HIERARCHY.md` §2.5, exotic extension).
//!
//! # The gap this closes
//!
//! The cube's first generation aggregated **vanilla** legs only: a [`RiskFact`]
//! carried a `celnet-risk-normalize` [`CanonicalLeaf`] (additive Greeks) and the
//! originating *vanilla* [`PositionRisk`], and the non-additive reducers re-priced
//! that vanilla position under shocks via `celnet-vanilla`. A booked **exotic**
//! (barrier, digital) had no vanilla leaf, so it was silently **excluded** from
//! firm/desk/book risk roll-ups — a position on the books contributing zero to the
//! firm's Greeks, VaR/ES and curvature. That is a correctness hole, not a
//! simplification: an excluded leg understates the firm's risk.
//!
//! This module gives an exotic leg a genuine seat in the cube:
//!
//! 1. **Additive** — its full [`CanonicalGreeks`] set + premium line is produced
//!    here and carried as a [`CanonicalLeaf`], so it sums into the net-Greeks /
//!    vega-ladder roll-up exactly like a vanilla leaf. The Greeks are the **real
//!    exotic** sensitivities, not a vanilla proxy: digitals use the closed-form
//!    `celnet-exotics` digital Greeks; barriers use a central finite-difference of
//!    the closed-form Reiner-Rubinstein barrier price (the barrier closed form has
//!    no published higher-order Greek set, so FD of the exact price is the honest
//!    source — deterministic and `libm`-routed, hence bit-reproducible).
//! 2. **Non-additive** — the leg re-prices the **real exotic** under each
//!    [`Scenario`] for VaR/ES, and supplies its own up/down curvature legs for
//!    FRTB-SbM, so the firm tail and curvature reflect the exotic's true,
//!    non-vanilla payoff (a knock-out's gamma sign flip near the barrier, a
//!    digital's pin risk) — never a vanilla approximation.
//!
//! # Scope: deterministic closed-form exotics (honest)
//!
//! The first-class exotic legs here are the **deterministic, closed-form** members
//! of the `celnet-exotics` catalogue — the **single barrier** (Reiner-Rubinstein)
//! and the **European digital** (cash- / asset-or-nothing). They re-price exactly
//! under shocks and yield exact FD Greeks with **no Monte-Carlo estimator noise**,
//! so they slot into the cube's machine-exact VaR/curvature path without the
//! regression that mixing MC noise into an exact reval would cause (the same stance
//! the crate takes on the GPU-MC lever — see [`crate::nonadditive`]).
//!
//! MC-priced exotics (Asian, TARF, accumulator, lookback-discrete) are a **named
//! extension**, not a stub: their honest seat is a finite-difference of an
//! MC-with-common-random-numbers price (the same approach the server's exotic
//! pricer uses), which carries an explicit Monte-Carlo standard-error caveat on the
//! resulting Greeks. They are deliberately **not** added here so this module's legs
//! stay exact; the [`ExoticKind`] enum is the seam they extend.

use celnet_core::ExoticLegPricer;
use celnet_risk_normalize::{CanonicalGreeks, CanonicalLeaf};
use celnet_types::{Ccy, CcyPair, ExoticKind, Underlying, VanillaInputs};

use crate::nonadditive::Scenario;

/// Apply a [`Scenario`]'s shocks to an FX-exotic leg's [`VanillaInputs`]: spot
/// relative, vol absolute, and the FX rates from the asset-agnostic carry shocks
/// (`Δr_dom = discount_abs`, `Δr_for = discount_abs − carry_abs`, the inverse of the
/// FX carry packing `r = r_dom`, `b = r_dom − r_for`). This is the FX projection of
/// [`crate::nonadditive::shift_carry`] for the FX-only exotic catalogue.
#[must_use]
fn apply_fx(scenario: Scenario, i: &VanillaInputs) -> VanillaInputs {
    VanillaInputs::new(
        i.spot * (1.0 + scenario.spot_rel),
        i.strike,
        i.vol + scenario.vol_abs,
        i.t,
        i.r_dom + scenario.discount_abs,
        i.r_for + (scenario.discount_abs - scenario.carry_abs),
    )
}

// [`ExoticKind`] (and its `SingleBarrier` / `DigitalKind` payload types) now live in
// `celnet-types` so the cube names a closed-form exotic family without depending on
// the `celnet-exotics` pricing crate; the pricing itself is performed by the
// injected [`ExoticLegPricer`] (`celnet-core`). See `celnet_types::ExoticKind`.

/// A booked exotic position as a cube risk leg: its closed-form kind, the market
/// inputs it is marked under, its signed notional, its currency pair, and the
/// quoted/quote-currency metadata the leaf needs.
///
/// The leg is the **re-derivation source** for the non-additive reducers (it
/// re-prices the real exotic under shocks) and the source of the additive
/// [`CanonicalLeaf`] (its real exotic Greeks). It is `Copy` so it flows through the
/// cube/fleet exactly like a vanilla [`PositionRisk`](celnet_risk_normalize::PositionRisk).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExoticLeg {
    /// The currency pair (BASE/QUOTE).
    pub pair: CcyPair,
    /// Which closed-form exotic, with its payoff parameters.
    pub kind: ExoticKind,
    /// Signed notional (positive = long), in units of the payout (base ccy for a
    /// barrier; payout units for a digital). Every per-unit sensitivity is scaled
    /// by this.
    pub notional: f64,
    /// The market inputs the leg is marked under (spot, strike, vol, time, the two
    /// rates). The non-additive reducers shock *these* and re-price the exotic.
    pub inputs: VanillaInputs,
}

impl ExoticLeg {
    /// Construct an exotic leg.
    #[must_use]
    pub const fn new(
        pair: CcyPair,
        kind: ExoticKind,
        notional: f64,
        inputs: VanillaInputs,
    ) -> Self {
        Self {
            pair,
            kind,
            notional,
            inputs,
        }
    }

    /// The leg's value (notional-scaled premium) in the **quote** (domestic)
    /// currency, under the supplied market inputs, priced through the injected seam.
    #[must_use]
    fn value(&self, pricer: &dyn ExoticLegPricer, inputs: &VanillaInputs) -> f64 {
        pricer.unit_price(self.kind, inputs) * self.notional
    }

    /// The leg's base value under its own (unshocked) inputs, priced through the
    /// injected seam.
    #[must_use]
    pub fn base_value(&self, pricer: &dyn ExoticLegPricer) -> f64 {
        self.value(pricer, &self.inputs)
    }

    /// Re-price the leg under a [`Scenario`] and return its **P&L** vs base in the
    /// quote (domestic) currency, signed by the long/short notional — the exotic
    /// analogue of [`crate::nonadditive::position_pnl`].
    ///
    /// The FX-exotic legs are priced in the FX two-rate basis, so the scenario's
    /// asset-agnostic `(discount_abs, carry_abs)` carry shocks are mapped back to the
    /// FX rate moves (`Δr_dom = discount_abs`, `Δr_for = discount_abs − carry_abs`)
    /// before repricing — the same carry transform [`crate::nonadditive::shift_carry`]
    /// applies to the [`celnet_types::Carry::FxRates`] arm, keeping a mixed
    /// vanilla+exotic node's scenario consistent.
    #[must_use]
    pub fn pnl(&self, pricer: &dyn ExoticLegPricer, scenario: Scenario) -> f64 {
        let shocked = apply_fx(scenario, &self.inputs);
        self.value(pricer, &shocked) - self.base_value(pricer)
    }

    /// The exotic's **spot delta** `∂V/∂S` per unit payout, by central finite
    /// difference of the closed-form price (digitals could use their closed-form
    /// delta, but a single FD path keeps the curvature linear term consistent
    /// across both kinds). Deterministic and `libm`-routed.
    #[must_use]
    fn unit_delta_spot(&self, pricer: &dyn ExoticLegPricer) -> f64 {
        let h = spot_bump(self.inputs.spot);
        let up = pricer.unit_price(self.kind, &with_spot(&self.inputs, self.inputs.spot + h));
        let down = pricer.unit_price(self.kind, &with_spot(&self.inputs, self.inputs.spot - h));
        (up - down) / (2.0 * h)
    }

    /// The FRTB-SbM curvature legs `(CVR_up, CVR_down)` for this exotic along the
    /// spot factor under a relative risk weight `rw`: the up/down full reprice net
    /// of the leg's own linear (delta) term, signed so a loss is positive
    /// (`docs/RISK-HIERARCHY.md` §2.5, MAR21). Returned per leg so a node sums the
    /// vanilla and exotic legs into one `(up, down)` pair before the `max`.
    #[must_use]
    pub fn curvature_legs(&self, pricer: &dyn ExoticLegPricer, rw: f64) -> (f64, f64) {
        let base = self.base_value(pricer);
        let up = self.value(
            pricer,
            &with_spot(&self.inputs, self.inputs.spot * (1.0 + rw)),
        );
        let down = self.value(
            pricer,
            &with_spot(&self.inputs, self.inputs.spot * (1.0 - rw)),
        );
        // Linear term: ∂V/∂S · notional · rw · S (the exact absolute spot move).
        let linear = self.unit_delta_spot(pricer) * self.notional * rw * self.inputs.spot;
        let cvr_up = -((up - base) - linear);
        let cvr_down = -((down - base) + linear);
        (cvr_up, cvr_down)
    }

    /// The exotic's full canonical Greek **leaf** (additive measure), notional-scaled
    /// and in the canonical convention (spot-unadjusted base delta, premium split
    /// out as a quote-currency line). The Greek set is the **real exotic**'s:
    /// digitals via the closed-form `celnet-exotics` digital Greeks; barriers via a
    /// central finite-difference of the closed-form barrier price.
    ///
    /// Higher-order Greeks the closed forms do not publish (a barrier's charm /
    /// speed / zomma / color, a digital's vanna / volga / charm / speed / zomma /
    /// color) are produced by the same deterministic FD scheme rather than left at
    /// zero (which would understate the roll-up) — the leaf carries the full set the
    /// additive ladder sums.
    #[must_use]
    pub fn canonical_leaf(&self, pricer: &dyn ExoticLegPricer) -> CanonicalLeaf {
        let g = self.canonical_greeks(pricer);
        CanonicalLeaf {
            underlying: Underlying::Fx(self.pair),
            spot: self.inputs.spot,
            greeks: g,
            premium_quote: pricer.unit_price(self.kind, &self.inputs) * self.notional,
            // Vega P&L is in premium-currency terms; the exotic premium line is the
            // quote currency, so vega is in the quote currency too (mirrors the
            // vanilla canonical leaf).
            vega_premium_ccy: self.pair.quote,
            // An exotic leg is not quoted under a premium-adjusted vanilla delta
            // convention; the canonical delta is already convention-free.
            quoted_was_premium_adjusted: false,
        }
    }

    /// The notional-scaled canonical Greek set. The first-order block + key
    /// second-order Greeks come from the exotic's own analytics where a closed form
    /// exists; the remainder are central finite differences of the closed-form
    /// price — every value is the **exotic**'s sensitivity, never a vanilla proxy.
    #[must_use]
    fn canonical_greeks(&self, pricer: &dyn ExoticLegPricer) -> CanonicalGreeks {
        let n = self.notional;
        let i = &self.inputs;
        // Spot bumps for FD (relative to spot scale); vol bump in absolute vol;
        // time bump in years. All chosen on the standard √ε scale for a central
        // second difference, clamped so a short tenor / tiny spot stays well-posed.
        let ds = spot_bump(i.spot);
        let dv = 1e-4_f64;
        let dt = (i.t * 1e-4).max(1e-6).min(i.t * 0.5);

        // Helper: unit price under (spot, vol, t) perturbations, through the seam.
        let p = |spot: f64, vol: f64, t: f64| -> f64 {
            pricer.unit_price(
                self.kind,
                &VanillaInputs::new(spot, i.strike, vol, t, i.r_dom, i.r_for),
            )
        };
        let s = i.spot;
        let v = i.vol;
        let t = i.t;
        let base = p(s, v, t);

        // First/second order spot: central differences.
        let delta_unit = (p(s + ds, v, t) - p(s - ds, v, t)) / (2.0 * ds);
        let gamma_unit = (p(s + ds, v, t) - 2.0 * base + p(s - ds, v, t)) / (ds * ds);
        // Speed ∂³V/∂S³ (central 4-point).
        let speed_unit = (p(s + 2.0 * ds, v, t) - 2.0 * p(s + ds, v, t) + 2.0 * p(s - ds, v, t)
            - p(s - 2.0 * ds, v, t))
            / (2.0 * ds * ds * ds);

        // Vol: vega ∂V/∂σ, volga ∂²V/∂σ².
        let vega_unit = (p(s, v + dv, t) - p(s, v - dv, t)) / (2.0 * dv);
        let volga_unit = (p(s, v + dv, t) - 2.0 * base + p(s, v - dv, t)) / (dv * dv);
        // Vanna ∂²V/∂S∂σ (cross central difference).
        let vanna_unit = (p(s + ds, v + dv, t) - p(s + ds, v - dv, t) - p(s - ds, v + dv, t)
            + p(s - ds, v - dv, t))
            / (4.0 * ds * dv);

        // Theta ∂V/∂t (per year, calendar decay = −∂V/∂T at fixed expiry; the
        // pricer is parameterized by time-to-expiry T): theta = −central-diff in T.
        // This matches the vanilla closed form's theta sign, so vanilla and exotic
        // leaves SUM consistently in the additive roll-up.
        let theta_unit = -(p(s, v, t + dt) - p(s, v, t - dt)) / (2.0 * dt);
        // Charm ∂(delta_spot)/∂T — the canonical-leaf convention (CanonicalGreeks
        // doc; identical to the vanilla closed form's charm, which is validated
        // against a +∂/∂T finite difference). NOT calendar-negated: a sign flip
        // here would net an exotic's charm against the vanillas' with the wrong
        // sign (the defect the far-barrier vanilla-limit oracle pins).
        let charm_unit = ((p(s + ds, v, t + dt) - p(s - ds, v, t + dt))
            - (p(s + ds, v, t - dt) - p(s - ds, v, t - dt)))
            / (4.0 * ds * dt);
        // Zomma ∂gamma/∂σ.
        let zomma_unit = ((p(s + ds, v + dv, t) - 2.0 * p(s, v + dv, t) + p(s - ds, v + dv, t))
            - (p(s + ds, v - dv, t) - 2.0 * p(s, v - dv, t) + p(s - ds, v - dv, t)))
            / (2.0 * dv * ds * ds);
        // Color ∂gamma/∂T — same canonical ∂/∂T convention as charm (matches the
        // vanilla closed form summed alongside in the ladder).
        let color_unit = ((p(s + ds, v, t + dt) - 2.0 * p(s, v, t + dt) + p(s - ds, v, t + dt))
            - (p(s + ds, v, t - dt) - 2.0 * p(s, v, t - dt) + p(s - ds, v, t - dt)))
            / (2.0 * dt * ds * ds);

        // For a digital we PREFER the published closed-form first/second order spot
        // + vega (exact, no FD round-off); FD supplies only the remainder.
        let (delta_base_unit, gamma_final, vega_final) = match self.kind {
            ExoticKind::Digital(kind) => pricer.digital_greeks(kind, i),
            ExoticKind::SingleBarrier(_) => (delta_unit, gamma_unit, vega_unit),
        };

        CanonicalGreeks {
            // Canonical delta is the spot-unadjusted spot delta × notional. For an
            // exotic there is no separate "convention delta"; the spot delta IS the
            // convention-free hedge ratio.
            delta_base: delta_base_unit * n,
            gamma: gamma_final * n,
            vega: vega_final * n,
            theta: theta_unit * n,
            vanna: vanna_unit * n,
            volga: volga_unit * n,
            charm: charm_unit * n,
            speed: speed_unit * n,
            zomma: zomma_unit * n,
            color: color_unit * n,
        }
    }

    /// The leg's quote (premium) currency — for the vega-ladder premium-ccy guard.
    #[must_use]
    pub fn quote_ccy(&self) -> Ccy {
        self.pair.quote
    }
}

/// The total exotic-leg P&L of a node under one scenario, in the common premium
/// currency assumption (mirrors [`crate::nonadditive::node_pnl`] for vanilla legs).
/// Each leg re-prices the real exotic through the injected [`ExoticLegPricer`] seam.
#[must_use]
pub fn exotic_node_pnl(
    pricer: &dyn ExoticLegPricer,
    legs: &[ExoticLeg],
    scenario: Scenario,
) -> f64 {
    legs.iter().map(|l| l.pnl(pricer, scenario)).sum()
}

/// The summed FRTB-SbM curvature legs `(Σ CVR_up, Σ CVR_down)` of a set of exotic
/// legs — added to the vanilla legs' `(up, down)` before the node `max`.
#[must_use]
pub fn exotic_curvature_legs(
    pricer: &dyn ExoticLegPricer,
    legs: &[ExoticLeg],
    rw: f64,
) -> (f64, f64) {
    let mut up = 0.0;
    let mut down = 0.0;
    for l in legs {
        let (u, d) = l.curvature_legs(pricer, rw);
        up += u;
        down += d;
    }
    (up, down)
}

/// Build the perturbed inputs with a replaced spot (strike/vol/time/rates fixed).
#[must_use]
fn with_spot(i: &VanillaInputs, spot: f64) -> VanillaInputs {
    VanillaInputs::new(spot, i.strike, i.vol, i.t, i.r_dom, i.r_for)
}

/// A central-difference spot bump on the standard relative scale, with an absolute
/// floor so a tiny spot stays well-posed.
#[must_use]
fn spot_bump(spot: f64) -> f64 {
    (spot.abs() * 1e-4).max(1e-7)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The FD spot bump is the documented `max(|S|·1e-4, 1e-7)`: relative on a
    /// normal spot, floored absolutely for a tiny spot (exact dyadic pins), and
    /// `with_spot` replaces ONLY the spot.
    ///
    /// `spot_bump` / `with_spot` are crate-private FD helpers that need no exotic
    /// pricer, so they stay in-crate. The price / Greek / curvature / PnL oracle
    /// tests that DO need a concrete closed-form exotic pricer (barriers have no
    /// closed form without `celnet-exotics`, which the cube deliberately does not
    /// depend on) were moved verbatim to `celnet-parity/tests/exotic_leg_pricing.rs`,
    /// where the real `celnet-exotics` engines back an injected [`ExoticLegPricer`].
    #[test]
    fn spot_bump_is_relative_with_absolute_floor() {
        assert_eq!(spot_bump(2.0).to_bits(), 2e-4_f64.to_bits());
        assert_eq!(spot_bump(-2.0).to_bits(), 2e-4_f64.to_bits(), "uses |S|");
        assert_eq!(spot_bump(1e-9).to_bits(), 1e-7_f64.to_bits(), "floor binds");
        let vi = VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02);
        let w = with_spot(&vi, 9.0);
        assert_eq!(w.spot.to_bits(), 9.0_f64.to_bits());
        assert_eq!(w.strike.to_bits(), vi.strike.to_bits());
        assert_eq!(w.vol.to_bits(), vi.vol.to_bits());
        assert_eq!(w.t.to_bits(), vi.t.to_bits());
        assert_eq!(w.r_dom.to_bits(), vi.r_dom.to_bits());
        assert_eq!(w.r_for.to_bits(), vi.r_for.to_bits());
    }
}
