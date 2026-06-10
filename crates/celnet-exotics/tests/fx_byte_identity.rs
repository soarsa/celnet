//! FX byte-identity gate for the ADR-0008 Wave-B/C/D carry-seam migration
//! (spec §6).
//!
//! Every reference value below was captured from the **pre-migration** engines
//! (the FX-only `VanillaInputs` two-rate forms) as raw `f64::to_bits`, on this
//! exact grid, immediately before the engines were rewired onto the agnostic
//! [`celnet_exotics::ExoticInputs`] / `Carry` seam. The gate asserts the migrated
//! engines reproduce those prices **bit-for-bit** (`to_bits` equality — never an
//! epsilon): the whole ADR-0008 claim is that `Carry::FxRates` reproduces the FX
//! arithmetic exactly, so anything looser would hide a regression.
//!
//! Engines covered: the Wave-B set — the Monte-Carlo path core (`mc::*`), the
//! accumulator, the TARF, the lookback closed forms + MC, the forward-start /
//! cliquet family, and the analytic Asian estimators (Turnbull-Wakeman-style
//! moment matching, geometric-conditioning, geometric closed forms — provenance
//! in the engine docs) — the Wave-C finite-difference set — the 1-D
//! Crank-Nicolson/Rannacher PDE, the 2-D Hundsdorfer-Verwer ADI LSV solver, and
//! the American/Bermudan PSOR + Longstaff-Schwartz engines including the full
//! FD Greek strip with the §3.4 rate re-tag — the Wave-D composite set —
//! the variance/volatility-swap replication strips, the quanto closed forms +
//! MC, the correlated multi-asset basket QMC engine, and the LSV orchestration
//! (MC + ADI, vanilla / knock-out / window barrier) — and the ADR-0008 tail:
//! the pivot target-redemption accumulator MC engine (plain + control-variate).

use celnet_exotics::ExoticInputs;
use celnet_exotics::asian::{AnalyticAsian, AveragingSchedule, geometric_average_price};
use celnet_exotics::mc::{McConfig, geometric_asian_price, price_asian, price_barrier};
use celnet_exotics::payoff::{ArithmeticAsian, DiscreteBarrier};
use celnet_exotics::{
    Accumulator, AccumulatorMcConfig, Cliquet, CliquetMcConfig, CliquetSchedule, ForwardStart,
    Lookback, LookbackMcConfig, LookbackStyle, Monitoring, PivotTra, PivotTraMcConfig,
    RedemptionStyle, Tarf, TarfMcConfig, accumulator_price, cliquet_price_capped_mc,
    cliquet_price_plain, cliquet_price_plain_mc, curran_price, fixed_lookback_price,
    floating_lookback_price, forward_start_price, lookback_mc, mc::price_barrier_bgk_shifted,
    pivot_tra_price, pivot_tra_price_cv, tarf_price, turnbull_wakeman_price,
};
use celnet_types::{OptionType, VanillaInputs};

/// EURUSD-like: 1Y, 10 vol, dual carry.
fn fx_a() -> ExoticInputs {
    VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01).into()
}

/// Equity-scale FX fixture: S=K=100, 20 vol, 1Y.
fn fx_b() -> ExoticInputs {
    VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02).into()
}

/// Negative-domestic-rate, long-dated, strike above spot.
fn fx_c() -> ExoticInputs {
    VanillaInputs::new(0.85, 0.90, 0.12, 2.5, -0.004, 0.031).into()
}

/// Assert a migrated price reproduces the frozen pre-migration bits exactly.
#[track_caller]
fn gate(label: &str, value: f64, frozen_bits: u64) {
    assert_eq!(
        value.to_bits(),
        frozen_bits,
        "{label}: carry-seam price {value:?} (0x{:016x}) != pre-migration FX bits 0x{frozen_bits:016x}",
        value.to_bits(),
    );
}

#[test]
fn mc_engine_byte_identical() {
    let b = fx_b();
    let a = fx_a();

    let barrier_spec = DiscreteBarrier {
        option: OptionType::Call,
        strike: 100.0,
        barrier: 130.0,
        up: true,
        knock_in: false,
    };
    let mc_cfg = McConfig {
        pairs: 4_000,
        steps: 32,
        seed: 0x5EED,
    };
    let ko = price_barrier(&b, barrier_spec, mc_cfg);
    gate("mc_barrier_ko_price", ko.price, 0x4009c31a0d5ca3a4);
    gate("mc_barrier_ko_se", ko.std_error, 0x3faac5fd5e67dc33);
    let ki = price_barrier(
        &b,
        DiscreteBarrier {
            knock_in: true,
            ..barrier_spec
        },
        mc_cfg,
    );
    gate("mc_barrier_ki_price", ki.price, 0x401734c4ebb53186);
    let bgk = price_barrier_bgk_shifted(&b, barrier_spec, mc_cfg);
    gate("mc_barrier_bgk_price", bgk.price, 0x400955cad9ef7d74);

    let asian_spec = ArithmeticAsian {
        option: OptionType::Call,
        strike: 100.0,
        observations: 12,
    };
    let asian = price_asian(
        &b,
        asian_spec,
        McConfig {
            pairs: 4_000,
            steps: 12,
            seed: 0xBEEF,
        },
    );
    gate("mc_asian_price", asian.price, 0x401614ce9b409223);
    gate("mc_asian_se", asian.std_error, 0x3f64ad36253310a1);
    gate(
        "mc_geo_asian_b",
        geometric_asian_price(&b, asian_spec),
        0x40154f921b1f9188,
    );
    gate(
        "mc_geo_asian_a_put",
        geometric_asian_price(
            &a,
            ArithmeticAsian {
                option: OptionType::Put,
                strike: 1.31,
                observations: 6,
            },
        ),
        0x3f9fb740537fe600,
    );
}

#[test]
fn accumulator_byte_identical() {
    let a = fx_a();
    let spec = |m: Monitoring| Accumulator {
        pivot: 1.28,
        barrier: 1.36,
        fixings: 12,
        leverage: 2.0,
        notional: 1.0,
        monitoring: m,
    };
    let cfg = AccumulatorMcConfig {
        pairs: 4_000,
        seed: 0xAC11,
    };
    let d = accumulator_price(&a, spec(Monitoring::Discrete), cfg);
    gate("accumulator_discrete_price", d.price, 0xbfd809897304728a);
    gate(
        "accumulator_discrete_settled",
        d.expected_settled_fixings,
        0x401cc5a1cac0830c,
    );
    let c = accumulator_price(&a, spec(Monitoring::Continuous), cfg);
    gate("accumulator_continuous_price", c.price, 0xbfd81a817c37295a);
}

