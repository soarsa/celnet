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
use celnet_core::math::{ln, norm_cdf, norm_pdf, sqrt};
use celnet_types::{Carry, OptionType, RateSensitivities};

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

/// Intermediate quantities shared by price and Greeks.
struct Aux {
    /// Outright forward `F`.
    f: f64,
    /// Discount factor `e^{−r·t}`.
    df: f64,
    d1: f64,
    d2: f64,
    sqt: f64,
    vsqt: f64,
}

/// Recover the FX-equivalent `(r_dom, r_for)` from the carry, so the spot-space
/// arithmetic reproduces the Garman-Kohlhagen leaf BIT-FOR-BIT: `r_dom = r` is the
/// discount rate, `r_for = r − b` is the effective foreign/funding yield. The linear
/// crypto vanilla is the agnostic generalized-BSM, and FX is the SAME engine, so the
/// price is `to_bits`-identical to the FX leaf called with this `(r_dom, r_for)`
/// (gated in [`tests::funding_maps_to_fx_foreign_rate_bit_identical`]).
#[inline]
fn fx_equiv_rates(i: &LinearInputs) -> (f64, f64) {
    let r_dom = i.carry.discount_rate();
    let r_for = r_dom - i.carry.carry_rate(); // = r − b = funding
    (r_dom, r_for)
}

#[inline]
fn aux(i: &LinearInputs) -> Aux {
    let sqt = sqrt(i.t);
    let vsqt = i.vol * sqt;
    let f = i.forward();
    let df = i.discount_df();
    let (r_dom, r_for) = fx_equiv_rates(i);
    // SPOT-space generalized-BSM — the SAME operation order as the FX (Garman-
    // Kohlhagen) leaf, so the linear crypto price is BIT-IDENTICAL (`to_bits`) to it.
    let d1 = (ln(i.spot / i.strike) + (r_dom - r_for + 0.5 * i.vol * i.vol) * i.t) / vsqt;
    let d2 = d1 - vsqt;
    Aux {
        f,
        df,
        d1,
        d2,
        sqt,
        vsqt,
    }
}

/// Present value (USD premium per 1 USD of notional), discounted.
///
/// Computed in spot-space `S·e^{−r_for t}·Φ(d1) − K·e^{−r_dom t}·Φ(d2)` with the
/// FX-equivalent rates — the identical operation order to the FX (Garman-Kohlhagen)
/// leaf, so it is `to_bits`-identical to that leaf called with `r_dom = r`,
/// `r_for = r − b`.
#[must_use]
pub fn price(opt: OptionType, i: &LinearInputs) -> f64 {
    let a = aux(i);
    let (r_dom, r_for) = fx_equiv_rates(i);
    let s_disc = i.spot * celnet_core::math::exp(-r_for * i.t);
    let k_disc = i.strike * celnet_core::math::exp(-r_dom * i.t);
    match opt {
        OptionType::Call => s_disc * norm_cdf(a.d1) - k_disc * norm_cdf(a.d2),
        OptionType::Put => k_disc * norm_cdf(-a.d2) - s_disc * norm_cdf(-a.d1),
    }
}

