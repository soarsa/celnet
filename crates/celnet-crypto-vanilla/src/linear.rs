//! Linear / USD(T)-margined crypto vanilla — the asset-class-agnostic
//! generalized-BSM path.
//!
//! A linear-settled crypto option (USDT-/USDC-margined, the OKX/Binance "linear"
//! contract and the USD-cash-settled chains) has its premium **and** its payoff in
//! USD(T): `payoff = max(φ(S_T − K), 0)` USD. That is *exactly* generalized
//! Black-Scholes-Merton off the carry seam — there is **zero new payoff math**. The
//! leaf assembles the crypto cost-of-carry `b = r − funding`
//! ([`crate::funding`]) and prices off
//!
//! ```text
//! F  = spot · forward_factor(t),         df = discount_df(t) = e^{−r·t}
//! d1 = [ln(F/K) + ½σ²·t] / (σ·√t),       d2 = d1 − σ·√t
//! Call = df · [ F·Φ(d1) − K·Φ(d2) ]
//! Put  = df · [ K·Φ(−d2) − F·Φ(−d1) ]
//! ```
//!
//! reading the forward and discount **only** through the carry seam
//! ([`celnet_types::Carry::forward_factor`] / [`celnet_types::Carry::discount_df`]).
//! It **never** matches on the [`celnet_types::Carry`] variant nor branches on an
//! [`celnet_types::Underlying`] — the cost-of-carry parameters fully determine the
//! price (ADR-0008 "no-match-carry" rule). This is the SAME forward-space engine
//! the commodity (Black-76) leaf uses; the only thing that differs from FX/equity/
//! commodity is how `b` is assembled upstream (`r − funding`).
//!
//! The full desk Greek strip is produced in one pass with the rate sensitivities
//! reported as [`celnet_types::RateSensitivities::Carry`]: the discount-rho
//! `∂V/∂r` and the **funding-rho** `∂V/∂b` (the crypto carry sensitivity; a
//! coin-funding move is `−∂V/∂funding = ∂V/∂b`).

use celnet_core::carry::CarryGreeks;
use celnet_core::{gbsm_carry_greeks, gbsm_carry_price};
use celnet_types::{Carry, OptionType};

/// Linear (USD-margined) crypto vanilla input.
///
/// `spot` is the USD price of one coin (`S`), `strike` the USD strike `K`. The
/// forward and discount are derived from `carry` through the seam, so this is the
/// asset-class-agnostic generalized-BSM input — identical in shape to the commodity
/// leaf, differing only in that `carry` is assembled by [`crate::funding`] as
/// `b = r − funding`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearInputs {
    /// Spot: USD price of one coin (`S`).
    pub spot: f64,
    /// Strike `K` in USD.
    pub strike: f64,
    /// Annualized volatility `σ` (absolute, e.g. `0.65` = 65 vol — crypto vols run high).
    pub vol: f64,
    /// Time to expiry `t` in years.
    pub t: f64,
    /// Cost-of-carry model `Carry::CostOfCarry { r, b = r − funding }`.
    pub carry: Carry,
}

impl LinearInputs {
    /// Construct directly from a [`Carry`]. Prefer [`LinearInputs::funded`], which
    /// names the funding assembly explicitly.
    #[must_use]
    pub const fn new(spot: f64, strike: f64, vol: f64, t: f64, carry: Carry) -> Self {
        Self {
            spot,
            strike,
            vol,
            t,
            carry,
        }
    }

    /// Construct a linear crypto vanilla with the funding carry `b = r − funding`.
    #[must_use]
    pub fn funded(spot: f64, strike: f64, vol: f64, t: f64, r: f64, funding: f64) -> Self {
        Self::new(
            spot,
            strike,
            vol,
            t,
            crate::funding::funding_carry(r, funding),
        )
    }

    /// Outright forward `F = spot · e^{b·t}`, via the carry seam.
    #[must_use]
    pub fn forward(&self) -> f64 {
        self.spot * self.carry.forward_factor(self.t)
    }