#[test]
fn tarf_byte_identical() {
    let a = fx_a();
    let spec = |r: RedemptionStyle| Tarf {
        strike: 1.32,
        fixings: 12,
        target: 0.06,
        leverage: 2.0,
        favourable_side: OptionType::Put,
        notional: 1.0,
        redemption: r,
    };
    let cfg = TarfMcConfig {
        pairs: 4_000,
        seed: 0x7A4F,
    };
    let full = tarf_price(&a, spec(RedemptionStyle::FullGain), cfg);
    gate("tarf_full_price", full.price, 0x3fe13d9fd73467ff);
    gate(
        "tarf_full_overshoot",
        full.expected_overshoot,
        0x3f972059913e5b3a,
    );
    let capped = tarf_price(&a, spec(RedemptionStyle::CappedGain), cfg);
    gate("tarf_capped_price", capped.price, 0x3fe1f514ecf9569b);
}

/// ADR-0008 tail (the verifier-caught straggler): the pivot target-redemption
/// accumulator MC engine onto the carry seam. Every frozen value below was
/// captured from the pre-migration FX-only (`VanillaInputs` two-rate) engine on
/// this exact grid — both redemption styles × plain/control-variate × two
/// market points × two fixing counts. The control-variate gates guard the §8
/// sharp edge directly: the closed-form control mean reads the stored `r_for`
/// via `Carry::yield_rate` (never `discount_rate() − carry_rate()`).
#[test]
fn pivot_tra_byte_identical() {
    let a = fx_a();
    let c = fx_c();
    // Dead-band call-favourable (P > K) on the EURUSD-like point.
    let spec_a = |fixings: usize, r: RedemptionStyle| PivotTra {
        strike: 1.28,
        pivot: 1.33,
        fixings,
        target: 0.08,
        leverage: 2.0,
        favourable_side: OptionType::Call,
        notional: 1.0,
        redemption: r,
    };
    // Overlap put-favourable (P < K) on the negative-rate long-dated point.
    let spec_c = |r: RedemptionStyle| PivotTra {
        strike: 0.87,
        pivot: 0.83,
        fixings: 6,
        target: 0.05,
        leverage: 1.5,
        favourable_side: OptionType::Put,
        notional: 1.0,
        redemption: r,
    };
    let cfg = PivotTraMcConfig {
        pairs: 4_000,
        seed: 0x9147,
    };

    // EURUSD-like, 12 fixings, FullGain — plain + CV.
    let p = pivot_tra_price(&a, spec_a(12, RedemptionStyle::FullGain), cfg);
    gate("pivot_a12_full_price", p.price, 0x3fcf48502cee20bd);
    gate("pivot_a12_full_se", p.std_error, 0x3f7e9ea150acb80a);
    gate(
        "pivot_a12_full_overshoot",
        p.expected_overshoot,
        0x3fa06218c37727e7,
    );
    let v = pivot_tra_price_cv(&a, spec_a(12, RedemptionStyle::FullGain), cfg);
    gate("pivot_a12_full_cv_price", v.price, 0x3fd0696cfa369314);
    gate("pivot_a12_full_cv_se", v.std_error, 0x3f721bc5cca5d631);

    // EURUSD-like, 12 fixings, CappedGain — plain + CV.
    let p = pivot_tra_price(&a, spec_a(12, RedemptionStyle::CappedGain), cfg);
    gate("pivot_a12_capped_price", p.price, 0x3fd1ac4be80c17ad);
    let v = pivot_tra_price_cv(&a, spec_a(12, RedemptionStyle::CappedGain), cfg);
    gate("pivot_a12_capped_cv_price", v.price, 0x3fd27049d7ee35ff);

    // EURUSD-like, 26 fixings, FullGain — plain + CV.
    let p = pivot_tra_price(&a, spec_a(26, RedemptionStyle::FullGain), cfg);
    gate("pivot_a26_full_price", p.price, 0x3fdbcd322da8dd30);
    let v = pivot_tra_price_cv(&a, spec_a(26, RedemptionStyle::FullGain), cfg);
    gate("pivot_a26_full_cv_price", v.price, 0x3fdc2f03799864ea);

    // Negative-rate long-dated, 6 fixings, FullGain — plain + CV.
    let p = pivot_tra_price(&c, spec_c(RedemptionStyle::FullGain), cfg);
    gate("pivot_c6_full_price", p.price, 0x3fb11a145a1cb449);
    gate("pivot_c6_full_se", p.std_error, 0x3f6f3395f21fb017);
    let v = pivot_tra_price_cv(&c, spec_c(RedemptionStyle::FullGain), cfg);
    gate("pivot_c6_full_cv_price", v.price, 0x3fb298df92e852e4);

    // Negative-rate long-dated, 6 fixings, CappedGain — plain + CV.
    let p = pivot_tra_price(&c, spec_c(RedemptionStyle::CappedGain), cfg);
    gate("pivot_c6_capped_price", p.price, 0x3fbaf6ca5fda66a1);
    let v = pivot_tra_price_cv(&c, spec_c(RedemptionStyle::CappedGain), cfg);
    gate("pivot_c6_capped_cv_price", v.price, 0x3fbc74c47053804e);
}

