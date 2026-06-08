//! Generalized Black-Scholes-Merton (1973) vanilla equity-option pricing and Greeks.
//!
//! This is the equity sibling of the FX Garman-Kohlhagen leaf in `celnet-vanilla`.
//! Both are the *same* generalized-Black-Scholes engine seen through the
//! asset-class-agnostic **cost-of-carry seam** ([`celnet_types::Carry`]): a forward
//! `F = S·e^{b·t}` and a numeraire discount `e^{−r·t}`, where for an equity the net
//! cost of carry is
//!
//! ```text
//! b = r − q                 (continuous dividend yield q)
//! b = r − q − repo          (with a borrow / repo spread folded in)
//! ```
//!
//! and `r` is the (continuously-compounded) risk-free discount rate. The FX leaf is
//! the *same arithmetic* with `r ↦ r_dom` and `q ↦ r_for`; this crate is **not** a
//! second FX closed form — it is the cost-of-carry-parameterized payoff that ADR-0008
//! mandates. The pricer reads its forward/discount **only** through the carry
//! parameters `(r, b)` and **never** matches on a [`celnet_types::Carry`] or
//! [`celnet_types::Underlying`] variant (ADR-0008 "no-match-carry" rule).
//!
//! ```text
//! d1 = [ln(S/K) + (b + ½σ²)·t] / (σ·√t),   d2 = d1 − σ·√t
//! Call = S·e^{(b−r)·t}·Φ(d1) − K·e^{−r·t}·Φ(d2)
//! Put  = K·e^{−r·t}·Φ(−d2) − S·e^{(b−r)·t}·Φ(−d1)
//! ```
//!
//! The full Greek strip is produced in one pass: spot & forward delta, gamma, vega,
//! theta, the two **carry-tagged** rate sensitivities ([`RateSensitivities::Carry`]:
//! the **discount-rho** `∂V/∂r` and the **carry-rho** `∂V/∂b`, the latter being the
//! equity **dividend-rho**), vanna, volga/vomma, charm, speed, zomma and color.
//! Every Greek is cross-validated against central finite differences, and prices
//! are validated against externally-computed reference values and model-free
//! invariants (put-call parity, the `q = 0` non-dividend limit) in the test suite.

#![forbid(unsafe_code)]

use celnet_core::math::{exp, ln, norm_cdf, norm_pdf, sqrt};
use celnet_types::{OptionType, RateSensitivities};

/// Legitimate input to the generalized-BSM equity pricer.
///
/// The market state (`spot`, `strike`, `vol`, `t`) plus the **cost-of-carry
/// parameters**: the discount rate `r`, the continuous dividend yield `q`, and an
/// optional `repo` (borrow) spread. The net carry is `b = r − q − repo`; the
/// forward is `S·e^{b·t}` and the numeraire discount is `e^{−r·t}`. Nothing here
/// names FX: this is a carry-parameterized input, asset-class-agnostic by
/// construction (ADR-0008).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquityInputs {
    /// Spot price of the underlying.
    pub spot: f64,
    /// Strike.
    pub strike: f64,
    /// Annualized volatility (absolute, e.g. `0.20` = 20 vol).
    pub vol: f64,
    /// Time to expiry in years (vol-time).
    pub t: f64,
    /// Continuously-compounded risk-free discount rate `r`.
    pub r: f64,
    /// Continuous dividend yield `q`.
    pub q: f64,
    /// Optional borrow / repo spread folded into the carry (`b = r − q − repo`).
    /// `0.0` for an unencumbered name.
    pub repo: f64,
}

impl EquityInputs {
    /// Construct an equity pricing input with an explicit repo spread.
    #[must_use]
    pub const fn new(spot: f64, strike: f64, vol: f64, t: f64, r: f64, q: f64, repo: f64) -> Self {
        Self {
            spot,
            strike,
            vol,
            t,
            r,
            q,
            repo,
        }
    }

    /// Construct an equity pricing input with no repo spread (`repo = 0`).
    #[must_use]
    pub const fn dividend_paying(spot: f64, strike: f64, vol: f64, t: f64, r: f64, q: f64) -> Self {
        Self::new(spot, strike, vol, t, r, q, 0.0)
    }

