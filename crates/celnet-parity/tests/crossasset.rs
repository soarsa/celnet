//! Parity rows for the **cross-asset vanilla leaves** — the `vanilla` product arm of
//! the one `celnet.proto` contract seen through a NON-FX `Underlying.ref` arm
//! (equity / commodity / digital-asset), each priced on the asset-class-agnostic
//! cost-of-carry seam (ADR-0008):
//!
//!   * `celnet-equity-vanilla`   — generalized Black-Scholes-Merton, `b = r − q − repo`,
//!   * `celnet-commodity-vanilla`— Black-76 (`b = 0` on a future / `b = r − convenience`),
//!   * `celnet-crypto-vanilla`   — linear (funded BSM) + inverse (coin-margined `1/S_T`).
//!
//! ## The crux: an INDEPENDENT, code-disjoint oracle (anti-circular discipline)
//!
//! Each production leaf is reconciled to the closed-form oracle in
//! [`celnet_golden::oracle`], which is reached by a **different special-function
//! route**: the oracle's normal CDF is `½·(1 + erf(x/√2))` over **`libm::erf`**,
//! while every leaf computes `celnet_core::math::norm_cdf = ½·erfc(−x/√2)` over
//! **`libm::erfc`**. `erf` and `erfc` are independent library routines (different
//! rational/polynomial branches; `erfc` is not internally `1 − erf` in the tail), so
//! a closed form built on one shares no arithmetic with the other — the FRTB-0.75ρ
//! circular-oracle lesson made concrete. `celnet-golden` does not depend on the
//! equity/commodity/crypto leaves, so these rows never re-use a crate to check
//! itself.
//!
//! In addition each arm carries **model-free gates that can disagree**: put-call
//! parity (a different expression than the per-leg Φ-weighted price), the structural
//! limits the brief calls out (`q = 0` collapses the equity engine to standard BSM;
//! `b = 0` collapses commodity/crypto to the flat-forward Black-76; `funding = r ⇒
//! b = 0`), and **central finite-difference Greeks** (delta/vega cross-checked
//! against a difference of the production price). The inverse-crypto leaf also gets
//! its signed convexity sandwich (`V_coin·S₀ < V_lin` for calls, `>` for puts —
//! strictly, which a naive `V_lin/S₀` rescale violates).
//!
//! ## Honest boundary (VERIFICATION-CONTRACT §g)
//!
//! The in-repo proof is the payoff/measure math and the carry assembly. Live
//! dividend/convenience/funding VALUES and live vol surfaces are ENV — only the carry
//! seam and the convention identities are encoded in-repo.

use celnet_commodity_vanilla as commodity;
use celnet_crypto_vanilla::{inverse, linear};
use celnet_equity_vanilla as equity;
use celnet_golden::oracle::{self, Cp};
use celnet_types::{OptionType, RateSensitivities};

fn cp(opt: OptionType) -> Cp {
    match opt {
        OptionType::Call => Cp::Call,
        OptionType::Put => Cp::Put,
    }
}

/// Relative-or-absolute closeness (closed-form to closed-form).
fn close(a: f64, b: f64, rel: f64, abs: f64) -> bool {
    (a - b).abs() <= abs + rel * a.abs().max(b.abs())
}

/// Central finite difference of `f` at `x` with step `h`.
fn fd1<F: Fn(f64) -> f64>(f: F, x: f64, h: f64) -> f64 {
    (f(x + h) - f(x - h)) / (2.0 * h)
}

// ===========================================================================
// Equity — generalized Black-Scholes-Merton (b = r − q − repo)
// ===========================================================================