#[test]
fn lookback_byte_identical() {
    let b = fx_b();
    let c = fx_c();

    gate(
        "lookback_float_call_b",
        floating_lookback_price(&b, OptionType::Call),
        0x402ff3aa71e963b4,
    );
    gate(
        "lookback_float_put_b",
        floating_lookback_price(&b, OptionType::Put),
        0x402e0541dd9b47d8,
    );
    gate(
        "lookback_fixed_call_b",
        fixed_lookback_price(&b, OptionType::Call),
        0x4031e83dcd42cdc0,
    );
    gate(
        "lookback_fixed_put_b",
        fixed_lookback_price(&b, OptionType::Put),
        0x402a2870b4ff100c,
    );
    // C has strike 0.90 > spot 0.85 — exercises the other closed-form branch.
    gate(
        "lookback_fixed_call_c",
        fixed_lookback_price(&c, OptionType::Call),
        0x3faec11a480efd49,
    );
    gate(
        "lookback_fixed_put_c",
        fixed_lookback_price(&c, OptionType::Put),
        0x3fcd4c0c5d7b3e34,
    );

    let cfg = LookbackMcConfig {
        pairs: 3_000,
        steps: 24,
        seed: 0x100B,
    };
    let float_call = lookback_mc(
        &b,
        Lookback {
            style: LookbackStyle::FloatingStrike,
            option: OptionType::Call,
        },
        cfg,
    );
    gate(
        "lookback_mc_float_call",
        float_call.price,
        0x402ffd448a4cf8e8,
    );
    gate(
        "lookback_mc_float_call_se",
        float_call.std_error,
        0x3fbae7605d719a32,
    );
    let fixed_put = lookback_mc(
        &b,
        Lookback {
            style: LookbackStyle::FixedStrike,
            option: OptionType::Put,
        },
        cfg,
    );
    gate("lookback_mc_fixed_put", fixed_put.price, 0x402a23cb0037fc8a);
}

#[test]
fn forward_start_and_cliquet_byte_identical() {
    let a = fx_a();
    let c = fx_c();

    gate(
        "forward_start_call_a",
        forward_start_price(
            &a,
            ForwardStart {
                option: OptionType::Call,
                moneyness: 1.0,
                reset: 0.25,
                expiry: 1.0,
            },
        ),
        0x3fabd57fa25a7aa5,
    );
    gate(
        "forward_start_put_a",
        forward_start_price(
            &a,
            ForwardStart {
                option: OptionType::Put,
                moneyness: 1.05,
                reset: 0.5,
                expiry: 1.0,
            },
        ),
        0x3fb1620a0852136c,
    );
    gate(
        "forward_start_call_c",
        forward_start_price(
            &c,
            ForwardStart {
                option: OptionType::Call,
                moneyness: 0.95,
                reset: 1.0,
                expiry: 2.5,
            },
        ),
        0x3fa75d54ad18ba30,
    );

    let cliquet = |floor: Option<f64>, cap: Option<f64>| Cliquet {
        option: OptionType::Call,
        moneyness: 1.0,
        schedule: CliquetSchedule::equal(4, 1.0),
        local_floor: floor,
        local_cap: cap,
        global_floor: None,
        global_cap: None,
    };
    gate(
        "cliquet_plain_a",
        cliquet_price_plain(&a, &cliquet(None, None)),
        0x3fbdc00cd49eacf0,
    );
    let cfg = CliquetMcConfig {
        pairs: 3_000,
        seed: 0xC119_0001,
    };
    let capped = cliquet_price_capped_mc(&a, &cliquet(Some(0.0), Some(0.02)), cfg);
    gate("cliquet_capped_mc_a", capped.price, 0x3fa769a28af5fafa);
    gate(
        "cliquet_capped_mc_a_se",
        capped.std_error,
        0x3f190cf36648c54a,
    );
    let plain_mc = cliquet_price_plain_mc(&a, &cliquet(None, None), cfg);
    gate("cliquet_plain_mc_a", plain_mc.price, 0x3fbdbd18dcdc9e73);
}

#[test]
fn analytic_asian_byte_identical() {
    let a = fx_a();
    let b = fx_b();

    let asian12 = AnalyticAsian::fresh_discrete(OptionType::Call, 100.0, 12);
    gate(
        "asian_tw_discrete_b",
        turnbull_wakeman_price(&b, asian12),
        0x40162369b984ac79,
    );
    gate(
        "asian_tw_continuous_b",
        turnbull_wakeman_price(&b, AnalyticAsian::fresh_continuous(OptionType::Call, 100.0)),
        0x4014c6f621d40987,
    );
    let seasoned = AnalyticAsian {
        option: OptionType::Call,
        strike: 100.0,
        schedule: AveragingSchedule::Discrete { future_obs: 9 },
        t_start: 0.3,
        elapsed_avg: 102.0,
        elapsed_weight: 0.25,
    };
    gate(
        "asian_tw_seasoned_b",
        turnbull_wakeman_price(&b, seasoned),
        0x401597cee78982f3,
    );
    gate(
        "asian_curran_discrete_b",
        curran_price(&b, asian12),
        0x401614b230d965c1,
    );
    gate(
        "asian_curran_seasoned_b",
        curran_price(&b, seasoned),
        0x40159350537fbf0f,
    );
    gate(
        "asian_geo_discrete_b",
        geometric_average_price(&b, asian12),
        0x40154f921b1f9188,
    );
    gate(
        "asian_geo_continuous_b",
        geometric_average_price(&b, AnalyticAsian::fresh_continuous(OptionType::Call, 100.0)),
        0x4013f16b062e5118,
    );
    gate(
        "asian_tw_put_a",
        turnbull_wakeman_price(&a, AnalyticAsian::fresh_discrete(OptionType::Put, 1.31, 6)),
        0x3f9f4a27330cfd0f,
    );
    gate(
        "asian_curran_put_a",
        curran_price(&a, AnalyticAsian::fresh_discrete(OptionType::Put, 1.31, 6)),
        0x3f9f449667ec0e15,
    );
}

