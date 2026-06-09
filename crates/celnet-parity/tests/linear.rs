//! Parity rows for the **linear FX book** (`celnet-linear`): the outright forward,
//! the FX swap (near + far legs) and the non-deliverable forward (NDF), each
//! driven against an **independent oracle** reached by a route disjoint from the
//! production discounted-cashflow algebra, plus structural / limit / netting gates
//! that can disagree (per `docs/W2-LINEAR-PLAN.md` §3 and
//! `docs/VERIFICATION-CONTRACT.md` (a)–(g)).
//!
//! ## The crux: a HIGH circular-oracle risk on the forward formula
//!
//! A forward's PV is a *linear* discounted-cashflow identity, not an option
//! payoff, so a naïve "second implementation" would re-derive the same
//! `F = spot·e^{(r_dom−r_for)·t}` the production code uses — a circular oracle (the
//! FRTB 0.75ρ lesson). The discipline here is to reach the reference by a
//! genuinely **independent route** and by structural identities that the
//! production algebra cannot satisfy if it is wrong:
//!
//!   * **Independent route (two-zero-coupon-bond decomposition).** A long forward
//!     is long a base-currency discount bond worth `spot·e^{−r_for·t}` minus `K`
//!     quote-currency discount bonds worth `K·e^{−r_dom·t}`. Re-derived from
//!     covered-interest-parity FIRST PRINCIPLES (not the impl), the PV is
//!     `side·notional·(spot·e^{−r_for·t} − K·e^{−r_dom·t})` `(★)`. The production
//!     code computes `side·notional·df·(F−K)` with `F = spot·forward_factor(t)`,
//!     `df = discount_df(t)`. These are algebraically identical, but the oracle
//!     route never forms `F` or `df` — it goes straight to the two discount-bond
//!     legs from the raw rates, a different rounding path. A forward/discount/sign
//!     slip surfaces as a disagreement. (The independent oracle lives in
//!     `celnet_golden::oracle`, which does NOT depend on `celnet-linear` — so this
//!     row never re-uses the crate under test to check itself.)
//!   * **Structural gates that CAN disagree.** A forward struck at the fair
//!     forward has PV exactly 0; PV is linear in notional; long + short at the same
//!     rate net to 0. None of these reuse the forward expression, so a shared slip
//!     cannot hide.
//!   * **Limit gate (a different expression).** `t → 0` ⇒ PV → side·N·(spot − K),
//!     the undiscounted intrinsic, with no carry/discount factors at all.
//!   * **NDF.** A HAND-PINNED absolute literal (computed externally, derivation
//!     in-comment) AND the structural identity NDF == deliverable-forward PV in the
//!     same numeraire — two independent checks (one an absolute value, one an
//!     equality), so a shared slip cannot hide.
//!
//! ## Honest boundary (VERIFICATION-CONTRACT §g)
//!
//! The in-repo proof is the payoff / discounted-cashflow math and the settlement
//! convention identity. **Live NDF fixing VALUES are ENV** — only the fixing
//! *identity* ([`celnet_types::FixingSource`]) and the settlement convention are
//! encoded in-repo; the realized fixing rate is an estate-gated feed, never
//! sourced here. Performance: a linear PV is a handful of `exp`/multiply ops, far
//! inside the §1.2 in-core budget — trivially within budget, not a new hot path.

use celnet_golden::oracle;
use celnet_linear::{
    LinearInputs, LinearTerms, Ndf, Side, fair_forward, forward, swap, swap_points,
};
use celnet_types::{Carry, CcyPair, FixingSource, Underlying};

/// Map the `celnet-linear` [`Side`] to the `±1` sign the independent oracle uses.
fn sign(side: Side) -> f64 {
    match side {
        Side::Buy => 1.0,
        Side::Sell => -1.0,
    }
}

fn fx(r_dom: f64, r_for: f64) -> Carry {
    Carry::FxRates { r_dom, r_for }
}

fn pair(token: &str) -> Underlying {
    Underlying::Fx(CcyPair::parse(token).unwrap())
}

/// An outright-forward production input.
#[allow(clippy::too_many_arguments)]
fn outright(
    token: &str,
    spot: f64,
    k: f64,
    n: f64,
    side: Side,
    r_dom: f64,
    r_for: f64,
    t: f64,
) -> LinearInputs {
    LinearInputs::outright(
        spot,
        pair(token),
        fx(r_dom, r_for),
        LinearTerms::new(k, n, side),
        t,
    )
    .unwrap()
}

/// Relative-or-absolute closeness (closed-form to closed-form).
fn close(a: f64, b: f64, rel: f64, abs: f64) -> bool {
    (a - b).abs() <= abs + rel * a.abs().max(b.abs())
}