/// PRIMARY: the production equity price matches the INDEPENDENT generalized-BSM
/// oracle (the `libm::erf` route in `celnet-golden`, disjoint from the leaf's
/// `erfc`-based `norm_cdf`). Closed-form to closed-form ⇒ tight tolerance.
#[test]
fn equity_price_matches_independent_bsm_oracle() {
    let cases = [
        (930.0, 900.0, 0.20, 1.0 / 6.0, 0.08, 0.03, 0.0),
        (100.0, 105.0, 0.30, 1.0, 0.05, 0.02, 0.01),
        (100.0, 100.0, 0.20, 1.0, 0.05, 0.03, 0.0),
        (50.0, 55.0, 0.45, 3.0, -0.01, 0.0, 0.0),
    ];
    for (s, k, vol, t, r, q, repo) in cases {
        let i = equity::EquityInputs::new(s, k, vol, t, r, q, repo);
        for opt in [OptionType::Call, OptionType::Put] {
            let prod = equity::price(opt, &i);
            let orc = oracle::equity_bsm_price(cp(opt), s, k, vol, t, r, q, repo);
            assert!(
                close(prod, orc, 1e-9, 1e-9),
                "equity {opt:?}: production {prod} vs independent erf-BSM oracle {orc}"
            );
        }
    }
}

/// MODEL-FREE (can disagree): put-call parity `C − P = e^{(b−r)t}·S − e^{−rt}·K` —
/// a different expression than the per-leg price, so a sign/weight slip surfaces.
#[test]
fn equity_put_call_parity() {
    let i = equity::EquityInputs::new(123.45, 130.0, 0.22, 0.75, 0.03, 0.018, 0.004);
    let lhs = equity::price(OptionType::Call, &i) - equity::price(OptionType::Put, &i);
    let b = i.carry();
    let rhs = i.spot * ((b - i.r) * i.t).exp() - i.strike * (-i.r * i.t).exp();
    assert!(
        close(lhs, rhs, 1e-12, 1e-12),
        "equity parity {lhs} vs {rhs}"
    );
}

/// STRUCTURAL LIMIT (a different expression): `q = 0`, `repo = 0` ⇒ `b = r`, the
/// standard non-dividend Black-Scholes-Merton with the spot leg UNDISCOUNTED — an
/// independent closed form (`S·Φ(d1) − K·e^{−rt}·Φ(d2)`) the production engine must
/// degenerate to.
#[test]
fn equity_zero_dividend_is_standard_bsm() {
    for (s, k, vol, t, r) in [
        (100.0, 100.0, 0.20, 1.0, 0.05),
        (42.0, 40.0, 0.25, 0.5, 0.10),
    ] {
        let i = equity::EquityInputs::new(s, k, vol, t, r, 0.0, 0.0);
        // Independent standard (q=0) BSM via the erf-route oracle with b=r ⇒ q=repo=0.
        let std_call = oracle::equity_bsm_price(Cp::Call, s, k, vol, t, r, 0.0, 0.0);
        assert!(close(
            equity::price(OptionType::Call, &i),
            std_call,
            1e-12,
            1e-12
        ));
        // The undiscounted-spot leg is the structural signature: at b=r the d1 carry
        // term is +r and the spot coefficient e^{(b−r)t}=1.
        let vsqt = vol * t.sqrt();
        let d1 = ((s / k).ln() + (r + 0.5 * vol * vol) * t) / vsqt;
        let d2 = d1 - vsqt;
        let manual = s * oracle::xerf_norm_cdf(d1) - k * (-r * t).exp() * oracle::xerf_norm_cdf(d2);
        assert!(close(
            equity::price(OptionType::Call, &i),
            manual,
            1e-12,
            1e-12
        ));
    }
}

/// GREEKS (independent central FD): delta/vega + the dividend-rho (carry-rho) cross-
/// checked against a finite difference of the production price. The carry-rho is
/// `∂V/∂b` at fixed `r`, reached by moving `q` (`∂b/∂q = −1` ⇒ `∂V/∂b = −∂V/∂q`).
#[test]
fn equity_greeks_match_central_fd() {
    let cases = [
        equity::EquityInputs::dividend_paying(100.0, 100.0, 0.20, 1.0, 0.05, 0.03),
        equity::EquityInputs::new(110.0, 95.0, 0.30, 0.25, 0.01, 0.04, 0.005),
    ];
    for i in &cases {
        for opt in [OptionType::Call, OptionType::Put] {
            let g = equity::greeks(opt, i);
            let hs = 1e-4 * i.spot;
            assert!(close(
                g.delta_spot,
                fd1(
                    |s| equity::price(opt, &equity::EquityInputs { spot: s, ..*i }),
                    i.spot,
                    hs
                ),
                1e-4,
                1e-7
            ));
            assert!(close(
                g.vega,
                fd1(
                    |v| equity::price(opt, &equity::EquityInputs { vol: v, ..*i }),
                    i.vol,
                    1e-5
                ),
                1e-4,
                1e-7
            ));
            let RateSensitivities::Carry { carry_rho, .. } = g.rates else {
                panic!("equity greeks must tag as Carry");
            };
            // carry-rho = ∂V/∂b = −∂V/∂q (move q at fixed r).
            let fd = -fd1(
                |q| equity::price(opt, &equity::EquityInputs { q, ..*i }),
                i.q,
                1e-6,
            );
            assert!(
                close(carry_rho, fd, 1e-5, 1e-7),
                "equity carry-rho {carry_rho} vs FD {fd}"
            );
        }
    }
}

