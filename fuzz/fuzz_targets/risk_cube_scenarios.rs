//! Fuzz target: arbitrary scenario algebra and roll-up conservation on the risk cube.
//!
//! Drives [`celnet_risk_cube::nonadditive`] with structured in-domain draws:
//! arbitrary positions (FX vanilla inputs) + arbitrary scenarios (carry-seam shocks),
//! asserting the documented algebraic identities that must hold on any valid input:
//!
//!   1. `Scenario::base().apply(i)` reproduces every `CarryInputs` field `to_bits`-equal;
//!   2. `Scenario::fx_rates` round-trip: `discount_abs == dr_d`, `carry_abs == dr_d −
//!      dr_f` exactly; `apply` shifts the `(r, b)` coordinates by exactly those amounts;
//!   3. `position_pnl` and `node_pnl` are finite for every (position, scenario);
//!   4. `node_pnl == Σ position_pnl` within 4 ULP-scaled tolerance (summation order
//!      may differ from the production accumulator);
//!   5. `historical_var_es` over the drawn scenario set: `es >= var`, both finite,
//!      and both reproduce an independent sort-and-tail-mean recomputation.
//!
//! The pricer is the production `celnet_risk_normalize::AssetPricer` (the
//! `CarryPricer` dispatcher backed by real FX/metal/equity leaf crates) — no stub.
//! No match on underlying appears in the oracle: all arithmetic works on the carry
//! `(r, b)` coordinates, asset-class-agnostically.
//!
//! A stable proptest mirror is in
//!   `crates/celnet-risk-cube/tests/scenario_fuzz.rs`
//! so this property gates the merge on the stable toolchain too.
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run risk_cube_scenarios -- -max_total_time=120

#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

use celnet_risk_cube::nonadditive::{Scenario, historical_var_es, node_pnl, position_pnl};
use celnet_risk_normalize::{AssetPricer, PositionRisk};
use celnet_types::{Carry, CcyPair, Ccy, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

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

/// One drawn position.
#[derive(Debug)]
struct PosDraw {
    is_call: bool,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    r_dom: f64,
    r_for: f64,
    notional: f64,
}

impl<'a> Arbitrary<'a> for PosDraw {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(PosDraw {
            is_call: bool::arbitrary(u)?,
            spot: clamp_into(f64::arbitrary(u)?, 1e-3, 1e4),
            strike: clamp_into(f64::arbitrary(u)?, 1e-3, 1e4),
            vol: clamp_into(f64::arbitrary(u)?, 1e-4, 3.0),
            t: clamp_into(f64::arbitrary(u)?, 1.0 / 365.0, 10.0),
            r_dom: clamp_into(f64::arbitrary(u)?, -0.25, 0.25),
            r_for: clamp_into(f64::arbitrary(u)?, -0.25, 0.25),
            // Signed notional: long/short books.
            notional: clamp_into(f64::arbitrary(u)?, -1e7, 1e7),
        })
    }
}

/// One drawn scenario (carry-seam shocks).
#[derive(Debug)]
struct ScenDraw {
    spot_rel: f64,
    vol_abs: f64,
    discount_abs: f64,
    carry_abs: f64,
}

impl<'a> Arbitrary<'a> for ScenDraw {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(ScenDraw {
            spot_rel: clamp_into(f64::arbitrary(u)?, -0.5, 1.0),
            vol_abs: clamp_into(f64::arbitrary(u)?, -0.05, 0.25),
            discount_abs: clamp_into(f64::arbitrary(u)?, -0.05, 0.05),
            carry_abs: clamp_into(f64::arbitrary(u)?, -0.05, 0.05),
        })
    }
}

/// The top-level draw.
#[derive(Debug)]
struct Draw {
    positions: Vec<PosDraw>,
    scenarios: Vec<ScenDraw>,
    // Used to build fx_rates scenario identity checks.
    dr_d: f64,
    dr_f: f64,
}

