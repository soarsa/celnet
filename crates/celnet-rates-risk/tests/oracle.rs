//! Non-circular oracle validation of the FI rate-scenario VaR/ES engine (central-core Phase C2a).
//!
//! Every check re-derives the reference a DIFFERENT way than the engine's path:
//!
//! - **(a) reprice-under-shock == independent bump reference (≤1e-12).** The OIS reprice is checked
//!   against a hand discounted-cashflow that computes shocked discount factors by hand
//!   (`exp(−(zᵢ+sᵢ)·tᵢ)`) and against the base-curve analytic parallel-move identity; the bond reprice
//!   is checked against a discount-factor-space curve construction and a fully independent
//!   single-cashflow zero-coupon reprice.
//! - **(b) first-order −DV01·shock with the signed convention.** A small parallel shock's P&L is
//!   matched to `−DD·δ`, where `DD` is an INDEPENDENT closed-form dollar duration, with the residual
//!   shrinking as `O(δ²)` (second-order convexity) and the receive-/pay-fixed sign both checked.
//! - **(c) VaR/ES on a known distribution.** The engine's VaR/ES and scenario P&L are matched to a
//!   fully independent hand pipeline (independent reprice + independent sort/quantile).

use celnet_bond::{AccrualBasis, Bond, PaymentFrequency, price_from_curve};
use celnet_rates::{Curve, FixedPeriod, OisSchedule};
use celnet_types::{Df, Rate, Time};
use time::{Date, Month};

use celnet_rates_risk::{
    CorrelationScenario, CurveId, FiPosition, GirrSensitivity, LadderPoint, RatePillars, RateShock,
    girr_delta_charge, map_ladder_to_vertices, rate_scenario_var_es, scenario_pnls,
    standard_bump_scenarios,
};

const TOL: f64 = 1e-12;

// --------------------------------------------------------------------------------------------
// Independent reference machinery (re-derived, never calling the engine's reprice path).
// --------------------------------------------------------------------------------------------

/// A representative upward-sloping curve: dated pillars `(time, cc zero rate)`.
fn base_rates() -> (Vec<f64>, Vec<f64>) {
    let times = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let rates = vec![0.0430, 0.0410, 0.0400, 0.0405, 0.0420];
    (times, rates)
}

fn rate_pillars() -> RatePillars {
    let (times, rates) = base_rates();
    let pillars = times
        .iter()
        .zip(&rates)
        .map(|(&t, &z)| (Time(t), Rate(z)))
        .collect();
    RatePillars::new(pillars).expect("valid pillars")
}

/// An annual receive-fixed OIS whose payment times coincide with the curve pillars (so its discount
/// factors are pillar-exact — the cleanest independent reprice check).
fn annual_ois(n: usize, fixed_rate: f64, notional: f64, receive_fixed: bool) -> FiPosition {
    let periods = (1..=n)
        .map(|i| FixedPeriod {
            pay: Time(i as f64),
            accrual: Time(1.0),
        })
        .collect();
    FiPosition::OisSwap {
        schedule: OisSchedule::new(Time(0.0), periods).expect("valid schedule"),
        fixed_rate: Rate(fixed_rate),
        notional,
        receive_fixed,
    }
}

/// Independent hand DCF of the annual OIS under a per-pillar shift vector. Discount factors are
/// formed BY HAND as `exp(−(zᵢ+sᵢ)·tᵢ)` (never via the shocked-curve construction), the fixed-leg
/// annuity and the telescoped float leg summed directly. This is a different computation than
/// `ois_pv`-on-a-shocked-curve.
fn hand_ois_pv(
    times: &[f64],
    rates: &[f64],
    accruals: &[f64],
    shifts: &[f64],
    k: f64,
    notional: f64,
    receive_fixed: bool,
) -> f64 {
    let df = |i: usize| (-(rates[i] + shifts[i]) * times[i]).exp();
    let annuity: f64 = (0..times.len()).map(|i| accruals[i] * df(i)).sum();
    let df_start = 1.0; // spot-starting: DF(0) = 1
    let df_maturity = df(times.len() - 1);
    let receive = notional * (k * annuity - (df_start - df_maturity));
    if receive_fixed { receive } else { -receive }
}

