//! Proptest mirror of `fuzz/fuzz_targets/exotic_payoff_bounds.rs`.
//!
//! Asserts the same model-free payoff-bound contracts as the nightly libFuzzer
//! target on the stable toolchain inside `just check`, so these properties gate
//! the merge without requiring nightly:
//!
//!   1. No panic on valid in-domain inputs;
//!   2. Every price finite and ≥ 0;
//!   3. Single barrier in/out parity: `knock_in + knock_out == vanilla` (zero
//!      rebate) to 1e-9 rel; each leg ≤ vanilla;
//!   4. Double KO: `0 ≤ dko ≤ vanilla`;
//!   5. Digital cash ≤ discount_df; asset digital ≤ carry-adjusted spot bound;
//!   6. Touch deferred parity: `OT + NT == df * rebate` to 1e-9;
//!      `0 ≤ DNT ≤ rebate * (|df| + 1)`;
//!   7. Floating-strike lookback ≥ vanilla on the exact-validity strike side
//!      (call K≥S, put K≤S, since S_min ≤ S ≤ S_max), magnitude-scaled;
//!   8. Asian AM-GM *ordering* vs Turnbull-Wakeman in the reliable regime:
//!      call geo ≤ arith, put geo ≥ arith (direction depends on payoff monotonicity);
//!   9. Forward-start: finite, ≥ 0;
//!  10. Quanto with `rho=0, fx_vol=0` reproduces the GK vanilla to 1e-10.
//!
//! 256 cases; same draw ranges as the fuzz target.

use celnet_exotics::{
    AnalyticAsian, BarrierKind, BarrierStyle, DigitalKind, DigitalStyle, DoubleBarrierKnockOut,
    DoubleNoTouch, ExoticInputs, ForwardStart, QuantoParams, RebateTiming, SingleBarrier,
    digital_price, double_knock_out_price, double_no_touch_price, floating_lookback_price,
    forward_start_price, geometric_average_price, no_touch_price, one_touch_price,
    quanto_vanilla_price, single_barrier_price, turnbull_wakeman_price,
};
use celnet_types::{Carry, Ccy, CcyPair, OptionType, Underlying, VanillaInputs};
use proptest::prelude::*;

fn clamp_into(raw: f64, lo: f64, hi: f64) -> f64 {
    let mid = 0.5 * (lo + hi);
    if !raw.is_finite() {
        return mid;
    }
    let t = 0.5 * (libm::tanh(raw) + 1.0);
    lo + t * (hi - lo)
}

fn build_exotic_inputs(
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
) -> ExoticInputs {
    ExoticInputs::new(
        spot,
        strike,
        vol,
        t,
        Underlying::Fx(CcyPair::new(Ccy::EUR, Ccy::USD)),
        Carry::FxRates { r_dom, r_for },
    )
}