// ===========================================================================
// Commodity — Black-76 (b = 0 on a future / b = r − convenience on spot)
// ===========================================================================

/// PRIMARY: the production commodity price (both representations) matches the
/// INDEPENDENT Black-76 oracle reached on the FORWARD directly — the oracle never
/// re-forms the leaf's `spot · forward_factor(t)`, a different route.
#[test]
fn commodity_price_matches_independent_black76_oracle() {
    // (representation, spot/future, K, vol, t, r, convenience)
    for opt in [OptionType::Call, OptionType::Put] {
        // Future representation: spot IS the forward, b = 0.
        let fut = commodity::CommodityInputs::on_future(20.0, 19.0, 0.25, 1.0 / 3.0, 0.09);
        let f_fut = 20.0;
        let orc_fut = oracle::black76_price(cp(opt), f_fut, 19.0, 0.25, 1.0 / 3.0, 0.09);
        assert!(close(commodity::price(opt, &fut), orc_fut, 1e-9, 1e-9));

        // Spot + convenience: F = S·e^{(r−conv)t}.
        let (s, k, vol, t, r, conv) = (100.0, 98.0, 0.28, 0.5, 0.05, 0.02);
        let spot = commodity::CommodityInputs::on_spot(s, k, vol, t, r, conv);
        let f_spot = s * ((r - conv) * t).exp();
        let orc_spot = oracle::black76_price(cp(opt), f_spot, k, vol, t, r);
        assert!(close(commodity::price(opt, &spot), orc_spot, 1e-9, 1e-9));
    }
}

/// MODEL-FREE (can disagree): put-call parity on the future `C − P = df·(F − K)`.
#[test]
fn commodity_put_call_parity() {
    let i = commodity::CommodityInputs::on_spot(100.0, 105.0, 0.3, 0.75, 0.04, 0.015);
    let lhs = commodity::price(OptionType::Call, &i) - commodity::price(OptionType::Put, &i);
    let rhs = i.discount_df() * (i.forward() - 105.0);
    assert!(
        close(lhs, rhs, 1e-12, 1e-12),
        "commodity parity {lhs} vs {rhs}"
    );
}

/// STRUCTURAL LIMIT (`b = 0`): an `on_future` input has a FLAT forward (`F = spot`),
/// so the price is pure Black-76 off the unchanged level — and equals the `on_spot`
/// price whose convenience exactly offsets the rate (`convenience = r ⇒ b = 0`).
#[test]
fn commodity_zero_carry_flat_forward() {
    let (lvl, k, vol, t, r) = (50.0, 52.0, 0.3, 1.0, 0.06);
    let fut = commodity::CommodityInputs::on_future(lvl, k, vol, t, r);
    assert_eq!(fut.forward().to_bits(), lvl.to_bits());
    let spot_b0 = commodity::CommodityInputs::on_spot(lvl, k, vol, t, r, r); // conv = r ⇒ b = 0
    for opt in [OptionType::Call, OptionType::Put] {
        assert!(close(
            commodity::price(opt, &fut),
            commodity::price(opt, &spot_b0),
            1e-12,
            1e-12
        ));
        // And both equal the independent Black-76 on the flat forward F = lvl.
        let orc = oracle::black76_price(cp(opt), lvl, k, vol, t, r);
        assert!(close(commodity::price(opt, &fut), orc, 1e-9, 1e-9));
    }
}

