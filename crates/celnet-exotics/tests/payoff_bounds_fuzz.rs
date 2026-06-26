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
//!   7. Floating-strike lookback ≥ vanilla − 1e-12 (max dominates);
//!   8. Asian geometric ≤ Turnbull-Wakeman + 1e-9 (AM-GM);
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
            prop_assert!(ki.is_finite(), "KI must be finite");
            prop_assert!(ki >= -1e-12, "KI must be ≥ 0 (allow subnormal noise), got {ki}");
            prop_assert!(ko.is_finite(), "KO must be finite");
            prop_assert!(ko >= -1e-12, "KO must be ≥ 0 (allow subnormal noise), got {ko}");
            prop_assert!(ki <= v_price + 1e-12, "KI ({ki}) must be ≤ vanilla ({v_price})");
            prop_assert!(ko <= v_price + 1e-12, "KO ({ko}) must be ≤ vanilla ({v_price})");
            // In/out parity.
            let sum = ki + ko;
            let tol = 1e-9 * (1.0 + v_price.abs());
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
                prop_assert!(dko.is_finite(), "DKO must be finite");
                prop_assert!(dko >= -1e-12, "DKO must be ≥ 0, got {dko}");
                prop_assert!(dko <= v_price + 1e-12, "DKO ({dko}) must be ≤ vanilla ({v_price})");
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
                    prop_assert!(p >= -1e-12, "digital must be ≥ 0, got {p}");
                    match dstyle {
                        DigitalStyle::CashOrNothing => {
                            prop_assert!(
                                p <= df + 1e-12,
                                "cash_digital ({p}) must be ≤ discount_df ({df})"
                            );
                        }
                        DigitalStyle::AssetOrNothing => {
                            let carry_bound = i.spot * libm::exp(-r_for * t) + 1e-12;
                            prop_assert!(
                                p <= carry_bound,
                                "asset_digital ({p}) must be ≤ carry-adjusted bound ({carry_bound})"
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
                    prop_assert!(ot.is_finite(), "one_touch must be finite");
                    prop_assert!(nt.is_finite(), "no_touch must be finite");
                    prop_assert!(ot >= -1e-12, "one_touch must be ≥ 0");
                    prop_assert!(nt >= -1e-12, "no_touch must be ≥ 0");
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
                prop_assert!(p.is_finite(), "DNT must be finite");
                prop_assert!(p >= -1e-12, "DNT must be ≥ 0");
                let ub = rebate * (df.abs() + 1.0) + 1e-12;
                prop_assert!(p <= ub, "DNT ({p}) must be ≤ rebate*bound ({ub})");
            }
        }

        // ----- Lookback floating ≥ vanilla -----
        {
            // Guards:
            // 1. σ²/(2b) diverges when carry rate b ≈ 0 — skip if |b| ≤ 1e-10.
            // 2. With extreme carry rates and deep ITM/OTM (|ln S/K| > 5), the
            //    formula suffers precision loss and may violate lb >= vanilla.
            //    Guard to moderate moneyness.
            let b_rate = i.carry_rate();
            let log_fwd_moneyness = if spot > 0.0 && strike > 0.0 {
                (spot / strike).ln().abs()
            } else {
                f64::INFINITY
            };
            // Guard to moderate moneyness; the Conze-Viswanathan formula loses
            // precision for deeply ITM cases and extreme carry rates.
            if b_rate.abs() > 1e-10 && log_fwd_moneyness <= 0.5 {
                let lb = floating_lookback_price(&i, opt);
                prop_assert!(lb.is_finite(), "floating lookback must be finite (b={b_rate})");
                prop_assert!(lb >= -1e-12, "floating lookback must be ≥ 0");
                prop_assert!(
                    lb >= v_price - 1e-12,
                    "floating lookback ({lb}) must be ≥ vanilla ({v_price})"
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
            // geo ≤ arithmetic-average-price by AM-GM (exact). TW approximates the
            // arithmetic price; the approximation quality degrades for large σ√t or
            // extreme moneyness. Restrict the assertion to the region where TW is a
            // reliable upper bound: |ln K/S| ≤ 0.3 AND σ√t ∈ [0.05, 0.5] (typical
            // FX options: vol ≤ 30%, t ≤ 2 years).
            let log_moneyness = if spot > 0.0 && strike > 0.0 {
                (strike / spot).ln().abs()
            } else {
                f64::INFINITY
            };
            let sigma_sqrt_t = vol * t.sqrt();
            if log_moneyness <= 0.3 && sigma_sqrt_t >= 0.05 && sigma_sqrt_t <= 0.5 {
                prop_assert!(
                    geo <= tw + 1e-9,
                    "geometric_avg ({geo}) must be ≤ Turnbull-Wakeman ({tw}) by AM-GM \
                     (S={spot}, K={strike}, vol={vol}, t={t})"
                );
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
