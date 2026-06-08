//! Gate tests for the LSV booking-model selector on the pricing route.
//!
//! Three independent claims (Wave-6 Track B, Lesson c — validate against an
//! independent oracle and pin the constants):
//!
//! 1. **DEFAULT path is byte-identical.** Pricing a vanilla / barrier with the
//!    pricing-model field unset (or explicitly `PRICING_MODEL_DEFAULT`) returns a
//!    bit-identical result to the pre-change analytic path. Proven by `to_bits`
//!    equality against the analytic Garman-Kohlhagen / Reiner-Rubinstein price.
//!
//! 2. **LSV-selected barrier prices via the LSV engine.** A single-barrier
//!    knock-out priced through `price_instrument` with `LOCAL_STOCH_VOL` selected
//!    equals a *direct* `celnet_exotics::lsv::LsvModel` reprice (the engine the
//!    route calls) to `~1e-9`. The oracle re-derives the model parameters /
//!    grids **independently** in this file (it does not call `lsv_pricer`), and a
//!    separate assertion pins the canonical variance parameters to their
//!    hand-stated values so the oracle and the code cannot silently share a
//!    mis-stated constant.
//!
//! 3. **LSV on an unsupported product errors clearly.** Selecting LSV for an
//!    Asian option (a product the LSV engine does not price) returns a
//!    `PriceError::UnsupportedModel` (`INVALID_ARGUMENT` at the boundary), never
//!    a silent fallback to the analytic engine.

use celnet_exotics::{
    AdiGrid, ImpliedVolSurface, LsvModel, ParticleConfig, VarianceParams,
    WindowBarrier as ExWindowBarrier,
};
use celnet_proto::{
    Conventions as WireConventions, Instrument, MarketContext, SingleBarrier, StrikeOrDelta,
    Vanilla, WindowBarrier, instrument, strike_or_delta,
};
use celnet_server::pricer::{ConventionSet, PriceError, price_instrument};
use celnet_types::{OptionType, VanillaInputs};

// --- shared market / conventions -------------------------------------------

fn market() -> MarketContext {
    MarketContext::fx(1.30, 0.10, 0.03, 0.01)
}

fn conv() -> ConventionSet {
    ConventionSet::decode(&WireConventions {
        delta_convention: celnet_proto::DeltaConvention::SpotUnadjusted as i32,
        atm_convention: celnet_proto::AtmConvention::AtmForward as i32,
        premium_style: celnet_proto::PremiumStyle::DomesticPips as i32,
        cut: celnet_proto::Cut::NewYork1000 as i32,
        day_count: celnet_proto::DayCount::Act365Fixed as i32,
        settlement: celnet_proto::Settlement::Deliverable as i32,
    })
    .expect("conventions decode")
}

const EXPIRY: f64 = 1.0;

fn vanilla_call(strike: f64, model: celnet_proto::PricingModel) -> Instrument {
    Instrument {
        underlying: None,
        tenor: None,
        expiry_years: EXPIRY,
        quantity: None,
        side: celnet_proto::Side::Buy as i32,
        solve: None,
        pricing_model: model as i32,
        product: Some(instrument::Product::Vanilla(Vanilla {
            option_type: celnet_proto::OptionType::Call as i32,
            strike: Some(StrikeOrDelta {
                spec: Some(strike_or_delta::Spec::Strike(strike)),
            }),
        })),
    }
}

fn down_out_call(strike: f64, barrier: f64, model: celnet_proto::PricingModel) -> Instrument {
    Instrument {
        underlying: None,
        tenor: None,
        expiry_years: EXPIRY,
        quantity: None,
        side: celnet_proto::Side::Buy as i32,
        solve: None,
        pricing_model: model as i32,
        product: Some(instrument::Product::SingleBarrier(SingleBarrier {
            vanilla: Some(Vanilla {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(strike)),
                }),
            }),
            kind: celnet_proto::BarrierKind::KnockOut as i32,
            side: celnet_proto::BarrierSide::Down as i32,
            barrier,
            rebate: 0.0,
            monitoring: celnet_proto::MonitoringStyle::Continuous as i32,
        })),
    }
}