/// Cross-asset enablement (the payoff Wave B unlocks, spec §6): an **equity**
/// geometric-average Asian on `Carry::CostOfCarry { r, b = r − q }` reconciled to
/// the generalized closed form **re-derived by hand** — the geometric average of
/// `n` equally-spaced lognormal observations is lognormal with
/// `σ_G² = σ²(n+1)(2n+1)/(6n²)` and carry `b_G = ½(b − ½σ²)(n+1)/n + ½σ_G²`, so
/// the option prices as `e^{−r·T}·[F_G·Φ(d₁) − K·Φ(d₂)]` with `F_G = S·e^{b_G·T}`
/// (Kemna-Vorst law, generalized-BSM measure — independent, non-circular oracle).
#[test]
fn equity_geometric_asian_matches_hand_derived_closed_form() {
    use celnet_types::{Carry, Ccy, EquityRef, Symbol, Underlying};

    let (s0, k, vol, t, r, q) = (50.0, 52.0, 0.25, 0.75, 0.04, 0.015);
    let n = 12usize;
    let inputs = ExoticInputs::new(
        s0,
        k,
        vol,
        t,
        Underlying::Equity(EquityRef::new(
            Symbol::new("ACME", "XLON"),
            Ccy::parse("GBP").unwrap(),
        )),
        Carry::CostOfCarry { r, b: r - q },
    );
    let engine = geometric_average_price(
        &inputs,
        AnalyticAsian::fresh_discrete(OptionType::Call, k, n),
    );

    // Hand re-derivation (no engine code shared).
    let nf = n as f64;
    let b = r - q;
    let sig_g2 = vol * vol * (nf + 1.0) * (2.0 * nf + 1.0) / (6.0 * nf * nf);
    let b_g = 0.5 * (b - 0.5 * vol * vol) * (nf + 1.0) / nf + 0.5 * sig_g2;
    let f_g = s0 * (b_g * t).exp();
    let sd = (sig_g2 * t).sqrt();
    let d1 = ((f_g / k).ln() + 0.5 * sig_g2 * t) / sd;
    let d2 = d1 - sd;
    let phi = |x: f64| 0.5 * libm::erfc(-x / std::f64::consts::SQRT_2);
    let hand = (-r * t).exp() * (f_g * phi(d1) - k * phi(d2));

    assert!(
        (engine - hand).abs() < 1e-12,
        "equity geometric Asian: engine {engine} vs hand-derived {hand}"
    );
}

// ===========================================================================
// Wave C — PDE / ADI / American finite-difference engines (spec §3.3 Wave C +
// §3.4). Every frozen value below was captured from the pre-migration FX-only
// engines on this exact grid immediately before the carry-seam rewiring. The
// far-field boundaries read the STORED r_for via Carry::yield_rate, so these
// gates are the direct guard on the §8 sharp edge (a discount−carry
// reconstruction would flip low bits here first).
// ===========================================================================

#[test]
fn pde_engine_byte_identical() {
    use celnet_exotics::pde::{PdeGrid, PdeProblem, solve as pde_solve};

    let grid = PdeGrid::default();
    let prob = |option, strike, knock_out| PdeProblem {
        option,
        strike,
        knock_out,
    };
    gate(
        "pde_b_call_vanilla",
        pde_solve(&fx_b(), prob(OptionType::Call, 100.0, None), grid),
        0x4022740854d740ee,
    );
    gate(
        "pde_b_call_uo",
        pde_solve(
            &fx_b(),
            prob(OptionType::Call, 100.0, Some((130.0, true))),
            grid,
        ),
        0x40091c28c451ced7,
    );
    gate(
        "pde_b_put_do",
        pde_solve(
            &fx_b(),
            prob(OptionType::Put, 100.0, Some((80.0, false))),
            grid,
        ),
        0x3ffbb777d45d44ef,
    );
    gate(
        "pde_c_put_vanilla",
        pde_solve(&fx_c(), prob(OptionType::Put, 0.90, None), grid),
        0x3fc24ceee5840456,
    );
    gate(
        "pde_a_call_uo",
        pde_solve(
            &fx_a(),
            prob(OptionType::Call, 1.25, Some((1.45, true))),
            grid,
        ),
        0x3f9d59fea9e28a8c,
    );
}

/// Unit leverage surface over a wide spot grid — the fixed LSV scaffolding the
/// ADI gates price against (identical pre/post migration; the migration touches
/// only the rate/carry reads).
fn unit_leverage() -> celnet_exotics::LeverageSurface {
    let spots: Vec<f64> = (0..41).map(|k| 20.0 + 6.0 * k as f64).collect();
    let times = vec![0.0, 0.5, 1.0];
    let mut lev = celnet_exotics::LeverageSurface::new(spots, times);
    for j in 0..lev.time_len() {
        for i in 0..lev.spot_len() {
            lev.set(i, j, 1.0);
        }
    }
    lev
}

#[test]
fn adi_engine_byte_identical() {
    use celnet_exotics::adi::{AdiGrid, AdiProblem, WindowSpec, solve as adi_solve, solve_window};
    use celnet_exotics::stochvol::VarianceParams;

    let sigma = 0.20f64;
    let var = VarianceParams::new(sigma * sigma, 1.0, sigma * sigma, 0.4, -0.4);
    let lev = unit_leverage();
    let agrid = AdiGrid {
        x_steps: 80,
        v_steps: 30,
        time_steps: 40,
        ..AdiGrid::default()
    };
    gate(
        "adi_b_call_vanilla",
        adi_solve(
            &fx_b(),
            &var,
            &lev,
            AdiProblem {
                option: OptionType::Call,
                strike: 100.0,
                knock_out: None,
            },
            agrid,
        ),
        0x401dea282ab68a6c,
    );
    gate(
        "adi_b_call_uo",
        adi_solve(
            &fx_b(),
            &var,
            &lev,
            AdiProblem {
                option: OptionType::Call,
                strike: 100.0,
                knock_out: Some((130.0, true)),
            },
            agrid,
        ),
        0x4011ef424f18dbef,
    );
    gate(
        "adi_b_call_window",
        solve_window(
            &fx_b(),
            &var,
            &lev,
            WindowSpec {
                option: OptionType::Call,
                strike: 100.0,
                barrier: 130.0,
                up: true,
                t_after: 0.3,
                t_window: 0.4,
                t_before: 0.3,
            },
            agrid,
        ),
        0x40159f5ecf5cfa0f,
    );
}