/// Independent log-linear ("tent") interpolation, over the pillar times, of the log-discount shift
/// values `{sᵢ·tᵢ}` with the un-shocked curve origin as the knot `(0, 0)`. On a log-linear-on-log-DF
/// curve, `DF_shocked(t) = DF_base(t)·exp(−S(t))` exactly — the identity these tests exploit.
fn tent_s(times: &[f64], shifts: &[f64], t: f64) -> f64 {
    let mut kt = vec![0.0];
    let mut kv = vec![0.0];
    for (i, &ti) in times.iter().enumerate() {
        kt.push(ti);
        kv.push(shifts[i] * ti);
    }
    for w in 1..kt.len() {
        if t <= kt[w] {
            let (t0, t1, v0, v1) = (kt[w - 1], kt[w], kv[w - 1], kv[w]);
            return v0 + (v1 - v0) * (t - t0) / (t1 - t0);
        }
    }
    *kv.last().expect("at least the origin knot")
}

// --------------------------------------------------------------------------------------------
// (a0) The shift identity foundation: a general (non-parallel) shock reproduces the tent identity
//      at intermediate times, ≤1e-12 — validating the shock construction AND `tent_s`.
// --------------------------------------------------------------------------------------------

#[test]
fn key_rate_shock_matches_tent_interpolation_identity() {
    let (times, _) = base_rates();
    let p = rate_pillars();
    // A key-rate bump at pillar 2 (the 3y) plus an arbitrary combined vector.
    for shifts in [
        vec![0.0, 0.0, 30e-4, 0.0, 0.0],
        vec![12e-4, -7e-4, 20e-4, 4e-4, -15e-4],
    ] {
        let shocked = p.shocked_curve(&RateShock::new(shifts.clone())).unwrap();
        for &t in &[0.0, 0.4, 1.0, 1.5, 2.0, 2.9, 3.0, 4.3, 5.0] {
            let base_df = p.base_curve().discount_factor(Time(t)).0;
            let expected = base_df * (-tent_s(&times, &shifts, t)).exp();
            let got = shocked.discount_factor(Time(t)).0;
            assert!(
                (got - expected).abs() <= TOL,
                "tent identity at t={t}: {got} vs {expected} (shifts {shifts:?})"
            );
        }
    }
}

// --------------------------------------------------------------------------------------------
// (a1) OIS reprice-under-shock == independent hand DCF, ≤1e-12.
// --------------------------------------------------------------------------------------------

#[test]
fn ois_reprice_under_shock_matches_hand_dcf() {
    let (times, rates) = base_rates();
    let accruals = vec![1.0; 5];
    let k = 0.0410;
    let n = 1.0; // per-unit notional keeps magnitudes O(0.1) so 1e-12 absolute is strict.
    let p = rate_pillars();
    let pos = annual_ois(5, k, n, true);

    // Base PV self-consistency: engine base == hand DCF with zero shift.
    let zero = vec![0.0; 5];
    let base_engine = pos.pv_on_curve(p.base_curve()).unwrap();
    let base_hand = hand_ois_pv(&times, &rates, &accruals, &zero, k, n, true);
    assert!(
        (base_engine - base_hand).abs() <= TOL,
        "base {base_engine} vs {base_hand}"
    );

    for shifts in [
        vec![10e-4; 5],                          // parallel +10bp
        vec![-10e-4; 5],                         // parallel -10bp
        vec![0.0, 0.0, 25e-4, 0.0, 0.0],         // key-rate +25bp at 3y
        vec![18e-4, -9e-4, 33e-4, 5e-4, -22e-4], // arbitrary historical scenario
    ] {
        let shocked = p.shocked_curve(&RateShock::new(shifts.clone())).unwrap();
        let engine = pos.pv_on_curve(&shocked).unwrap();
        let hand = hand_ois_pv(&times, &rates, &accruals, &shifts, k, n, true);
        assert!(
            (engine - hand).abs() <= TOL,
            "reprice {engine} vs hand {hand} (shifts {shifts:?})"
        );
    }
}