// ===========================================================================
// FX outright forward
// ===========================================================================

/// PRIMARY: the production forward PV matches the INDEPENDENT two-zero-coupon-bond
/// route `(★)` from `celnet_golden::oracle` (which does not depend on
/// `celnet-linear`). Closed-form to closed-form ⇒ tight tolerance.
#[test]
fn forward_pv_matches_independent_two_bond_route() {
    let cases = [
        (
            "EURUSD",
            1.2345,
            1.30,
            1_000_000.0,
            Side::Buy,
            0.04,
            0.01,
            0.75,
        ),
        (
            "EURUSD",
            1.2345,
            1.10,
            5_000_000.0,
            Side::Sell,
            0.02,
            0.05,
            1.5,
        ),
        (
            "USDJPY",
            150.0,
            145.0,
            2_000_000.0,
            Side::Buy,
            0.005,
            0.001,
            0.25,
        ),
        (
            "AUDUSD",
            0.65,
            0.70,
            3_000_000.0,
            Side::Sell,
            0.03,
            0.04,
            2.0,
        ),
    ];
    for (tok, s, k, n, side, rd, rf, t) in cases {
        let li = outright(tok, s, k, n, side, rd, rf, t);
        let prod = forward::pv(&li);
        let orc = oracle::fx_forward_pv(sign(side), s, k, n, t, rd, rf);
        assert!(
            close(prod, orc, 1e-9, 1e-6),
            "{tok}: production {prod} vs independent two-bond oracle {orc}"
        );
    }
}

/// HAND-PINNED absolute literal, computed EXTERNALLY (not from the impl), full
/// derivation in-comment — an absolute anchor independent of the oracle route.
///
/// Terms: spot = 1.2000, K = 1.2500, r_dom = 0.0500, r_for = 0.0200, t = 1.0,
///        notional = 1.0, side = Buy.
///   spot·e^{−r_for·t} = 1.2000 · e^{−0.02} = 1.176238407968106…
///   K·e^{−r_dom·t}    = 1.2500 · e^{−0.05} = 1.189036780625892…
///   PV = +1 · 1.0 · (1.176238407968106… − 1.189036780625892…)
///      = −0.012798372657786…
/// (A BUY forward struck ABOVE the fair forward F = 1.2·e^{0.03} = 1.236545…
///  has negative PV — economically correct.)
#[test]
fn forward_pv_hand_pinned_literal() {
    let li = outright("EURUSD", 1.2000, 1.2500, 1.0, Side::Buy, 0.05, 0.02, 1.0);
    let expected = -0.012_798_372_657_786_272_f64;
    assert!(
        (forward::pv(&li) - expected).abs() <= 1e-13,
        "production pv {} vs hand-pinned {expected}",
        forward::pv(&li)
    );
}

/// STRUCTURAL (can disagree): a forward struck at the fair forward has PV exactly
/// 0 — a different gate that does not reuse the forward expression.
#[test]
fn forward_struck_at_fair_forward_is_zero() {
    let base = outright(
        "EURUSD",
        1.2345,
        999.0,
        1_000_000.0,
        Side::Buy,
        0.04,
        0.01,
        0.75,
    );
    let k = fair_forward(&base);
    let at_fair = LinearInputs {
        contract_rate: k,
        ..base
    };
    // F − K = spot·ff − spot·ff = 0 exactly (same product) ⇒ PV is ±0.0.
    let v = forward::pv(&at_fair) + 0.0; // normalise −0.0 → +0.0
    assert_eq!(v.to_bits(), 0.0_f64.to_bits());
}

/// STRUCTURAL: PV is linear in notional (2× notional ⇒ 2× PV).
#[test]
fn forward_pv_linear_in_notional() {
    let a = outright(
        "EURUSD",
        1.2345,
        1.30,
        1_000_000.0,
        Side::Buy,
        0.04,
        0.01,
        0.75,
    );
    let b = outright(
        "EURUSD",
        1.2345,
        1.30,
        2_000_000.0,
        Side::Buy,
        0.04,
        0.01,
        0.75,
    );
    assert!(
        (forward::pv(&b) - 2.0 * forward::pv(&a)).abs() <= 1e-9 * forward::pv(&a).abs().max(1.0)
    );
}

/// STRUCTURAL: a long + a short at the same rate net to exactly 0 (to_bits).
#[test]
fn forward_long_plus_short_nets_to_zero() {
    let long = outright(
        "EURUSD",
        1.2345,
        1.30,
        1_000_000.0,
        Side::Buy,
        0.04,
        0.01,
        0.75,
    );
    let short = outright(
        "EURUSD",
        1.2345,
        1.30,
        1_000_000.0,
        Side::Sell,
        0.04,
        0.01,
        0.75,
    );
    let net = forward::pv(&long) + forward::pv(&short);
    assert_eq!((net + 0.0).to_bits(), 0.0_f64.to_bits());
}