#[test]
fn american_engine_byte_identical() {
    use celnet_exotics::{
        AmericanGrid, AmericanOption, ExerciseStyle, LsmConfig, american_fd, american_lsm,
    };

    let amg = AmericanGrid::default();
    let amr = |option, strike| AmericanOption {
        option,
        strike,
        style: ExerciseStyle::American,
    };
    gate(
        "am_b_put_fd",
        american_fd(&fx_b(), &amr(OptionType::Put, 100.0), amg),
        0x401aa3d3f6419478,
    );
    gate(
        "am_b_call_fd",
        american_fd(&fx_b(), &amr(OptionType::Call, 100.0), amg),
        0x402274085607d60c,
    );
    gate(
        "am_c_put_fd",
        american_fd(&fx_c(), &amr(OptionType::Put, 0.90), amg),
        0x3fc24ceee580fdf0,
    );
    gate(
        "am_b_put_bermudan",
        american_fd(
            &fx_b(),
            &AmericanOption {
                option: OptionType::Put,
                strike: 100.0,
                style: ExerciseStyle::Bermudan {
                    dates: vec![0.25, 0.5, 0.75],
                },
            },
            amg,
        ),
        0x401a4538ba4c6598,
    );
    let lsm = american_lsm(
        &fx_b(),
        &amr(OptionType::Put, 100.0),
        LsmConfig {
            paths: 20_000,
            exercise_dates: 20,
            seed: 0xA11CE,
        },
    );
    gate("am_b_put_lsm_price", lsm.price, 0x401a72617c3a8f0a);
    gate("am_b_put_lsm_se", lsm.std_error, 0x3fac7b4974317b8b);
}

/// The full American FD Greek strip, including the §3.4 rate re-tag: the FX
/// input's rhos are bumped along `r_dom`/`r_for` natively (the historical
/// perturbations bit-for-bit) and emerge as `RateSensitivities::Fx` — every
/// member equals the pre-migration `Greeks` field exactly.
#[test]
fn american_fd_greeks_byte_identical() {
    use celnet_exotics::{AmericanGrid, AmericanOption, ExerciseStyle, american_fd_greeks};
    use celnet_types::RateSensitivities;

    let ggrid = AmericanGrid {
        space_steps: 300,
        time_steps: 150,
        ..AmericanGrid::default()
    };
    let g = american_fd_greeks(
        &fx_b(),
        &AmericanOption {
            option: OptionType::Put,
            strike: 100.0,
            style: ExerciseStyle::American,
        },
        ggrid,
    );
    gate("am_g_price", g.price, 0x401aa07e0369565a);
    gate("am_g_delta_spot", g.delta_spot, 0xbfdb12e2a25c58c0);
    gate("am_g_delta_forward", g.delta_forward, 0xbfdb9ee6017f777a);
    gate("am_g_gamma", g.gamma, 0x3ff88d3a3e004380);
    gate("am_g_vega", g.vega, 0x4043022824315e28);
    gate("am_g_theta", g.theta, 0xc0058828fd050dc0);
    gate("am_g_vanna", g.vanna, 0x3fc649a42ff9d1ff);
    gate("am_g_volga", g.volga, 0x3fdc2a7c52b80000);
    gate("am_g_charm", g.charm, 0x3f9049f96801c000);
    gate("am_g_speed", g.speed, 0xbfcad94810404fff);
    gate("am_g_zomma", g.zomma, 0x40199b3912e1de80);
    gate("am_g_color", g.color, 0xbff1a299f947a800);
    match g.rates {
        RateSensitivities::Fx { rho_dom, rho_for } => {
            gate("am_g_rho_dom", rho_dom, 0xc0412872bda814c5);
            gate("am_g_rho_for", rho_for, 0x403e45d24ebe5f6c);
        }
        RateSensitivities::Carry { .. } => {
            panic!("FX American input must report RateSensitivities::Fx")
        }
    }
}

/// Cross-asset enablement (the payoff Wave C unlocks, spec §6/§7): the 1-D PDE
/// prices an **equity** vanilla on `Carry::CostOfCarry { r, b = r − q }` against
/// the generalized closed form **re-derived by hand** (`d₁ = [ln(S/K) +
/// (b+½σ²)T]/(σ√T)`, `C = S·e^{(b−r)T}·Φ(d₁) − K·e^{−rT}·Φ(d₂)` — independent,
/// non-circular oracle), and the American FD on the same carry dominates the
/// European value while reporting carry-tagged rate rhos with the documented
/// signs (`∂C/∂r > 0` net of carry, `∂C/∂b > 0` for a call).
#[test]
fn equity_pde_and_american_price_on_the_carry_seam() {
    use celnet_exotics::pde::{PdeGrid, PdeProblem, solve as pde_solve};
    use celnet_exotics::{
        AmericanGrid, AmericanOption, ExerciseStyle, american_fd, american_fd_greeks,
    };
    use celnet_types::{Carry, Ccy, EquityRef, RateSensitivities, Symbol, Underlying};

    let (s0, k, vol, t, r, q) = (50.0, 52.0, 0.25, 1.0, 0.04, 0.025);
    let b = r - q;
    let inputs = ExoticInputs::new(
        s0,
        k,
        vol,
        t,
        Underlying::Equity(EquityRef::new(
            Symbol::new("ACME", "XLON"),
            Ccy::parse("GBP").unwrap(),
        )),
        Carry::CostOfCarry { r, b },
    );

    // Hand re-derivation of the generalized lognormal closed form.
    let sd = vol * t.sqrt();
    let d1 = ((s0 / k).ln() + (b + 0.5 * vol * vol) * t) / sd;
    let d2 = d1 - sd;
    let phi = |x: f64| 0.5 * libm::erfc(-x / std::f64::consts::SQRT_2);
    let hand = s0 * ((b - r) * t).exp() * phi(d1) - k * (-r * t).exp() * phi(d2);

    let pde = pde_solve(
        &inputs,
        PdeProblem {
            option: OptionType::Call,
            strike: k,
            knock_out: None,
        },
        PdeGrid {
            space_steps: 1000,
            time_steps: 600,
            ..PdeGrid::default()
        },
    );
    assert!(
        (pde - hand).abs() < 5e-3,
        "equity PDE {pde} vs hand-derived closed form {hand}"
    );

    // American call on a dividend-paying equity: dominates European, and with a
    // positive dividend the early-exercise premium is non-negative by the LCP.
    let spec = AmericanOption {
        option: OptionType::Call,
        strike: k,
        style: ExerciseStyle::American,
    };
    let am = american_fd(&inputs, &spec, AmericanGrid::default());
    assert!(
        am >= hand - 5e-3,
        "equity American call {am} must dominate European {hand}"
    );

    // The rate strip is carry-tagged (the §3.4 re-tag) with the documented signs.
    let g = american_fd_greeks(
        &inputs,
        &spec,
        AmericanGrid {
            space_steps: 300,
            time_steps: 150,
            ..AmericanGrid::default()
        },
    );
    match g.rates {
        RateSensitivities::Carry {
            discount_rho,
            carry_rho,
        } => {
            assert!(
                carry_rho > 0.0,
                "call carry-rho must be positive, got {carry_rho}"
            );
            assert!(
                discount_rho.is_finite() && carry_rho.is_finite(),
                "rate strip must be finite"
            );
        }
        RateSensitivities::Fx { .. } => {
            panic!("equity cost-of-carry input must report RateSensitivities::Carry")
        }
    }
}