// --------------------------------------------------------------------------------------------
// (a2) Parallel shock == base-curve analytic move (DF_base(t)·exp(−δ·t)) hand DCF, ≤1e-12.
//      This computes the shocked DFs from the BASE curve object + the analytic factor, a path that
//      never touches the shocked-curve construction.
// --------------------------------------------------------------------------------------------

#[test]
fn parallel_shock_reprice_matches_base_curve_analytic_move() {
    let (times, _) = base_rates();
    let accruals = [1.0; 5];
    let k = 0.0410;
    let n = 1.0;
    let p = rate_pillars();
    let pos = annual_ois(5, k, n, true);

    for delta in [5e-4, 20e-4, -12e-4] {
        let shocked = p.shocked_curve(&RateShock::parallel(5, delta)).unwrap();
        let engine = pos.pv_on_curve(&shocked).unwrap();

        // Independent DCF: DF_shocked(tᵢ) = DF_base(tᵢ)·exp(−δ·tᵢ), from the BASE curve.
        let annuity: f64 = (0..5)
            .map(|i| {
                let t = times[i];
                accruals[i] * p.base_curve().discount_factor(Time(t)).0 * (-delta * t).exp()
            })
            .sum();
        let df_mat = p.base_curve().discount_factor(Time(times[4])).0 * (-delta * times[4]).exp();
        let hand = n * (k * annuity - (1.0 - df_mat));
        assert!(
            (engine - hand).abs() <= TOL,
            "delta {delta}: {engine} vs {hand}"
        );
    }
}

// --------------------------------------------------------------------------------------------
// (a3) Bond reprice-under-shock == an independently-CONSTRUCTED shocked curve (discount-factor
//      space), repriced via price_from_curve, ≤1e-12. A different curve-construction path.
// --------------------------------------------------------------------------------------------

fn coupon_bond() -> Bond {
    Bond::new(
        Date::from_calendar_date(2030, Month::January, 1).unwrap(),
        Date::from_calendar_date(2034, Month::January, 1).unwrap(),
        0.035,
        PaymentFrequency::SemiAnnual,
        AccrualBasis::Act365Fixed,
        1.0, // per-unit redemption keeps price O(1) so 1e-12 absolute is strict.
    )
    .expect("valid bond")
}

fn bond_pillars() -> (Vec<f64>, Vec<f64>, RatePillars) {
    let times = vec![1.0, 2.0, 3.0, 5.0, 7.0, 10.0];
    let rates = vec![0.030, 0.032, 0.033, 0.035, 0.036, 0.038];
    let pillars = times
        .iter()
        .zip(&rates)
        .map(|(&t, &z)| (Time(t), Rate(z)))
        .collect();
    (
        times.clone(),
        rates,
        RatePillars::new(pillars).expect("valid"),
    )
}

#[test]
fn bond_reprice_matches_discount_factor_space_construction() {
    let (times, _rates, p) = bond_pillars();
    let bond = coupon_bond();

    for shifts in [
        vec![15e-4; 6],
        vec![0.0, 0.0, 0.0, 40e-4, 0.0, 0.0],
        vec![22e-4, -11e-4, 30e-4, 8e-4, -14e-4, 19e-4],
    ] {
        // Engine path: shift zero-rate pillars, from_zero_rates (inside shocked_curve).
        let shocked_zero = p.shocked_curve(&RateShock::new(shifts.clone())).unwrap();
        let engine = price_from_curve(&bond, &shocked_zero).unwrap();

        // Independent path: build the SAME shocked curve from DISCOUNT-FACTOR pillars, computed as
        // DF_base(tᵢ)·exp(−sᵢ·tᵢ) (a different constructor), then reprice.
        let mut df_pillars: Vec<(Time, Df)> = vec![(Time(0.0), Df(1.0))];
        for (i, &t) in times.iter().enumerate() {
            let df = p.base_curve().discount_factor(Time(t)).0 * (-shifts[i] * t).exp();
            df_pillars.push((Time(t), Df(df)));
        }
        let shocked_df = Curve::from_log_linear_dfs(&df_pillars).unwrap();
        let reference = price_from_curve(&bond, &shocked_df).unwrap();

        assert!(
            (engine - reference).abs() <= TOL,
            "bond reprice {engine} vs df-space {reference} (shifts {shifts:?})"
        );
    }
}

