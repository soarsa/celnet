//! Parity rows 4–5: the **full 13-Greek set** finite-difference-validated, and
//! put-call parity across regimes.
//!
//! Bloomberg OVML and Fenics kACE expose Greeks from closed pricers that are not
//! published as FD-validated; Celnet's claim
//! (`docs/CAPABILITIES-VS-COMPETITION.md` §2) is that *every* Greek agrees with a
//! central finite difference of the price (or of the relevant first-order Greek
//! for the second-order ones). We gate all thirteen reported sensitivities
//! (the `price` field of [`celnet_types::Greeks`] is the value, not a Greek):
//!
//!  - price, delta_spot, delta_forward, gamma, vega, theta, rho_dom, rho_for,
//!    vanna, volga, charm, speed, zomma, color.
//!
//! Nine are validated through the shared testkit oracle; the remaining four
//! (delta_forward, speed, zomma, color) are differenced directly here so the
//! parity matrix gates the *entire* set, not a subset.

use celnet_core::is_close;
use celnet_testkit::{
    GreekKind, assert_greek_matches_fd, assert_put_call_parity, central_difference,
    reference_markets,
};
use celnet_types::{OptionType, VanillaInputs};
use celnet_vanilla::{greeks, price};

/// The nine Greeks the shared testkit FD oracle knows how to bump.
const TESTKIT_GREEKS: [GreekKind; 9] = [
    GreekKind::DeltaSpot,
    GreekKind::Vega,
    GreekKind::RhoDom,
    GreekKind::RhoFor,
    GreekKind::Theta,
    GreekKind::Gamma,
    GreekKind::Vanna,
    GreekKind::Volga,
    GreekKind::Charm,
];

/// Row 4 — the nine "core" Greeks each agree with a central finite difference of
/// the price function, on every curated regime, for calls and puts. The
/// tolerances are loose enough only to admit FD truncation error, tight enough
/// to catch a sign or factor mistake (this is the harness that caught the
/// production sign-inverted charm noted in §5).
#[test]
fn full_greek_set_matches_finite_difference() {
    let mut rows = 0usize;
    for m in reference_markets() {
        for opt in [OptionType::Call, OptionType::Put] {
            for kind in TESTKIT_GREEKS {
                // Second-order Greeks differenced from a first-order Greek carry
                // more FD noise; scale tolerance with the difference order.
                let (rel, abs) = match kind {
                    GreekKind::Gamma | GreekKind::Vanna | GreekKind::Volga | GreekKind::Charm => {
                        (1e-4, 1e-6)
                    }
                    _ => (1e-5, 1e-7),
                };
                assert_greek_matches_fd(opt, &m.inputs, kind, rel, abs);
                rows += 1;
            }
        }
    }
    assert!(rows >= 126, "core-Greek FD rows under-gated: {rows}");
}

/// Row 4 (wing Greeks) — the four higher-order/forward sensitivities the shared
/// oracle does not cover (`delta_forward`, `speed`, `zomma`, `color`) are each
/// **finite-difference-validated against their defining derivative** here, so the
/// matrix proves the *complete* 13-Greek set, not a subset:
///
///  * `speed = ∂gamma/∂S`, `zomma = ∂gamma/∂σ`, `color = ∂gamma/∂T` — central
///    differences of the analytic `gamma`;
///  * `delta_forward = ∂V_fwd/∂F`, the derivative of the **undiscounted forward
///    value** `V_fwd = price·e^{r_d T}` with respect to the **forward** `F`. We
///    difference that quantity directly: the forward is bumped by bumping spot
///    (since `F = S·e^{(r_d−r_f)T}`, `∂/∂F = e^{−(r_d−r_f)T}·∂/∂S`), and the
///    discounted price is undiscounted by `e^{r_d T}`. This is the real
///    definition — not a finiteness/bound placeholder.
#[test]
fn second_order_wing_greeks_match_fd() {
    let mut rows = 0usize;
    for m in reference_markets() {
        let i = m.inputs;
        for opt in [OptionType::Call, OptionType::Put] {
            let g = greeks(opt, &i);
            let gamma_of = |x: &VanillaInputs| greeks(opt, x).gamma;

            let hs = 1e-4 * i.spot;
            let hv = 1e-5;
            let ht = 1e-5 * i.t.max(1.0);

            // speed = ∂gamma/∂S.
            let speed_fd =
                central_difference(|s| gamma_of(&VanillaInputs { spot: s, ..i }), i.spot, hs);
            assert!(
                is_close(g.speed, speed_fd, 1e-3, 1e-6),
                "{} {opt:?} speed: analytic {} vs FD {speed_fd}",
                m.name,
                g.speed
            );

            // zomma = ∂gamma/∂σ.
            let zomma_fd =
                central_difference(|v| gamma_of(&VanillaInputs { vol: v, ..i }), i.vol, hv);
            assert!(
                is_close(g.zomma, zomma_fd, 1e-3, 1e-6),
                "{} {opt:?} zomma: analytic {} vs FD {zomma_fd}",
                m.name,
                g.zomma
            );

            // color = ∂gamma/∂T.
            let color_fd = central_difference(|t| gamma_of(&VanillaInputs { t, ..i }), i.t, ht);
            assert!(
                is_close(g.color, color_fd, 1e-3, 1e-6),
                "{} {opt:?} color: analytic {} vs FD {color_fd}",
                m.name,
                g.color
            );

            // delta_forward = ∂V_fwd/∂F, the derivative of the UNDISCOUNTED
            // forward value w.r.t. the forward. Bump the forward through spot:
            //   F = S·e^{(r_d−r_f)T} ⇒ dF = e^{(r_d−r_f)T}·dS,
            // difference the undiscounted value V_fwd = price·e^{r_d T} in spot,
            // then divide by the carry factor e^{(r_d−r_f)T} to convert ∂/∂S to
            // ∂/∂F. This validates delta_forward against its real definition.
            let carry = ((i.r_dom - i.r_for) * i.t).exp();
            let edomt = (i.r_dom * i.t).exp();
            let v_fwd = |x: &VanillaInputs| price(opt, x) * edomt;
            let dv_ds = central_difference(|s| v_fwd(&VanillaInputs { spot: s, ..i }), i.spot, hs);
            let delta_forward_fd = dv_ds / carry;
            assert!(
                is_close(g.delta_forward, delta_forward_fd, 1e-4, 1e-7),
                "{} {opt:?} delta_forward: analytic {} vs FD {delta_forward_fd}",
                m.name,
                g.delta_forward
            );
            rows += 1;
        }
    }
    assert!(rows >= 14, "wing-Greek rows under-gated: {rows}");
}

/// Row 5 — put-call parity `C − P = S·e^{−r_f T} − K·e^{−r_d T}` holds across
/// every regime. The correctness floor underpinning all of the above; a parity
/// break is the loudest possible pricing defect.
#[test]
fn put_call_parity_across_regimes() {
    let mut rows = 0usize;
    for m in reference_markets() {
        assert_put_call_parity(&m.inputs);
        rows += 1;
    }
    assert!(rows >= 7, "parity rows under-gated: {rows}");
}