// ===========================================================================
// Wave D — composite / specialized engines (spec §3.3 Wave D): variance /
// volatility swap replication, quanto, correlated multi-asset basket, and the
// LSV orchestration. Every frozen value below was captured from the
// pre-migration FX-only engines on this exact grid immediately before the
// carry-seam rewiring. The quanto gates guard the §8 sharp edge directly: the
// carry shift is realised on the yield side via Carry::yield_rate (the stored
// r_for verbatim), reproducing the historical r_for ← r_for − adjustment
// arithmetic bit-for-bit.
// ===========================================================================

#[test]
fn var_vol_swap_byte_identical() {
    use celnet_core::FlatSmile;
    use celnet_exotics::{VarSwapContext, fair_variance, fair_volatility};
    use celnet_surface::MarketHedgeSmile;

    let ctx = VarSwapContext::from_inputs(&fx_a());
    let fv = fair_variance(&FlatSmile::new(0.10), &ctx);
    gate(
        "varswap_flat_fair_variance",
        fv.fair_variance,
        0x3f847ae147ae1488,
    );
    gate("varswap_flat_put_leg", fv.put_leg, 0x3f7506491bf3bc0e);
    gate("varswap_flat_call_leg", fv.call_leg, 0x3f73ef7973686d01);
    let f = ctx.forward;
    let convex = MarketHedgeSmile::new([f / 1.10, f, f * 1.10], [0.115, 0.10, 0.115], f, ctx.t);
    gate(
        "varswap_convex_fair_variance",
        fair_variance(&convex, &ctx).fair_variance,
        0x3f8c563c794c8588,
    );
    let wings = MarketHedgeSmile::new([f / 1.10, f, f * 1.10], [0.13, 0.10, 0.13], f, ctx.t);
    let vs = fair_volatility(&wings, &ctx);
    gate("volswap_convex_fair_vol", vs.fair_vol, 0x3fbf7be1698beda4);
    gate(
        "volswap_convex_correction",
        vs.convexity_correction,
        0x3f8ac9439c58b6cd,
    );
    gate(
        "volswap_convex_vov",
        vs.variance_of_variance,
        0x3f3145f0fd42b040,
    );
    gate(
        "volswap_flat_fair_vol",
        fair_volatility(&FlatSmile::new(0.10), &ctx).fair_vol,
        0x3fb99999999999a2,
    );
}

#[test]
fn quanto_byte_identical() {
    use celnet_exotics::{
        QuantoMcConfig, QuantoParams, quanto_digital_mc, quanto_digital_price, quanto_vanilla_mc,
        quanto_vanilla_price,
    };

    let qi: ExoticInputs = VanillaInputs::new(1.30, 1.30, 0.12, 1.0, 0.03, 0.01).into();
    let qv = QuantoParams::new(0.18, 0.55);
    gate(
        "quanto_vanilla_call",
        quanto_vanilla_price(OptionType::Call, &qi, qv),
        0x3fb0ddf6d850f4c8,
    );
    gate(
        "quanto_vanilla_put",
        quanto_vanilla_price(OptionType::Put, &qi, qv),
        0x3fac77c1fa051fd0,
    );
    let qd = QuantoParams::new(0.14, -0.30);
    gate(
        "quanto_digital_call",
        quanto_digital_price(OptionType::Call, &qi, qd),
        0x3fe15cb7972f63f3,
    );
    gate(
        "quanto_digital_put",
        quanto_digital_price(OptionType::Put, &qi, qd),
        0x3fdb625866ea2fba,
    );
    let qmc = quanto_vanilla_mc(
        OptionType::Call,
        &qi,
        qv,
        QuantoMcConfig {
            pairs: 50_000,
            seed: 0x5EED,
        },
    );
    gate("quanto_mc_vanilla_price", qmc.price, 0x3fb0df662bfcfa38);
    gate("quanto_mc_vanilla_se", qmc.std_error, 0x3f2ea1056b8d8c31);
    let dmc = quanto_digital_mc(
        OptionType::Put,
        &qi,
        qd,
        QuantoMcConfig {
            pairs: 50_000,
            seed: 0xD161,
        },
    );
    gate("quanto_mc_digital_price", dmc.price, 0x3fdb614243e3b84f);
}

#[test]
fn basket_byte_identical() {
    use celnet_exotics::{BasketKind, BasketLeg, BasketMcConfig, BasketSpec, price_basket};
    use celnet_types::Carry;

    // FX legs under a shared settlement numeraire: the per-leg carry is the
    // same r_dom − r_for float op the engine performed pre-migration, and the
    // settlement-cash numeraire (forward 1 ⇒ b = 0) discounts at r_dom.
    let r_dom = 0.02;
    let numeraire = Carry::CostOfCarry { r: r_dom, b: 0.0 };
    let spec = BasketSpec {
        legs: vec![
            BasketLeg::new(1.0, 0.10, r_dom - 0.01, 1.0),
            BasketLeg::new(1.2, 0.12, r_dom - 0.015, 1.0),
        ],
        correlation: vec![vec![1.0, 0.3], vec![0.3, 1.0]],
        option_type: OptionType::Call,
        strike: 2.2,
        kind: BasketKind::Basket,
    };
    let est = price_basket(
        &spec,
        numeraire,
        1.0,
        BasketMcConfig {
            budget: 512,
            replications: 4,
            steps: 1,
            seed: 7,
        },
    )
    .unwrap();
    gate("basket_call_price", est.price, 0x3fb6002db31d6bc8);
    gate("basket_call_se", est.std_error, 0x3f113f77d196a145);

    let r_dom = 0.025;
    let worst = BasketSpec {
        legs: vec![
            BasketLeg::new(1.0, 0.15, r_dom - 0.010, 1.0),
            BasketLeg::new(1.0, 0.18, r_dom - 0.012, 1.0),
        ],
        correlation: vec![vec![1.0, 0.4], vec![0.4, 1.0]],
        option_type: OptionType::Call,
        strike: 1.0,
        kind: BasketKind::WorstOf,
    };
    let west = price_basket(
        &worst,
        Carry::CostOfCarry { r: r_dom, b: 0.0 },
        1.0,
        BasketMcConfig {
            budget: 1024,
            replications: 4,
            steps: 2,
            seed: 9,
        },
    )
    .unwrap();
    gate("basket_worstof_price", west.price, 0x3fa001039c58902d);
}