// --------------------------------------------------------------------------------------------
// (a4) Zero-coupon bond: fully independent single-cashflow reprice. price = redemption·DF(T), so
//      pnl = holdings·redemption·DF_base(T)·(exp(−S(T)) − 1), computed from the BASE curve + tent_s.
// --------------------------------------------------------------------------------------------

#[test]
fn zero_coupon_bond_reprice_matches_single_cashflow_oracle() {
    let (times, _rates, p) = bond_pillars();
    let redemption = 1.0;
    let holdings = 3.0;
    let zcb = Bond::new(
        Date::from_calendar_date(2030, Month::January, 1).unwrap(),
        Date::from_calendar_date(2034, Month::January, 1).unwrap(),
        0.0, // zero coupon: the only non-zero cashflow is the redemption at maturity.
        PaymentFrequency::SemiAnnual,
        AccrualBasis::Act365Fixed,
        redemption,
    )
    .unwrap();
    let pos = FiPosition::CashBond {
        bond: zcb,
        holdings,
    };

    // ACT/365F curve time from settlement to maturity (2030-01-01 → 2034-01-01, incl. leap 2032).
    let t_mat = (zcb.maturity() - zcb.settlement()).whole_days() as f64 / 365.0;
    // Sanity: the ZCB curve price is exactly redemption·DF(T_mat) — pins the day-count model.
    let base_df_t = p.base_curve().discount_factor(Time(t_mat)).0;
    let base_price = price_from_curve(&zcb, p.base_curve()).unwrap();
    assert!(
        (base_price - redemption * base_df_t).abs() <= 1e-9,
        "ZCB base price {base_price} vs redemption·DF {}",
        redemption * base_df_t
    );

    let base_pv = pos.pv_on_curve(p.base_curve()).unwrap();
    for shifts in [
        vec![20e-4; 6],
        vec![0.0, 0.0, 0.0, 35e-4, 0.0, 0.0],
        vec![14e-4, -8e-4, 26e-4, 6e-4, -18e-4, 11e-4],
    ] {
        let shocked = p.shocked_curve(&RateShock::new(shifts.clone())).unwrap();
        let engine_pnl = pos.pv_on_curve(&shocked).unwrap() - base_pv;

        let s_t = tent_s(&times, &shifts, t_mat);
        let oracle_pnl = holdings * redemption * base_df_t * ((-s_t).exp() - 1.0);
        assert!(
            (engine_pnl - oracle_pnl).abs() <= TOL,
            "zcb pnl {engine_pnl} vs oracle {oracle_pnl} (shifts {shifts:?})"
        );
    }
}

// --------------------------------------------------------------------------------------------
// (b) First-order check: scenario-P&L ≈ −DV01·shock, with the OIS signed convention and an O(δ²)
//     convexity residual. DD is an INDEPENDENT closed-form dollar duration.
// --------------------------------------------------------------------------------------------

