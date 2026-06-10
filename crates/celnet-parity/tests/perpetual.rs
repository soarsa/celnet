//! Parity rows — the **perpetual (no-expiry) American vanilla** (proto arm 30,
//! `Instrument.product.perpetual_option`) priced by
//! [`celnet_exotics::perpetual_price`] on the agnostic carry seam reproduces
//! genuinely independent oracles.
//!
//! Four gates, none of which re-runs the engine as its own check (the
//! FRTB-0.75ρ circular-oracle lesson):
//!
//!   (i)   **Code-disjoint closed-form oracle**
//!         ([`celnet_golden::oracle::perpetual_american_price`]): the
//!         characteristic root is found by expanding-bracket **bisection of
//!         `ψ(y) = ½σ²·y·(y−1) + b·y − r` in its product form** and the value is
//!         completed with `libm::pow` — sharing neither the engine's
//!         standard-form discriminant + cancellation-free root pairing nor its
//!         `exp(y·ln x)` seam power route. `celnet-golden` does not depend on
//!         `celnet-exotics`' perpetual module, so the two routes share no code.
//!   (ii)  **Finite-maturity American-FD `T → ∞` sandwich**: the projected-SOR
//!         free-boundary finite difference ([`celnet_exotics::american_fd`] — a
//!         genuinely code-disjoint early-exercise pricer, PDE grid vs closed
//!         form) at `t = 50y` and `t = 100y` must bracket the perpetual from
//!         below, monotonically, and converge toward it: an American option with
//!         more time is worth more, and the perpetual is the supremum.
//!   (iii) **The carry/discount call law** (structural, model-free): at
//!         `b == r` **exactly** the perpetual call is never exercised and equals
//!         the spot **exactly** (`ψ(1) = 0` ⇒ `y₁ = 1`; the `T → ∞` limit of the
//!         same-terms European call) — asserted `to_bits` (the documented
//!         exact-arm carve-out from "no float `==`"), independently on the
//!         engine and on the golden oracle. For `b > r` **strictly** the call
//!         has NO finite value (stopping at `L` is worth `(L−K)(S/L)^{y₁}` with
//!         `y₁ < 1` → ∞; `e^{−rt}S_t` is a strict submartingale): the engine
//!         refuses with its typed error and the oracle refuses with `None` —
//!         the same financial law encoded twice, disjointly. Puts are
//!         unaffected (the `y₂` branch; payoff bounded by `K`).
//!   (iv)  **The European-dominance law** (model-free no-arbitrage): the
//!         perpetual American call dominates the same-terms European call at
//!         EVERY finite maturity (more rights, more time). This is the law the
//!         refuted `V = S` pin under `b > r` violated (at `r = 0.02`,
//!         `b = 0.025` the pinned 1.25 sat BELOW the 100y European's 1.9127 —
//!         an internal arbitrage), so it is locked permanently here on the
//!         `b < r` and `b == r` rows via [`celnet_vanilla::price`].

use celnet_core::assert_close;
use celnet_exotics::{
    AmericanGrid, AmericanOption, ExerciseStyle, PerpetualError, PerpetualInputs, american_fd,
    perpetual_exercise_boundary, perpetual_greeks, perpetual_price,
};
use celnet_golden::oracle::{self, Cp};
use celnet_types::{Carry, OptionType, VanillaInputs};

fn cp(opt: OptionType) -> Cp {
    match opt {
        OptionType::Call => Cp::Call,
        OptionType::Put => Cp::Put,
    }
}

