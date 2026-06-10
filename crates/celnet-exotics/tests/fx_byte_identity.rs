//! FX byte-identity gate for the ADR-0008 Wave-B carry-seam migration (spec §6).
//!
//! Every reference value below was captured from the **pre-migration** engines
//! (the FX-only `VanillaInputs` two-rate forms) as raw `f64::to_bits`, on this
//! exact grid, immediately before the engines were rewired onto the agnostic
//! [`celnet_exotics::ExoticInputs`] / `Carry` seam. The gate asserts the migrated
//! engines reproduce those prices **bit-for-bit** (`to_bits` equality — never an
//! epsilon): the whole ADR-0008 claim is that `Carry::FxRates` reproduces the FX
//! arithmetic exactly, so anything looser would hide a regression.
//!
//! Engines covered (the Wave-B set): the Monte-Carlo path core (`mc::*`), the
//! accumulator, the TARF, the lookback closed forms + MC, the forward-start /
//! cliquet family, and the analytic Asian estimators (Turnbull-Wakeman-style
//! moment matching, geometric-conditioning, geometric closed forms — provenance
//! in the engine docs).

use celnet_exotics::ExoticInputs;
use celnet_exotics::asian::{AnalyticAsian, AveragingSchedule, geometric_average_price};
use celnet_exotics::mc::{McConfig, geometric_asian_price, price_asian, price_barrier};
use celnet_exotics::payoff::{ArithmeticAsian, DiscreteBarrier};
use celnet_exotics::{
    Accumulator, AccumulatorMcConfig, Cliquet, CliquetMcConfig, CliquetSchedule, ForwardStart,
    Lookback, LookbackMcConfig, LookbackStyle, Monitoring, RedemptionStyle, Tarf, TarfMcConfig,
    accumulator_price, cliquet_price_capped_mc, cliquet_price_plain, cliquet_price_plain_mc,
    curran_price, fixed_lookback_price, floating_lookback_price, forward_start_price, lookback_mc,
    mc::price_barrier_bgk_shifted, tarf_price, turnbull_wakeman_price,
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