/// GREEKS (independent central FD): delta/vega cross-checked against a difference of
/// the production commodity price.
#[test]
fn commodity_greeks_match_central_fd() {
    let i = commodity::CommodityInputs::on_spot(100.0, 100.0, 0.28, 0.5, 0.05, 0.02);
    for opt in [OptionType::Call, OptionType::Put] {
        let g = commodity::greeks(opt, &i);
        let hs = 1e-4 * i.spot;
        assert!(close(
            g.delta_spot,
            fd1(
                |s| commodity::price(opt, &commodity::CommodityInputs { spot: s, ..i }),
                i.spot,
                hs
            ),
            1e-4,
            1e-7
        ));
        assert!(close(
            g.vega,
            fd1(
                |v| commodity::price(opt, &commodity::CommodityInputs { vol: v, ..i }),
                i.vol,
                1e-5
            ),
            1e-4,
            1e-7
        ));
    }
}

// ===========================================================================
// Crypto — linear (funded BSM) + inverse (coin-margined 1/S_T)
// ===========================================================================

/// PRIMARY (linear): the production linear-crypto price matches the INDEPENDENT
/// forward-space generalized-BSM oracle (`b = r − funding`), reached on `libm::erf`
/// and in forward space — a different operation order than the leaf's spot-space
/// (FX-tied) arithmetic.
#[test]
fn crypto_linear_price_matches_independent_oracle() {
    let cases = [
        (30_000.0, 31_000.0, 0.65, 0.5, 0.05, 0.02),
        (2_000.0, 1_800.0, 0.80, 0.5, 0.04, 0.10),
        (45_000.0, 45_000.0, 0.55, 1.0, 0.03, -0.01),
    ];
    for (s, k, vol, t, r, funding) in cases {
        let i = linear::LinearInputs::funded(s, k, vol, t, r, funding);
        for opt in [OptionType::Call, OptionType::Put] {
            let prod = linear::price(opt, &i);
            let orc = oracle::crypto_linear_price(cp(opt), s, k, vol, t, r, funding);
            assert!(
                close(prod, orc, 1e-9, 1e-9),
                "crypto linear {opt:?}: production {prod} vs independent oracle {orc}"
            );
        }
    }
}

/// PRIMARY (inverse): the production inverse/coin-margined price matches the
/// INDEPENDENT `1/S_T` closed-form oracle
/// `V_coin = φ·df·[Φ(φ·d2) − (K/F)·e^{σ²t}·Φ(φ·d3)]` (the genuine convexity
/// transform, NOT `V_lin/S₀`), reached on the disjoint `libm::erf` route.
#[test]
fn crypto_inverse_price_matches_independent_oracle() {
    let cases = [
        (30_000.0, 31_000.0, 0.65, 0.5, 0.05, 0.02),
        (2_000.0, 1_800.0, 0.80, 0.5, 0.04, 0.10),
        (45_000.0, 45_000.0, 0.55, 1.0, 0.03, -0.01),
    ];
    for (s, k, vol, t, r, funding) in cases {
        let i = inverse::InverseInputs::funded(s, k, vol, t, r, funding);
        for opt in [OptionType::Call, OptionType::Put] {
            let prod = inverse::price(opt, &i);
            let orc = oracle::crypto_inverse_price(cp(opt), s, k, vol, t, r, funding);
            assert!(
                close(prod, orc, 1e-9, 1e-9),
                "crypto inverse {opt:?}: production {prod} vs independent oracle {orc}"
            );
        }
    }
}

/// MODEL-FREE (linear, can disagree): put-call parity `C − P = df·(F − K)`.
#[test]
fn crypto_linear_put_call_parity() {
    let i = linear::LinearInputs::funded(30_000.0, 32_000.0, 0.65, 0.25, 0.05, 0.02);
    let lhs = linear::price(OptionType::Call, &i) - linear::price(OptionType::Put, &i);
    let rhs = i.discount_df() * (i.forward() - 32_000.0);
    assert!(
        close(lhs, rhs, 1e-9, 1e-9),
        "crypto linear parity {lhs} vs {rhs}"
    );
}

