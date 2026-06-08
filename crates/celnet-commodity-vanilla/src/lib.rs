//! Black-76 (1976) European commodity-option pricing and Greeks.
//!
//! Black's 1976 model prices a European option on a *forward/futures* price `F`.
//! It is the cost-of-carry-`b = 0` degenerate of the generalized Black-Scholes-
//! Merton model: under the futures (risk-neutral forward) measure the futures
//! price is a martingale, so the outright forward equals `F` itself and the only
//! discounting is the numeraire factor `e^{−r·t}`.
//!
//! This crate prices a listed commodity option two equivalent ways through the
//! **single, asset-class-agnostic carry seam** ([`celnet_types::Carry::CostOfCarry`]):
//!
//! * **Option on a listed future** — the input *spot* IS the futures price `F`, with
//!   carry `b = 0` (`Carry::CostOfCarry { r, b: 0.0 }`). Then `forward_factor(t) = 1`
//!   so the seam's `forward()` returns `F` unchanged and `discount_df(t) = e^{−r·t}`.
//!   This is pure Black-76.
//! * **Spot + convenience yield** — the input *spot* is the physical spot `S`, the net
//!   cost-of-carry is `b = r − convenience` (`Carry::CostOfCarry { r, b }`). The seam's
//!   `forward() = S·e^{b·t}` is the implied futures price, and Black-76 is recovered as
//!   the `b = 0` degenerate priced off that forward. Storage cost adds to `b`,
//!   convenience yield (and any lease/dividend-like income) subtracts.
//!
//! In **both** representations the pricer reads the forward and discount **only**
//! through the carry seam ([`celnet_types::Carry::forward_factor`] /
//! [`celnet_types::Carry::discount_df`]). It **never** matches on the [`celnet_types::Carry`]
//! variant nor branches on an [`celnet_types::Underlying`] — the cost-of-carry
//! parameters fully determine the price (ADR-0008 "no-match-carry" rule). The forward
//! representation makes the model identically:
//!
//! ```text
//! F  = spot · forward_factor(t),         df = discount_df(t) = e^{−r·t}
//! d1 = [ln(F/K) + ½σ²·t] / (σ·√t),       d2 = d1 − σ·√t
//! Call = df · [ F·Φ(d1) − K·Φ(d2) ]
//! Put  = df · [ K·Φ(−d2) − F·Φ(−d1) ]
//! ```
//!
//! The full desk Greek strip is produced in one pass: spot & forward delta, gamma,
//! vega, theta, the **carry-tagged rate sensitivities** (discount-rho `∂V/∂r` and
//! the **carry/convenience-rho** `∂V/∂b` — for a commodity, the sensitivity to the
//! net carry, i.e. minus the convenience-yield sensitivity), vanna, volga/vomma,
//! charm, speed, zomma and color. Every Greek is cross-validated against central
//! finite differences in the test suite, and prices are validated against an
//! independent, model-disjoint oracle (put-call parity on the future, the
//! generalized-BSM-on-spot reparameterization, and a hand-pinned published
//! reference value — see the tests).

#![forbid(unsafe_code)]

use celnet_core::carry::CarryGreeks;
use celnet_core::math::{exp, ln, norm_cdf, norm_pdf, sqrt};
use celnet_types::{Carry, OptionType, RateSensitivities};

/// Black-76 commodity-option input.
///
/// The `spot` field carries either the listed **futures price** `F` (with
/// [`CommodityInputs::on_future`], which sets carry `b = 0`) or the physical
/// **spot** `S` (with [`CommodityInputs::on_spot`], which sets `b = r − convenience`).
/// In both cases the forward and discount are derived from `carry` through the seam,
/// so the pricer is identical and never inspects which representation was used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommodityInputs {
    /// Underlying level: the futures price `F` (`on_future`) or physical spot `S`
    /// (`on_spot`). The seam's `forward()` lifts this to the outright forward.
    pub spot: f64,
    /// Strike `K` (price units of the underlying).
    pub strike: f64,
    /// Annualized volatility `σ` (absolute, e.g. `0.25` = 25 vol).
    pub vol: f64,
    /// Time to expiry `t` in years.
    pub t: f64,
    /// Cost-of-carry model: `Carry::CostOfCarry { r, b }`. `b = 0` for an option on
    /// a listed future (pure Black-76); `b = r − convenience` for the spot
    /// representation.
    pub carry: Carry,
}