// --- the LSV oracle (independent re-derivation, does NOT call lsv_pricer) ---

/// The canonical variance parameters, hand-stated here so a single mis-stated
/// constant in the production route would diverge from this oracle (Lesson c).
const ORACLE_KAPPA: f64 = 2.0;
const ORACLE_XI: f64 = 0.18;
const ORACLE_RHO: f64 = -0.30;

struct OracleFlatIv {
    sigma: f64,
    spot: f64,
    carry: f64,
}
impl ImpliedVolSurface for OracleFlatIv {
    fn implied_vol(&self, _k: f64, _t: f64) -> f64 {
        self.sigma
    }
    fn forward(&self, t: f64) -> f64 {
        self.spot * (self.carry * t).exp()
    }
}

/// Build the LSV model exactly as the server route documents it. Re-derived in
/// the test independently of `lsv_pricer`, so it is a genuine cross-check.
fn oracle_model(m: &MarketContext) -> LsvModel {
    let sigma = m.vol;
    let v = sigma * sigma;
    let var = VarianceParams::new(v, ORACLE_KAPPA, v, ORACLE_XI, ORACLE_RHO);
    let carry = m.r_dom() - m.r_for();
    let iv = OracleFlatIv {
        sigma,
        spot: m.spot,
        carry,
    };
    let n = 41usize;
    let spot_grid: Vec<f64> = (0..n)
        .map(|k| {
            let x = -0.6 + 1.2 * (k as f64) / ((n - 1) as f64);
            m.spot * x.exp()
        })
        .collect();
    let inputs = VanillaInputs::new(m.spot, m.spot, sigma, EXPIRY, m.r_dom(), m.r_for());
    let particle = ParticleConfig {
        particles: 30_000,
        steps: 40,
        seed: 0x0001_0CA1,
        ..ParticleConfig::default()
    };
    LsvModel::calibrate(inputs, var, &iv, &spot_grid, particle)
}

fn oracle_price_grid() -> AdiGrid {
    AdiGrid {
        x_steps: 160,
        v_steps: 48,
        time_steps: 100,
        ..AdiGrid::default()
    }
}

// --- gate (a): DEFAULT path byte-identical ---------------------------------

#[test]
fn default_path_is_byte_identical_to_analytic_vanilla() {
    let m = market();
    // Unset model (proto3 zero == DEFAULT) and explicit DEFAULT must both equal
    // the analytic Garman-Kohlhagen price bit-for-bit.
    let analytic = celnet_vanilla::price(
        OptionType::Call,
        &VanillaInputs::new(m.spot, 1.30, m.vol, EXPIRY, m.r_dom(), m.r_for()),
    );

    let unset = price_instrument(
        &vanilla_call(1.30, celnet_proto::PricingModel::Default),
        &m,
        &conv(),
    )
    .expect("default vanilla prices");
    assert_eq!(
        unset.greeks.price.to_bits(),
        analytic.to_bits(),
        "DEFAULT vanilla must be byte-identical to the analytic GK price"
    );
}

#[test]
fn default_path_is_byte_identical_to_analytic_barrier() {
    let m = market();
    let instr = down_out_call(1.30, 1.15, celnet_proto::PricingModel::Default);
    let priced = price_instrument(&instr, &m, &conv()).expect("default barrier prices");

    // The analytic Reiner-Rubinstein price the route has always produced.
    let spec = celnet_exotics::SingleBarrier {
        kind: celnet_exotics::BarrierKind {
            up: false,
            style: celnet_exotics::BarrierStyle::KnockOut,
            option: OptionType::Call,
        },
        strike: 1.30,
        barrier: 1.15,
        rebate: 0.0,
    };
    let analytic = celnet_exotics::single_barrier_price(
        &VanillaInputs::new(m.spot, 1.30, m.vol, EXPIRY, m.r_dom(), m.r_for()),
        spec,
    );
    assert_eq!(
        priced.greeks.price.to_bits(),
        analytic.to_bits(),
        "DEFAULT barrier must be byte-identical to the analytic Reiner-Rubinstein price"
    );
}

// --- Lesson c: pin the canonical constants ---------------------------------