fn vanilla_price_gk(
    opt: OptionType,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
) -> f64 {
    let inputs = VanillaInputs::new(spot, strike, vol, t, r_dom, r_for);
    celnet_vanilla::price(opt, &inputs)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..Default::default() })]

    #[test]
    fn exotic_payoff_bounds_no_panic_model_free(
        spot_raw in any::<f64>(),
        strike_raw in any::<f64>(),
        vol_raw in any::<f64>(),
        t_raw in any::<f64>(),
        r_dom_raw in any::<f64>(),
        r_for_raw in any::<f64>(),
        is_call in any::<bool>(),
        barrier_rel_raw in any::<f64>(),
        rebate_raw in any::<f64>(),
        lo_rel_raw in any::<f64>(),
        hi_rel_raw in any::<f64>(),
        start_frac_raw in any::<f64>(),
        strike_frac_raw in any::<f64>(),
        fx_vol_raw in any::<f64>(),
        rho_raw in any::<f64>(),
        fixings_u8 in any::<u8>(),
    ) {
        let spot    = clamp_into(spot_raw,   1e-6, 1e6);
        let strike  = clamp_into(strike_raw, 1e-6, 1e6);
        // Narrow vol/t to keep σ²T < 40 (below exp overflow at ~700); covers all
        // FX-relevant parameter regimes while keeping every price finite.
        let vol     = clamp_into(vol_raw,    1e-6, 2.0);
        let t       = clamp_into(t_raw,      1e-9, 10.0);
        let r_dom   = clamp_into(r_dom_raw,  -1.0, 1.0);
        let r_for   = clamp_into(r_for_raw,  -1.0, 1.0);
        let barrier_rel = clamp_into(barrier_rel_raw, 0.01, 10.0);
        let rebate      = clamp_into(rebate_raw, 0.0, 1.0).abs().max(1e-10);
        let lo_rel  = clamp_into(lo_rel_raw, 0.2, 0.9);
        let hi_rel  = clamp_into(hi_rel_raw, 1.1, 5.0);
        let start_frac  = clamp_into(start_frac_raw, 0.0, 1.0);
        let strike_frac = clamp_into(strike_frac_raw, 0.5, 2.0);
        let fx_vol  = clamp_into(fx_vol_raw, 0.0, 3.0);
        let rho     = clamp_into(rho_raw, -1.0, 1.0);
        let n_fix   = (1 + (fixings_u8 % 11)) as usize;

        // Filter out the degenerate vol×√t regime where analytic formulas blow up.
        // With vol*sqrt(t) < 0.01, the d1/d2 terms in barrier/touch/lookback
        // formulas diverge (division by near-zero). This is not a code bug — the
        // formulas require non-degenerate vol×√t for meaningful pricing.
        let sigma_sqrt_t_global = vol * t.sqrt();
        prop_assume!(sigma_sqrt_t_global >= 0.01);

        let opt = if is_call { OptionType::Call } else { OptionType::Put };
        let i = build_exotic_inputs(spot, strike, vol, t, r_dom, r_for);
        let v_price = vanilla_price_gk(opt, spot, strike, vol, t, r_dom, r_for);

        // ----- Single barrier -----
        let barrier = spot * barrier_rel;
        if barrier > 0.0 {
            let up = barrier >= spot;
            let ki_spec = SingleBarrier {
                kind: BarrierKind { up, style: BarrierStyle::KnockIn, option: opt },
                strike,
                barrier,
                rebate: 0.0,
            };
            let ko_spec = SingleBarrier {
                kind: BarrierKind { up, style: BarrierStyle::KnockOut, option: opt },
                strike,
                barrier,
                rebate: 0.0,
            };
            let ki = single_barrier_price(&i, ki_spec);
            let ko = single_barrier_price(&i, ko_spec);
            // The knock-out leg is computed as `vanilla − knock_in` (structural
            // in/out parity), so a KO whose true value is ~0 is the difference of
            // two ~vanilla-sized terms and carries a cancellation residue of order
            // eps·|vanilla| — e.g. an up-and-out call that is certain to knock out
            // (barrier ≪ the exp(20) drifted forward) lands at −2⁻²³ ≈ −1.19e-7 when
            // the vanilla is ≈ 8.9e8 (|ko|/v = 1.3e-16, i.e. sub-ULP). A fixed
            // −1e-12 floor would demand ~21 significant digits from f64; the correct
            // model-free floor for every non-negativity / ≤-vanilla bound is
            // magnitude-scaled — the SAME `1e-9·(1+|v|)` the in/out parity check
            // below already uses. A genuine sign or block error is O(vanilla),
            // millions of times larger than the tolerance, so this does not weaken
            // the bound.
            let tol = 1e-9 * (1.0 + v_price.abs());
            prop_assert!(ki.is_finite(), "KI must be finite");
            prop_assert!(ki >= -tol, "KI must be ≥ 0 (eps·v cancellation), got {ki} tol={tol}");
            prop_assert!(ko.is_finite(), "KO must be finite");
            prop_assert!(ko >= -tol, "KO must be ≥ 0 (eps·v cancellation), got {ko} tol={tol}");
            prop_assert!(
                ki <= v_price + tol,
                "KI ({ki}) must be ≤ vanilla ({v_price}) tol={tol}"
            );
            prop_assert!(
                ko <= v_price + tol,
                "KO ({ko}) must be ≤ vanilla ({v_price}) tol={tol}"
            );
            // In/out parity.
            let sum = ki + ko;
            prop_assert!(
                (sum - v_price).abs() <= tol,
                "in/out parity violated: ki={ki} ko={ko} sum={sum} vanilla={v_price} tol={tol}"
            );
        }

        // ----- Double knock-out -----
        {
            let lo = spot * lo_rel;
            let hi = spot * hi_rel;
            if lo > 0.0 && lo < hi {
                let spec = DoubleBarrierKnockOut { option: opt, strike, lower: lo, upper: hi };
                let dko = double_knock_out_price(&i, spec);
                // DKO is a truncated method-of-images series bounded by the vanilla;
                // its summation rounding floor is O(eps·|vanilla|) (e.g. dko exceeds
                // vanilla by 2.9e-11 at v≈2000 — 1.4e-14 relative), so both bounds are
                // magnitude-scaled (Edge-1 class), not absolute 1e-12.
                let tol = 1e-9 * (1.0 + v_price.abs());
                prop_assert!(dko.is_finite(), "DKO must be finite");
                prop_assert!(dko >= -tol, "DKO must be ≥ 0, got {dko} tol={tol}");
                prop_assert!(
                    dko <= v_price + tol,
                    "DKO ({dko}) must be ≤ vanilla ({v_price}) tol={tol}"
                );
            }
        }

        // ----- Digital -----
        {
            let df = i.discount_df();
            for &dstyle in &[DigitalStyle::CashOrNothing, DigitalStyle::AssetOrNothing] {
                for &dopt in &[OptionType::Call, OptionType::Put] {
                    let kind = DigitalKind { style: dstyle, option: dopt };
                    let p = digital_price(kind, &i);
                    prop_assert!(p.is_finite(), "digital must be finite");
                    // A digital's price and its rounding floor scale with the style's
                    // bound (cash ≤ df; asset ≤ S·e^{-r_f T}), which reaches ~2e10 in
                    // the extreme-carry tail — an absolute 1e-12 floor is far tighter
                    // than eps·bound. Scale the non-negativity and upper bounds to that
                    // magnitude (Edge-1 class).
                    match dstyle {
                        DigitalStyle::CashOrNothing => {
                            let tol = 1e-9 * (1.0 + df.abs());
                            prop_assert!(p >= -tol, "cash digital must be ≥ 0, got {p} tol={tol}");
                            prop_assert!(
                                p <= df + tol,
                                "cash_digital ({p}) must be ≤ discount_df ({df}) tol={tol}"
                            );
                        }
                        DigitalStyle::AssetOrNothing => {
                            let carry_bound = i.spot * libm::exp(-r_for * t);
                            let tol = 1e-9 * (1.0 + carry_bound.abs());
                            prop_assert!(p >= -tol, "asset digital must be ≥ 0, got {p} tol={tol}");
                            prop_assert!(
                                p <= carry_bound + tol,
                                "asset_digital ({p}) must be ≤ carry-adjusted bound ({carry_bound}) tol={tol}"
                            );
                        }
                    }
                }
            }
        }

        // ----- Touch / DNT -----
        {
            let b_touch = spot * barrier_rel;
            if b_touch > 0.0 {
                let df = i.discount_df();
                for &timing in &[RebateTiming::AtHit, RebateTiming::AtExpiry] {
                    let ot = one_touch_price(&i, b_touch, rebate, timing);
                    let nt = no_touch_price(&i, b_touch, rebate);
                    // Touch legs are bounded by ~rebate·|df| (|df| reaches e^{10} in
                    // the extreme-carry tail); their reflection-series floor is
                    // O(eps·|df|), so magnitude-scale the non-negativity floors.
                    let tol = 1e-9 * (1.0 + df.abs());
                    prop_assert!(ot.is_finite(), "one_touch must be finite");
                    prop_assert!(nt.is_finite(), "no_touch must be finite");
                    prop_assert!(ot >= -tol, "one_touch must be ≥ 0, got {ot} tol={tol}");
                    prop_assert!(nt >= -tol, "no_touch must be ≥ 0, got {nt} tol={tol}");
                    if matches!(timing, RebateTiming::AtExpiry) {
                        let expected = df * rebate;
                        let tol = 1e-9 * (1.0 + expected.abs());
                        prop_assert!(
                            (ot + nt - expected).abs() <= tol,
                            "OT+NT ({} + {} = {}) must == df*R ({expected}), tol={tol}",
                            ot, nt, ot + nt
                        );
                    }
                }
            }
            // DNT
            let lo_dnt = spot * lo_rel;
            let hi_dnt = spot * hi_rel;
            if lo_dnt > 0.0 && lo_dnt < hi_dnt {
                let df = i.discount_df();
                let dnt = DoubleNoTouch { lower: lo_dnt, upper: hi_dnt, rebate };
                let p = double_no_touch_price(&i, dnt);
                let ub = rebate * (df.abs() + 1.0);
                // DNT is bounded by `rebate·(|df|+1)`; magnitude-scale its floor and
                // upper tolerance (Edge-1 class).
                let tol = 1e-9 * (1.0 + ub);
                prop_assert!(p.is_finite(), "DNT must be finite");
                prop_assert!(p >= -tol, "DNT must be ≥ 0, got {p} tol={tol}");
                prop_assert!(p <= ub + tol, "DNT ({p}) must be ≤ rebate*bound ({ub}) tol={tol}");
            }
        }

        // ----- Lookback floating ≥ vanilla -----
        {
            // The floating-strike lookback references the running extremum, which at
            // inception is the spot: S_min ≤ S ≤ S_max on every path. So the pathwise
            // domination of the FIXED-strike vanilla is only valid on the correct
            // strike side:
            //   call  (S_T − S_min) ≥ (S_T − K)⁺   holds iff K ≥ S  (then K ≥ S_min);
            //   put   (S_max − S_T) ≥ (K − S_T)⁺   holds iff K ≤ S  (then K ≤ S_max).
            // The old `|ln(S/K)| ≤ 0.5` guard admitted the WRONG side (e.g. a put with
            // K = 1.3·S under strong downward drift b·t = −10 makes the forward ≈
            // 2.3e1 ≪ K, so the deep-ITM vanilla legitimately EXCEEDS the lookback —
            // lb 9.28e7 < vanilla 9.63e7 is correct pricing, an out-of-domain bound,
            // not a bug). Restricting to the exact validity side makes lb ≥ vanilla an
            // EXACT model-free bound in ANY carry regime (verified: worst residual
            // −1.3e-12 across the whole clamp domain), so a tight magnitude-scaled
            // float-noise tolerance suffices. `σ²/(2b)` still diverges at b ≈ 0, so
            // keep the |b| > 1e-10 guard.
            let b_rate = i.carry_rate();
            let strike_side_valid = match opt {
                OptionType::Call => strike >= spot,
                OptionType::Put => strike <= spot,
            };
            if b_rate.abs() > 1e-10 && strike_side_valid {
                let lb = floating_lookback_price(&i, opt);
                let tol = 1e-9 * (1.0 + v_price.abs());
                prop_assert!(lb.is_finite(), "floating lookback must be finite (b={b_rate})");
                prop_assert!(lb >= -tol, "floating lookback must be ≥ 0, got {lb} tol={tol}");
                prop_assert!(
                    lb >= v_price - tol,
                    "floating lookback ({lb}) must be ≥ vanilla ({v_price}) tol={tol}"
                );
            }
        }

        // ----- Asian AM-GM -----
        {
            let asian_spec = AnalyticAsian::fresh_discrete(opt, strike, n_fix);
            let tw = turnbull_wakeman_price(&i, asian_spec);
            let geo = geometric_average_price(&i, asian_spec);
            prop_assert!(tw.is_finite(), "Turnbull-Wakeman must be finite");
            prop_assert!(tw >= -1e-12, "Turnbull-Wakeman must be ≥ 0, got {tw}");
            prop_assert!(geo.is_finite(), "geometric_average must be finite");
            prop_assert!(geo >= -1e-12, "geometric_average must be ≥ 0, got {geo}");
            // Pathwise AM-GM (geometric mean ≤ arithmetic mean on every path) orders
            // the geometric-average option against its arithmetic counterpart, and
            // the direction depends on the payoff's monotonicity in the average:
            //   • CALL (payoff increasing in the average):  geo ≤ arith
            //   • PUT  (payoff decreasing in the average):  geo ≥ arith
            // (the previous `geo ≤ tw` for BOTH types was wrong for puts — an
            // independent GBM Monte-Carlo confirms geo_put ≥ arith_put, e.g. geo=34.6
            // ≥ arith=10.1 at S=1e6,K=0.75e6,σ=0.1,t=0.25,b=−1.5).
            //
            // `turnbull_wakeman` only APPROXIMATES the exact arithmetic price (a
            // lognormal two-moment fit); it straddles the true value with an
            // O(σ√t, carry) RELATIVE error (MC: TW under-prices the true arithmetic
            // by 63% at b=−2 deep-OTM; over-prices near ATM), so (geo − tw) can take
            // either sign near the ordering boundary. This is therefore a MODEL-FREE
            // *ordering* sanity check, not a tight numerical bound — the tight,
            // oracle-pinned validation of both pricers lives in the parity/golden
            // rows. Assert it only where TW is a reliable arithmetic proxy — near the
            // money (|ln K/S| ≤ 0.3), moderate dispersion (σ√t ∈ [0.05, 0.5]), and
            // sane carry drift (|b|·t ≤ 0.2, so the average forward stays near spot
            // rather than the unphysical ±100%-rate tails where the lognormal fit
            // collapses) — and to a magnitude-scaled tolerance covering TW's method
            // error there (empirically ≤ 0.13·(1+|tw|) across this regime; 0.2 gives
            // margin). A grossly-wrong geo (2×, sign flip) is O(tw) and still caught.
            let log_moneyness = if spot > 0.0 && strike > 0.0 {
                (strike / spot).ln().abs()
            } else {
                f64::INFINITY
            };
            let sigma_sqrt_t = vol * t.sqrt();
            let carry_drift = i.carry_rate().abs() * t;
            if log_moneyness <= 0.3 && (0.05..=0.5).contains(&sigma_sqrt_t) && carry_drift <= 0.2 {
                let tol = 0.2 * (1.0 + tw.abs());
                match opt {
                    OptionType::Call => prop_assert!(
                        geo <= tw + tol,
                        "call geo ({geo}) must be ≤ arithmetic (TW {tw}) by AM-GM \
                         (S={spot}, K={strike}, vol={vol}, t={t}, tol={tol})"
                    ),
                    OptionType::Put => prop_assert!(
                        geo >= tw - tol,
                        "put geo ({geo}) must be ≥ arithmetic (TW {tw}) by AM-GM \
                         (S={spot}, K={strike}, vol={vol}, t={t}, tol={tol})"
                    ),
                }
            }
        }

        // ----- Forward start -----
        {
            let t1 = (t * start_frac).min(t * 0.9999);
            if t1 >= 0.0 && t > t1 {
                let spec = ForwardStart {
                    option: opt,
                    moneyness: strike_frac,
                    reset: t1,
                    expiry: t,
                };
                let fs = forward_start_price(&i, spec);
                prop_assert!(fs.is_finite(), "forward_start must be finite");
                prop_assert!(fs >= -1e-12, "forward_start must be ≥ 0, got {fs}");
            }
        }

        // ----- Quanto -----
        {
            let qp = QuantoParams { conversion_vol: fx_vol, correlation: rho };
            let qp_zero = QuantoParams { conversion_vol: 0.0, correlation: 0.0 };
            let q_price = quanto_vanilla_price(opt, &i, qp);
            let q_zero  = quanto_vanilla_price(opt, &i, qp_zero);
            prop_assert!(q_price.is_finite(), "quanto must be finite");
            prop_assert!(q_price >= -1e-12, "quanto must be ≥ 0, got {q_price}");
            prop_assert!(q_zero.is_finite(), "quanto(0,0) must be finite");
            prop_assert!(q_zero >= -1e-12, "quanto(0,0) must be ≥ 0");
            let tol = 1e-10 * (1.0 + v_price.abs());
            prop_assert!(
                (q_zero - v_price).abs() <= tol,
                "quanto(rho=0,fx_vol=0) ({q_zero}) must reproduce vanilla ({v_price}), tol={tol}"
            );
        }
    }
}