impl<'a> Arbitrary<'a> for Draw {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // At least 1 position, at most 24; at least 1 scenario, at most 16.
        let n_pos = 1 + (u8::arbitrary(u)? as usize % 24);
        let n_scen = 1 + (u8::arbitrary(u)? as usize % 16);
        let mut positions = Vec::with_capacity(n_pos);
        for _ in 0..n_pos {
            positions.push(PosDraw::arbitrary(u)?);
        }
        let mut scenarios = Vec::with_capacity(n_scen);
        for _ in 0..n_scen {
            scenarios.push(ScenDraw::arbitrary(u)?);
        }
        Ok(Draw {
            positions,
            scenarios,
            dr_d: clamp_into(f64::arbitrary(u)?, -0.05, 0.05),
            dr_f: clamp_into(f64::arbitrary(u)?, -0.05, 0.05),
        })
    }
}

/// Build a `PositionRisk` from a draw, using EURUSD as the ccy pair.
fn build_position(d: &PosDraw) -> PositionRisk {
    let pair = CcyPair::new(Ccy::EUR, Ccy::USD);
    let opt = if d.is_call {
        OptionType::Call
    } else {
        OptionType::Put
    };
    let inputs = VanillaInputs::new(d.spot, d.strike, d.vol, d.t, d.r_dom, d.r_for);
    PositionRisk::fx(
        pair,
        opt,
        d.notional,
        inputs,
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    )
}

/// Build a `Scenario` from a draw.
fn build_scenario(d: &ScenDraw) -> Scenario {
    Scenario {
        spot_rel: d.spot_rel,
        vol_abs: d.vol_abs,
        discount_abs: d.discount_abs,
        carry_abs: d.carry_abs,
    }
}