impl CommodityInputs {
    /// Construct directly from a [`Carry`] (any [`Carry::CostOfCarry`]). Prefer the
    /// [`CommodityInputs::on_future`] / [`CommodityInputs::on_spot`] constructors,
    /// which name the representation explicitly.
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

    /// Option on a **listed future**: `spot` is the futures price `F`, carry `b = 0`
    /// (the futures price is a martingale under the futures measure, so the only
    /// discounting is `e^{−r·t}`). This is pure Black-76.
    #[must_use]
    pub const fn on_future(future: f64, strike: f64, vol: f64, t: f64, r: f64) -> Self {
        Self {
            spot: future,
            strike,
            vol,
            t,
            carry: Carry::CostOfCarry { r, b: 0.0 },
        }
    }

    /// Spot + convenience representation: `spot` is the physical spot `S`, with net
    /// cost-of-carry `b = r − convenience` (storage cost adds to `b`, convenience
    /// yield subtracts). The implied futures price is `S·e^{b·t}`; Black-76 is
    /// recovered off that forward.
    #[must_use]
    pub fn on_spot(spot: f64, strike: f64, vol: f64, t: f64, r: f64, convenience: f64) -> Self {
        Self {
            spot,
            strike,
            vol,
            t,
            carry: Carry::CostOfCarry {
                r,
                b: r - convenience,
            },
        }
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

#[inline]
fn aux(i: &CommodityInputs) -> Aux {
    let sqt = sqrt(i.t);
    let vsqt = i.vol * sqt;
    let f = i.forward();
    let df = i.discount_df();
    // Black-76 in forward space: d1 = [ln(F/K) + ½σ²t]/(σ√t).
    let d1 = (ln(f / i.strike) + 0.5 * i.vol * i.vol * i.t) / vsqt;
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

/// Present value (premium per 1 unit of the underlying), discounted.
#[must_use]
pub fn price(opt: OptionType, i: &CommodityInputs) -> f64 {
    let a = aux(i);
    match opt {
        OptionType::Call => a.df * (a.f * norm_cdf(a.d1) - i.strike * norm_cdf(a.d2)),
        OptionType::Put => a.df * (i.strike * norm_cdf(-a.d2) - a.f * norm_cdf(-a.d1)),
    }
}

/// Price and the full generalized Greek strip in a single pass.
///
/// The rate sensitivities are reported as [`RateSensitivities::Carry`]: the
/// `discount_rho = ∂V/∂r` and the `carry_rho = ∂V/∂b` (the commodity carry /
/// convenience sensitivity). See [`CarryGreeks`] for the precise definition and
/// units of each carry-neutral sensitivity.
#[must_use]
#[allow(clippy::similar_names)] // d1/d2, nd1/nd2 are the canonical option-pricing names
pub fn greeks(opt: OptionType, i: &CommodityInputs) -> CarryGreeks {
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

    let price = match opt {
        OptionType::Call => df * (f * nd1 - k * nd2),
        OptionType::Put => df * (k * nmd2 - f * nmd1),
    };

    // Spot delta ∂V/∂S. With F = S·e^{b t}, ∂F/∂S = e^{b t}, and V = df·BS76(F):
    //   ∂V/∂S = e^{b t}·∂V/∂F = e^{(b−r) t}·Φ(±d1).
    // (∂d1/∂F·… cancels by the standard option identity, leaving the clean delta.)
    let fwd_factor = i.carry.forward_factor(t); // e^{b t}
    let delta_spot = match opt {
        OptionType::Call => fwd_factor * df * nd1,
        OptionType::Put => fwd_factor * df * (nd1 - 1.0),
    };
    // Forward (driftless) delta ∂V/∂F = df·Φ(±d1) — the undiscounted Black-76 delta.
    let delta_forward = match opt {
        OptionType::Call => df * nd1,
        OptionType::Put => df * (nd1 - 1.0),
    };

    // Symmetric across call/put. Gamma is ∂²V/∂S²; with ∂²V/∂F² = df·φ(d1)/(F·σ√t)
    // and ∂F/∂S = e^{b t}: gamma = e^{2 b t}·df·φ(d1)/(F·σ√t).
    let gamma = fwd_factor * fwd_factor * df * pd1 / (f * vsqt);
    let vega = df * f * sqt * pd1;
    let vanna = -fwd_factor * df * pd1 * d2 / vol;
    let volga = vega * d1 * d2 / vol;
    // speed = ∂gamma/∂S, zomma = ∂gamma/∂σ — Black-76 forms scaled by the carry factor.
    let speed = -gamma / s * (d1 / vsqt + 1.0);
    let zomma = gamma * (d1 * d2 - 1.0) / vol;

    // theta = ∂V/∂(calendar time) = −∂V/∂T (T = time to expiry). Differentiate the
    // forward-space call V_T = S e^{(b−r)T}Φ(d1) − K e^{−rT}Φ(d2) w.r.t. T:
    //   ∂V/∂T = (b−r)·S e^{(b−r)T}Φ(d1) + r·K e^{−rT}Φ(d2)
    //           + [e^{−rT}F·φ(d1)(∂d1/∂T) − e^{−rT}K·φ(d2)(∂d2/∂T)].
    // The cross-multiplied pdf identity F·φ(d1) = K·φ(d2) collapses the bracket to
    //   e^{−rT}·K·φ(d2)·(∂d1/∂T − ∂d2/∂T) = e^{−rT}·F·φ(d1)·σ/(2√T)   (since d1−d2 = σ√T),
    // a POSITIVE pdf term. So with the conventional sign theta = −∂V/∂T:
    //   theta_call = −[ e^{−rT}·F·φ(d1)·σ/(2√T) + (b−r)·S e^{(b−r)T}Φ(d1) + r·K e^{−rT}Φ(d2) ].
    // (Cross-validated against central FD in the test suite.)
    let theta_pdf = df * f * pd1 * vol / (2.0 * sqt);
    let theta = match opt {
        OptionType::Call => -(theta_pdf + (b - r) * s * fwd_factor * df * nd1 + r * k * df * nd2),
        OptionType::Put => -(theta_pdf - (b - r) * s * fwd_factor * df * nmd1 - r * k * df * nmd2),
    };

    // Discount-rho ∂V/∂r at FIXED b (the carry parameterization is (r, b)).
    //   V = e^{−r t}·[F·Φ(d1) − K·Φ(d2)] with F = S e^{b t} independent of r ⇒
    //   ∂V/∂r = −t·V.
    let discount_rho = -t * price;
    // Carry-rho ∂V/∂b at FIXED r (the convenience/carry sensitivity). Only F = S e^{b t}
    // depends on b: ∂F/∂b = t·F, and ∂V/∂F = df·Φ(±d1) (forward delta), so
    //   ∂V/∂b = (∂V/∂F)·(∂F/∂b) = t·F·df·Φ(±d1).
    let carry_rho = match opt {
        OptionType::Call => t * f * df * nd1,
        OptionType::Put => -t * f * df * nmd1,
    };

    // charm = ∂(delta_spot)/∂T (delta decay per year of remaining maturity, matching
    // the FX-leaf convention). delta_spot = e^{(b−r) T}·Φ(±d1) (Φ(d1) for a call,
    // Φ(d1)−1 for a put), so
    //   ∂(delta_spot)/∂T = (b−r)·e^{(b−r) T}·Φ(±d1) + e^{(b−r) T}·φ(d1)·∂d1/∂T.
    // In forward space ln(F/K) = ln(S/K) + b·T, so
    //   ∂d1/∂T = b/(σ√T) + ½σ/√T − d1/(2T).
    let dd1_dt = b / vsqt + 0.5 * vol / sqt - d1 / (2.0 * t);
    let charm = match opt {
        OptionType::Call => (b - r) * fwd_factor * df * nd1 + fwd_factor * df * pd1 * dd1_dt,
        OptionType::Put => (b - r) * fwd_factor * df * (nd1 - 1.0) + fwd_factor * df * pd1 * dd1_dt,
    };

    // color = ∂gamma/∂T. With gamma = e^{(b−r) T}·φ(d1)/(S·σ√T) (the ff²·df/F factor
    // collapses to e^{(b−r) T}/S since F = S·e^{b T}), and φ'(d1) = −d1·φ(d1):
    //   ∂gamma/∂T = gamma·[ (b−r) − 1/(2T) − d1·∂d1/∂T ].
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

/// Black-76 forward-space delta `∂V/∂F = df·Φ(±d1)` (undiscounted in spot terms).
///
/// Exposed because it is the driftless delta a futures-options desk hedges with.
#[must_use]
pub fn forward_delta(opt: OptionType, i: &CommodityInputs) -> f64 {
    let a = aux(i);
    match opt {
        OptionType::Call => a.df * norm_cdf(a.d1),
        OptionType::Put => a.df * (norm_cdf(a.d1) - 1.0),
    }
}

/// Convenience accessor: the (price-independent) discount factor — kept for callers
/// that need to discount intrinsic / payoff legs consistently with the pricer.
#[must_use]
pub fn discount_factor(i: &CommodityInputs) -> f64 {
    exp(-i.carry.discount_rate() * i.t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    fn cost_of_carry(r: f64, b: f64) -> Carry {
        Carry::CostOfCarry { r, b }
    }

    // ---------------------------------------------------------------------------
    // (a) INDEPENDENT, model-disjoint oracle — hand-pinned published reference.
    //
    // Espen Gaarder Haug, "The Complete Guide to Option Pricing Formulas",
    // 2nd ed. (2007), §1.2.2 "The Black-1976 Model", worked example:
    //   F = 19, K = 19, t = 0.75 yr, r = 0.10, σ = 0.28.
    //   Call value = 1.7011.
    //
    // RE-DERIVATION from the PRIMARY formula (Black 1976; Haug §1.2.2), computed
    // entirely OUTSIDE this crate's code path (longhand, with externally-tabulated
    // standard-normal CDF values), to avoid a circular oracle:
    //   v√t   = 0.28·√0.75 = 0.24248711305964282
    //   ln(F/K) = ln(1) = 0
    //   d1 = (0 + 0.5·0.28²·0.75) / v√t = 0.0294 / 0.24248711305964282 = 0.12124355652982143
    //   d2 = d1 − v√t = −0.12124355652982143
    //   Φ(d1) = Φ( 0.12124355652982143) = 0.5482509372784995   (erf-based, external)
    //   Φ(d2) = Φ(−0.12124355652982143) = 0.4517490627215005
    //   e^{−rt} = e^{−0.075} = 0.9277434863285529
    //   Call = e^{−rt}·[F·Φ(d1) − K·Φ(d2)]
    //        = 0.9277434863285529·19·(0.5482509372784995 − 0.4517490627215005)
    //        = 0.9277434863285529·19·0.09650187455699899 = 1.7010507252362674,
    //        which rounds to Haug's published 1.7011.
    // We assert against the FULL-PRECISION value our independent longhand reaches.
    #[test]
    fn black76_haug_published_reference() {
        let i = CommodityInputs::on_future(19.0, 19.0, 0.28, 0.75, 0.10);
        let c = price(OptionType::Call, &i);
        // Haug's published value (1.7011, 4 dp) and our re-derived full-precision value.
        assert_close!(c, 1.7011, 5e-4, 5e-4);
        assert_close!(c, 1.701_050_725_236_267_4, 1e-10, 1e-9);
    }

    // (a)/(b) A SECOND, model-disjoint hand-pinned value computed offline with a
    // higher-precision standard-normal CDF (erf-based), on a DIFFERENT (in-the-money)
    // point so a shared scale/sign error cannot hide. Inputs: F=45, K=40, t=0.5,
    // r=0.04, σ=0.35. RE-DERIVED offline from the Black-1976 primary formula
    // (Φ via erf, independent of this crate's `norm_cdf`):
    //   v√t = 0.35·√0.5 = 0.24748737341529164
    //   ln(F/K) = ln(45/40) = ln(1.125) = 0.11778303565638351
    //   d1 = (0.11778303565638351 + 0.5·0.35²·0.5)/v√t
    //      = (0.11778303565638351 + 0.030625)/0.24748737341529164 = 0.5996590194011638
    //   d2 = d1 − v√t = 0.3521716459858722
    //   Φ(d1) = 0.7256332475037105,  Φ(d2) = 0.6376452301181443
    //   e^{−rt} = e^{−0.02} = 0.9801986733067553
    //   Call = e^{−rt}·(F·Φ(d1) − K·Φ(d2))
    //        = 0.9801986733067553·(45·0.7256332475037105 − 40·0.6376452301181443)
    //        = 0.9801986733067553·(32.65349613766697 − 25.50580920472577)
    //        = 0.9801986733067553·7.1476869329412 = 7.006153248880993.
    #[test]
    fn black76_offline_high_precision_itm() {
        let i = CommodityInputs::on_future(45.0, 40.0, 0.35, 0.5, 0.04);
        let c = price(OptionType::Call, &i);
        assert_close!(c, 7.006_153_248_880_993, 1e-10, 1e-9);
    }

    // (a) CAN-DISAGREE gate #1: put-call parity ON THE FUTURE.
    //   C − P = e^{−rt}·(F − K).  This is model-free given the discounted forward,
    //   reached by a DIFFERENT route than the pricer's Φ-weighted sum, so a sign/
    //   weight bug in either leg is exposed. ~1e-12.
    #[test]
    fn put_call_parity_on_future() {
        for &(s, k, vol, t, r, b) in &[
            (50.0, 55.0, 0.30, 1.0, 0.05, 0.0),
            (100.0, 90.0, 0.22, 0.5, 0.03, 0.01),
            (19.0, 19.0, 0.28, 0.75, 0.10, -0.02),
            (1.5, 1.7, 0.45, 2.0, 0.02, 0.06),
        ] {
            let i = CommodityInputs::new(s, k, vol, t, cost_of_carry(r, b));
            let c = price(OptionType::Call, &i);
            let p = price(OptionType::Put, &i);
            let rhs = i.discount_df() * (i.forward() - k);
            assert_close!(c - p, rhs, 1e-12, 1e-12);
        }
    }

    // (a) CAN-DISAGREE gate #2: the b=0 "option on a future" representation equals the
    // generalized-BSM-on-spot reparameterization that reaches F by a DIFFERENT
    // parameterization — spot S with b chosen so S·e^{b t} == F. Same number, two
    // routes through the carry seam. ~last-bit (the inputs are arithmetically tied).
    #[test]
    fn future_equals_spot_reparameterization() {
        // Pick a spot S and r, b such that S·e^{b t} == F (a chosen future).
        let s = 80.0;
        let r = 0.06;
        let b = 0.015;
        let (k, vol, t) = (75.0, 0.33, 1.25);
        let spot_repr = CommodityInputs::new(s, k, vol, t, cost_of_carry(r, b));
        let f = spot_repr.forward();
        // Now price the SAME option as a pure option-on-future (spot := F, b := 0).
        let future_repr = CommodityInputs::on_future(f, k, vol, t, r);
        for opt in [OptionType::Call, OptionType::Put] {
            let p_spot = price(opt, &spot_repr);
            let p_fut = price(opt, &future_repr);
            assert_close!(p_spot, p_fut, 1e-12, 1e-12);
        }
    }

    // `on_spot` builds b = r − convenience and reaches the same forward as the
    // explicit (r, b) construction (independent route to the same carry).
    #[test]
    fn on_spot_sets_convenience_carry() {
        let (s, k, vol, t, r, conv) = (62.0, 60.0, 0.40, 0.75, 0.05, 0.08);
        let a = CommodityInputs::on_spot(s, k, vol, t, r, conv);
        let b = CommodityInputs::new(s, k, vol, t, cost_of_carry(r, r - conv));
        assert_eq!(a.forward().to_bits(), b.forward().to_bits());
        assert_eq!(a.discount_df().to_bits(), b.discount_df().to_bits());
        for opt in [OptionType::Call, OptionType::Put] {
            assert_eq!(price(opt, &a).to_bits(), price(opt, &b).to_bits());
        }
    }

    // (b) Zero-vol intrinsic: as σ→0 the option → discounted (F−K)⁺ (call) /
    //   (K−F)⁺ (put). A small-σ limit reached without the Φ machinery.
    #[test]
    fn zero_vol_discounted_intrinsic() {
        for &(s, k, t, r, b) in &[
            (100.0, 90.0, 1.0, 0.05, 0.0), // call ITM, put OTM
            (80.0, 95.0, 0.5, 0.03, 0.02), // call OTM, put ITM
        ] {
            let i = CommodityInputs::new(s, k, 1e-7, t, cost_of_carry(r, b));
            let f = i.forward();
            let df = i.discount_df();
            let call_intrinsic = df * (f - k).max(0.0);
            let put_intrinsic = df * (k - f).max(0.0);
            assert_close!(price(OptionType::Call, &i), call_intrinsic, 1e-9, 1e-8);
            assert_close!(price(OptionType::Put, &i), put_intrinsic, 1e-9, 1e-8);
        }
    }

    // Structural sandwich: vega ≥ 0 and the call is monotone increasing in F.
    #[test]
    fn vega_nonneg_and_monotone_in_forward() {
        let base = CommodityInputs::new(50.0, 50.0, 0.30, 1.0, cost_of_carry(0.04, 0.0));
        assert!(greeks(OptionType::Call, &base).vega >= 0.0);
        assert!(greeks(OptionType::Put, &base).vega >= 0.0);
        // Bump the future (spot, b=0) up — the call must rise, the put must fall.
        let up = CommodityInputs::new(55.0, 50.0, 0.30, 1.0, cost_of_carry(0.04, 0.0));
        assert!(price(OptionType::Call, &up) > price(OptionType::Call, &base));
        assert!(price(OptionType::Put, &up) < price(OptionType::Put, &base));
    }

    // ---- finite-difference Greek oracle (independent, central FD) ----

    fn fd1<F: Fn(f64) -> f64>(f: F, x: f64, h: f64) -> f64 {
        (f(x + h) - f(x - h)) / (2.0 * h)
    }

    fn with_spot(i: &CommodityInputs, s: f64) -> CommodityInputs {
        CommodityInputs { spot: s, ..*i }
    }
    fn with_vol(i: &CommodityInputs, v: f64) -> CommodityInputs {
        CommodityInputs { vol: v, ..*i }
    }
    fn with_t(i: &CommodityInputs, t: f64) -> CommodityInputs {
        CommodityInputs { t, ..*i }
    }
    fn with_r(i: &CommodityInputs, r: f64) -> CommodityInputs {
        let b = i.carry.carry_rate();
        CommodityInputs {
            carry: Carry::CostOfCarry { r, b },
            ..*i
        }
    }
    fn with_b(i: &CommodityInputs, b: f64) -> CommodityInputs {
        let r = i.carry.discount_rate();
        CommodityInputs {
            carry: Carry::CostOfCarry { r, b },
            ..*i
        }
    }

    fn check_greeks(opt: OptionType, i: &CommodityInputs) {
        let g = greeks(opt, i);
        let p = |x: &CommodityInputs| price(opt, x);

        let hs = 1e-4 * i.spot;
        assert_close!(
            g.delta_spot,
            fd1(|s| p(&with_spot(i, s)), i.spot, hs),
            1e-4,
            1e-7
        );
        assert_close!(g.vega, fd1(|v| p(&with_vol(i, v)), i.vol, 1e-5), 1e-4, 1e-7);

        // forward delta = ∂V/∂F. F = spot·e^{b t}, so ∂F/∂spot = e^{b t}; convert
        // the spot-FD into a forward-FD by dividing by that carry factor.
        let carry = i.carry.forward_factor(i.t);
        let dvfwd = fd1(|s| p(&with_spot(i, s)), i.spot, hs) / carry;
        assert_close!(g.delta_forward, dvfwd, 1e-4, 1e-7);

        // theta = −∂V/∂T
        assert_close!(g.theta, -fd1(|t| p(&with_t(i, t)), i.t, 1e-5), 5e-4, 1e-6);

        // discount_rho = ∂V/∂r at fixed b; carry_rho = ∂V/∂b at fixed r.
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
            RateSensitivities::Fx { .. } => panic!("commodity greeks must tag as Carry"),
        }

        // Second-order via differencing the first-order Greek.
        let ds = |s: f64| greeks(opt, &with_spot(i, s)).delta_spot;
        let gam = |x: &CommodityInputs| greeks(opt, x).gamma;
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
        // A spread of regimes: ITM/OTM, low/high vol, short/long, futures (b=0)
        // and spot+convenience (b≠0), positive and negative carry.
        let cases = [
            CommodityInputs::on_future(100.0, 100.0, 0.20, 1.0, 0.05),
            CommodityInputs::on_future(19.0, 19.0, 0.28, 0.75, 0.10),
            CommodityInputs::on_spot(80.0, 75.0, 0.33, 1.25, 0.06, 0.045),
            CommodityInputs::on_spot(62.0, 70.0, 0.40, 0.5, 0.03, 0.09),
            CommodityInputs::new(110.0, 95.0, 0.30, 0.25, cost_of_carry(0.01, -0.02)),
        ];
        for i in &cases {
            check_greeks(OptionType::Call, i);
            check_greeks(OptionType::Put, i);
        }
    }

    /// `greeks(opt, i).price` MUST be the bit-identical value returned by
    /// `price(opt, i)` — the Greek pass recomputes the present value from the same
    /// shared `aux`/discount factors. A `to_bits` reproducibility tie (the documented
    /// carve-out from "no float `==`"); kills mutation survivors where the greeks-pass
    /// price recomputation is perturbed.
    #[test]
    fn greeks_price_is_bit_identical_to_price() {
        let cases = [
            CommodityInputs::on_future(100.0, 100.0, 0.20, 1.0, 0.05),
            CommodityInputs::on_spot(80.0, 75.0, 0.33, 1.25, 0.06, 0.045),
            CommodityInputs::new(110.0, 95.0, 0.30, 0.25, cost_of_carry(0.01, -0.02)),
        ];
        for i in &cases {
            for opt in [OptionType::Call, OptionType::Put] {
                let standalone = price(opt, i);
                let from_greeks = greeks(opt, i).price;
                assert_eq!(
                    standalone.to_bits(),
                    from_greeks.to_bits(),
                    "greeks().price must be bit-identical to price(): {opt:?} {i:?}"
                );
            }
        }
    }

    /// The exposed `forward_delta` helper agrees with the strip's `delta_forward`.
    #[test]
    fn forward_delta_helper_matches_strip() {
        let i = CommodityInputs::on_spot(80.0, 75.0, 0.33, 1.25, 0.06, 0.045);
        for opt in [OptionType::Call, OptionType::Put] {
            assert_eq!(
                forward_delta(opt, &i).to_bits(),
                greeks(opt, &i).delta_forward.to_bits()
            );
        }
    }
}