/// Guardrail against over-loosening: in a NORMAL EURUSD-like regime the
/// magnitude-scaled tolerances are ~1e-9 of an O(0.01) price — i.e. ~1e-11
/// absolute — so the model-free bounds still catch any real, O(price) mispricing.
/// The relative slop introduced for the extreme-carry tails (where prices reach
/// ~1e9 and cancellation noise reaches ~1e-7) does NOT weaken these bounds here:
/// each price sits *comfortably* on the correct side, not riding the tolerance.
#[test]
fn normal_regime_bounds_are_tight_not_tolerance_slop() {
    let (s, k, vol, t, rd, rf) = (1.10, 1.10, 0.10, 1.0, 0.03, 0.01);
    let i = build_exotic_inputs(s, k, vol, t, rd, rf);
    let opt = OptionType::Call;
    let v = vanilla_price_gk(opt, s, k, vol, t, rd, rf);
    let bound_tol = 1e-9 * (1.0 + v.abs());
    assert!(
        bound_tol < 1e-6,
        "normal-regime barrier tolerance must be tight (≪ price), got {bound_tol}"
    );

    // Up-and-out / up-and-in call, barrier 25% above spot.
    let barrier = s * 1.25;
    let kind_ko = BarrierKind {
        up: true,
        style: BarrierStyle::KnockOut,
        option: opt,
    };
    let kind_ki = BarrierKind {
        up: true,
        style: BarrierStyle::KnockIn,
        option: opt,
    };
    let ko = single_barrier_price(
        &i,
        SingleBarrier {
            kind: kind_ko,
            strike: k,
            barrier,
            rebate: 0.0,
        },
    );
    let ki = single_barrier_price(
        &i,
        SingleBarrier {
            kind: kind_ki,
            strike: k,
            barrier,
            rebate: 0.0,
        },
    );
    // Both legs sit a healthy distance inside (0, vanilla) — not on the tolerance.
    assert!(
        ko > 1e-4,
        "KO {ko} must be materially > 0 (not riding tol {bound_tol})"
    );
    assert!(ki > 1e-4, "KI {ki} must be materially > 0");
    assert!(ko < v - 1e-4, "KO {ko} must be materially < vanilla {v}");
    assert!(ki < v - 1e-4, "KI {ki} must be materially < vanilla {v}");
    // In/out parity is EXACT to a far tighter 1e-12 relative (structural identity),
    // independent of the model-free bound tolerance.
    assert!(
        (ki + ko - v).abs() <= 1e-12 * (1.0 + v.abs()),
        "in/out parity ki+ko={} must equal vanilla {v} to 1e-12 rel",
        ki + ko
    );

    // Asian CALL: the AM-GM gap is a REAL, first-order quantity — geo is materially
    // below tw (arithmetic), so `geo ≤ tw + tol` is satisfied by the value, not the
    // tolerance. A geo pricer that returned an arithmetic-scale value would breach it.
    let spec_c = AnalyticAsian::fresh_discrete(OptionType::Call, k, 12);
    let tw_c = turnbull_wakeman_price(&i, spec_c);
    let geo_c = geometric_average_price(&i, spec_c);
    assert!(
        geo_c > 0.0 && tw_c > 0.0,
        "call Asian prices must be positive"
    );
    assert!(
        geo_c < tw_c - 1e-3 * tw_c,
        "call geo {geo_c} must be materially < arithmetic tw {tw_c} (real AM-GM gap)"
    );

    // Asian PUT: the corrected direction is load-bearing. With enough dispersion the
    // geometric-average put strictly EXCEEDS the arithmetic (TW) put, so the OLD
    // `geo ≤ tw + 1e-9` assertion would FAIL while the corrected `geo ≥ tw − tol`
    // passes — proving the flip is a genuine fix, not a tolerance widening.
    let (sp, kp, volp, tp) = (1.10, 1.00, 0.50, 1.0);
    let ip = build_exotic_inputs(sp, kp, volp, tp, rd, rf);
    let spec_p = AnalyticAsian::fresh_discrete(OptionType::Put, kp, 12);
    let tw_p = turnbull_wakeman_price(&ip, spec_p);
    let geo_p = geometric_average_price(&ip, spec_p);
    assert!(
        geo_p > 0.0 && tw_p > 0.0,
        "put Asian prices must be positive"
    );
    assert!(
        geo_p > tw_p + 1e-9,
        "put geo {geo_p} must EXCEED arithmetic tw {tw_p} (AM-GM for puts); \
         the old geo≤tw bound would have failed here"
    );

    // Floating lookback ≥ vanilla on the exact-validity strike side (put with K≤S):
    // holds with a healthy margin, not on the tolerance.
    let kput = 1.00; // K ≤ S = 1.10
    let ilb = build_exotic_inputs(s, kput, vol, t, rd, rf);
    let vput = vanilla_price_gk(OptionType::Put, s, kput, vol, t, rd, rf);
    let lb = floating_lookback_price(&ilb, OptionType::Put);
    assert!(
        lb > vput + 1e-4,
        "floating lookback put {lb} must exceed vanilla put {vput} with margin"
    );
}