/// (i) Engine vs the independent bisection-+-`libm::pow` oracle across
/// continuation, exercised and degenerate regimes — including the `σ → 0`
/// stiffness regime where `|b|/σ² → ∞` (the engine's cancellation-free pairing
/// is load-bearing there; the oracle's bisection is immune by construction, so
/// agreement kills a shared-route cancellation bug). Rows with `b > r` carry a
/// **refused** call (both routes must refuse — gate (iii)'s law) while their
/// puts still price and must agree.
#[test]
fn perpetual_engine_matches_independent_bisection_oracle() {
    let cases = [
        // (spot, strike, vol, r, b)
        (100.0, 100.0, 0.30, 0.08, 0.04),  // call/put continuation
        (100.0, 110.0, 0.25, 0.06, 0.02),  // ITM put continuation
        (1.30, 1.25, 0.10, 0.05, 0.01),    // FX-scale levels
        (80.0, 95.0, 0.40, 0.07, -0.03),   // negative carry
        (50.0, 45.0, 0.15, 0.03, 0.0),     // zero carry
        (100.0, 100.0, 1e-3, 0.08, 0.04),  // σ→0 regime: stable pairing matters
        (90.0, 100.0, 0.20, 0.0, 0.05),    // r = 0, b > ½σ²: call REFUSED (b > r)
        (100.0, 90.0, 0.20, 0.0, 0.01),    // r = 0, b ≤ ½σ²: call REFUSED, put = K
        (1.40, 1.10, 0.10, 0.05, -0.02),   // call beyond the boundary: intrinsic
        (100.0, 150.0, 0.10, 0.04, 0.035), // put beyond the boundary: intrinsic
    ];
    for &(s, k, vol, r, b) in &cases {
        let i = PerpetualInputs::new(s, k, vol, Carry::CostOfCarry { r, b });
        for opt in [OptionType::Call, OptionType::Put] {
            let engine = perpetual_price(opt, &i);
            let reference = oracle::perpetual_american_price(cp(opt), s, k, vol, r, b);
            if opt == OptionType::Call && b > r {
                // Both disjoint routes refuse: the b > r call has no finite value.
                assert_eq!(
                    engine,
                    Err(PerpetualError::CallCarryExceedsDiscount),
                    "engine must refuse the b > r call at r={r} b={b}"
                );
                assert_eq!(
                    reference, None,
                    "oracle must refuse the b > r call at r={r} b={b}"
                );
                continue;
            }
            let engine = engine.unwrap();
            let reference = reference.expect("priceable contract must have an oracle value");
            assert_close!(engine, reference, 1e-10, 1e-12);
        }
    }
}

/// (i) The FX two-rate carry arm maps onto the oracle's `(r, b)` exactly as the
/// seam defines it (`r = r_dom`, `b = r_dom − r_for`) — the wire shape every
/// arm-30 FX instrument prices through.
#[test]
fn perpetual_fx_carry_matches_oracle_two_rate_mapping() {
    let cases = [
        // (spot, strike, vol, r_dom, r_for)
        (1.30, 1.25, 0.10, 0.05, 0.01),
        (1.10, 1.15, 0.105, 0.03, 0.015),
        (100.0, 150.0, 0.10, 0.04, 0.005),
    ];
    for &(s, k, vol, r_dom, r_for) in &cases {
        let i = PerpetualInputs::new(s, k, vol, Carry::FxRates { r_dom, r_for });
        for opt in [OptionType::Call, OptionType::Put] {
            let engine = perpetual_price(opt, &i).unwrap();
            let reference =
                oracle::perpetual_american_price(cp(opt), s, k, vol, r_dom, r_dom - r_for)
                    .expect("r_for > 0 keeps b < r: priceable on both sides");
            assert_close!(engine, reference, 1e-10, 1e-12);
        }
    }
}

/// (ii) The American-FD `T → ∞` sandwich. At `S = K = 100`, `σ = 0.20`,
/// `r = 0.04`, `b = 0` (FX `r_dom = r_for`) the characteristic quadratic factors
/// exactly — `ψ(y) = 0.02·y² − 0.02·y − 0.04 = 0.02·(y − 2)·(y + 1)`, so
/// `y₁ = 2`, `y₂ = −1` — and the perpetual closed form is derived BY HAND (never
/// read back from the engine):
///
/// ```text
/// call: S*  = K·2/(2−1) = 200,  V = (200−100)·(100/200)²    = 25
/// put:  S** = K·(−1)/(−2) = 50, V = (100−50)·(100/50)^{−1}  = 25
/// ```
///
/// The PSOR free-boundary finite difference at `t = 50y` and `t = 100y` must
/// satisfy `FD(50) < FD(100) < perpetual` (more exercise time is worth strictly
/// more; the perpetual is the supremum) and genuinely converge. Measured on the
/// default grid: gaps `2.7e-1` (50y) → `1.7e-2` (100y), i.e. ~250× and ~17× the
/// grid error, so the strict inequalities cannot be satisfied by grid noise.
#[test]
fn american_fd_long_maturity_sandwich_brackets_perpetual() {
    let (s, k, vol, r) = (100.0, 100.0, 0.20, 0.04);
    let perp_in = PerpetualInputs::new(s, k, vol, Carry::FxRates { r_dom: r, r_for: r });
    for opt in [OptionType::Call, OptionType::Put] {
        let perp = perpetual_price(opt, &perp_in).unwrap();
        // The hand-derived factored value (independent of every pricer here).
        assert_close!(perp, 25.0, 1e-12, 1e-12);

        let fd = |t: f64| {
            let i = VanillaInputs::new(s, k, vol, t, r, r);
            american_fd(
                &(&i).into(),
                &AmericanOption {
                    option: opt,
                    strike: k,
                    style: ExerciseStyle::American,
                },
                AmericanGrid::default(),
            )
        };
        let fd50 = fd(50.0);
        let fd100 = fd(100.0);
        assert!(
            fd50 < fd100,
            "{opt:?}: American FD must be monotone in maturity: FD(50y) {fd50} >= FD(100y) {fd100}"
        );
        assert!(
            fd100 < perp,
            "{opt:?}: the perpetual is the supremum: FD(100y) {fd100} >= perpetual {perp}"
        );
        // Genuine convergence: the residual T-gap shrinks by far more than grid
        // noise (measured ratio ≈ 0.06; the 0.25 bound leaves 4× headroom while
        // still rejecting a stalled or wrong-limit solver).
        assert!(
            perp - fd100 < 0.25 * (perp - fd50),
            "{opt:?}: FD not converging to the perpetual: gaps {} (50y) vs {} (100y)",
            perp - fd50,
            perp - fd100
        );
        // And by 100y the FD value has essentially reached the perpetual
        // (measured 0.07% of the value; bound 0.2%).
        assert!(
            perp - fd100 < 2e-3 * perp,
            "{opt:?}: FD(100y) {fd100} too far below the perpetual {perp}"
        );
    }
}