/// Independent closed-form dollar duration (positive magnitude) of the receive-fixed annual OIS for
/// a parallel zero-rate shift: `DD = N·(K·Σ accrualᵢ·tᵢ·DFᵢ + t_N·DF_N)`, `DFᵢ = exp(−zᵢ·tᵢ)`.
/// Derived by hand as `−∂PV/∂δ`, so a +δ parallel rise gives receive-fixed P&L `≈ −DD·δ` (a loss).
fn hand_dollar_duration(times: &[f64], rates: &[f64], accruals: &[f64], k: f64, n: f64) -> f64 {
    let df = |i: usize| (-rates[i] * times[i]).exp();
    let last = times.len() - 1;
    let fixed_term: f64 = (0..times.len())
        .map(|i| accruals[i] * times[i] * df(i))
        .sum();
    n * (k * fixed_term + times[last] * df(last))
}

#[test]
fn first_order_pnl_matches_signed_dollar_duration() {
    let (times, rates) = base_rates();
    let accruals = vec![1.0; 5];
    let k = 0.0410;
    let n = 1.0;
    let p = rate_pillars();
    let recv = annual_ois(5, k, n, true);
    let pay = annual_ois(5, k, n, false);

    let dd = hand_dollar_duration(&times, &rates, &accruals, k, n);
    assert!(dd > 0.0, "dollar duration must be positive, got {dd}");

    let base_recv = recv.pv_on_curve(p.base_curve()).unwrap();
    let base_pay = pay.pv_on_curve(p.base_curve()).unwrap();

    let pnl_recv = |delta: f64| -> f64 {
        let sc = p.shocked_curve(&RateShock::parallel(5, delta)).unwrap();
        recv.pv_on_curve(&sc).unwrap() - base_recv
    };
    let pnl_pay = |delta: f64| -> f64 {
        let sc = p.shocked_curve(&RateShock::parallel(5, delta)).unwrap();
        pay.pv_on_curve(&sc).unwrap() - base_pay
    };

    // Signed convention: a +1bp parallel RISE is a LOSS for the receiver (< 0), a GAIN for the payer.
    let bp = 1e-4;
    assert!(
        pnl_recv(bp) < 0.0,
        "receive-fixed +1bp must lose: {}",
        pnl_recv(bp)
    );
    assert!(
        pnl_pay(bp) > 0.0,
        "pay-fixed +1bp must gain: {}",
        pnl_pay(bp)
    );
    // Pay-fixed is exactly the negation of receive-fixed.
    assert!((pnl_pay(bp) + pnl_recv(bp)).abs() <= TOL);

    // First order: P&L(+1bp) ≈ −DD·1bp within ~1% (the rest is second-order convexity).
    let first_order = -dd * bp;
    assert!(
        (pnl_recv(bp) - first_order).abs() / first_order.abs() < 1e-2,
        "pnl {} vs −DD·δ {first_order}",
        pnl_recv(bp)
    );

    // The residual (P&L − (−DD·δ)) is O(δ²): halving δ quarters it.
    let res1 = (pnl_recv(bp) - (-dd * bp)).abs();
    let res2 = (pnl_recv(bp / 2.0) - (-dd * bp / 2.0)).abs();
    let ratio = res1 / res2;
    assert!(
        (ratio - 4.0).abs() < 0.4,
        "O(δ²) residual ratio {ratio} (expected ≈4)"
    );
}

// --------------------------------------------------------------------------------------------
// (c) VaR/ES on a known scenario distribution == fully independent hand pipeline.
// --------------------------------------------------------------------------------------------

/// Independent VaR/ES: sort the (already independently computed) P&L, take the ⌊(1−α)n⌋-tail loss as
/// VaR and the mean tail loss as ES. Re-derives the reduction convention with no engine call.
fn hand_var_es(mut pnl: Vec<f64>, alpha: f64) -> (f64, f64) {
    pnl.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = pnl.len();
    let tail = (((1.0 - alpha) * n as f64).floor() as usize).max(1).min(n);
    let es = -(pnl[..tail].iter().sum::<f64>() / tail as f64);
    let var = -pnl[tail - 1];
    (var.max(0.0), es.max(0.0))
}