#[test]
fn canonical_variance_constants_are_pinned() {
    // Lesson c: bind the production constants to hand-stated literals so a shared
    // mis-statement between the route and this oracle cannot pass silently. The
    // oracle re-derives its own params from the SAME literals (ORACLE_*), and
    // here we assert those literals equal the production constants the server
    // route actually uses — drift in either direction fails loudly.
    assert_eq!(ORACLE_KAPPA, celnet_server::lsv_pricer::LSV_MEAN_REVERSION);
    assert_eq!(ORACLE_XI, celnet_server::lsv_pricer::LSV_VOL_OF_VAR);
    assert_eq!(ORACLE_RHO, celnet_server::lsv_pricer::LSV_CORRELATION);
    // And pin the literals themselves to their hand-computed values.
    assert_eq!(ORACLE_KAPPA, 2.0);
    assert_eq!(ORACLE_XI, 0.18);
    assert_eq!(ORACLE_RHO, -0.30);
    // Feller must hold at a representative FX vol (θ = σ² with σ = 0.10):
    let v = 0.10_f64 * 0.10;
    assert!(
        2.0 * ORACLE_KAPPA * v >= ORACLE_XI * ORACLE_XI,
        "the canonical params must respect Feller at σ=10%"
    );
}

// --- gate (b): LSV-selected barrier == direct LsvModel reprice -------------

#[test]
fn lsv_barrier_matches_direct_exotics_reprice() {
    let m = market();
    let instr = down_out_call(1.30, 1.15, celnet_proto::PricingModel::LocalStochVol);
    let priced =
        price_instrument(&instr, &m, &conv()).expect("LSV barrier prices via the LSV engine");
    assert!(
        priced.std_error.is_none(),
        "the PDE-priced LSV barrier carries no MC std-error"
    );

    // Independent oracle: the SAME engine the route calls, set up by hand here.
    let model = oracle_model(&m);
    let oracle = model.price_barrier_pde(OptionType::Call, 1.30, 1.15, false, oracle_price_grid());

    assert!(
        (priced.greeks.price - oracle).abs() < 1e-9,
        "LSV route {} vs direct exotics reprice {} (diff {:e})",
        priced.greeks.price,
        oracle,
        (priced.greeks.price - oracle).abs()
    );

    // The LSV barrier must also be a genuinely different number from the analytic
    // barrier (otherwise the selector would be a no-op). The LSV model carries
    // skew/stoch-vol the flat-GK barrier does not.
    let analytic = celnet_exotics::single_barrier_price(
        &VanillaInputs::new(m.spot, 1.30, m.vol, EXPIRY, m.r_dom(), m.r_for()),
        celnet_exotics::SingleBarrier {
            kind: celnet_exotics::BarrierKind {
                up: false,
                style: celnet_exotics::BarrierStyle::KnockOut,
                option: OptionType::Call,
            },
            strike: 1.30,
            barrier: 1.15,
            rebate: 0.0,
        },
    );
    assert!(
        (priced.greeks.price - analytic).abs() > 1e-6,
        "the LSV barrier {} should differ from the analytic barrier {}",
        priced.greeks.price,
        analytic
    );
}

#[test]
fn lsv_window_barrier_matches_direct_exotics_reprice() {
    let m = market();
    let instr = Instrument {
        underlying: None,
        tenor: None,
        expiry_years: EXPIRY,
        quantity: None,
        side: celnet_proto::Side::Buy as i32,
        solve: None,
        pricing_model: celnet_proto::PricingModel::LocalStochVol as i32,
        product: Some(instrument::Product::WindowBarrier(WindowBarrier {
            vanilla: Some(Vanilla {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(1.30)),
                }),
            }),
            barrier: 1.50,
            side: celnet_proto::BarrierSide::Up as i32,
            window_start: 0.5,
            window_end: 1.0,
            mc_pairs: 0,
            mc_steps: 0,
            mc_seed: 0,
        })),
    };
    let priced = price_instrument(&instr, &m, &conv())
        .expect("LSV window barrier prices via the LSV engine");

    let model = oracle_model(&m);
    let spec = ExWindowBarrier {
        option: OptionType::Call,
        strike: 1.30,
        barrier: 1.50,
        up: true,
        start: 0.5,
        end: 1.0,
    };
    let oracle = model.price_window_barrier_pde(spec, oracle_price_grid());
    assert!(
        (priced.greeks.price - oracle).abs() < 1e-9,
        "LSV window route {} vs direct exotics reprice {} (diff {:e})",
        priced.greeks.price,
        oracle,
        (priced.greeks.price - oracle).abs()
    );
}

