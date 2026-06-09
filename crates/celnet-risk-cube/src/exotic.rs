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

use celnet_exotics::{
    DigitalKind, SingleBarrier, digital_greeks, digital_price, single_barrier_price,
};
use celnet_risk_normalize::{CanonicalGreeks, CanonicalLeaf};
use celnet_types::{Ccy, CcyPair, Underlying, VanillaInputs};

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

/// Which closed-form exotic an [`ExoticLeg`] prices.
///
/// Each variant carries the **exotic-specific** payoff parameters; the shared
/// market inputs (spot, vol, time, the two rates) live on the [`ExoticLeg`]'s
/// [`VanillaInputs`], so a scenario shocks one consistent input set across the leg.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExoticKind {
    /// A single-barrier option (Reiner-Rubinstein closed form). The strike on the
    /// spec is the barrier-option strike; the barrier level and knock direction /
    /// style live on the spec.
    SingleBarrier(SingleBarrier),
    /// A European digital (cash- or asset-or-nothing). Priced **per one payout
    /// unit** (one unit of domestic cash, or one unit of the foreign asset), so the
    /// leg's `notional` is in payout units; the strike comes from the leg's inputs.
    Digital(DigitalKind),
}

impl ExoticKind {
    /// Price this exotic for one **unit** of payout (per unit base for a barrier,
    /// per payout unit for a digital), under the supplied market inputs.
    #[must_use]
    fn unit_price(self, inputs: &VanillaInputs) -> f64 {
        let i: celnet_exotics::ExoticInputs = inputs.into();
        match self {
            ExoticKind::SingleBarrier(spec) => single_barrier_price(&i, spec),
            ExoticKind::Digital(kind) => digital_price(kind, &i),
        }
    }
}

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
    /// currency, under the supplied market inputs.
    #[must_use]
    fn value(&self, inputs: &VanillaInputs) -> f64 {
        self.kind.unit_price(inputs) * self.notional
    }

    /// The leg's base value under its own (unshocked) inputs.
    #[must_use]
    pub fn base_value(&self) -> f64 {
        self.value(&self.inputs)
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
    pub fn pnl(&self, scenario: Scenario) -> f64 {
        let shocked = apply_fx(scenario, &self.inputs);
        self.value(&shocked) - self.base_value()
    }

    /// The exotic's **spot delta** `∂V/∂S` per unit payout, by central finite
    /// difference of the closed-form price (digitals could use their closed-form
    /// delta, but a single FD path keeps the curvature linear term consistent
    /// across both kinds). Deterministic and `libm`-routed.
    #[must_use]
    fn unit_delta_spot(&self) -> f64 {
        let h = spot_bump(self.inputs.spot);
        let up = self
            .kind
            .unit_price(&with_spot(&self.inputs, self.inputs.spot + h));
        let down = self
            .kind
            .unit_price(&with_spot(&self.inputs, self.inputs.spot - h));
        (up - down) / (2.0 * h)
    }

    /// The FRTB-SbM curvature legs `(CVR_up, CVR_down)` for this exotic along the
    /// spot factor under a relative risk weight `rw`: the up/down full reprice net
    /// of the leg's own linear (delta) term, signed so a loss is positive
    /// (`docs/RISK-HIERARCHY.md` §2.5, MAR21). Returned per leg so a node sums the
    /// vanilla and exotic legs into one `(up, down)` pair before the `max`.
    #[must_use]
    pub fn curvature_legs(&self, rw: f64) -> (f64, f64) {
        let base = self.base_value();
        let up = self.value(&with_spot(&self.inputs, self.inputs.spot * (1.0 + rw)));
        let down = self.value(&with_spot(&self.inputs, self.inputs.spot * (1.0 - rw)));
        // Linear term: ∂V/∂S · notional · rw · S (the exact absolute spot move).
        let linear = self.unit_delta_spot() * self.notional * rw * self.inputs.spot;
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
    pub fn canonical_leaf(&self) -> CanonicalLeaf {
        let g = self.canonical_greeks();
        CanonicalLeaf {
            underlying: Underlying::Fx(self.pair),
            spot: self.inputs.spot,
            greeks: g,
            premium_quote: self.kind.unit_price(&self.inputs) * self.notional,
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
    fn canonical_greeks(&self) -> CanonicalGreeks {
        let n = self.notional;
        let i = &self.inputs;
        // Spot bumps for FD (relative to spot scale); vol bump in absolute vol;
        // time bump in years. All chosen on the standard √ε scale for a central
        // second difference, clamped so a short tenor / tiny spot stays well-posed.
        let ds = spot_bump(i.spot);
        let dv = 1e-4_f64;
        let dt = (i.t * 1e-4).max(1e-6).min(i.t * 0.5);

        // Helper: unit price under (spot, vol, t) perturbations.
        let p = |spot: f64, vol: f64, t: f64| -> f64 {
            self.kind.unit_price(&VanillaInputs::new(
                spot, i.strike, vol, t, i.r_dom, i.r_for,
            ))
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

        // Theta ∂V/∂t (per year). The pricer is parameterized by time-to-expiry t;
        // ∂V/∂(calendar time) = −∂V/∂t, so theta = −central-difference in t.
        let theta_unit = -(p(s, v, t + dt) - p(s, v, t - dt)) / (2.0 * dt);
        // Charm ∂(delta_spot)/∂(calendar time) = −∂(delta)/∂t.
        let charm_unit = -((p(s + ds, v, t + dt) - p(s - ds, v, t + dt))
            - (p(s + ds, v, t - dt) - p(s - ds, v, t - dt)))
            / (4.0 * ds * dt);
        // Zomma ∂gamma/∂σ.
        let zomma_unit = ((p(s + ds, v + dv, t) - 2.0 * p(s, v + dv, t) + p(s - ds, v + dv, t))
            - (p(s + ds, v - dv, t) - 2.0 * p(s, v - dv, t) + p(s - ds, v - dv, t)))
            / (2.0 * dv * ds * ds);
        // Color ∂gamma/∂(calendar time) = −∂gamma/∂t.
        let color_unit = -((p(s + ds, v, t + dt) - 2.0 * p(s, v, t + dt) + p(s - ds, v, t + dt))
            - (p(s + ds, v, t - dt) - 2.0 * p(s, v, t - dt) + p(s - ds, v, t - dt)))
            / (2.0 * dt * ds * ds);

        // For a digital we PREFER the published closed-form first/second order spot
        // + vega (exact, no FD round-off); FD supplies only the remainder.
        let (delta_base_unit, gamma_final, vega_final) = match self.kind {
            ExoticKind::Digital(kind) => {
                let dg = digital_greeks(kind, &i.into());
                (dg.delta, dg.gamma, dg.vega)
            }
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
#[must_use]
pub fn exotic_node_pnl(legs: &[ExoticLeg], scenario: Scenario) -> f64 {
    legs.iter().map(|l| l.pnl(scenario)).sum()
}

/// The summed FRTB-SbM curvature legs `(Σ CVR_up, Σ CVR_down)` of a set of exotic
/// legs — added to the vanilla legs' `(up, down)` before the node `max`.
#[must_use]
pub fn exotic_curvature_legs(legs: &[ExoticLeg], rw: f64) -> (f64, f64) {
    let mut up = 0.0;
    let mut down = 0.0;
    for l in legs {
        let (u, d) = l.curvature_legs(rw);
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
    use celnet_core::is_close;
    use celnet_exotics::{BarrierKind, BarrierStyle};
    use celnet_types::{Ccy, OptionType};

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    fn up_out_call() -> SingleBarrier {
        SingleBarrier {
            kind: BarrierKind {
                up: true,
                style: BarrierStyle::KnockOut,
                option: OptionType::Call,
            },
            strike: 1.10,
            barrier: 1.25,
            rebate: 0.0,
        }
    }

    /// The leaf's premium line equals the closed-form barrier price × notional.
    #[test]
    fn barrier_leaf_premium_matches_closed_form() {
        let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02);
        let leg = ExoticLeg::new(
            eurusd(),
            ExoticKind::SingleBarrier(up_out_call()),
            10_000_000.0,
            inputs,
        );
        let leaf = leg.canonical_leaf();
        let want = single_barrier_price(&(&inputs).into(), up_out_call()) * 10_000_000.0;
        assert!(is_close(leaf.premium_quote, want, 1e-12, 1e-6));
    }

    /// A digital leg's leaf delta/gamma/vega equal the closed-form digital Greeks ×
    /// notional (the published exact values, not FD).
    #[test]
    fn digital_leaf_uses_closed_form_greeks() {
        let kind = DigitalKind::cash(OptionType::Call);
        let inputs = VanillaInputs::new(1.30, 1.32, 0.12, 0.75, 0.03, 0.01);
        let n = 5_000_000.0;
        let leg = ExoticLeg::new(
            CcyPair::new(Ccy::GBP, Ccy::USD),
            ExoticKind::Digital(kind),
            n,
            inputs,
        );
        let leaf = leg.canonical_leaf();
        let dg = digital_greeks(kind, &(&inputs).into());
        assert!(is_close(leaf.greeks.delta_base, dg.delta * n, 1e-12, 1e-3));
        assert!(is_close(leaf.greeks.gamma, dg.gamma * n, 1e-12, 1e-3));
        assert!(is_close(leaf.greeks.vega, dg.vega * n, 1e-12, 1e-3));
    }

    /// The exotic P&L is the real exotic reprice difference (not a vanilla proxy):
    /// shocking spot up toward the knock-out barrier of an up-and-out call DESTROYS
    /// value (the option approaches extinction), so the long leg's P&L is negative —
    /// the opposite sign a long *vanilla* call would show for the same up-shock.
    #[test]
    fn knock_out_pnl_is_exotic_not_vanilla() {
        let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02);
        let leg = ExoticLeg::new(
            eurusd(),
            ExoticKind::SingleBarrier(up_out_call()),
            10_000_000.0,
            inputs,
        );
        // A +5% spot move pushes toward the 1.25 up-and-out barrier.
        let up = Scenario::spot(0.05);
        let exotic_pnl = leg.pnl(up);
        // Independent: real barrier reprice difference (FX projection of the shock).
        // Merge: W5-A's `apply_fx` (yields a VanillaInputs shock, used by the vanilla
        // sanity check below) + ADR-0008's `ExoticInputs` barrier signature (`.into()`).
        let shocked = apply_fx(up, &inputs);
        let want = (single_barrier_price(&(&shocked).into(), up_out_call())
            - single_barrier_price(&(&inputs).into(), up_out_call()))
            * 10_000_000.0;
        assert!(is_close(exotic_pnl, want, 1e-9, 1e-3));
        // A long up-and-out call LOSES value as spot rises toward the barrier.
        assert!(
            exotic_pnl < 0.0,
            "up-and-out call should lose value on an up-move toward the barrier, got {exotic_pnl}"
        );
        // Sanity: a long vanilla call would GAIN on the same up-move — proving the
        // exotic path is genuinely different from a vanilla proxy.
        let vanilla_up = celnet_vanilla::price(OptionType::Call, &shocked)
            - celnet_vanilla::price(OptionType::Call, &inputs);
        assert!(vanilla_up > 0.0);
    }

    /// FD Greeks of a barrier leg match an independent in-test central FD of the
    /// closed-form price (the canonical leaf does not silently zero higher orders).
    #[test]
    fn barrier_fd_greeks_match_independent_fd() {
        let inputs = VanillaInputs::new(1.10, 1.08, 0.11, 0.5, 0.03, 0.01);
        let spec = up_out_call();
        let n = 7_000_000.0;
        let leg = ExoticLeg::new(eurusd(), ExoticKind::SingleBarrier(spec), n, inputs);
        let leaf = leg.canonical_leaf();

        // Independent central FD of the closed-form price for delta/gamma/vega.
        let pr = |s: f64, vol: f64| {
            single_barrier_price(
                &(&VanillaInputs::new(s, inputs.strike, vol, inputs.t, inputs.r_dom, inputs.r_for))
                    .into(),
                spec,
            )
        };
        let ds = (inputs.spot * 1e-4).max(1e-7);
        let dv = 1e-4;
        let s = inputs.spot;
        let v = inputs.vol;
        let base = pr(s, v);
        let ref_delta = (pr(s + ds, v) - pr(s - ds, v)) / (2.0 * ds) * n;
        let ref_gamma = (pr(s + ds, v) - 2.0 * base + pr(s - ds, v)) / (ds * ds) * n;
        let ref_vega = (pr(s, v + dv) - pr(s, v - dv)) / (2.0 * dv) * n;
        assert!(is_close(leaf.greeks.delta_base, ref_delta, 1e-9, 1e-2));
        assert!(is_close(leaf.greeks.gamma, ref_gamma, 1e-9, 1e-2));
        assert!(is_close(leaf.greeks.vega, ref_vega, 1e-9, 1e-2));
    }
}