/// STRUCTURAL LIMIT (`funding = r ⇒ b = 0`): the linear-crypto forward is flat
/// (`F = S`) and the price is the Black-76 limit on the unchanged spot — an
/// independent route the production carry assembly must collapse to.
#[test]
fn crypto_linear_zero_carry_is_black76_limit() {
    let (s, k, vol, t, r) = (30_000.0, 31_000.0, 0.65, 0.5, 0.05);
    let i = linear::LinearInputs::funded(s, k, vol, t, r, r); // funding = r ⇒ b = 0
    assert_eq!(i.forward().to_bits(), s.to_bits());
    for opt in [OptionType::Call, OptionType::Put] {
        let orc = oracle::black76_price(cp(opt), s, k, vol, t, r);
        assert!(close(linear::price(opt, &i), orc, 1e-9, 1e-9));
    }
}

/// STRUCTURAL (inverse, SIGNED convexity sandwich — disagree-capable): `V_coin·S₀`
/// is STRICTLY below `V_lin` for a call (the `1/S_T` weight down-weights the growing
/// payoff) and STRICTLY above for a put. A naive `V_lin/S₀` rescale gives exact
/// equality, so this signed gate fails loudly on it (the binding anti-circular
/// statement for the inverse leaf).
#[test]
fn crypto_inverse_convexity_sandwich_is_signed() {
    for (s, k, vol, t, r, funding) in [
        (30_000.0, 30_000.0, 0.65, 0.5, 0.05, 0.02),
        (2_000.0, 2_500.0, 0.80, 1.0, 0.04, 0.10),
        (45_000.0, 40_000.0, 0.55, 0.25, 0.03, -0.01),
    ] {
        let inv = inverse::InverseInputs::funded(s, k, vol, t, r, funding);
        let lin = linear::LinearInputs::funded(s, k, vol, t, r, funding);
        let coin_call = inverse::price(OptionType::Call, &inv) * s;
        let usd_call = linear::price(OptionType::Call, &lin);
        assert!(
            coin_call < usd_call * (1.0 - 1e-6),
            "call: {coin_call} must be < {usd_call}"
        );
        let coin_put = inverse::price(OptionType::Put, &inv) * s;
        let usd_put = linear::price(OptionType::Put, &lin);
        assert!(
            coin_put > usd_put * (1.0 + 1e-6),
            "put: {coin_put} must be > {usd_put}"
        );
    }
}

/// GREEKS (independent central FD): linear and inverse coin-measure delta/vega cross-
/// checked against a difference of the respective production price.
#[test]
fn crypto_greeks_match_central_fd() {
    let li = linear::LinearInputs::funded(30_000.0, 30_000.0, 0.65, 0.5, 0.05, 0.02);
    let ii = inverse::InverseInputs::funded(30_000.0, 30_000.0, 0.65, 0.5, 0.05, 0.02);
    for opt in [OptionType::Call, OptionType::Put] {
        let lg = linear::greeks(opt, &li);
        let hs = 1e-4 * li.spot;
        assert!(close(
            lg.delta_spot,
            fd1(
                |s| linear::price(opt, &linear::LinearInputs { spot: s, ..li }),
                li.spot,
                hs
            ),
            1e-4,
            1e-7
        ));
        assert!(close(
            lg.vega,
            fd1(
                |v| linear::price(opt, &linear::LinearInputs { vol: v, ..li }),
                li.vol,
                1e-5
            ),
            1e-4,
            1e-7
        ));

        let ig = inverse::greeks(opt, &ii).coin;
        let his = 1e-4 * ii.spot;
        assert!(close(
            ig.delta_spot,
            fd1(
                |s| inverse::price(opt, &inverse::InverseInputs { spot: s, ..ii }),
                ii.spot,
                his
            ),
            1e-4,
            1e-9
        ));
        assert!(close(
            ig.vega,
            fd1(
                |v| inverse::price(opt, &inverse::InverseInputs { vol: v, ..ii }),
                ii.vol,
                1e-5
            ),
            1e-4,
            1e-9
        ));
    }
}