/// (iii) The `b == r` exact law: the perpetual call is never exercised and
/// equals the spot EXACTLY — bitwise on the engine, bitwise-agreeing on the
/// independently coded golden oracle (the same financial law encoded twice,
/// disjointly), with an infinite exercise boundary.
#[test]
fn carry_equal_to_discount_perpetual_call_is_spot_exactly() {
    for &(r, b) in &[(0.05, 0.05), (0.0, 0.0), (0.123, 0.123)] {
        let i = PerpetualInputs::new(123.45, 100.0, 0.2, Carry::CostOfCarry { r, b });
        let engine = perpetual_price(OptionType::Call, &i).unwrap();
        assert_eq!(engine.to_bits(), 123.45f64.to_bits(), "r={r} b={b}");
        assert_eq!(
            oracle::perpetual_american_price(Cp::Call, 123.45, 100.0, 0.2, r, b)
                .expect("b == r is the finite V = S degenerate")
                .to_bits(),
            engine.to_bits(),
            "oracle disagrees on the b == r law at r={r} b={b}"
        );
        assert!(
            perpetual_exercise_boundary(OptionType::Call, &i)
                .unwrap()
                .is_infinite()
        );
    }
    // The FX form of the law: r_for = 0 ⇔ b = r_dom − r_for = r_dom exactly.
    let fx = PerpetualInputs::new(
        1.25,
        1.10,
        0.10,
        Carry::FxRates {
            r_dom: 0.02,
            r_for: 0.0,
        },
    );
    assert_eq!(
        perpetual_price(OptionType::Call, &fx).unwrap().to_bits(),
        1.25f64.to_bits()
    );
}

/// (iii) The `b > r` STRICT refusal law (the adversarial-verify refutation of
/// the old `V = S` pin): a perpetual call whose carry strictly exceeds the
/// discount rate has NO finite value — the engine refuses with the typed
/// [`PerpetualError::CallCarryExceedsDiscount`] (price, boundary AND Greek
/// strip), the code-disjoint golden oracle refuses with `None`, and the puts
/// on the very same carries are UNAFFECTED (finite, within intrinsic ≤ V ≤ K).
#[test]
fn carry_exceeding_discount_perpetual_call_is_refused_on_both_routes() {
    for &(r, b) in &[(0.03, 0.06), (0.02, 0.025), (0.0, 0.05)] {
        let i = PerpetualInputs::new(123.45, 100.0, 0.2, Carry::CostOfCarry { r, b });
        assert_eq!(
            perpetual_price(OptionType::Call, &i),
            Err(PerpetualError::CallCarryExceedsDiscount),
            "engine price must refuse at r={r} b={b}"
        );
        assert_eq!(
            perpetual_exercise_boundary(OptionType::Call, &i),
            Err(PerpetualError::CallCarryExceedsDiscount),
            "engine boundary must refuse at r={r} b={b}"
        );
        assert_eq!(
            perpetual_greeks(OptionType::Call, &i),
            Err(PerpetualError::CallCarryExceedsDiscount),
            "engine greeks must refuse at r={r} b={b}"
        );
        assert_eq!(
            oracle::perpetual_american_price(Cp::Call, 123.45, 100.0, 0.2, r, b),
            None,
            "oracle must refuse at r={r} b={b}"
        );
        // Puts price on the y₂ branch for every carry — finite and bounded.
        let put = perpetual_price(OptionType::Put, &i).unwrap();
        let put_ref = oracle::perpetual_american_price(Cp::Put, 123.45, 100.0, 0.2, r, b)
            .expect("the put is unaffected by the call's divergence");
        assert!(
            (0.0..=100.0).contains(&put),
            "put out of [0, K] at r={r} b={b}"
        );
        assert_close!(put, put_ref, 1e-10, 1e-12);
    }
    // The FX form of the law: r_for < 0 ⇔ b = r_dom − r_for > r_dom — the
    // refuted golden-vector shape (spot 1.25, strike 1.10, r_dom 0.02,
    // r_for −0.005 ⇒ b = 0.025 > r): refused, never the old 1.25 pin.
    let fx = PerpetualInputs::new(
        1.25,
        1.10,
        0.10,
        Carry::FxRates {
            r_dom: 0.02,
            r_for: -0.005,
        },
    );
    assert_eq!(
        perpetual_price(OptionType::Call, &fx),
        Err(PerpetualError::CallCarryExceedsDiscount)
    );
    assert!(perpetual_price(OptionType::Put, &fx).unwrap().is_finite());
}