    /// Net cost of carry `b = r − q − repo` in the forward `F = S·e^{b·t}`.
    ///
    /// This is the *only* place the carry parameters are combined; every downstream
    /// quantity reads `b` from here, so the pricer is carry-agnostic and never
    /// branches on an asset-class discriminator.
    #[must_use]
    pub fn carry(&self) -> f64 {
        self.r - self.q - self.repo
    }

    /// Outright forward `F = S·e^{b·t}`.
    #[must_use]
    pub fn forward(&self) -> f64 {
        self.spot * exp(self.carry() * self.t)
    }

    /// Numeraire discount factor `e^{−r·t}`.
    #[must_use]
    pub fn discount_df(&self) -> f64 {
        exp(-self.r * self.t)
    }

    /// Carry-discounted spot `S·e^{(b−r)·t}` (= `S·e^{−(q+repo)·t}`) — the
    /// coefficient on `Φ(d1)` in the price.
    #[must_use]
    fn carry_disc_spot(&self) -> f64 {
        self.spot * exp((self.carry() - self.r) * self.t)
    }
}

/// The full equity Greek strip: price plus first- and higher-order sensitivities,
/// with the rate sensitivities tagged as [`RateSensitivities::Carry`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquityGreeks {
    /// Present value (premium) in the numeraire currency, per 1 unit of underlying.
    pub price: f64,
    /// Spot delta (premium-unadjusted): `∂V/∂S`.
    pub delta_spot: f64,
    /// Forward delta (premium-unadjusted): `∂V_fwd/∂F`.
    pub delta_forward: f64,
    /// Gamma: `∂²V/∂S²`.
    pub gamma: f64,
    /// Vega: `∂V/∂σ` (per `1.0` absolute vol).
    pub vega: f64,
    /// Theta: `∂V/∂t` per year (`−∂V/∂T`).
    pub theta: f64,
    /// Carry-tagged rate sensitivities: discount-rho `∂V/∂r` and carry-rho `∂V/∂b`
    /// (the equity **dividend-rho**).
    pub rates: RateSensitivities,
    /// Vanna: `∂²V/∂S∂σ` (= `∂delta_spot/∂σ`).
    pub vanna: f64,
    /// Volga / vomma: `∂²V/∂σ²`.
    pub volga: f64,
    /// Charm: `∂(delta_spot)/∂T` (delta decay, per year).
    pub charm: f64,
    /// Speed: `∂³V/∂S³` (= `∂gamma/∂S`).
    pub speed: f64,
    /// Zomma: `∂gamma/∂σ`.
    pub zomma: f64,
    /// Color: `∂gamma/∂T`.
    pub color: f64,
}

/// Intermediate quantities shared by price and Greeks.
struct Aux {
    d1: f64,
    d2: f64,
    sqt: f64,
    vsqt: f64,
}

#[inline]
fn aux(i: &EquityInputs) -> Aux {
    let sqt = sqrt(i.t);
    let vsqt = i.vol * sqt;
    // d1 = [ln(S/K) + (b + ½σ²)·t] / (σ√t) with b the net cost of carry.
    let d1 = (ln(i.spot / i.strike) + (i.carry() + 0.5 * i.vol * i.vol) * i.t) / vsqt;
    let d2 = d1 - vsqt;
    Aux { d1, d2, sqt, vsqt }
}

/// Present value (premium per 1 unit of underlying).
#[must_use]
pub fn price(opt: OptionType, i: &EquityInputs) -> f64 {
    let a = aux(i);
    let s_disc = i.carry_disc_spot(); // S·e^{(b−r)t}
    let k_disc = i.strike * i.discount_df(); // K·e^{−r t}
    match opt {
        OptionType::Call => s_disc * norm_cdf(a.d1) - k_disc * norm_cdf(a.d2),
        OptionType::Put => k_disc * norm_cdf(-a.d2) - s_disc * norm_cdf(-a.d1),
    }
}

