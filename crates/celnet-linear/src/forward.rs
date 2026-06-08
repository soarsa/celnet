//! FX outright forward — a first-class discounted-cashflow priced product.
//!
//! An outright forward is an agreement to exchange `notional` units of base for
//! quote at the agreed `contract_rate` `K` on the settlement date. Its present
//! value is the discounted expected payoff under the `t`-forward measure:
//!
//! ```text
//! PV = side · notional · df · (F − K),   F = spot · forward_factor(t),  df = discount_df(t)
//! ```
//!
//! This is **linear** in spot (no optionality), so the PV, the fair forward and
//! every Greek are exact closed forms — no Monte-Carlo, no `price_std_error`.
//!
//! Asset-class-agnostic (ADR-0008): `F` and `df` are taken through the carry
//! producer only; this engine prices a forward on *any* [`celnet_types::Underlying`]
//! whose [`celnet_types::Carry`] yields a forward factor and a discount factor.

use crate::inputs::LinearInputs;

/// Present value of an FX outright forward, in the numeraire (quote) currency.
///
/// `PV = side · notional · discount_df(t) · (forward(t) − contract_rate)`,
/// evaluated at the input's [`LinearInputs::near_settle_t`].
#[must_use]
pub fn pv(inputs: &LinearInputs) -> f64 {
    pv_at(inputs, inputs.near_settle_t)
}

/// Present value of an outright forward settling at an explicit time `t`.
///
/// Exposed so the swap's near and far legs (which settle at distinct times)
/// reuse the identical forward-PV expression rather than re-deriving it.
#[must_use]
pub fn pv_at(inputs: &LinearInputs, t: f64) -> f64 {
    let f = inputs.forward(t);
    let df = inputs.discount_df(t);
    inputs.side.sign() * inputs.notional * df * (f - inputs.contract_rate)
}

/// The fair forward rate `F = spot · forward_factor(t)` at the near settlement
/// time — the contract rate at which the forward has zero PV.
#[must_use]
pub fn fair_forward(inputs: &LinearInputs) -> f64 {
    inputs.forward(inputs.near_settle_t)
}

/// The exact linear Greeks of an outright forward.
///
/// Every sensitivity is an exact analytic derivative of
/// `PV = side · notional · (spot · e^{−r_for·t} − K · e^{−r_dom·t})` (the FX
/// rate-decomposed form of the discounted payoff), so they share **no**
/// intermediate with the `forward()`/`discount_df()` route the PV is computed
/// through — a built-in cross-check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForwardGreeks {
    /// Present value (premium) in the numeraire currency.
    pub pv: f64,
    /// Spot delta `∂PV/∂spot = side · notional · forward_factor(t) · df`.
    pub delta: f64,
    /// Rho to the discount (domestic/quote) rate `∂PV/∂r_dom`.
    pub rho_dom: f64,
    /// Rho to the base (foreign) rate `∂PV/∂r_for`.
    pub rho_for: f64,
    /// Theta `∂PV/∂t` per year (time decay; the sign is `+∂PV/∂t`).
    pub theta: f64,
}