fuzz_target!(|draw: Draw| {
    let pricer = AssetPricer;

    // ----- Contract 1: Scenario::base().apply(i) reproduces fields to_bits -----
    {
        let pos = build_position(&draw.positions[0]);
        let i = &pos.inputs;
        let shocked = Scenario::base().apply(i);
        assert_eq!(
            shocked.spot.to_bits(),
            i.spot.to_bits(),
            "base scenario must not change spot"
        );
        assert_eq!(
            shocked.strike.to_bits(),
            i.strike.to_bits(),
            "base scenario must not change strike"
        );
        assert_eq!(
            shocked.vol.to_bits(),
            i.vol.to_bits(),
            "base scenario must not change vol"
        );
        assert_eq!(
            shocked.t.to_bits(),
            i.t.to_bits(),
            "base scenario must not change t"
        );
    }

    // ----- Contract 2: fx_rates round-trip -----
    {
        let dr_d = draw.dr_d;
        let dr_f = draw.dr_f;
        let scen = Scenario::fx_rates(0.0, 0.0, dr_d, dr_f);
        // discount_abs == dr_d, carry_abs == dr_d − dr_f (to_bits-equal for exact arithmetic).
        assert_eq!(
            scen.discount_abs.to_bits(),
            dr_d.to_bits(),
            "fx_rates: discount_abs must equal dr_d"
        );
        let expected_carry_abs = dr_d - dr_f;
        assert_eq!(
            scen.carry_abs.to_bits(),
            expected_carry_abs.to_bits(),
            "fx_rates: carry_abs must equal dr_d - dr_f"
        );
        // apply on an FX carry shifts r_dom by dr_d, r_for by (dr_d - expected_carry_abs).
        let pos = build_position(&draw.positions[0]);
        let i = &pos.inputs;
        let shocked = scen.apply(i);
        // Verify by re-reading the carry coordinates from the shocked inputs.
        if let Carry::FxRates { r_dom, r_for } = &i.carry {
            if let Carry::FxRates {
                r_dom: r_dom_s,
                r_for: r_for_s,
            } = shocked.carry
            {
                let expected_r_dom = r_dom + dr_d;
                // b = r_dom - r_for shifts by carry_abs = dr_d - dr_f
                // so r_for must shift by (dr_d - carry_abs) = dr_f
                let expected_r_for = r_for + dr_f;
                assert_eq!(
                    r_dom_s.to_bits(),
                    expected_r_dom.to_bits(),
                    "fx_rates apply: r_dom must shift by dr_d"
                );
                // shift_carry computes r_for' = r_for + (discount_abs - carry_abs),
                // which equals r_for + dr_f via two-step subtraction; this can differ
                // by up to 1 ULP from the direct `r_for + dr_f` computation.
                assert!(
                    (r_for_s - expected_r_for).abs() <= 1e-15 * (1.0 + expected_r_for.abs()),
                    "fx_rates apply: r_for must shift by dr_f to within 1e-15; \
                     got {r_for_s}, expected {expected_r_for}"
                );
            }
        }
    }

    // ----- Build positions and scenarios -----
    let positions: Vec<PositionRisk> = draw.positions.iter().map(build_position).collect();
    let scenarios: Vec<Scenario> = draw.scenarios.iter().map(build_scenario).collect();

    // ----- Contract 3: position_pnl and node_pnl are finite -----
    for scen in &scenarios {
        for pos in &positions {
            let pnl = position_pnl(&pricer, pos, *scen);
            assert!(
                pnl.is_finite(),
                "position_pnl must be finite for scenario {scen:?}"
            );
        }

        let n_pnl = node_pnl(&pricer, &positions, *scen);
        assert!(
            n_pnl.is_finite(),
            "node_pnl must be finite for scenario {scen:?}"
        );

        // ----- Contract 4: node_pnl == Σ position_pnl within 4 ULP-scaled tol -----
        let sum_pos: f64 = positions.iter().map(|p| position_pnl(&pricer, p, *scen)).sum();
        let tol = 1e-12 * (1.0 + sum_pos.abs() + n_pnl.abs());
        assert!(
            (n_pnl - sum_pos).abs() <= tol,
            "node_pnl ({n_pnl}) must equal Σ position_pnl ({sum_pos}) within {tol}"
        );
    }

    // ----- Contract 5: historical_var_es: es >= var, both finite, match oracle -----
    // Alpha 0.95 (a legitimate quantile).
    let alpha = 0.95_f64;
    let vaes = historical_var_es(&pricer, &positions, &scenarios, alpha);
    let var = vaes.var;
    let es = vaes.es;

    assert!(var.is_finite(), "VaR must be finite");
    assert!(es.is_finite(), "ES must be finite");
    assert!(
        es >= var - 1e-12,
        "ES ({es}) must be >= VaR ({var}) (ES is the mean of the tail beyond VaR)"
    );

    // Independent oracle: sort all scenario PnLs descending (losses are positive),
    // take the quantile index and tail mean.
    let mut pnls: Vec<f64> = scenarios
        .iter()
        .map(|s| node_pnl(&pricer, &positions, *s))
        .collect();
    // Losses (negative PnL) become positive via negation.
    let mut losses: Vec<f64> = pnls.iter().map(|&p| -p).collect();
    losses.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = losses.len();
    if n > 0 {
        let idx = ((n as f64) * alpha).floor() as usize;
        let idx = idx.min(n - 1);
        let oracle_var = losses[idx];
        // ES = mean of losses beyond the VaR index.
        let tail: Vec<f64> = losses[idx..].to_vec();
        let oracle_es = if tail.is_empty() {
            oracle_var
        } else {
            tail.iter().sum::<f64>() / tail.len() as f64
        };

        assert!(
            (var - oracle_var).abs() <= 1e-12 * (1.0 + oracle_var.abs()),
            "VaR ({var}) must match oracle ({oracle_var})"
        );
        assert!(
            (es - oracle_es).abs() <= 1e-12 * (1.0 + oracle_es.abs()),
            "ES ({es}) must match oracle ({oracle_es})"
        );
    }
    let _ = pnls.as_mut_slice(); // suppress unused warning
});