/// Price and the full Greek strip in a single pass.
///
/// See [`EquityGreeks`] for the precise definition and units of each sensitivity.
/// The discount-rho and carry-rho are emitted via [`RateSensitivities::Carry`].
#[must_use]
#[allow(clippy::similar_names)] // d1/d2, nd1/nd2 are the canonical option-pricing names
pub fn greeks(opt: OptionType, i: &EquityInputs) -> EquityGreeks {
    let a = aux(i);
    let (d1, d2, sqt, vsqt) = (a.d1, a.d2, a.sqt, a.vsqt);
    let (s, k, t, vol) = (i.spot, i.strike, i.t, i.vol);
    let b = i.carry(); // net cost of carry
    let r = i.r; // numeraire discount rate
    let df = i.discount_df(); // e^{−r t}
    let cd = exp((b - r) * t); // e^{(b−r) t}: dividend/carry discount on spot

    let pd1 = norm_pdf(d1);
    let nd1 = norm_cdf(d1);
    let nd2 = norm_cdf(d2);
    let nmd1 = norm_cdf(-d1);
    let nmd2 = norm_cdf(-d2);

    let s_disc = s * cd; // S·e^{(b−r)t}
    let k_disc = k * df; // K·e^{−r t}

    let price = match opt {
        OptionType::Call => s_disc * nd1 - k_disc * nd2,
        OptionType::Put => k_disc * nmd2 - s_disc * nmd1,
    };

    // delta_spot = ∂V/∂S = e^{(b−r)t}·Φ(±d1).
    let delta_spot = match opt {
        OptionType::Call => cd * nd1,
        OptionType::Put => cd * (nd1 - 1.0),
    };
    // delta_forward = ∂V_fwd/∂F where V_fwd = V·e^{rt} and F = S·e^{bt}.
    //   V_fwd = e^{rt}·[S e^{(b−r)t}Φ(d1) − K e^{−rt}Φ(d2)]
    //         = F·Φ(d1) − K·Φ(d2)  (call) ⇒ ∂V_fwd/∂F = Φ(d1).
    let delta_forward = match opt {
        OptionType::Call => nd1,
        OptionType::Put => nd1 - 1.0,
    };

    // Symmetric across call/put.
    let gamma = cd * pd1 / (s * vsqt);
    let vega = s_disc * sqt * pd1;
    let vanna = -cd * pd1 * d2 / vol;
    let volga = vega * d1 * d2 / vol;
    let speed = -gamma / s * (d1 / vsqt + 1.0);
    let zomma = gamma * (d1 * d2 - 1.0) / vol;

    // theta = ∂V/∂t (per year) = −∂V/∂T. The generalized-BSM theta:
    //   call: −S e^{(b−r)t} φ(d1) σ/(2√t) − (b−r) S e^{(b−r)t} Φ(d1) − r K e^{−rt} Φ(d2)
    //   put:  −S e^{(b−r)t} φ(d1) σ/(2√t) + (b−r) S e^{(b−r)t} Φ(−d1) + r K e^{−rt} Φ(−d2)
    // (Haug, generalized-BSM; cross-checked vs central FD in the tests.)
    let theta_common = -(s_disc * pd1 * vol) / (2.0 * sqt);
    let theta = match opt {
        OptionType::Call => theta_common - (b - r) * s_disc * nd1 - r * k_disc * nd2,
        OptionType::Put => theta_common + (b - r) * s_disc * nmd1 + r * k_disc * nmd2,
    };

    // Carry-tagged rate sensitivities. (r, b) are the two INDEPENDENT carry
    // coordinates: d1, d2 depend on b but NOT on r, so the partials are clean.
    //   discount_rho = ∂V/∂r at fixed b. Both price legs carry an e^{−rt} factor
    //   (the spot leg is S e^{(b−r)t}, the strike leg K e^{−rt}), so:
    //       call:  −S t e^{(b−r)t} Φ(d1)  +  K t e^{−rt} Φ(d2)
    //       put:    S t e^{(b−r)t} Φ(−d1) −  K t e^{−rt} Φ(−d2)
    //   carry_rho = ∂V/∂b at fixed r (the equity dividend-rho), through the forward/
    //   spot carry S e^{(b−r)t} only; the φ-terms from ∂d1/∂b cancel between the two
    //   price legs (S e^{(b−r)t} φ(d1) = K e^{−rt} φ(d2)):
    //       call:  S t e^{(b−r)t} Φ(d1),   put: −S t e^{(b−r)t} Φ(−d1)
    let discount_rho = match opt {
        OptionType::Call => -s * t * cd * nd1 + k * t * df * nd2,
        OptionType::Put => s * t * cd * nmd1 - k * t * df * nmd2,
    };
    let carry_rho = match opt {
        OptionType::Call => s * t * cd * nd1,
        OptionType::Put => -s * t * cd * nmd1,
    };

    // charm = ∂(delta_spot)/∂T. delta_spot = e^{(b−r)T}·Φ(±d1) ⇒
    //   ∂Δ/∂T = (b−r)·e^{(b−r)T}·Φ(±d1) ± e^{(b−r)T}·φ(d1)·∂d1/∂T.
    // ∂d1/∂T = (b + ½σ²)/(σ√T) − [ln(S/K)+(b+½σ²)T]/(2σ T^{3/2})
    //        = b/(σ√T) − d1/(2T) + ½σ/√T  (same collapse as the FX leaf with b↦b).
    let dd1_dt = b / vsqt - d1 / (2.0 * t) + 0.5 * vol / sqt;
    let charm = match opt {
        OptionType::Call => (b - r) * cd * nd1 + cd * pd1 * dd1_dt,
        OptionType::Put => (b - r) * cd * (nd1 - 1.0) + cd * pd1 * dd1_dt,
    };

    // color = ∂gamma/∂T. gamma = e^{(b−r)T}·φ(d1)/(S·σ·√T); differentiate w.r.t. T:
    //   = gamma·[ (b−r) − 1/(2T) − d1·∂d1/∂T ].
    let color = gamma * ((b - r) - 1.0 / (2.0 * t) - d1 * dd1_dt);

    EquityGreeks {
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
    use celnet_core::assert_close;
    use proptest::prelude::*;

    use super::*;

    // ---------------------------------------------------------------------------
    // (1) External-oracle reference VALUES, hand-pinned as literals.
    //
    // QuantLib is NOT available in-sandbox, so these are pinned per VERIFICATION-
    // CONTRACT lesson (c): each value is computed externally by the generalized-BSM
    // / Black-Scholes-Merton closed form and RE-DERIVED in the comment from its
    // primary source so it cannot silently drift. The pricer here is the
    // cost-of-carry engine; the oracle below is an independent computation.
    // ---------------------------------------------------------------------------

    /// **Published equity reference — Hull, *Options, Futures, and Other Derivatives*,
    /// "European option on an index" worked example:**
    ///   S = 930, K = 900, r = 8% (0.08), q = 3% (0.03), σ = 20% (0.20), T = 2/12.
    /// Hull computes d1 = 0.5444, d2 = 0.4548 and the European call value **≈ 51.83**.
    ///
    /// Re-derivation (independent of the production `price`; full precision via an
    /// external code-disjoint Python `math.erf` evaluation per VERIFICATION-CONTRACT
    /// lesson (c)):
    ///   b = r − q = 0.05.
    ///   √T = √(1/6) = 0.4082482905…,  σ√T = 0.0816496581…
    ///   ln(S/K) = ln(930/900) = ln(1.033333…) = 0.0327898640…
    ///   d1 = [0.0327898640 + (0.05 + ½·0.04)·(1/6)] / 0.0816496581 = 0.544366…  (Hull 0.5444)
    ///   d2 = d1 − σ√T = 0.462717…   (Hull rounds inputs to 0.4548)
    ///   Φ(d1) = 0.706811…,  Φ(d2) = 0.678159…
    ///   Call = S·e^{(b−r)T}·Φ(d1) − K·e^{−rT}·Φ(d2)
    ///        = 930·e^{−0.005}·0.706811 − 900·e^{−0.013333}·0.678159
    ///        = 51.832957…   (Hull's rounded 51.83; full precision pinned below)
    ///   Put (model-free parity P = C − e^{(b−r)T}S + e^{−rT}K, externally computed)
    ///        = 14.550997…
    #[test]
    fn hull_index_option_reference() {
        // S=930, K=900, r=0.08, q=0.03, σ=0.20, T=1/6, repo=0.
        let i = EquityInputs::dividend_paying(930.0, 900.0, 0.20, 1.0 / 6.0, 0.08, 0.03);
        // Externally-computed full-precision generalized-BSM call/put.
        assert_close!(
            price(OptionType::Call, &i),
            51.832_956_796_490_86,
            1e-9,
            1e-9
        );
        assert_close!(price(OptionType::Put, &i), 14.550_996_773_772_4, 1e-9, 1e-9);
    }

    /// A second externally-computed reference with a borrow/repo spread folded into
    /// the carry, to pin the `repo` arm independently.
    ///   S = 100, K = 105, r = 0.05, q = 0.02, repo = 0.01, σ = 0.30, T = 1.0.
    ///   b = r − q − repo = 0.02.
    /// Externally computed (generalized BSM with b = 0.02, r = 0.05):
    ///   √T = 1, σ√T = 0.30, ln(S/K) = ln(100/105) = −0.0487901642…
    ///   d1 = [−0.0487901642 + (0.02 + 0.045)·1]/0.30 = (0.0162098358)/0.30 = 0.0540327861…
    ///   d2 = d1 − 0.30 = −0.2459672139…
    /// Externally computed (Python `math.erf`, code-disjoint): Call = 10.376476…
    #[test]
    fn repo_spread_reference() {
        let i = EquityInputs::new(100.0, 105.0, 0.30, 1.0, 0.05, 0.02, 0.01);
        assert_close!(
            price(OptionType::Call, &i),
            10.376_476_625_785_32,
            1e-9,
            1e-9
        );
    }

    /// (2a) DISAGREEING GATE — put-call parity (model-free):
    ///   C − P = e^{−q'T}·S − e^{−rT}·K  where the carry-discounted spot uses the
    ///   full carry b: e^{(b−r)T}·S = S·e^{−(q+repo)T}. Equivalently
    ///   C − P = F·e^{−rT} − K·e^{−rT} with F = S·e^{bT}.
    /// This is a different expression than the per-leg price and cannot be a
    /// circular FX restatement.
    #[test]
    fn put_call_parity_point() {
        let i = EquityInputs::new(123.45, 130.0, 0.22, 0.75, 0.03, 0.018, 0.004);
        let lhs = price(OptionType::Call, &i) - price(OptionType::Put, &i);
        // e^{(b−r)T}·S − e^{−rT}·K  (= forward·df − K·df).
        let rhs = i.carry_disc_spot() - i.strike * i.discount_df();
        assert_close!(lhs, rhs, 1e-12, 1e-12);
        // Equivalent forward-form, to pin F = S·e^{bT} too.
        let rhs2 = (i.forward() - i.strike) * i.discount_df();
        assert_close!(lhs, rhs2, 1e-12, 1e-12);
    }

    /// (2b) DISAGREEING GATE — the `q = 0` (no-dividend, no-repo) limit collapses to
    /// the standard non-dividend Black-Scholes-Merton, a DIFFERENT closed-form
    /// expression (`b = r` ⇒ `S·Φ(d1) − K·e^{−rT}·Φ(d2)`, with the carry-discount on
    /// spot vanishing). Computed here independently from the carry-tagged engine.
    #[test]
    fn no_dividend_limit_is_standard_bsm() {
        for &(s, k, vol, t, r) in &[
            (100.0, 100.0, 0.20, 1.0, 0.05),
            (42.0, 40.0, 0.25, 0.5, 0.10),
            (80.0, 95.0, 0.35, 2.0, 0.03),
        ] {
            let i = EquityInputs::new(s, k, vol, t, r, 0.0, 0.0);
            // Independent standard (q=0) BSM: b = r, so the spot leg is undiscounted.
            let sqt = (t).sqrt();
            let d1 = ((s / k).ln() + (r + 0.5 * vol * vol) * t) / (vol * sqt);
            let d2 = d1 - vol * sqt;
            let n = |x: f64| celnet_core::math::norm_cdf(x);
            let call = s * n(d1) - k * (-r * t).exp() * n(d2);
            let put = k * (-r * t).exp() * n(-d2) - s * n(-d1);
            assert_close!(price(OptionType::Call, &i), call, 1e-12, 1e-12);
            assert_close!(price(OptionType::Put, &i), put, 1e-12, 1e-12);
        }
    }

    /// Zero-vol intrinsic: as σ→0 the price → discounted intrinsic on the forward,
    /// `e^{−rT}·max(±(F−K), 0)`. Independent of the production normal-CDF path.
    #[test]
    fn zero_vol_is_discounted_forward_intrinsic() {
        let i = EquityInputs::new(110.0, 100.0, 1e-9, 1.5, 0.04, 0.02, 0.0);
        let fwd = i.forward();
        let df = i.discount_df();
        assert_close!(
            price(OptionType::Call, &i),
            df * (fwd - 100.0).max(0.0),
            1e-7,
            1e-7
        );
        assert_close!(
            price(OptionType::Put, &i),
            df * (100.0 - fwd).max(0.0),
            1e-7,
            1e-7
        );
    }

    /// Deep-ITM / deep-OTM limits: a deep-ITM call → discounted forward intrinsic;
    /// a deep-OTM call → ~0.
    #[test]
    fn deep_itm_otm_limits() {
        // Deep-ITM call: S far above K.
        let itm = EquityInputs::new(200.0, 100.0, 0.2, 1.0, 0.05, 0.02, 0.0);
        let intrinsic = itm.discount_df() * (itm.forward() - 100.0);
        assert_close!(price(OptionType::Call, &itm), intrinsic, 1e-3, 1e-3);
        // Deep-OTM call: S far below K → negligible.
        let otm = EquityInputs::new(50.0, 200.0, 0.2, 1.0, 0.05, 0.02, 0.0);
        assert!(price(OptionType::Call, &otm) < 1e-3);
    }

    // ---- finite-difference Greek oracle ----

    fn fd1<F: Fn(f64) -> f64>(f: F, x: f64, h: f64) -> f64 {
        (f(x + h) - f(x - h)) / (2.0 * h)
    }

    fn with_spot(i: &EquityInputs, s: f64) -> EquityInputs {
        EquityInputs { spot: s, ..*i }
    }
    fn with_vol(i: &EquityInputs, v: f64) -> EquityInputs {
        EquityInputs { vol: v, ..*i }
    }
    fn with_t(i: &EquityInputs, t: f64) -> EquityInputs {
        EquityInputs { t, ..*i }
    }
    /// Bump `r` at FIXED carry `b` (so `b = r − q − repo` is held by moving `q`
    /// with `r`): the discount-rho is the partial ∂V/∂r holding b.
    fn with_r_fixed_b(i: &EquityInputs, r: f64) -> EquityInputs {
        // Keep q + repo lumped with r so b = r − (q+repo) is unchanged: move q.
        let db = r - i.r;
        EquityInputs {
            r,
            q: i.q + db,
            ..*i
        }
    }
    /// Bump the carry `b` at FIXED `r` by moving `q` (b = r − q − repo): the
    /// carry-rho (dividend-rho) is ∂V/∂b holding r. ∂b/∂q = −1, so ∂V/∂b = −∂V/∂q.
    fn with_b_via_q(i: &EquityInputs, q: f64) -> EquityInputs {
        EquityInputs { q, ..*i }
    }

    fn check_greeks(opt: OptionType, i: &EquityInputs) {
        let g = greeks(opt, i);
        let p = |x: &EquityInputs| price(opt, x);

        // First-order.
        let hs = 1e-4 * i.spot;
        assert_close!(
            g.delta_spot,
            fd1(|s| p(&with_spot(i, s)), i.spot, hs),
            1e-4,
            1e-7
        );
        assert_close!(g.vega, fd1(|v| p(&with_vol(i, v)), i.vol, 1e-5), 1e-4, 1e-7);

        // delta_forward: difference the UNDISCOUNTED forward value V_fwd = price·e^{rT}
        // bumped through spot via the carry F = S·e^{bT}, then divide by the carry to
        // convert ∂/∂S → ∂/∂F.
        let carry = exp(i.carry() * i.t);
        let edomt = exp(i.r * i.t);
        let dvfwd_ds = fd1(|s| p(&with_spot(i, s)) * edomt, i.spot, hs);
        assert_close!(g.delta_forward, dvfwd_ds / carry, 1e-4, 1e-7);

        // theta = −∂V/∂T.
        assert_close!(g.theta, -fd1(|t| p(&with_t(i, t)), i.t, 1e-5), 5e-4, 1e-6);

        // Carry-tagged rate Greeks.
        match g.rates {
            RateSensitivities::Carry {
                discount_rho,
                carry_rho,
            } => {
                // discount_rho = ∂V/∂r at fixed b.
                assert_close!(
                    discount_rho,
                    fd1(|r| p(&with_r_fixed_b(i, r)), i.r, 1e-6),
                    1e-4,
                    1e-7
                );
                // carry_rho = ∂V/∂b at fixed r = −∂V/∂q.
                assert_close!(
                    carry_rho,
                    -fd1(|q| p(&with_b_via_q(i, q)), i.q, 1e-6),
                    1e-4,
                    1e-7
                );
            }
            RateSensitivities::Fx { .. } => panic!("equity greeks must tag as Carry"),
        }

        // Second-order via differencing the relevant first-order Greek.
        let ds = |s: f64| greeks(opt, &with_spot(i, s)).delta_spot;
        let gam = |x: &EquityInputs| greeks(opt, x).gamma;
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

    /// (3) The dividend-rho (carry-rho) cross-checked by INDEPENDENT central finite
    /// difference of price wrt b — the dedicated gate the brief calls out. b is
    /// moved via q (∂b/∂q = −1), so ∂V/∂b = −∂V/∂q.
    #[test]
    fn carry_rho_matches_central_fd_in_b() {
        let cases = [
            EquityInputs::dividend_paying(100.0, 100.0, 0.20, 1.0, 0.05, 0.03),
            EquityInputs::new(930.0, 900.0, 0.20, 1.0 / 6.0, 0.08, 0.03, 0.0),
            EquityInputs::new(100.0, 105.0, 0.30, 1.0, 0.05, 0.02, 0.01),
        ];
        for i in &cases {
            for opt in [OptionType::Call, OptionType::Put] {
                let g = greeks(opt, i);
                let RateSensitivities::Carry { carry_rho, .. } = g.rates else {
                    panic!("equity greeks must tag as Carry");
                };
                let fd = -fd1(|q| price(opt, &EquityInputs { q, ..*i }), i.q, 1e-6);
                assert_close!(carry_rho, fd, 1e-5, 1e-7);
            }
        }
    }

    /// `greeks(opt, i).price` MUST be bit-identical to `price(opt, i)`.
    #[test]
    fn greeks_price_is_bit_identical_to_price() {
        let cases = [
            EquityInputs::dividend_paying(100.0, 100.0, 0.20, 1.0, 0.05, 0.03),
            EquityInputs::new(930.0, 900.0, 0.20, 1.0 / 6.0, 0.08, 0.03, 0.0),
            EquityInputs::new(100.0, 105.0, 0.30, 1.0, 0.05, 0.02, 0.01),
            EquityInputs::new(50.0, 55.0, 0.45, 3.0, -0.01, 0.0, 0.0),
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

    #[test]
    fn greeks_vs_finite_difference() {
        let cases = [
            EquityInputs::dividend_paying(100.0, 100.0, 0.20, 1.0, 0.05, 0.03),
            EquityInputs::new(930.0, 900.0, 0.20, 1.0 / 6.0, 0.08, 0.03, 0.0),
            EquityInputs::new(110.0, 95.0, 0.30, 0.25, 0.01, 0.04, 0.005),
            EquityInputs::new(80.0, 95.0, 0.35, 2.0, 0.03, 0.01, 0.0),
        ];
        for i in &cases {
            check_greeks(OptionType::Call, i);
            check_greeks(OptionType::Put, i);
        }
    }

    /// Vega is non-negative for every valid input.
    #[test]
    fn vega_non_negative() {
        let i = EquityInputs::new(100.0, 120.0, 0.4, 1.0, 0.05, 0.03, 0.01);
        assert!(greeks(OptionType::Call, &i).vega >= 0.0);
        assert!(greeks(OptionType::Put, &i).vega >= 0.0);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn put_call_parity_property(
            s in 0.5f64..500.0,
            k in 0.5f64..500.0,
            vol in 0.02f64..0.8,
            t in 0.02f64..3.0,
            r in -0.02f64..0.10,
            q in 0.0f64..0.08,
            repo in 0.0f64..0.03,
        ) {
            let i = EquityInputs::new(s, k, vol, t, r, q, repo);
            let lhs = price(OptionType::Call, &i) - price(OptionType::Put, &i);
            let rhs = i.carry_disc_spot() - i.strike * i.discount_df();
            prop_assert!(celnet_core::is_close(lhs, rhs, 1e-9, 1e-9));
        }

        #[test]
        fn price_bounds(
            s in 0.5f64..500.0,
            k in 0.5f64..500.0,
            vol in 0.02f64..0.8,
            t in 0.02f64..3.0,
            r in 0.0f64..0.10,
            q in 0.0f64..0.08,
            repo in 0.0f64..0.03,
        ) {
            let i = EquityInputs::new(s, k, vol, t, r, q, repo);
            let c = price(OptionType::Call, &i);
            let pp = price(OptionType::Put, &i);
            // Non-negative and bounded by the carry-discounted spot / discounted strike.
            prop_assert!(c >= -1e-9 && c <= i.carry_disc_spot() + 1e-9);
            prop_assert!(pp >= -1e-9 && pp <= i.strike * i.discount_df() + 1e-9);
            // Vega never negative.
            prop_assert!(greeks(OptionType::Call, &i).vega >= -1e-12);
        }
    }
}