/// LIMIT (a different expression): `t → 0` ⇒ PV → side·N·(spot − K), the
/// undiscounted intrinsic, with no carry/discount factors.
#[test]
fn forward_t_to_zero_is_undiscounted_intrinsic() {
    let (s, k, n) = (1.2345, 1.30, 1_000_000.0);
    for side in [Side::Buy, Side::Sell] {
        let li = outright("EURUSD", s, k, n, side, 0.04, 0.01, 0.0);
        let expected = sign(side) * n * (s - k);
        assert!(
            close(forward::pv(&li), expected, 1e-9, 1e-6),
            "t→0 pv {} vs undiscounted intrinsic {expected}",
            forward::pv(&li)
        );
    }
}

// ===========================================================================
// FX swap (near + far legs)
// ===========================================================================

/// PRIMARY: the production swap PV equals the INDEPENDENT sum of two outright
/// forwards (near at the input side, far at the opposite side), each computed by
/// the two-bond oracle from `celnet_golden::oracle` — an assembly disjoint from
/// the in-swap `pv_at`/struct-update route.
#[test]
fn swap_pv_matches_independent_two_leg_sum() {
    let cases = [
        (
            "EURUSD",
            1.2345,
            1.30,
            1e6,
            Side::Buy,
            0.04,
            0.01,
            0.25,
            0.75,
        ),
        (
            "USDJPY",
            150.0,
            145.0,
            2e6,
            Side::Sell,
            0.005,
            0.02,
            0.5,
            1.5,
        ),
        ("AUDUSD", 0.65, 0.70, 3e6, Side::Buy, 0.03, 0.01, 0.1, 2.0),
    ];
    for (tok, s, k, n, side, rd, rf, nt, ft) in cases {
        let swap_inp = outright(tok, s, k, n, side, rd, rf, nt)
            .with_far(ft)
            .unwrap();
        let prod = swap::pv(&swap_inp).unwrap();
        let orc = oracle::fx_swap_pv(sign(side), s, k, n, nt, ft, rd, rf);
        assert!(
            close(prod, orc, 1e-9, 1e-6),
            "{tok}: production swap {prod} vs independent two-leg oracle {orc}"
        );
    }
}

/// CROSS-CHECK (independent route): the production swap points
/// (`far_forward − near_forward`) match the CIP forward-points identity
/// `spot·(e^{b·far_t} − e^{b·near_t})` re-derived in the oracle.
#[test]
fn swap_points_match_independent_cip_identity() {
    let cases = [
        ("EURUSD", 1.2345, 0.04, 0.01, 0.25, 0.75),
        ("USDJPY", 150.0, 0.005, 0.02, 0.5, 1.5),
        ("AUDUSD", 0.65, 0.03, 0.01, 0.1, 2.0),
    ];
    for (tok, s, rd, rf, nt, ft) in cases {
        let inp = outright(tok, s, 1.0, 1e6, Side::Buy, rd, rf, nt)
            .with_far(ft)
            .unwrap();
        let prod = swap_points(&inp).unwrap();
        let orc = oracle::fx_swap_points(s, nt, ft, rd, rf);
        assert!(
            close(prod, orc, 1e-12, 1e-12),
            "{tok}: production swap points {prod} vs CIP identity {orc}"
        );
    }
}

/// STRUCTURAL: a swap with equal near/far dates and opposite legs nets to exactly
/// 0 (to_bits) — the two legs are identical forwards at opposite signs.
#[test]
fn swap_equal_dates_opposite_legs_net_to_zero() {
    let t = 0.5;
    let inp = outright("EURUSD", 1.2345, 1.30, 1e6, Side::Buy, 0.04, 0.01, t)
        .with_far(t)
        .unwrap();
    assert_eq!((swap::pv(&inp).unwrap() + 0.0).to_bits(), 0.0_f64.to_bits());
}

/// STRUCTURAL: swap points carry the sign of the net carry `b = r_dom − r_for`.
#[test]
fn swap_points_sign_matches_carry() {
    let pos = outright("EURUSD", 1.2345, 1.30, 1e6, Side::Buy, 0.05, 0.01, 0.25)
        .with_far(0.75)
        .unwrap();
    assert!(swap_points(&pos).unwrap() > 0.0);
    let neg = outright("EURUSD", 1.2345, 1.30, 1e6, Side::Buy, 0.01, 0.05, 0.25)
        .with_far(0.75)
        .unwrap();
    assert!(swap_points(&neg).unwrap() < 0.0);
}