/// Compute the exact linear Greeks of the outright forward.
///
/// The rate Greeks are reported in the FX `(r_dom, r_for)` decomposition (the
/// discount and base rates), the natural risk view for the linear FX book.
#[must_use]
pub fn greeks(inputs: &LinearInputs) -> ForwardGreeks {
    let t = inputs.near_settle_t;
    let s = inputs.spot;
    let k = inputs.contract_rate;
    let r_dom = inputs.carry.discount_rate();
    // b = r_dom − r_for ⇒ r_for = r_dom − b.
    let r_for = r_dom - inputs.carry.carry_rate();
    let sign = inputs.side.sign();
    let n = inputs.notional;

    // Route exp through `celnet_core::math::exp` (libm-backed, bit-identical to
    // the `Carry` accessors) so the rate-decomposed Greek route matches the
    // production forward/df route bit-for-bit at the transcendental level.
    let df_for = celnet_core::math::exp(-r_for * t); // e^{−r_for·t}
    let df_dom = celnet_core::math::exp(-r_dom * t); // e^{−r_dom·t} = df

    let pv = sign * n * (s * df_for - k * df_dom);
    // ∂PV/∂S = side·N·e^{−r_for·t}.
    let delta = sign * n * df_for;
    // ∂PV/∂r_dom = side·N·K·t·e^{−r_dom·t}.
    let rho_dom = sign * n * k * t * df_dom;
    // ∂PV/∂r_for = −side·N·S·t·e^{−r_for·t}.
    let rho_for = -sign * n * s * t * df_for;
    // ∂PV/∂t = side·N·(−r_for·S·e^{−r_for·t} + r_dom·K·e^{−r_dom·t}).
    let theta = sign * n * (-r_for * s * df_for + r_dom * k * df_dom);

    ForwardGreeks {
        pv,
        delta,
        rho_dom,
        rho_for,
        theta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputs::Side;
    use celnet_types::{Carry, CcyPair, Underlying};

    fn eurusd() -> Underlying {
        Underlying::Fx(CcyPair::parse("EURUSD").unwrap())
    }

    fn mk(spot: f64, k: f64, n: f64, side: Side, r_dom: f64, r_for: f64, t: f64) -> LinearInputs {
        LinearInputs::outright(
            spot,
            eurusd(),
            Carry::FxRates { r_dom, r_for },
            crate::inputs::LinearTerms::new(k, n, side),
            t,
        )
        .unwrap()
    }

    // ----------------------------------------------------------------------
    // INDEPENDENT ORACLE (a route that shares no intermediate with the impl).
    //
    // Covered-interest-parity, re-derived from FIRST PRINCIPLES (not the impl):
    //   A long base position held to t worth `spot·notional` today, financed by
    //   borrowing in the quote ccy, accrues the base deposit rate r_for and is
    //   discounted at the quote (discount) rate r_dom. No-arbitrage forces the
    //   forward price F to satisfy  F·e^{−r_dom·t} = spot·e^{−r_for·t}, i.e.
    //       F = spot · e^{(r_dom − r_for)·t}.
    //   Hence the forward's PV, valuing each leg as a zero-coupon discount bond:
    //       PV = side·notional·( spot·e^{−r_for·t}  −  K·e^{−r_dom·t} )         (★)
    //   = long a base-discount-bond worth spot·e^{−r_for·t}, short K quote-
    //     discount-bonds worth K·e^{−r_dom·t}.
    //
    // The PRODUCTION code computes  side·notional·df·(F − K)  with
    //   F = spot·forward_factor(t) = spot·e^{(r_dom−r_for)t},  df = e^{−r_dom·t}.
    // Expanding:  df·(F−K) = e^{−r_dom·t}·spot·e^{(r_dom−r_for)t} − e^{−r_dom·t}·K
    //                       = spot·e^{−r_for·t} − K·e^{−r_dom·t}  ≡ (★).
    // Algebraically identical, but the oracle ROUTE never forms F or df — it goes
    // straight to the two discount-bond legs from the raw rates, a genuinely
    // different rounding path. A forward/discount/sign slip in the impl shows up
    // as a disagreement here.
    // ----------------------------------------------------------------------
    fn oracle_pv(spot: f64, k: f64, n: f64, side: Side, r_dom: f64, r_for: f64, t: f64) -> f64 {
        side.sign()
            * n
            * (spot * celnet_core::math::exp(-r_for * t) - k * celnet_core::math::exp(-r_dom * t))
    }

    #[test]
    fn pv_matches_independent_discount_bond_route() {
        let cases = [
            (1.2345, 1.30, 1_000_000.0, Side::Buy, 0.04, 0.01, 0.75),
            (1.2345, 1.10, 5_000_000.0, Side::Sell, 0.02, 0.05, 1.5),
            (150.0, 145.0, 2_000_000.0, Side::Buy, 0.005, 0.001, 0.25),
            (0.65, 0.70, 3_000_000.0, Side::Sell, 0.03, 0.04, 2.0),
        ];
        for (s, k, n, side, rd, rf, t) in cases {
            let li = mk(s, k, n, side, rd, rf, t);
            let prod = pv(&li);
            let orc = oracle_pv(s, k, n, side, rd, rf, t);
            // Closed-form to closed-form: tight relative tolerance.
            assert!(
                (prod - orc).abs() <= 1e-9 * orc.abs().max(1.0),
                "prod {prod} vs oracle {orc}"
            );
        }
    }

    /// One HAND-PINNED absolute PV literal, computed externally (not from the
    /// impl), with the full derivation in-comment.
    ///
    /// Terms: spot = 1.2000, K = 1.2500, r_dom = 0.0500, r_for = 0.0200,
    ///        t = 1.0 (yr), notional = 1.0, side = Buy.
    ///   spot·e^{−r_for·t} = 1.2000 · e^{−0.02}  = 1.176238407968106…
    ///   K·e^{−r_dom·t}    = 1.2500 · e^{−0.05}  = 1.189036780625892…
    ///   PV = +1 · 1.0 · (1.176238407968106… − 1.189036780625892…)
    ///      = −0.012798372657786…
    /// (A BUY forward struck ABOVE the fair forward F = 1.2·e^{0.03}=1.236545…
    ///  has negative PV — economically correct.)
    #[test]
    fn pv_hand_pinned_literal() {
        let li = mk(1.2000, 1.2500, 1.0, Side::Buy, 0.05, 0.02, 1.0);
        let expected = -0.012_798_372_657_786_272_f64;
        assert!(
            (pv(&li) - expected).abs() <= 1e-13,
            "pv {} vs pinned {expected}",
            pv(&li)
        );
    }

    /// Structural gate (CAN DISAGREE): a forward struck at the fair forward has
    /// PV exactly 0.0 (to_bits after normalising −0.0 → +0.0).
    #[test]
    fn fair_forward_has_zero_pv_to_bits() {
        let base = mk(1.2345, 999.0, 1_000_000.0, Side::Buy, 0.04, 0.01, 0.75);
        let k = fair_forward(&base);
        // Re-strike the same input at the fair forward.
        let at_fair = LinearInputs {
            contract_rate: k,
            ..base
        };
        // F − K = spot·ff − spot·ff = 0 exactly (same product), so PV is ±0.0.
        let v = pv(&at_fair) + 0.0; // normalise −0.0 → +0.0
        assert_eq!(v.to_bits(), 0.0_f64.to_bits());
    }

    /// Structural gate: PV is linear in notional (2× notional ⇒ 2× PV).
    #[test]
    fn pv_linear_in_notional() {
        let a = mk(1.2345, 1.30, 1_000_000.0, Side::Buy, 0.04, 0.01, 0.75);
        let b = mk(1.2345, 1.30, 2_000_000.0, Side::Buy, 0.04, 0.01, 0.75);
        assert!((pv(&b) - 2.0 * pv(&a)).abs() <= 1e-12 * pv(&a).abs().max(1.0));
    }

    /// Structural gate: a long + a short at the same rate net to exactly 0.
    #[test]
    fn long_plus_short_nets_to_zero() {
        let long = mk(1.2345, 1.30, 1_000_000.0, Side::Buy, 0.04, 0.01, 0.75);
        let short = mk(1.2345, 1.30, 1_000_000.0, Side::Sell, 0.04, 0.01, 0.75);
        let net = pv(&long) + pv(&short);
        assert_eq!((net + 0.0).to_bits(), 0.0_f64.to_bits());
    }

    /// Limit gate (a DIFFERENT expression): `t → 0` ⇒ PV → side·N·(spot − K),
    /// the undiscounted intrinsic, with no carry/discount factors.
    #[test]
    fn t_to_zero_is_undiscounted_intrinsic() {
        let s = 1.2345;
        let k = 1.30;
        let n = 1_000_000.0;
        for side in [Side::Buy, Side::Sell] {
            let li = mk(s, k, n, side, 0.04, 0.01, 0.0);
            let expected = side.sign() * n * (s - k);
            assert!((pv(&li) - expected).abs() <= 1e-9 * expected.abs().max(1.0));
        }
    }

    // --- Greeks: exact analytic, cross-checked by independent central FD. ---

    fn fd_central(f: impl Fn(f64) -> f64, x: f64, h: f64) -> f64 {
        (f(x + h) - f(x - h)) / (2.0 * h)
    }

    #[test]
    fn greeks_match_central_finite_difference() {
        let s = 1.2345;
        let k = 1.30;
        let n = 1_000_000.0;
        let rd = 0.04;
        let rf = 0.01;
        let t = 0.75;
        let side = Side::Buy;
        let g = greeks(&mk(s, k, n, side, rd, rf, t));

        // delta = ∂PV/∂spot.
        let d_fd = fd_central(|x| pv(&mk(x, k, n, side, rd, rf, t)), s, 1e-6);
        assert!((g.delta - d_fd).abs() <= 1e-4 * g.delta.abs().max(1.0));

        // rho_dom = ∂PV/∂r_dom.
        let rd_fd = fd_central(|x| pv(&mk(s, k, n, side, x, rf, t)), rd, 1e-7);
        assert!((g.rho_dom - rd_fd).abs() <= 1e-3 * g.rho_dom.abs().max(1.0));

        // rho_for = ∂PV/∂r_for.
        let rf_fd = fd_central(|x| pv(&mk(s, k, n, side, rd, x, t)), rf, 1e-7);
        assert!((g.rho_for - rf_fd).abs() <= 1e-3 * g.rho_for.abs().max(1.0));

        // theta = ∂PV/∂t.
        let th_fd = fd_central(|x| pv(&mk(s, k, n, side, rd, rf, x)), t, 1e-7);
        assert!((g.theta - th_fd).abs() <= 1e-3 * g.theta.abs().max(1.0));

        // pv field agrees with pv().
        assert!((g.pv - pv(&mk(s, k, n, side, rd, rf, t))).abs() <= 1e-9 * g.pv.abs().max(1.0));
    }

    /// Greeks flip sign with the side (linear in the `±1` multiplier).
    #[test]
    fn greeks_sign_flips_with_side() {
        let buy = greeks(&mk(1.2345, 1.30, 1e6, Side::Buy, 0.04, 0.01, 0.75));
        let sell = greeks(&mk(1.2345, 1.30, 1e6, Side::Sell, 0.04, 0.01, 0.75));
        assert!((buy.delta + sell.delta).abs() <= 1e-9 * buy.delta.abs().max(1.0));
        assert!((buy.rho_dom + sell.rho_dom).abs() <= 1e-9 * buy.rho_dom.abs().max(1.0));
        assert!((buy.rho_for + sell.rho_for).abs() <= 1e-9 * buy.rho_for.abs().max(1.0));
        assert!((buy.theta + sell.theta).abs() <= 1e-9 * buy.theta.abs().max(1.0));
    }
}