#[test]
fn end_to_end_var_es_matches_independent_pipeline() {
    let (times, rates) = base_rates();
    let accruals = vec![1.0; 5];
    let k = 0.0410;
    let n = 1_000_000.0; // realistic 1mm notional
    let p = rate_pillars();
    let pos = annual_ois(5, k, n, true);
    let positions = vec![pos];

    // A rich shock set: the prescribed ±parallel/±key-rate grid at 50bp plus historical vectors.
    let mut shocks = standard_bump_scenarios(5, 50e-4);
    shocks.push(RateShock::new(vec![40e-4, 20e-4, -10e-4, 30e-4, 60e-4]));
    shocks.push(RateShock::new(vec![-55e-4, -35e-4, -20e-4, -45e-4, -70e-4]));

    let alpha = 0.975;
    let report = rate_scenario_var_es(&positions, &p, &shocks, alpha).unwrap();

    // Independent per-scenario P&L via the hand DCF (never the engine reprice).
    let base_hand = hand_ois_pv(&times, &rates, &accruals, &[0.0; 5], k, n, true);
    let hand_pnl: Vec<f64> = shocks
        .iter()
        .map(|s| hand_ois_pv(&times, &rates, &accruals, s.shifts(), k, n, true) - base_hand)
        .collect();

    // Scenario P&L is row-aligned and matches the independent reprice.
    assert_eq!(report.scenario_pnl.len(), shocks.len());
    for (got, want) in report.scenario_pnl.iter().zip(&hand_pnl) {
        assert!((got - want).abs() <= 1e-6, "scenario pnl {got} vs {want}");
    }

    // The engine's `scenario_pnls` helper agrees with the full report.
    let helper = scenario_pnls(&positions, &p, &shocks).unwrap();
    assert_eq!(helper, report.scenario_pnl);

    // VaR/ES matches the independent sort/quantile of the independent P&L.
    let (hand_var, hand_es) = hand_var_es(hand_pnl, alpha);
    assert!(
        (report.var_es.var - hand_var).abs() <= 1e-6,
        "var {} vs {hand_var}",
        report.var_es.var
    );
    assert!(
        (report.var_es.es - hand_es).abs() <= 1e-6,
        "es {} vs {hand_es}",
        report.var_es.es
    );
    // Base PV is the un-shocked book value.
    assert!((report.base_pv - base_hand).abs() <= 1e-6);
}

// ================================================================================================
// (d) FRTB SbM GIRR delta charge (central-core Phase C2b) — integration oracle through the PUBLIC
//     ladder -> vertices -> charge path the risk cube will use (C2c), validated against independent
//     hand computations (never the engine checking itself).
// ================================================================================================

/// Independent transcription of the MAR21.53 GIRR delta risk weights, aligned with the vertices
/// [0.25, 0.5, 1, 2, 3, 5, 10, 15, 20, 30]y. Re-typed here from the published standard so the
/// integration oracle never reads the engine's own table.
const RW_STD: [f64; 10] = [
    0.017, 0.017, 0.016, 0.013, 0.012, 0.011, 0.011, 0.011, 0.011, 0.011,
];
const VERT_STD: [f64; 10] = [0.25, 0.5, 1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 20.0, 30.0];

/// Independent MAR21.55 medium-scenario same-curve correlation between two vertices.
fn rho_std(i: usize, j: usize) -> f64 {
    let (a, b) = (VERT_STD[i], VERT_STD[j]);
    (-0.03 * (a - b).abs() / a.min(b)).exp().max(0.40)
}