/// (iii) The sub-ulp edge of the call law: with `b` strictly below `r` by
/// 1..64 ulps the finite-precision characteristic root collapses to exactly
/// `y₁ = 1.0` on BOTH disjoint routes (the engine's cancellation-free pairing
/// and the oracle's product-form bisection), and both must take the exact
/// `y₁ → 1⁺` limit arm — finite, no-arbitrage-sandwiched, on the spot limit —
/// never the `1/0` boundary whose value is `∞·0 = NaN` (adversarial-verify
/// regression: both unguarded routes returned NaN at `b = r − 1 ulp`).
#[test]
fn sub_ulp_carry_window_is_finite_on_both_routes() {
    let (s, k, vol, r) = (100.0, 100.0, 0.2, 0.05f64);
    for ulps in 1..=64u64 {
        let b = f64::from_bits(r.to_bits() - ulps);
        assert!(b < r, "scan must stay strictly inside b < r");
        let i = PerpetualInputs::new(s, k, vol, Carry::CostOfCarry { r, b });
        let engine = perpetual_price(OptionType::Call, &i)
            .expect("b < r strictly is inside the priced domain");
        let reference = oracle::perpetual_american_price(Cp::Call, s, k, vol, r, b)
            .expect("b < r strictly is inside the oracle's domain");
        assert!(
            engine.is_finite() && reference.is_finite(),
            "non-finite at b = r - {ulps} ulps: engine {engine}, oracle {reference}"
        );
        // Both sit on the y₁ → 1⁺ spot limit (measured collapse error ≤ 3e-11
        // across the window) and inside the no-arbitrage sandwich.
        assert!((0.0..=s).contains(&engine));
        assert_close!(engine, s, 1e-9, 1e-9);
        assert_close!(reference, s, 1e-9, 1e-9);
        assert_close!(engine, reference, 1e-9, 1e-9);
    }
}

/// (iv) The European-dominance law, locked permanently: a perpetual American
/// call may be exercised at ANY time, so its value dominates the same-terms
/// European call at EVERY finite maturity (perpetual ≥ American(T) ≥
/// European(T)). The European reference is [`celnet_vanilla::price`] — a fully
/// independent pricer (Φ-based closed form, no characteristic root) — under
/// the carry bijection `r_dom = r`, `r_for = r − b`. This is exactly the law
/// the refuted `b ≥ r ⇒ V = S` pin violated: at `r = 0.02`, `b = 0.025` the
/// pinned spot 1.25 sat BELOW the same-terms 100y European (1.9127), an
/// internal arbitrage — under the corrected law that region refuses instead,
/// and every priced region must satisfy dominance.
#[test]
fn perpetual_call_dominates_every_finite_maturity_european() {
    let (s, k, vol) = (123.45, 100.0, 0.2);
    let rows = [
        // b < r rows (continuation closed form)…
        (0.05, 0.02),
        (0.08, 0.04),
        // …and b == r rows (the exact V = S degenerate).
        (0.05, 0.05),
        (0.0, 0.0),
    ];
    for &(r, b) in &rows {
        let perp = perpetual_price(
            OptionType::Call,
            &PerpetualInputs::new(s, k, vol, Carry::CostOfCarry { r, b }),
        )
        .unwrap();
        for t in [1.0, 10.0, 50.0, 100.0] {
            // Carry bijection: b = r_dom − r_for ⇒ r_for = r − b.
            let euro = celnet_vanilla::price(
                OptionType::Call,
                &VanillaInputs::new(s, k, vol, t, r, r - b),
            );
            assert!(
                perp >= euro,
                "dominance violated at r={r} b={b} T={t}y: perpetual {perp} < European {euro}"
            );
        }
    }
}