/// Unit leverage surface on an LSV-scale (FX) spot grid — fixed scaffolding for
/// the LSV gates (identical pre/post migration; the migration touches only the
/// rate/carry reads of the orchestration layer).
fn lsv_unit_leverage() -> celnet_exotics::LeverageSurface {
    let spots: Vec<f64> = (0..41)
        .map(|k| 1.30 * celnet_core::math::exp(-0.6 + 0.03 * k as f64))
        .collect();
    let mut lev = celnet_exotics::LeverageSurface::new(spots, vec![0.0, 0.5, 1.0]);
    for j in 0..lev.time_len() {
        for s in 0..lev.spot_len() {
            lev.set(s, j, 1.0);
        }
    }
    lev
}

#[test]
fn lsv_model_byte_identical() {
    use celnet_exotics::stochvol::VarianceParams;
    use celnet_exotics::{AdiGrid, LsvModel, WindowBarrier};

    let var = VarianceParams::new(0.01, 2.0, 0.01, 0.18, -0.30);
    let model = LsvModel::from_leverage(fx_a(), var, lsv_unit_leverage());
    let mc_cfg = McConfig {
        pairs: 2_000,
        steps: 16,
        seed: 0xB17,
    };
    let emc = model.price_european_mc(OptionType::Call, 1.30, mc_cfg);
    gate("lsv_mc_european_price", emc.price, 0x3fafa4f5ba484b90);
    gate("lsv_mc_european_se", emc.std_error, 0x3f4cc436966cb775);
    let agrid = AdiGrid {
        x_steps: 64,
        v_steps: 20,
        time_steps: 24,
        ..AdiGrid::default()
    };
    gate(
        "lsv_pde_european_price",
        model.price_european_pde(OptionType::Call, 1.30, agrid),
        0x3fac9eef8c35f3f0,
    );
    gate(
        "lsv_pde_barrier_price",
        model.price_barrier_pde(OptionType::Call, 1.30, 1.45, true, agrid),
        0x3f94f5fa15db0f1c,
    );
    let window = WindowBarrier {
        option: OptionType::Call,
        strike: 1.30,
        barrier: 1.45,
        up: true,
        start: 0.25,
        end: 0.75,
    };
    gate(
        "lsv_pde_window_price",
        model.price_window_barrier_pde(window, agrid),
        0x3f9bfc239a0ad83d,
    );
    let wmc = model.price_window_barrier_mc(window, mc_cfg);
    gate("lsv_mc_window_price", wmc.price, 0x3fa0204d08c44b54);
    gate("lsv_mc_window_se", wmc.std_error, 0x3f44f71518056c86);
}

// ===========================================================================
// Wave D cross-asset enablement (spec §6): the same composite engines price a
// non-FX carry against independently re-derived oracles — no engine code (and
// no carry-accessor arithmetic) shared with the assertion side.
// ===========================================================================

/// An equity **quanto** vanilla on `Carry::CostOfCarry { r, b = r − q }`
/// reconciled to a hand-rederived generalized closed form at the shifted carry
/// `b_Q = b − ρ σ_S σ_Z`: `e^{−r t}·[S e^{b_Q t} Φ(d1) − K Φ(d2)]`.
#[test]
fn equity_quanto_matches_hand_derived_closed_form() {
    use celnet_exotics::{QuantoParams, quanto_vanilla_price};
    use celnet_types::{Carry, Ccy, EquityRef, Symbol, Underlying};

    let (s0, k, vol, t, r, q) = (50.0, 52.0, 0.25, 0.75, 0.04, 0.015);
    let (conv_vol, rho) = (0.18, 0.55);
    let b = r - q;
    let inputs = ExoticInputs::new(
        s0,
        k,
        vol,
        t,
        Underlying::Equity(EquityRef::new(
            Symbol::new("ACME", "XLON"),
            Ccy::parse("GBP").unwrap(),
        )),
        Carry::CostOfCarry { r, b },
    );
    let engine = quanto_vanilla_price(OptionType::Call, &inputs, QuantoParams::new(conv_vol, rho));

    // Hand re-derivation (no engine code shared).
    let b_q = b - rho * vol * conv_vol;
    let f_q = s0 * (b_q * t).exp();
    let sd = vol * t.sqrt();
    let d1 = ((f_q / k).ln() + 0.5 * vol * vol * t) / sd;
    let d2 = d1 - sd;
    let phi = |x: f64| 0.5 * libm::erfc(-x / std::f64::consts::SQRT_2);
    let hand = (-r * t).exp() * (f_q * phi(d1) - k * phi(d2));

    assert!(
        (engine - hand).abs() < 1e-12,
        "equity quanto vanilla: engine {engine} vs hand-derived {hand}"
    );
}

/// A flat smile must replicate to `K_var = σ²` for **any** carry — the
/// model-free log-contract identity is carry-independent, so an equity
/// cost-of-carry context recovers σ² exactly like the FX context does.
#[test]
fn equity_var_swap_flat_smile_recovers_sigma_squared() {
    use celnet_core::FlatSmile;
    use celnet_exotics::{VarSwapContext, fair_variance};
    use celnet_types::Carry;

    let (s0, t, r, q, sigma) = (50.0, 0.75, 0.04, 0.015, 0.20);
    let b = r - q;
    let ctx = VarSwapContext::new(s0 * f64::exp(b * t), t, Carry::CostOfCarry { r, b });
    let res = fair_variance(&FlatSmile::new(sigma), &ctx);
    assert!(
        (res.fair_variance - sigma * sigma).abs() < 1e-7,
        "equity flat-smile fair variance {} vs σ² {}",
        res.fair_variance,
        sigma * sigma
    );
}