    /// Numeraire discount factor `e^{−r·t}`, via the carry seam.
    #[must_use]
    pub fn discount_df(&self) -> f64 {
        self.carry.discount_df(self.t)
    }
}

/// Present value (USD premium per 1 USD of notional), discounted.
///
/// Delegates to the unified generalized-BSM forward-space kernel
/// ([`gbsm_carry_price`]) with the crypto carry `b = r − funding`, `r` the USD
/// discount rate. This replaces the former FX-equivalent spot-space detour: the
/// linear crypto vanilla is the SAME gBSM as every other cost-of-carry leaf, so it
/// now shares the one canonical kernel (ADR-0012 — the sub-1e-12 forward-vs-spot
/// rounding change is accepted; independent oracles hold at ≤1e-12).
#[must_use]
pub fn price(opt: OptionType, i: &LinearInputs) -> f64 {
    gbsm_carry_price(
        opt,
        i.carry.carry_rate(),
        i.carry.discount_rate(),
        i.spot,
        i.strike,
        i.vol,
        i.t,
    )
}

/// Price and the full generalized Greek strip in a single pass.
///
/// Delegates to the unified generalized-BSM forward-space kernel
/// ([`gbsm_carry_greeks`]); the rate sensitivities are reported as
/// [`celnet_types::RateSensitivities::Carry`]: `discount_rho = ∂V/∂r` and
/// `carry_rho = ∂V/∂b` (the crypto funding/carry sensitivity). `delta_forward` is
/// rescaled from the kernel's driftless `Φ(±d1)` to the discounted `∂V/∂F =
/// df·Φ(±d1)` this leaf's FD gate uses. See [`CarryGreeks`] for the precise
/// definition and units of each carry-neutral sensitivity.
#[must_use]
pub fn greeks(opt: OptionType, i: &LinearInputs) -> CarryGreeks {
    let cg = gbsm_carry_greeks(
        opt,
        i.carry.carry_rate(),
        i.carry.discount_rate(),
        i.spot,
        i.strike,
        i.vol,
        i.t,
    );
    CarryGreeks {
        delta_forward: cg.delta_forward * i.discount_df(),
        ..cg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;
    use celnet_types::RateSensitivities;

    fn cost_of_carry(r: f64, b: f64) -> Carry {
        Carry::CostOfCarry { r, b }
    }

    /// (oracle 1) The `funding → r_for` IDENTITY. A linear crypto vanilla with
    /// `(r, b = r − funding)` agrees with the FX (Garman-Kohlhagen) leaf called with
    /// `r_dom = r, r_for = funding`: both price the SAME forward-space generalized-BSM
    /// through a DIFFERENT (already-QuantLib-golden) code path — the crypto leaf via
    /// `b = carry_rate()`, the FX leaf via `b = r_dom − r_for`. This is the disjoint
    /// reference (W3 §6, MEDIUM circular-risk; routed through `celnet-vanilla`, not a
    /// re-derived GK here).
    ///
    /// ADR-0012 (unified gBSM kernel): both leaves now route through the ONE
    /// `gbsm_carry_greeks` kernel, but the crypto leaf passes `b` directly while the
    /// FX leaf reconstructs `b = r_dom − r_for` (which is `r − (r − b)`, not
    /// bit-equal to `b`). The two therefore agree to a **1e-12** tolerance rather
    /// than bit-for-bit; the residual is pure last-bit carry-reconstruction
    /// rounding (~1e-15), far under the independent-oracle correctness bar. The bit
    /// pin was a determinism/parity artefact, not a correctness statement.
    #[test]
    fn funding_maps_to_fx_foreign_rate_within_1e12() {
        for &(s, k, vol, t, r, funding) in &[
            (30_000.0, 32_000.0, 0.65, 0.25, 0.05, 0.02),
            (2_000.0, 1_800.0, 0.80, 0.5, 0.04, 0.10),
            (45_000.0, 45_000.0, 0.55, 1.0, 0.03, -0.01),
            (100.0, 110.0, 0.90, 0.08, 0.06, 0.0),
        ] {
            let ci = LinearInputs::funded(s, k, vol, t, r, funding);
            // The FX leaf: r_dom = r (discount), r_for = r − b (the effective funding
            // yield), recovered from the SAME carry the crypto leaf uses.
            let (r_dom, r_for) = (
                ci.carry.discount_rate(),
                ci.carry.discount_rate() - ci.carry.carry_rate(),
            );
            let vi = celnet_types::VanillaInputs::new(s, k, vol, t, r_dom, r_for);
            for opt in [OptionType::Call, OptionType::Put] {
                assert_close!(
                    price(opt, &ci),
                    celnet_vanilla::price(opt, &vi),
                    1e-12,
                    1e-12
                );
            }
        }
    }

    /// (oracle 2) Put-call parity `C − P = df·(F − K)`, reached by a DIFFERENT route
    /// than the per-leg Φ-weighted price, so a sign/weight bug surfaces. ~1e-12.
    #[test]
    fn put_call_parity() {
        for &(s, k, vol, t, r, funding) in &[
            (30_000.0, 32_000.0, 0.65, 0.25, 0.05, 0.02),
            (2_000.0, 1_800.0, 0.80, 0.5, 0.04, 0.10),
            (45_000.0, 45_000.0, 0.55, 1.0, 0.03, -0.01),
        ] {
            let i = LinearInputs::funded(s, k, vol, t, r, funding);
            let c = price(OptionType::Call, &i);
            let p = price(OptionType::Put, &i);
            assert_close!(c - p, i.discount_df() * (i.forward() - k), 1e-9, 1e-9);
        }
    }

    /// (oracle 3) `funding = r ⇒ b = 0` → the Black-76 forward limit: the forward is
    /// flat (`F = S`) and the price is `df·[S·Φ(d1) − K·Φ(d2)]` with the b=0 d-terms,
    /// computed here independently of the production carry assembly.
    #[test]
    fn zero_carry_is_black76_forward_limit() {
        let (s, k, vol, t, r) = (30_000.0, 31_000.0, 0.65, 0.5, 0.05);
        let i = LinearInputs::funded(s, k, vol, t, r, r); // funding = r ⇒ b = 0
        assert_eq!(i.forward().to_bits(), s.to_bits());
        // Independent Black-76 with F = S, b = 0.
        let df = (-r * t).exp();
        let vsqt = vol * t.sqrt();
        let d1 = ((s / k).ln() + 0.5 * vol * vol * t) / vsqt;
        let d2 = d1 - vsqt;
        let n = |x: f64| celnet_core::math::norm_cdf(x);
        let call = df * (s * n(d1) - k * n(d2));
        assert_close!(price(OptionType::Call, &i), call, 1e-12, 1e-12);
    }

    // ---- finite-difference Greek oracle (independent, central FD) ----

    fn fd1<F: Fn(f64) -> f64>(f: F, x: f64, h: f64) -> f64 {
        (f(x + h) - f(x - h)) / (2.0 * h)
    }
    fn with_spot(i: &LinearInputs, s: f64) -> LinearInputs {
        LinearInputs { spot: s, ..*i }
    }
    fn with_vol(i: &LinearInputs, v: f64) -> LinearInputs {
        LinearInputs { vol: v, ..*i }
    }
    fn with_t(i: &LinearInputs, t: f64) -> LinearInputs {
        LinearInputs { t, ..*i }
    }
    fn with_r(i: &LinearInputs, r: f64) -> LinearInputs {
        let b = i.carry.carry_rate();
        LinearInputs {
            carry: Carry::CostOfCarry { r, b },
            ..*i
        }
    }
    fn with_b(i: &LinearInputs, b: f64) -> LinearInputs {
        let r = i.carry.discount_rate();
        LinearInputs {
            carry: Carry::CostOfCarry { r, b },
            ..*i
        }
    }

    fn check_greeks(opt: OptionType, i: &LinearInputs) {
        let g = greeks(opt, i);
        let p = |x: &LinearInputs| price(opt, x);

        let hs = 1e-4 * i.spot;
        assert_close!(
            g.delta_spot,
            fd1(|s| p(&with_spot(i, s)), i.spot, hs),
            1e-4,
            1e-7
        );
        assert_close!(g.vega, fd1(|v| p(&with_vol(i, v)), i.vol, 1e-5), 1e-4, 1e-7);

        // forward delta = ∂V/∂F; convert spot-FD by the carry factor e^{b t}.
        let carry = i.carry.forward_factor(i.t);
        let dvfwd = fd1(|s| p(&with_spot(i, s)), i.spot, hs) / carry;
        assert_close!(g.delta_forward, dvfwd, 1e-4, 1e-7);

        // theta = −∂V/∂T.
        assert_close!(g.theta, -fd1(|t| p(&with_t(i, t)), i.t, 1e-5), 5e-4, 1e-6);

        match g.rates {
            RateSensitivities::Carry {
                discount_rho,
                carry_rho,
            } => {
                assert_close!(
                    discount_rho,
                    fd1(|r| p(&with_r(i, r)), i.carry.discount_rate(), 1e-6),
                    1e-4,
                    1e-7
                );
                assert_close!(
                    carry_rho,
                    fd1(|b| p(&with_b(i, b)), i.carry.carry_rate(), 1e-6),
                    1e-4,
                    1e-7
                );
            }
            RateSensitivities::Fx { .. } => panic!("crypto linear greeks must tag as Carry"),
        }

        // Second-order via differencing the first-order Greek.
        let ds = |s: f64| greeks(opt, &with_spot(i, s)).delta_spot;
        let gam = |x: &LinearInputs| greeks(opt, x).gamma;
        assert_close!(g.gamma, fd1(ds, i.spot, hs), 1e-3, 1e-6);
        assert_close!(
            g.vanna,
            fd1(|v| greeks(opt, &with_vol(i, v)).delta_spot, i.vol, 1e-5),
            1e-3,
            1e-6
        );
        assert_close!(
            g.volga,
            fd1(|v| greeks(opt, &with_vol(i, v)).vega, i.vol, 1e-5),
            1e-3,
            1e-6
        );
        assert_close!(
            g.charm,
            fd1(|t| greeks(opt, &with_t(i, t)).delta_spot, i.t, 1e-5),
            1e-3,
            1e-6
        );
        assert_close!(
            g.speed,
            fd1(|s| gam(&with_spot(i, s)), i.spot, hs),
            1e-2,
            1e-5
        );
        assert_close!(
            g.zomma,
            fd1(|v| gam(&with_vol(i, v)), i.vol, 1e-5),
            1e-2,
            1e-5
        );
        assert_close!(g.color, fd1(|t| gam(&with_t(i, t)), i.t, 1e-5), 1e-2, 1e-5);
    }

    #[test]
    fn greeks_vs_finite_difference() {
        let cases = [
            LinearInputs::funded(30_000.0, 30_000.0, 0.65, 0.5, 0.05, 0.02),
            LinearInputs::funded(2_000.0, 2_200.0, 0.80, 1.0, 0.04, 0.10),
            LinearInputs::new(45_000.0, 40_000.0, 0.55, 0.25, cost_of_carry(0.03, -0.04)),
            LinearInputs::funded(100.0, 110.0, 0.90, 0.08, 0.06, 0.0),
        ];
        for i in &cases {
            check_greeks(OptionType::Call, i);
            check_greeks(OptionType::Put, i);
        }
    }

    /// `greeks(opt, i).price` MUST be bit-identical to `price(opt, i)`.
    #[test]
    fn greeks_price_is_bit_identical_to_price() {
        let cases = [
            LinearInputs::funded(30_000.0, 30_000.0, 0.65, 0.5, 0.05, 0.02),
            LinearInputs::new(45_000.0, 40_000.0, 0.55, 0.25, cost_of_carry(0.03, -0.04)),
        ];
        for i in &cases {
            for opt in [OptionType::Call, OptionType::Put] {
                assert_eq!(price(opt, i).to_bits(), greeks(opt, i).price.to_bits());
            }
        }
    }
}