// ===========================================================================
// Non-deliverable forward (NDF)
// ===========================================================================

/// HAND-DERIVED absolute PV literal, computed EXTERNALLY (not from the impl), full
/// derivation in-comment — an absolute anchor.
///
/// Terms: spot = 5.0000 (BRL per USD), K = 5.1000, r_dom = 0.1000 (BRL),
///        r_for = 0.0500 (USD), t = 0.5, notional = 1_000_000, side = Buy.
///   F  = spot · e^{(r_dom−r_for)·t} = 5.0 · e^{0.025} = 5.126575602622144
///   df = e^{−r_dom·t}             = e^{−0.05}        = 0.951229424500714
///   PV = +1 · 1_000_000 · 0.951229424500714 · (5.126575602622144 − 5.1000)
///      = 25_279.4951880221…
#[test]
fn ndf_pv_hand_derived_literal() {
    let inp = outright("USDBRL", 5.0, 5.1, 1_000_000.0, Side::Buy, 0.10, 0.05, 0.5);
    let ndf = Ndf::new(inp, FixingSource::BrlPtax);
    let expected = 25_279.495_188_022_105_f64;
    assert!(
        (ndf.pv() - expected).abs() <= 1e-6,
        "ndf pv {} vs hand-derived {expected}",
        ndf.pv()
    );
}

/// PRIMARY: the production NDF PV matches the INDEPENDENT two-bond DCF oracle
/// (settled in the convertible numeraire). The hand-derived literal above and this
/// route are independent of each other (one an absolute value, one a derivation),
/// so a shared slip cannot hide.
#[test]
fn ndf_pv_matches_independent_two_bond_route() {
    let cases = [
        (
            "USDBRL",
            5.0,
            5.1,
            1_000_000.0,
            Side::Buy,
            0.10,
            0.05,
            0.5,
            FixingSource::BrlPtax,
        ),
        (
            "USDINR",
            83.0,
            84.0,
            2_000_000.0,
            Side::Sell,
            0.066,
            0.05,
            1.0,
            FixingSource::InrRbiRef,
        ),
        (
            "USDCOP",
            4000.0,
            4050.0,
            5_000_000.0,
            Side::Buy,
            0.095,
            0.05,
            0.25,
            FixingSource::CopTrm,
        ),
    ];
    for (tok, s, k, n, side, rd, rf, t, fixing) in cases {
        let inp = outright(tok, s, k, n, side, rd, rf, t);
        let ndf = Ndf::new(inp, fixing);
        let orc = oracle::ndf_pv(sign(side), s, k, n, t, rd, rf);
        assert!(
            close(ndf.pv(), orc, 1e-9, 1e-6),
            "{tok}: production NDF {} vs independent two-bond oracle {orc}",
            ndf.pv()
        );
    }
}

/// STRUCTURAL IDENTITY (can disagree): an NDF and a deliverable outright forward
/// of equal terms have the SAME risk-neutral PV in the same numeraire —
/// non-deliverability is a settlement-mechanics difference, not a valuation one.
#[test]
fn ndf_pv_equals_deliverable_forward_pv() {
    let cases = [
        ("USDBRL", 5.0, 5.1, 1_000_000.0, Side::Buy, 0.10, 0.05, 0.5),
        (
            "USDINR",
            83.0,
            84.0,
            2_000_000.0,
            Side::Sell,
            0.066,
            0.05,
            1.0,
        ),
        (
            "USDKRW",
            1300.0,
            1280.0,
            2_000_000.0,
            Side::Sell,
            0.03,
            0.045,
            1.0,
        ),
    ];
    for (tok, s, k, n, side, rd, rf, t) in cases {
        let deliverable = outright(tok, s, k, n, side, rd, rf, t);
        let ndf = Ndf::new(deliverable, FixingSource::BrlPtax);
        assert_eq!(ndf.pv().to_bits(), forward::pv(&deliverable).to_bits());
    }
}

/// HONEST BOUNDARY: the fixing identity is metadata only and does NOT enter the
/// PV — the same terms under two different fixings price identically (the live
/// fixing VALUE is an estate-gated feed, never sourced here).
#[test]
fn ndf_fixing_identity_is_metadata_only() {
    let inp = outright("USDBRL", 5.0, 5.1, 1_000_000.0, Side::Buy, 0.10, 0.05, 0.5);
    let a = Ndf::new(inp, FixingSource::BrlPtax);
    let b = Ndf::new(inp, FixingSource::InrRbiRef);
    assert_eq!(a.pv().to_bits(), b.pv().to_bits());
    assert_eq!(a.fixing(), FixingSource::BrlPtax);
    assert_eq!(b.fixing(), FixingSource::InrRbiRef);
}