/// A single-leg equity **basket** (`b = r − q` leg under an `r`-discounting
/// settlement numeraire) is a plain generalized-BSM vanilla: the QMC estimate
/// must reconcile with the hand-derived closed form within its measured error.
#[test]
fn equity_basket_single_leg_matches_hand_derived_closed_form() {
    use celnet_exotics::{BasketKind, BasketLeg, BasketMcConfig, BasketSpec, price_basket};
    use celnet_types::Carry;

    let (s0, k, vol, t, r, q) = (50.0, 52.0, 0.25, 0.75, 0.04, 0.015);
    let b = r - q;
    let spec = BasketSpec {
        legs: vec![BasketLeg::new(s0, vol, b, 1.0)],
        correlation: vec![vec![1.0]],
        option_type: OptionType::Call,
        strike: k,
        kind: BasketKind::Basket,
    };
    let est = price_basket(
        &spec,
        Carry::CostOfCarry { r, b: 0.0 },
        t,
        BasketMcConfig {
            budget: 8_192,
            replications: 8,
            steps: 1,
            seed: 0xE9,
        },
    )
    .unwrap();

    let f = s0 * (b * t).exp();
    let sd = vol * t.sqrt();
    let d1 = ((f / k).ln() + 0.5 * vol * vol * t) / sd;
    let d2 = d1 - sd;
    let phi = |x: f64| 0.5 * libm::erfc(-x / std::f64::consts::SQRT_2);
    let hand = (-r * t).exp() * (f * phi(d1) - k * phi(d2));

    let tol = 4.0 * est.std_error + 1e-4;
    assert!(
        (est.price - hand).abs() < tol,
        "equity single-leg basket: QMC {} vs hand-derived {hand} (se {}, tol {tol})",
        est.price,
        est.std_error
    );
}

/// The **LSV orchestration** prices an equity carry: in the pure-local-vol
/// limit (`ξ = 0`, `v0 = θ = σ²`, unit leverage) the 2-D ADI engine must
/// recover the hand-derived generalized-BSM vanilla within grid tolerance.
#[test]
fn equity_lsv_pure_local_vol_limit_matches_closed_form() {
    use celnet_exotics::stochvol::VarianceParams;
    use celnet_exotics::{AdiGrid, LsvModel};
    use celnet_types::{Carry, Ccy, EquityRef, Symbol, Underlying};

    let (s0, k, vol, t, r, q) = (50.0, 50.0, 0.20, 1.0, 0.04, 0.015);
    let b = r - q;
    let inputs = ExoticInputs::new(
        s0,
        k,
        vol,
        t,
        Underlying::Equity(EquityRef::new(
            Symbol::new("ACME", "XLON"),
            Ccy::parse("GBP").unwrap(),
        )),
        Carry::CostOfCarry { r, b },
    );
    let spots: Vec<f64> = (0..41)
        .map(|j| s0 * celnet_core::math::exp(-0.6 + 0.03 * j as f64))
        .collect();
    let mut lev = celnet_exotics::LeverageSurface::new(spots, vec![0.0, 0.5, 1.0]);
    for j in 0..lev.time_len() {
        for s in 0..lev.spot_len() {
            lev.set(s, j, 1.0);
        }
    }
    let var = VarianceParams::new(vol * vol, 1.0, vol * vol, 0.0, 0.0);
    let model = LsvModel::from_leverage(inputs, var, lev);
    let pde = model.price_european_pde(
        OptionType::Call,
        k,
        AdiGrid {
            x_steps: 140,
            v_steps: 30,
            time_steps: 80,
            ..AdiGrid::default()
        },
    );

    let f = s0 * (b * t).exp();
    let sd = vol * t.sqrt();
    let d1 = ((f / k).ln() + 0.5 * vol * vol * t) / sd;
    let d2 = d1 - sd;
    let phi = |x: f64| 0.5 * libm::erfc(-x / std::f64::consts::SQRT_2);
    let hand = (-r * t).exp() * (f * phi(d1) - k * phi(d2));

    assert!(
        (pde - hand).abs() < 5e-3,
        "equity LSV pure-LV limit: ADI {pde} vs hand-derived {hand}"
    );
}

/// ADR-0008 final straggler — the Vanna-Volga market-hedge overlay
/// (`market_price_of_hedge_smile`). Reference values captured from the
/// pre-migration `VanillaInputs` form on this exact grid (two market points ×
/// a convex and an asymmetric smile), asserted bit-for-bit post-migration.
#[test]
fn market_hedge_overlay_byte_identical() {
    use celnet_exotics::market_price_of_hedge_smile;
    use celnet_surface::MarketHedgeSmile;

    // P1: EURUSD-like 1Y, ATM 10 vol, symmetric convex smile.
    let i1 = VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01);
    let f1 = i1.forward();
    let (kp1, kc1) = (f1 / 1.10, f1 * 1.10);
    let s1 = MarketHedgeSmile::new([kp1, f1, kc1], [0.115, 0.10, 0.115], f1, i1.t);
    let m1 = market_price_of_hedge_smile(&s1, &(&i1).into(), kp1, kc1);

    // P2: negative-rate, long-dated, high-vol, skewed (risk-reversal) smile.
    let i2 = VanillaInputs::new(0.95, 0.95, 0.22, 2.5, -0.005, 0.012);
    let f2 = i2.forward();
    let (kp2, kc2) = (f2 / 1.25, f2 * 1.25);
    let s2 = MarketHedgeSmile::new([kp2, f2, kc2], [0.245, 0.22, 0.252], f2, i2.t);
    let m2 = market_price_of_hedge_smile(&s2, &(&i2).into(), kp2, kc2);

    assert_eq!(m1.vanna_price.to_bits(), 0x3f19fd9fb1e89fe7, "p1 vanna");
    assert_eq!(m1.volga_price.to_bits(), 0x3f5b20f48739ee41, "p1 volga");
    assert_eq!(m2.vanna_price.to_bits(), 0x3f6a2ca1eb654a68, "p2 vanna");
    assert_eq!(m2.volga_price.to_bits(), 0x3f9111e016da40ed, "p2 volga");
}