/// Price and the full generalized Greek strip in a single pass.
///
/// The rate sensitivities are reported as [`RateSensitivities::Carry`]: the
/// `discount_rho = ∂V/∂r` and the `carry_rho = ∂V/∂b` (the crypto funding/carry
/// sensitivity). See [`CarryGreeks`] for the precise definition and units of each
/// carry-neutral sensitivity.
#[must_use]
#[allow(clippy::similar_names)] // d1/d2, nd1/nd2 are the canonical option-pricing names
pub fn greeks(opt: OptionType, i: &LinearInputs) -> CarryGreeks {
    let a = aux(i);
    let (f, df, d1, d2, sqt, vsqt) = (a.f, a.df, a.d1, a.d2, a.sqt, a.vsqt);
    let (s, k, t, vol) = (i.spot, i.strike, i.t, i.vol);
    let r = i.carry.discount_rate();
    let b = i.carry.carry_rate();

    let pd1 = norm_pdf(d1);
    let nd1 = norm_cdf(d1);
    let nd2 = norm_cdf(d2);
    let nmd1 = norm_cdf(-d1);
    let nmd2 = norm_cdf(-d2);

    // Price in the SAME spot-space arithmetic as `price()` so `greeks().price` is
    // `to_bits`-identical to it (the documented reproducibility tie). `k_disc = k·df`
    // with `df = e^{−r_dom t}`, matching `price()`'s `k * exp(-r_dom t)`.
    let (_r_dom, r_for) = fx_equiv_rates(i);
    let s_disc = s * celnet_core::math::exp(-r_for * t);
    let k_disc = k * df;
    let price = match opt {
        OptionType::Call => s_disc * nd1 - k_disc * nd2,
        OptionType::Put => k_disc * nmd2 - s_disc * nmd1,
    };

    // Spot delta ∂V/∂S. With F = S·e^{b t}, ∂F/∂S = e^{b t}, and V = df·BSM(F):
    //   ∂V/∂S = e^{b t}·∂V/∂F = e^{(b−r) t}·Φ(±d1).
    let fwd_factor = i.carry.forward_factor(t); // e^{b t}
    let delta_spot = match opt {
        OptionType::Call => fwd_factor * df * nd1,
        OptionType::Put => fwd_factor * df * (nd1 - 1.0),
    };
    // Forward (driftless) delta ∂V/∂F = df·Φ(±d1).
    let delta_forward = match opt {
        OptionType::Call => df * nd1,
        OptionType::Put => df * (nd1 - 1.0),
    };

    // Symmetric across call/put. gamma = e^{2 b t}·df·φ(d1)/(F·σ√t).
    let gamma = fwd_factor * fwd_factor * df * pd1 / (f * vsqt);
    let vega = df * f * sqt * pd1;
    let vanna = -fwd_factor * df * pd1 * d2 / vol;
    let volga = vega * d1 * d2 / vol;
    let speed = -gamma / s * (d1 / vsqt + 1.0);
    let zomma = gamma * (d1 * d2 - 1.0) / vol;

    // theta = −∂V/∂T. Identical algebra to the commodity (Black-76) leaf: the
    // cross-multiplied pdf identity F·φ(d1) = K·φ(d2) collapses the bracket to a
    // POSITIVE pdf term e^{−rT}·F·φ(d1)·σ/(2√T):
    //   theta_call = −[ e^{−rT}·F·φ(d1)·σ/(2√T) + (b−r)·S e^{(b−r)T}Φ(d1) + r·K e^{−rT}Φ(d2) ].
    let theta_pdf = df * f * pd1 * vol / (2.0 * sqt);
    let theta = match opt {
        OptionType::Call => -(theta_pdf + (b - r) * s * fwd_factor * df * nd1 + r * k * df * nd2),
        OptionType::Put => -(theta_pdf - (b - r) * s * fwd_factor * df * nmd1 - r * k * df * nmd2),
    };

    // Discount-rho ∂V/∂r at FIXED b. V = e^{−r t}·[F·Φ(d1) − K·Φ(d2)] with F = S e^{b t}
    // independent of r ⇒ ∂V/∂r = −t·V.
    let discount_rho = -t * price;
    // Carry-rho ∂V/∂b at FIXED r (the funding/carry sensitivity). Only F = S e^{b t}
    // depends on b: ∂F/∂b = t·F, ∂V/∂F = df·Φ(±d1) ⇒ ∂V/∂b = t·F·df·Φ(±d1).
    let carry_rho = match opt {
        OptionType::Call => t * f * df * nd1,
        OptionType::Put => -t * f * df * nmd1,
    };

    // charm = ∂(delta_spot)/∂T. ln(F/K) = ln(S/K) + b·T ⇒ ∂d1/∂T = b/(σ√T) + ½σ/√T − d1/(2T).
    let dd1_dt = b / vsqt + 0.5 * vol / sqt - d1 / (2.0 * t);
    let charm = match opt {
        OptionType::Call => (b - r) * fwd_factor * df * nd1 + fwd_factor * df * pd1 * dd1_dt,
        OptionType::Put => (b - r) * fwd_factor * df * (nd1 - 1.0) + fwd_factor * df * pd1 * dd1_dt,
    };

    // color = ∂gamma/∂T = gamma·[ (b−r) − 1/(2T) − d1·∂d1/∂T ].
    let color = gamma * ((b - r) - 1.0 / (2.0 * t) - d1 * dd1_dt);

    CarryGreeks {
        price,
        delta_spot,
        delta_forward,
        gamma,
        vega,
        theta,
        rates: RateSensitivities::Carry {
            discount_rho,
            carry_rho,
        },
        vanna,
        volga,
        charm,
        speed,
        zomma,
        color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    fn cost_of_carry(r: f64, b: f64) -> Carry {
        Carry::CostOfCarry { r, b }
    }

    /// (oracle 1) The `funding → r_for` IDENTITY. A linear crypto vanilla with
    /// `(r, b = r − funding)` is **bit-identical** (`to_bits`) to the FX
    /// (Garman-Kohlhagen) leaf called with `r_dom = r, r_for = funding`: the FX leaf
    /// already prices `b = r_dom − r_for` off `F = S·e^{(r_dom−r_for)t}` and discounts
    /// `e^{−r_dom t}`, which is arithmetically the SAME forward-space BSM through a
    /// DIFFERENT (already-QuantLib-golden) code path. This is the disjoint, frozen
    /// reference (W3 §6, MEDIUM circular-risk; routed through `celnet-vanilla`, not a
    /// re-derived GK here).
    #[test]
    fn funding_maps_to_fx_foreign_rate_bit_identical() {
        for &(s, k, vol, t, r, funding) in &[
            (30_000.0, 32_000.0, 0.65, 0.25, 0.05, 0.02),
            (2_000.0, 1_800.0, 0.80, 0.5, 0.04, 0.10),
            (45_000.0, 45_000.0, 0.55, 1.0, 0.03, -0.01),
            (100.0, 110.0, 0.90, 0.08, 0.06, 0.0),
        ] {
            let ci = LinearInputs::funded(s, k, vol, t, r, funding);
            // The FX leaf: r_dom = r (discount), r_for = r − b (the effective funding
            // yield). Recover r_for from the SAME carry the leaf uses, so the bit-
            // identity is exact regardless of any float round-off in b = r − funding.
            let (r_dom, r_for) = (
                ci.carry.discount_rate(),
                ci.carry.discount_rate() - ci.carry.carry_rate(),
            );
            let vi = celnet_types::VanillaInputs::new(s, k, vol, t, r_dom, r_for);
            for opt in [OptionType::Call, OptionType::Put] {
                assert_eq!(
                    price(opt, &ci).to_bits(),
                    celnet_vanilla::price(opt, &vi).to_bits(),
                    "linear crypto must be bit-identical to the FX leaf with r_for=funding"
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
