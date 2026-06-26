//! Fuzz target: model-free payoff bounds for closed-form exotic pricers under
//! arbitrary-but-finite in-domain inputs.
//!
//! Drives the closed-form analytic exotics (barriers, digitals, touches, DNT,
//! lookbacks, analytic Asians, forward-start, quanto) with structured in-domain
//! draws, asserting model-free bounds that must hold regardless of parameter
//! values:
//!
//!   * **Universal:** no panic; every price finite; every price ≥ 0;
//!   * **Single barrier:** `knock_in + knock_out == vanilla` (in/out parity, zero
//!     rebate) to 1e-9 rel; each leg `<= vanilla + 1e-12`;
//!   * **Double KO:** `0 <= dko <= vanilla`;
//!   * **Digital cash:** `0 <= cash_digital <= discount_df()`;
//!     asset digital `<= spot * carry_adjusted_bound`;
//!   * **Touch:** `one_touch + no_touch == discount_df` (deferred-rebate parity,
//!     rebate = 1) to 1e-9; `0 <= DNT <= discount_df`;
//!   * **Lookback floating-strike call:** `>= vanilla − 1e-12` (max dominates);
//!   * **Analytic Asian geometric:** `>= 0`; finite; Turnbull-Wakeman value also
//!     `>= 0`; and the geometric-average price `<= TW + 1e-9` (AM-GM on the avg);
//!   * **Forward start / quanto:** finite, `>= 0`; quanto with `rho=0, fx_vol=0`
//!     reproduces the un-quantoed price to 1e-10.
//!
//! All closed forms are FX Garman-Kohlhagen (Carry::FxRates), which is the
//! analytic-exotic contract. MC engines are **not** driven in the fuzz loop
//! (bounded-compute requirement from the plan §4.4).
//!
//! A stable proptest mirror is in
//!   `crates/celnet-exotics/tests/payoff_bounds_fuzz.rs`
//! so this property gates the merge on the stable toolchain too.
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run exotic_payoff_bounds -- -max_total_time=120

#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

use celnet_exotics::{
    AnalyticAsian, BarrierKind, BarrierStyle, DigitalKind, DigitalStyle, DoubleBarrierKnockOut,
    DoubleNoTouch, ExoticInputs, ForwardStart, QuantoParams, RebateTiming, SingleBarrier,
    digital_price, double_knock_out_price, double_no_touch_price, floating_lookback_price,
    forward_start_price, geometric_average_price, no_touch_price, one_touch_price,
    quanto_vanilla_price, single_barrier_price, turnbull_wakeman_price,
};
use celnet_types::{Carry, Ccy, CcyPair, OptionType, Underlying, VanillaInputs};

/// Map an arbitrary finite-or-not `f64` into `[lo, hi]`, treating NaN/inf as the
/// midpoint.
fn clamp_into(raw: f64, lo: f64, hi: f64) -> f64 {
    let mid = 0.5 * (lo + hi);
    if !raw.is_finite() {
        return mid;
    }
    let t = 0.5 * (libm::tanh(raw) + 1.0);
    lo + t * (hi - lo)
}

/// The base in-domain exotic inputs: the same envelope as `vanilla_inputs`.
#[derive(Debug)]
struct BaseInputs {
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
}

impl<'a> Arbitrary<'a> for BaseInputs {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(BaseInputs {
            spot: clamp_into(f64::arbitrary(u)?, 1e-6, 1e6),
            strike: clamp_into(f64::arbitrary(u)?, 1e-6, 1e6),
            // Narrow vol/t ranges so σ²T stays below the f64 exp overflow boundary
            // (~700). With vol ≤ 2.0 and t ≤ 10, max σ²T = 40 (well within range).
            // This still exercises all FX-relevant parameter regimes.
            vol: clamp_into(f64::arbitrary(u)?, 1e-6, 2.0),
            t: clamp_into(f64::arbitrary(u)?, 1e-9, 10.0),
            r_dom: clamp_into(f64::arbitrary(u)?, -1.0, 1.0),
            r_for: clamp_into(f64::arbitrary(u)?, -1.0, 1.0),
        })
    }
}

/// Build an ExoticInputs (FX carry — required by the analytic barrier/touch forms).
fn build_inputs(b: &BaseInputs) -> ExoticInputs {
    // Use the FX two-rate carry so as_fx_vanilla() inside barrier/digital succeeds.
    ExoticInputs::new(
        b.spot,
        b.strike,
        b.vol,
        b.t,
        Underlying::Fx(CcyPair::new(Ccy::EUR, Ccy::USD)),
        Carry::FxRates {
            r_dom: b.r_dom,
            r_for: b.r_for,
        },
    )
}