#[test]
fn lsv_window_barrier_mc_carries_std_error() {
    let m = market();
    let instr = Instrument {
        underlying: None,
        tenor: None,
        expiry_years: EXPIRY,
        quantity: None,
        side: celnet_proto::Side::Buy as i32,
        solve: None,
        pricing_model: celnet_proto::PricingModel::LocalStochVol as i32,
        product: Some(instrument::Product::WindowBarrier(WindowBarrier {
            vanilla: Some(Vanilla {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(1.30)),
                }),
            }),
            barrier: 1.50,
            side: celnet_proto::BarrierSide::Up as i32,
            window_start: 0.5,
            window_end: 1.0,
            mc_pairs: 4_000,
            mc_steps: 48,
            mc_seed: 7,
        })),
    };
    let priced = price_instrument(&instr, &m, &conv()).expect("LSV window MC prices");
    let se = priced
        .std_error
        .expect("an MC-priced window barrier carries a std-error");
    assert!(
        se > 0.0 && se.is_finite(),
        "std-error must be positive: {se}"
    );
}

// --- gate (c): LSV on an unsupported product errors clearly ----------------

#[test]
fn lsv_on_unsupported_product_errors_clearly() {
    let m = market();
    // An Asian option: a product the LSV engine does not price.
    let instr = Instrument {
        underlying: None,
        tenor: None,
        expiry_years: EXPIRY,
        quantity: None,
        side: celnet_proto::Side::Buy as i32,
        solve: None,
        pricing_model: celnet_proto::PricingModel::LocalStochVol as i32,
        product: Some(instrument::Product::AsianOption(
            celnet_proto::AsianOption {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: 1.30,
                averaging: celnet_proto::AveragingStyle::Discrete as i32,
                observations: 12,
                method: celnet_proto::AsianMethod::Curran as i32,
                elapsed_avg: 0.0,
                elapsed_weight: 0.0,
            },
        )),
    };
    let err = price_instrument(&instr, &m, &conv())
        .expect_err("LSV on an Asian must be a hard error, never a silent fallback");
    match err {
        PriceError::UnsupportedModel { model, product } => {
            assert_eq!(model, "LOCAL_STOCH_VOL");
            assert_eq!(product, "asian_option");
        }
        other => panic!("expected UnsupportedModel, got {other:?}"),
    }
}

#[test]
fn default_model_on_window_barrier_errors_clearly() {
    // A window barrier has no closed form: selecting the DEFAULT model is a clear
    // error, never a silent fallback to (a non-existent) analytic engine.
    let m = market();
    let instr = Instrument {
        underlying: None,
        tenor: None,
        expiry_years: EXPIRY,
        quantity: None,
        side: celnet_proto::Side::Buy as i32,
        solve: None,
        pricing_model: celnet_proto::PricingModel::Default as i32,
        product: Some(instrument::Product::WindowBarrier(WindowBarrier {
            vanilla: Some(Vanilla {
                option_type: celnet_proto::OptionType::Call as i32,
                strike: Some(StrikeOrDelta {
                    spec: Some(strike_or_delta::Spec::Strike(1.30)),
                }),
            }),
            barrier: 1.50,
            side: celnet_proto::BarrierSide::Up as i32,
            window_start: 0.5,
            window_end: 1.0,
            mc_pairs: 0,
            mc_steps: 0,
            mc_seed: 0,
        })),
    };
    let err = price_instrument(&instr, &m, &conv())
        .expect_err("DEFAULT on a window barrier must be a hard error");
    assert!(
        matches!(err, PriceError::UnsupportedModel { model, .. } if model == "DEFAULT"),
        "expected UnsupportedModel{{DEFAULT}}, got {err:?}"
    );
}