#[test]
fn girr_ladder_path_reproduces_the_hand_worked_example() {
    // The verified worked example expressed as 1 bp DV01 ladders (s = dv01 / 1e-4):
    //   USD: s(2y)=+1000, s(5y)=+2000  ->  dv01 = +0.10, +0.20
    //   EUR: s(10y)=-1500              ->  dv01 = -0.15
    // Driving the PUBLIC map_ladder_to_vertices -> girr_delta_charge path must reproduce the
    // offline-computed aggregate charge 29.90953960876919 (see girr.rs worked_example test).
    let usd_ladder = [
        LadderPoint {
            tenor_years: 2.0,
            dv01: 0.10,
        },
        LadderPoint {
            tenor_years: 5.0,
            dv01: 0.20,
        },
    ];
    let eur_ladder = [LadderPoint {
        tenor_years: 10.0,
        dv01: -0.15,
    }];

    let mut sens = map_ladder_to_vertices(ccy_usd(), CurveId::OIS, &usd_ladder);
    sens.extend(map_ladder_to_vertices(ccy_eur(), CurveId::OIS, &eur_ladder));

    let charge = girr_delta_charge(&sens, CorrelationScenario::Medium);
    assert!(
        (charge.charge - 29.909_539_608_769_19).abs() <= 1e-11,
        "ladder-path charge {} vs hand 29.90953960876919",
        charge.charge
    );
}

#[test]
fn girr_offvertex_ladder_matches_independent_hand_bucket_charge() {
    // A single-currency book with an OFF-vertex ladder point (4y, between the 3y and 5y vertices),
    // so the mapping genuinely interpolates. Independent hand computation of K_b (= the aggregate,
    // one bucket) via the transcribed tables, asserted against the engine at <=1e-12.
    let ladder = [
        LadderPoint {
            tenor_years: 2.0,
            dv01: 0.05,
        }, // on vertex idx 3
        LadderPoint {
            tenor_years: 4.0,
            dv01: 0.08,
        }, // splits idx 4 (3y) / idx 5 (5y) half-half
    ];
    let sens = map_ladder_to_vertices(ccy_usd(), CurveId::OIS, &ladder);
    let charge = girr_delta_charge(&sens, CorrelationScenario::Medium);

    // Independent weighted sensitivities: s = dv01 / 1e-4, split by linear interpolation.
    let ws3 = RW_STD[3] * (0.05 / 1e-4);
    let ws4 = RW_STD[4] * (0.08 / 1e-4 * 0.5);
    let ws5 = RW_STD[5] * (0.08 / 1e-4 * 0.5);
    let sumsq = ws3 * ws3 + ws4 * ws4 + ws5 * ws5;
    let cross =
        2.0 * (rho_std(3, 4) * ws3 * ws4 + rho_std(3, 5) * ws3 * ws5 + rho_std(4, 5) * ws4 * ws5);
    let k_hand = (sumsq + cross).sqrt();

    assert!(
        (charge.charge - k_hand).abs() <= TOL,
        "off-vertex charge {} vs hand {k_hand}",
        charge.charge
    );
    // Offline Python literal, belt-and-suspenders.
    assert!((charge.charge - 15.563_021_331_109_258).abs() <= 1e-11);
}

#[test]
fn girr_directly_constructed_sensitivities_agree_with_ladder_mapping() {
    // Constructing the sensitivities directly (s = dv01 / 1e-4) must agree with the ladder path,
    // pinning the DV01 -> per-unit-rate conversion in the public mapping.
    let via_new = [
        GirrSensitivity::new(ccy_usd(), CurveId::OIS, 3, 0.05 / 1e-4).unwrap(),
        GirrSensitivity::new(ccy_usd(), CurveId::OIS, 5, 0.20 / 1e-4).unwrap(),
    ];
    let via_ladder = map_ladder_to_vertices(
        ccy_usd(),
        CurveId::OIS,
        &[
            LadderPoint {
                tenor_years: 2.0,
                dv01: 0.05,
            },
            LadderPoint {
                tenor_years: 5.0,
                dv01: 0.20,
            },
        ],
    );
    let a = girr_delta_charge(&via_new, CorrelationScenario::Medium).charge;
    let b = girr_delta_charge(&via_ladder, CorrelationScenario::Medium).charge;
    assert!((a - b).abs() <= TOL, "direct {a} vs ladder {b}");
}

fn ccy_usd() -> celnet_types::Ccy {
    celnet_types::Ccy::USD
}
fn ccy_eur() -> celnet_types::Ccy {
    celnet_types::Ccy::EUR
}