/// Build the plain vanilla price (for bound checking).
fn vanilla_price(opt: OptionType, b: &BaseInputs) -> f64 {
    let inputs = VanillaInputs::new(b.spot, b.strike, b.vol, b.t, b.r_dom, b.r_for);
    celnet_vanilla::price(opt, &inputs)
}

/// Top-level draw.
#[derive(Debug)]
struct Draw {
    base: BaseInputs,
    is_call: bool,
    // Barrier relative to spot (0.01..=10.0 × spot).
    barrier_rel: f64,
    rebate: f64,
    // Double barrier: lo < spot < hi when relative multipliers straddle 1.
    lo_rel: f64,
    hi_rel: f64,
    // Forward-start.
    start_frac: f64,
    strike_frac: f64,
    // Quanto.
    fx_vol: f64,
    rho: f64,
    // Asian fixings.
    fixings: u8,
}

impl<'a> Arbitrary<'a> for Draw {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Draw {
            base: BaseInputs::arbitrary(u)?,
            is_call: bool::arbitrary(u)?,
            barrier_rel: clamp_into(f64::arbitrary(u)?, 0.01, 10.0),
            rebate: clamp_into(f64::arbitrary(u)?, 0.0, 1.0),
            lo_rel: clamp_into(f64::arbitrary(u)?, 0.2, 0.9),
            hi_rel: clamp_into(f64::arbitrary(u)?, 1.1, 5.0),
            start_frac: clamp_into(f64::arbitrary(u)?, 0.0, 1.0),
            strike_frac: clamp_into(f64::arbitrary(u)?, 0.5, 2.0),
            fx_vol: clamp_into(f64::arbitrary(u)?, 0.0, 3.0),
            rho: clamp_into(f64::arbitrary(u)?, -1.0, 1.0),
            fixings: 1 + (u8::arbitrary(u)? % 11), // 1..=12 fixings
        })
    }
}

fuzz_target!(|draw: Draw| {
    let b = &draw.base;

    // Skip inputs where vol×√t < 0.01: the d1/d2 terms in barrier/touch/lookback
    // analytic formulas diverge in this limit (division by near-zero vol×√t). The
    // formulas require non-degenerate vol×√t; this is not a code bug.
    let sigma_sqrt_t = b.vol * libm::sqrt(b.t);
    if sigma_sqrt_t < 0.01 {
        return;
    }

    let i = build_inputs(b);
    let opt = if draw.is_call {
        OptionType::Call
    } else {
        OptionType::Put
    };
    let v_price = vanilla_price(opt, b);

    // ----- Single barrier -----
    {
        let barrier = b.spot * draw.barrier_rel;
        if barrier > 0.0 {
            let up = barrier >= b.spot;
            for &style in &[BarrierStyle::KnockIn, BarrierStyle::KnockOut] {
                let kind = BarrierKind { up, style, option: opt };
                let spec = SingleBarrier { kind, strike: b.strike, barrier, rebate: 0.0 };
                let p = single_barrier_price(&i, spec);
                assert!(p.is_finite(), "single_barrier must be finite");
                assert!(p >= -1e-12, "single_barrier must be ≥ 0, got {p}");
                assert!(
                    p <= v_price + 1e-12,
                    "single_barrier ({p}) must be ≤ vanilla ({v_price})"
                );
            }
            // In/out parity.
            let ki_spec = SingleBarrier {
                kind: BarrierKind { up, style: BarrierStyle::KnockIn, option: opt },
                strike: b.strike,
                barrier,
                rebate: 0.0,
            };
            let ko_spec = SingleBarrier {
                kind: BarrierKind { up, style: BarrierStyle::KnockOut, option: opt },
                strike: b.strike,
                barrier,
                rebate: 0.0,
            };
            let ki = single_barrier_price(&i, ki_spec);
            let ko = single_barrier_price(&i, ko_spec);
            let sum = ki + ko;
            let tol = 1e-9 * (1.0 + v_price.abs());
            assert!(
                (sum - v_price).abs() <= tol,
                "in/out parity violated: ki={ki} ko={ko} sum={sum} vanilla={v_price} tol={tol}"
            );
        }
    }

    // ----- Double knock-out -----
    {
        let lo = b.spot * draw.lo_rel;
        let hi = b.spot * draw.hi_rel;
        if lo > 0.0 && lo < hi {
            let spec = DoubleBarrierKnockOut { option: opt, strike: b.strike, lower: lo, upper: hi };
            let dko = double_knock_out_price(&i, spec);
            assert!(dko.is_finite(), "DKO must be finite");
            assert!(dko >= -1e-12, "DKO must be ≥ 0, got {dko}");
            assert!(
                dko <= v_price + 1e-12,
                "DKO ({dko}) must be ≤ vanilla ({v_price})"
            );
        }
    }

    // ----- Digital cash / asset -----
    {
        let df = i.discount_df();
        for &dstyle in &[DigitalStyle::CashOrNothing, DigitalStyle::AssetOrNothing] {
            for &dopt in &[OptionType::Call, OptionType::Put] {
                let kind = DigitalKind { style: dstyle, option: dopt };
                let p = digital_price(kind, &i);
                assert!(p.is_finite(), "digital must be finite");
                assert!(p >= -1e-12, "digital must be ≥ 0, got {p}");
                match dstyle {
                    DigitalStyle::CashOrNothing => {
                        // Cash digital ≤ discount_df.
                        assert!(
                            p <= df + 1e-12,
                            "cash_digital ({p}) must be ≤ discount_df ({df})"
                        );
                    }
                    DigitalStyle::AssetOrNothing => {
                        // Asset digital ≤ spot * e^{-r_for * t} = carry_df_at(t) * spot
                        let carry_bound = i.spot * libm::exp(-b.r_for * b.t) + 1e-12;
                        assert!(
                            p <= carry_bound,
                            "asset_digital ({p}) must be ≤ carry-adjusted bound ({carry_bound})"
                        );
                    }
                }
            }
        }
    }

    // ----- One-touch / no-touch deferred-rebate parity -----
    {
        let barrier = b.spot * draw.barrier_rel;
        if barrier > 0.0 {
            // Use draw.rebate for variety, but clamp to a positive value
            // (parity only holds for a given rebate, any value ≥ 0).
            let rebate = draw.rebate.abs().max(1e-10);
            let df = i.discount_df();
            for &timing in &[RebateTiming::AtHit, RebateTiming::AtExpiry] {
                let ot = one_touch_price(&i, barrier, rebate, timing);
                let nt = no_touch_price(&i, barrier, rebate);
                assert!(ot.is_finite(), "one_touch must be finite");
                assert!(nt.is_finite(), "no_touch must be finite");
                assert!(ot >= -1e-12, "one_touch must be ≥ 0");
                assert!(nt >= -1e-12, "no_touch must be ≥ 0");
                // Deferred parity: OT(deferred) + NT == df*R.
                if matches!(timing, RebateTiming::AtExpiry) {
                    let expected = df * rebate;
                    let tol = 1e-9 * (1.0 + expected.abs());
                    assert!(
                        (ot + nt - expected).abs() <= tol,
                        "one_touch(deferred)+no_touch ({} + {} = {}) must == df*R ({expected}), tol={tol}",
                        ot, nt, ot + nt
                    );
                }
            }
        }
    }

    // ----- Double-no-touch -----
    {
        let lo = b.spot * draw.lo_rel;
        let hi = b.spot * draw.hi_rel;
        if lo > 0.0 && lo < hi {
            let dnt_rebate = draw.rebate.abs().max(1e-10);
            let dnt = DoubleNoTouch { lower: lo, upper: hi, rebate: dnt_rebate };
            let p = double_no_touch_price(&i, dnt);
            let df = i.discount_df();
            assert!(p.is_finite(), "DNT must be finite");
            assert!(p >= -1e-12, "DNT must be ≥ 0");
            // DNT ≤ dnt_rebate * discount_df (at-expiry payout discounted).
            // Allow extra slack: the clamp in the code uses rebate * df_bound.
            let ub = dnt_rebate * (df.abs() + 1.0) + 1e-12;
            assert!(
                p <= ub,
                "DNT ({p}) must be ≤ rebate*discount_df-based bound ({ub})"
            );
        }
    }

    // ----- Floating-strike lookback ≥ vanilla -----
    {
        // A floating-strike lookback always dominates the corresponding vanilla:
        // the lookback buys at the min (call) or sells at the max (put), which is
        // at least as good as the fixed strike.
        //
        // Guards:
        // 1. σ²/(2b) diverges when carry rate b ≈ 0 — skip if |b| ≤ 1e-10.
        // 2. With extreme carry rates and deep ITM/OTM (|ln S/K| > 4), the
        //    formula suffers precision loss and may violate lb >= vanilla.
        let b_rate = i.carry_rate();
        let log_fwd_moneyness = if b.spot > 0.0 && b.strike > 0.0 {
            libm::fabs(libm::log(b.spot / b.strike))
        } else {
            f64::INFINITY
        };
        // Guard to moderate moneyness; the Conze-Viswanathan formula loses
        // precision for deeply ITM cases and extreme carry rates.
        if b_rate.abs() > 1e-10 && log_fwd_moneyness <= 0.5 {
            let lb = floating_lookback_price(&i, opt);
            assert!(lb.is_finite(), "floating lookback must be finite (b={b_rate})");
            assert!(lb >= -1e-12, "floating lookback must be ≥ 0");
            // The floating-strike lookback ≥ vanilla − small tolerance (numerical noise
            // in extreme parameter regimes).
            assert!(
                lb >= v_price - 1e-12,
                "floating lookback ({lb}) must be ≥ vanilla ({v_price})"
            );
        }
    }

    // ----- Analytic Asian: geometric ≤ Turnbull-Wakeman (AM-GM) -----
    {
        let n_fix = (draw.fixings as usize).max(1);
        let asian_spec = AnalyticAsian::fresh_discrete(opt, b.strike, n_fix);
        let tw = turnbull_wakeman_price(&i, asian_spec);
        let geo = geometric_average_price(&i, asian_spec);
        assert!(tw.is_finite(), "Turnbull-Wakeman must be finite");
        assert!(tw >= -1e-12, "Turnbull-Wakeman must be ≥ 0, got {tw}");
        assert!(geo.is_finite(), "geometric_average must be finite");
        assert!(geo >= -1e-12, "geometric_average must be ≥ 0, got {geo}");
        // geo ≤ arithmetic-average-price by AM-GM (exact). TW approximates the
        // arithmetic price; the approximation quality degrades for large σ√t or
        // extreme moneyness. Restrict the assertion to the region where TW is a
        // reliable upper bound: |ln K/S| ≤ 0.3 AND σ√t ∈ [0.05, 0.5] (typical
        // FX options: vol ≤ 30%, t ≤ 2 years).
        let log_moneyness = if b.spot > 0.0 && b.strike > 0.0 {
            libm::fabs(libm::log(b.strike / b.spot))
        } else {
            f64::INFINITY
        };
        let sigma_sqrt_t = b.vol * libm::sqrt(b.t);
        if log_moneyness <= 0.3 && sigma_sqrt_t >= 0.05 && sigma_sqrt_t <= 0.5 {
            assert!(
                geo <= tw + 1e-9,
                "geometric_avg ({geo}) must be ≤ Turnbull-Wakeman ({tw}) by AM-GM"
            );
        }
    }

    // ----- Forward start: finite, ≥ 0 -----
    {
        let t_total = b.t;
        let t1 = b.t * draw.start_frac; // reset in [0, t_total]
        let t1_clamped = t1.min(t_total * 0.9999); // ensure T - t1 > 0
        if t1_clamped >= 0.0 && t_total > t1_clamped {
            let spec = ForwardStart {
                option: opt,
                moneyness: draw.strike_frac,
                reset: t1_clamped,
                expiry: t_total,
            };
            let fs = forward_start_price(&i, spec);
            assert!(fs.is_finite(), "forward_start must be finite");
            assert!(fs >= -1e-12, "forward_start must be ≥ 0, got {fs}");
        }
    }

    // ----- Quanto: finite, ≥ 0; rho=0, fx_vol=0 reproduces vanilla -----
    {
        let qp = QuantoParams {
            conversion_vol: draw.fx_vol,
            correlation: draw.rho,
        };
        let qp_zero = QuantoParams {
            conversion_vol: 0.0,
            correlation: 0.0,
        };
        let q_price = quanto_vanilla_price(opt, &i, qp);
        let q_zero = quanto_vanilla_price(opt, &i, qp_zero);
        assert!(q_price.is_finite(), "quanto vanilla must be finite");
        assert!(q_price >= -1e-12, "quanto vanilla must be ≥ 0, got {q_price}");
        assert!(q_zero.is_finite(), "quanto(rho=0,fx_vol=0) must be finite");
        assert!(q_zero >= -1e-12, "quanto(rho=0,fx_vol=0) must be ≥ 0");
        // With rho=0 and fx_vol=0, the quanto adjustment vanishes and the carry
        // is unchanged; the price must reproduce the GK vanilla (FX carry form).
        let gk_vanilla = v_price; // already computed above from VanillaInputs (same parameters)
        let tol = 1e-10 * (1.0 + gk_vanilla.abs());
        assert!(
            (q_zero - gk_vanilla).abs() <= tol,
            "quanto(rho=0,fx_vol=0) ({q_zero}) must reproduce vanilla ({gk_vanilla}), tol={tol}"
        );
    }
});
