//! Phase C2c — fold linear-FI rate risk into the risk cube's **non-additive** path.
//!
//! This is the final step of R-a (the operator-chosen true single VaR engine): a
//! portfolio's tail risk becomes ONE cube in which the FI rate-scenario set (C2a) and
//! the FRTB GIRR delta charge (C2b) ride ALONGSIDE the options spot/vol scenarios,
//! rather than living in a separate additive-only rates path.
//!
//! # The modeling choice (surfaced deliberately — two measures, two aggregations)
//!
//! Options risk lives on **spot/vol** scenarios; linear-FI risk lives on **rate**
//! scenarios; they are different risk factors. There are two principled ways to
//! combine them, and this module ships BOTH because the platform needs both measures:
//!
//! 1. **Economic VaR/ES → JOINT bump-and-revalue.** A [`JointScenario`] pairs an
//!    options shock and a rate shock at the *same scenario index* — one market state
//!    (one historical draw / one scenario date). The portfolio's per-scenario P&L is
//!    the SUM of the option-leg and FI-leg P&L, and the VaR/ES is the ONE
//!    [`celnet_core::tail_var_es`] reduction over that joint distribution
//!    ([`node_var_es_joint`]). This captures cross-risk-class diversification (a
//!    rate-up scenario that simultaneously moves FX spot/vol nets within the same
//!    draw) — the economically correct tail, and the reason the cube supports
//!    *aligned* scenarios rather than aggregating marginals. When one side is empty it
//!    reduces bit-for-bit to the existing options path
//!    ([`crate::nonadditive::node_var_es_combined`]) or to the standalone FI path
//!    ([`celnet_rates_risk::rate_scenario_var_es`]).
//!
//! 2. **FRTB regulatory charge → SbM cross-risk-class aggregation.** GIRR is its own
//!    FRTB Standardised-Approach risk class (MAR21). The sensitivities-based method
//!    aggregates risk classes by **simple summation with NO cross-risk-class
//!    correlation**, under each of the three MAR21.6 correlation scenarios, taking the
//!    scenario that maximises the grand total. So the FI GIRR delta charge — computed
//!    per correlation scenario by C2b — is summed, per scenario, into the delta arm of
//!    the cube's [`crate::frtb::FrtbCapital`] ([`girr_delta_sbm`] + [`frtb_with_girr_delta`]);
//!    [`crate::frtb::FrtbCapital::sbm_total`] then takes the correct scenario maximum,
//!    now including the FI GIRR contribution. This is the standard's own aggregation —
//!    NOT a joint bump-and-revalue — because the regulatory number is defined that way.
//!
//! Both aggregations are sound and implemented; this is not a design fork. The joint
//! VaR/ES is the internal economic tail; the SbM sum is the regulatory capital.
//!
//! # What stays byte-identical
//!
//! This module ADDS functions; it changes none of the existing options non-additive
//! entry points. The FI numbers are produced by the C2a engine
//! ([`celnet_rates_risk::scenario_pnls`]) unchanged, so the cube reproduces the
//! standalone FI VaR/ES ≤1e-12 (in fact bit-for-bit), and every options result is
//! `to_bits`-unchanged.

use celnet_core::ExoticLegPricer;
use celnet_core::carry::CarryPricer;
use celnet_risk_normalize::PositionRisk;

use crate::exotic::{ExoticLeg, exotic_node_pnl};
use crate::frtb::{FrtbCapital, SbmCharge};
use crate::nonadditive::{Scenario, VarEs, node_pnl};

use celnet_rates_risk::{
    CorrelationScenario, FiPosition, GirrDeltaCharge, RatePillars, RateRiskError, RateShock,
    key_rate_ladder, scenario_pnls, signed_parallel_dv01,
};

/// The FI key-rate axis surfaced through the cube's output — the FI counterpart to the
/// options vega ladder. Re-exported from the FI engine so the cube exposes it in one place.
pub use celnet_rates_risk::{KeyRateLadder, KeyRatePoint};

/// A joint cross-risk-class scenario: an options spot/vol/carry shock applied to the
/// option legs AND a rate-curve shock applied to the FI legs, as ONE market state.
///
/// The two shocks share a scenario index (one historical draw / one scenario date), so
/// the portfolio's per-scenario P&L is the SUM of the option-leg and FI-leg P&L — the
/// basis of the joint (diversifying) VaR/ES.
#[derive(Clone, Debug, PartialEq)]
pub struct JointScenario {
    /// The options spot/vol/carry shock, applied to the vanilla + exotic option legs.
    pub option: Scenario,
    /// The per-pillar rate-curve shock, applied to the linear-FI legs.
    pub rate: RateShock,
}

impl JointScenario {
    /// A joint scenario pairing an option shock and a rate shock at the same index.
    #[must_use]
    pub fn new(option: Scenario, rate: RateShock) -> Self {
        Self { option, rate }
    }
}

/// A borrowed linear-FI book: the FI positions and the base discount-curve pillars they
/// reprice off. Bundled so the joint-tail entry points stay within the argument budget
/// and so a caller passes the FI side as one handle.
#[derive(Clone, Copy, Debug)]
pub struct FiBook<'a> {
    /// The linear-FI positions (fixed-vs-OIS swaps, cash bonds).
    pub positions: &'a [FiPosition],
    /// The base (un-shocked) discount-curve pillars the rate shocks perturb.
    pub base: &'a RatePillars,
}

impl<'a> FiBook<'a> {
    /// A borrowed FI book from its positions and base curve pillars.
    #[must_use]
    pub fn new(positions: &'a [FiPosition], base: &'a RatePillars) -> Self {
        Self { positions, base }
    }
}

/// **Joint** non-additive VaR/ES over a portfolio's option legs (vanilla + exotic) AND
/// its linear-FI legs, by full bump-and-revalue over aligned [`JointScenario`]s.
///
/// For each joint scenario the option legs reprice under its spot/vol/carry shock (the
/// established [`node_pnl`] / [`exotic_node_pnl`] seams) and the FI legs reprice under
/// its aligned rate shock (the C2a [`scenario_pnls`] engine — the single source of the
/// FI numbers). The per-scenario P&L is their SUM; the distribution is reduced by the
/// one platform-wide [`celnet_core::tail_var_es`]. Row-aligned by construction.
///
/// Reduces bit-for-bit to [`crate::nonadditive::node_var_es_combined`] when the FI book
/// is empty, and to [`celnet_rates_risk::rate_scenario_var_es`] when the option book is
/// empty (each side is the other's identity).
///
/// **Currency caveat (unchanged from the options path):** P&L is summed in each leg's
/// quote currency; a multi-currency portfolio must be numeraire-normalized before this
/// call (the cube does so via `celnet-risk-normalize`), exactly as
/// [`crate::nonadditive::historical_var_es`] documents.
///
/// # Errors
///
/// Propagates [`RateRiskError`] from building any shocked curve or repricing any FI leg.
pub fn node_var_es_joint<P: CarryPricer>(
    pricer: &P,
    exotic_pricer: &dyn ExoticLegPricer,
    positions: &[PositionRisk],
    exotic_legs: &[ExoticLeg],
    fi: FiBook<'_>,
    scenarios: &[JointScenario],
    alpha: f64,
) -> Result<VarEs, RateRiskError> {
    if scenarios.is_empty() {
        return Ok(VarEs { var: 0.0, es: 0.0 });
    }
    // FI leg P&L per joint scenario — row-aligned with `scenarios`. An empty FI book
    // contributes zero and needs no base curve (so the options-only case is a pure
    // options reduction).
    let fi_pnl: Vec<f64> = if fi.positions.is_empty() {
        vec![0.0; scenarios.len()]
    } else {
        let shocks: Vec<RateShock> = scenarios.iter().map(|s| s.rate.clone()).collect();
        scenario_pnls(fi.positions, fi.base, &shocks)?
    };
    // Joint per-scenario portfolio P&L: options (vanilla + exotic) under the option
    // shock PLUS FI under the aligned rate shock — one market state per index.
    let mut pnl: Vec<f64> = scenarios
        .iter()
        .zip(&fi_pnl)
        .map(|(s, &fi)| {
            node_pnl(pricer, positions, s.option)
                + exotic_node_pnl(exotic_pricer, exotic_legs, s.option)
                + fi
        })
        .collect();
    let t = celnet_core::tail_var_es(&mut pnl, alpha);
    Ok(VarEs {
        var: t.var,
        es: t.es,
    })
}

/// The combined non-additive FI risk view the cube surfaces: the joint options+FI tail,
/// the FI key-rate axis, and the FI signed parallel DV01 (all in the one signed convention).
#[derive(Clone, Debug, PartialEq)]
pub struct CombinedTailRisk {
    /// The joint options-spot/vol + FI-rate tail — one non-additive VaR/ES over the union.
    pub joint_var_es: VarEs,
    /// The FI per-tenor signed key-rate DV01 ladder (the FI counterpart to the vega ladder).
    pub key_rate: KeyRateLadder,
    /// The FI signed parallel DV01 (one signed convention; a rate rise is a loss ⇒ negative
    /// for a long bond / receive-fixed swap).
    pub fi_parallel_dv01: f64,
}

/// Compute the [`CombinedTailRisk`] for a portfolio in one pass: the joint tail
/// ([`node_var_es_joint`]) plus the FI key-rate axis and signed parallel DV01.
///
/// # Errors
///
/// Propagates [`RateRiskError`] from the FI reprice / shocked-curve construction.
pub fn combined_tail_risk<P: CarryPricer>(
    pricer: &P,
    exotic_pricer: &dyn ExoticLegPricer,
    positions: &[PositionRisk],
    exotic_legs: &[ExoticLeg],
    fi: FiBook<'_>,
    scenarios: &[JointScenario],
    alpha: f64,
) -> Result<CombinedTailRisk, RateRiskError> {
    let joint_var_es = node_var_es_joint(
        pricer,
        exotic_pricer,
        positions,
        exotic_legs,
        fi,
        scenarios,
        alpha,
    )?;
    let key_rate = key_rate_ladder(fi.positions, fi.base)?;
    let fi_parallel_dv01 = signed_parallel_dv01(fi.positions, fi.base)?;
    Ok(CombinedTailRisk {
        joint_var_es,
        key_rate,
        fi_parallel_dv01,
    })
}

/// Map the FI **FRTB SbM GIRR delta** charge — computed under all three MAR21.6
/// correlation scenarios ([`celnet_rates_risk::girr_delta_charges_all`], returned in
/// `[High, Medium, Low]` order) — into the cube's per-scenario [`SbmCharge`], so it can
/// be aggregated with the options risk classes under the SAME correlation scenario.
#[must_use]
pub fn girr_delta_sbm(charges: &[GirrDeltaCharge; 3]) -> SbmCharge {
    debug_assert!(matches!(charges[0].scenario, CorrelationScenario::High));
    debug_assert!(matches!(charges[1].scenario, CorrelationScenario::Medium));
    debug_assert!(matches!(charges[2].scenario, CorrelationScenario::Low));
    SbmCharge {
        high: charges[0].charge,
        medium: charges[1].charge,
        low: charges[2].charge,
    }
}

/// Fold the FI GIRR delta charge into a FRTB-SbM capital decomposition as the standard
/// **cross-risk-class aggregation** (MAR21.4): GIRR is its own SbM risk class, and the
/// sensitivities-based method aggregates risk classes by SIMPLE SUMMATION with NO
/// cross-risk-class correlation. GIRR delta therefore adds, per correlation scenario,
/// into the delta arm; [`FrtbCapital::sbm_total`] then takes the single scenario that
/// maximises the summed charge (MAR21.6), now including the FI GIRR contribution. The
/// vega, curvature, RRAO and DRC arms are unchanged.
#[must_use]
pub fn frtb_with_girr_delta(base: FrtbCapital, girr_delta: SbmCharge) -> FrtbCapital {
    FrtbCapital {
        delta: SbmCharge {
            high: base.delta.high + girr_delta.high,
            medium: base.delta.medium + girr_delta.medium,
            low: base.delta.low + girr_delta.low,
        },
        ..base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nonadditive::node_var_es_combined;
    use crate::test_support::DigitalTestPricer;
    use celnet_rates::{FixedPeriod, OisSchedule};
    use celnet_rates_risk::{
        CurveId, GirrSensitivity, girr_delta_charges_all, rate_scenario_var_es,
    };
    use celnet_risk_normalize::AssetPricer;
    use celnet_types::{
        Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, Rate, Time, VanillaInputs,
    };

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    fn option_pos(n: f64) -> PositionRisk {
        PositionRisk::fx(
            eurusd(),
            OptionType::Call,
            n,
            VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        )
    }

    fn fi_base() -> RatePillars {
        RatePillars::new(
            [
                (1.0, 0.030),
                (2.0, 0.032),
                (3.0, 0.033),
                (4.0, 0.034),
                (5.0, 0.035),
            ]
            .iter()
            .map(|&(t, z)| (Time(t), Rate(z)))
            .collect(),
        )
        .expect("valid pillars")
    }

    fn recv_ois() -> FiPosition {
        let periods = (1..=5)
            .map(|i| FixedPeriod {
                pay: Time(i as f64),
                accrual: Time(1.0),
            })
            .collect();
        FiPosition::OisSwap {
            schedule: OisSchedule::new(Time(0.0), periods).expect("valid schedule"),
            fixed_rate: Rate(0.033),
            notional: 1_000_000.0,
            receive_fixed: true,
        }
    }

    fn rate_shocks() -> Vec<RateShock> {
        celnet_rates_risk::standard_bump_scenarios(5, 1e-4)
    }

    fn joints(opt: &[Scenario], rates: &[RateShock]) -> Vec<JointScenario> {
        opt.iter()
            .zip(rates)
            .map(|(o, r)| JointScenario::new(*o, r.clone()))
            .collect()
    }

    /// FI-only: the joint path reduces bit-for-bit to the standalone
    /// `rate_scenario_var_es` (empty options ⇒ each scenario's option P&L is 0).
    #[test]
    fn joint_fi_only_equals_standalone_rate_var_es() {
        let base = fi_base();
        let fi = vec![recv_ois()];
        let shocks = rate_shocks();
        let js = joints(&vec![Scenario::base(); shocks.len()], &shocks);
        let got = node_var_es_joint(
            &AssetPricer,
            &DigitalTestPricer,
            &[],
            &[],
            FiBook::new(&fi, &base),
            &js,
            0.95,
        )
        .expect("joint");
        let want = rate_scenario_var_es(&fi, &base, &shocks, 0.95).expect("standalone");
        assert_eq!(got.var.to_bits(), want.var_es.var.to_bits());
        assert_eq!(got.es.to_bits(), want.var_es.es.to_bits());
    }

    /// Options-only: the joint path reduces bit-for-bit to `node_var_es_combined`
    /// (empty FI ⇒ each scenario's FI P&L is 0). Options byte-identity anchor.
    #[test]
    fn joint_options_only_equals_node_var_es_combined() {
        let base = fi_base();
        let positions = vec![option_pos(1_000_000.0), option_pos(-500_000.0)];
        let opt: Vec<Scenario> = (-5..=5)
            .filter(|i| *i != 0)
            .map(|i| Scenario::spot(f64::from(i) * 0.01))
            .collect();
        // The rate shock is irrelevant (FI book empty), but must be pillar-shaped.
        let rates = vec![RateShock::parallel(5, 0.0); opt.len()];
        let js = joints(&opt, &rates);
        let got = node_var_es_joint(
            &AssetPricer,
            &DigitalTestPricer,
            &positions,
            &[],
            FiBook::new(&[], &base),
            &js,
            0.9,
        )
        .expect("joint");
        let want =
            node_var_es_combined(&AssetPricer, &DigitalTestPricer, &positions, &[], &opt, 0.9);
        assert_eq!(got.var.to_bits(), want.var.to_bits());
        assert_eq!(got.es.to_bits(), want.es.to_bits());
    }

    /// Joint (both sides): the joint tail is the ONE reduction of the element-wise SUM
    /// of the option-leg and FI-leg per-scenario P&L — the diversifying joint model,
    /// NOT a marginal aggregation. Proven by reconstructing the joint P&L vector from
    /// the two independent legs and reducing it with the same primitive.
    #[test]
    fn joint_both_sides_reduces_the_summed_pnl_distribution() {
        let base = fi_base();
        let positions = vec![option_pos(1_000_000.0)];
        let fi = vec![recv_ois()];
        let shocks = rate_shocks();
        let opt: Vec<Scenario> = shocks
            .iter()
            .enumerate()
            .map(|(i, _)| Scenario::fx_rates(0.01 * (i as f64 - 4.0), 0.0, 0.0, 0.0))
            .collect();
        let js = joints(&opt, &shocks);

        let got = node_var_es_joint(
            &AssetPricer,
            &DigitalTestPricer,
            &positions,
            &[],
            FiBook::new(&fi, &base),
            &js,
            0.9,
        )
        .expect("joint");

        // Independent reconstruction: option P&L per scenario + FI P&L per scenario,
        // summed element-wise, then reduced by the SAME primitive.
        let fi_pnl = scenario_pnls(&fi, &base, &shocks).expect("fi pnl");
        let mut summed: Vec<f64> = opt
            .iter()
            .zip(&fi_pnl)
            .map(|(o, &fi)| node_pnl(&AssetPricer, &positions, *o) + fi)
            .collect();
        let want = celnet_core::tail_var_es(&mut summed, 0.9);
        assert_eq!(got.var.to_bits(), want.var.to_bits());
        assert_eq!(got.es.to_bits(), want.es.to_bits());
    }

    /// The combined report carries the joint tail, a 5-bucket key-rate ladder (one per
    /// pillar) with negative signed dv01 for a receive-fixed swap, and the signed
    /// parallel dv01, all consistent.
    #[test]
    fn combined_report_surfaces_the_key_rate_axis() {
        let base = fi_base();
        let fi = vec![recv_ois()];
        let shocks = rate_shocks();
        let js = joints(&vec![Scenario::base(); shocks.len()], &shocks);
        let report = combined_tail_risk(
            &AssetPricer,
            &DigitalTestPricer,
            &[],
            &[],
            FiBook::new(&fi, &base),
            &js,
            0.95,
        )
        .expect("combined");
        assert_eq!(report.key_rate.len(), 5);
        assert!(
            report.fi_parallel_dv01 < 0.0,
            "receive-fixed loses on a rate rise"
        );
        // The key-rate ladder totals to the parallel dv01 to first order.
        assert!(
            (report.key_rate.total_dv01() - report.fi_parallel_dv01).abs()
                / report.fi_parallel_dv01.abs()
                < 5e-3
        );
    }

    /// GIRR delta folds into the FRTB delta arm as the SbM cross-risk-class sum: the
    /// delta arm gains the GIRR charge per scenario, the other arms are untouched, and
    /// the SbM total rises by the GIRR contribution under the maximising scenario.
    #[test]
    fn girr_delta_folds_into_frtb_sbm_delta() {
        // The C2b worked GIRR portfolio (USD 2y/5y, EUR 10y).
        let sens = [
            GirrSensitivity::new(Ccy::USD, CurveId::OIS, 3, 1000.0).unwrap(),
            GirrSensitivity::new(Ccy::USD, CurveId::OIS, 5, 2000.0).unwrap(),
            GirrSensitivity::new(Ccy::EUR, CurveId::OIS, 6, -1500.0).unwrap(),
        ];
        let all = girr_delta_charges_all(&sens);
        let girr = girr_delta_sbm(&all);
        // Maps High/Medium/Low index-for-index.
        assert_eq!(girr.high, all[0].charge);
        assert_eq!(girr.medium, all[1].charge);
        assert_eq!(girr.low, all[2].charge);

        let base = FrtbCapital {
            delta: SbmCharge {
                high: 10.0,
                medium: 12.0,
                low: 14.0,
            },
            vega: SbmCharge {
                high: 3.0,
                medium: 3.5,
                low: 4.0,
            },
            curvature: SbmCharge {
                high: 1.0,
                medium: 1.5,
                low: 2.0,
            },
            rrao: 0.7,
            drc: 0.0,
        };
        let folded = frtb_with_girr_delta(base, girr);
        // Delta arm summed per scenario; the other arms untouched.
        assert_eq!(folded.delta.high, base.delta.high + girr.high);
        assert_eq!(folded.delta.medium, base.delta.medium + girr.medium);
        assert_eq!(folded.delta.low, base.delta.low + girr.low);
        assert_eq!(folded.vega, base.vega);
        assert_eq!(folded.curvature, base.curvature);
        assert_eq!(folded.rrao, base.rrao);
        assert_eq!(folded.drc, base.drc);
        // The SbM total is the per-scenario max of the summed arms, now including GIRR.
        let expected = [
            (base.delta.high + girr.high) + base.vega.high + base.curvature.high,
            (base.delta.medium + girr.medium) + base.vega.medium + base.curvature.medium,
            (base.delta.low + girr.low) + base.vega.low + base.curvature.low,
        ]
        .iter()
        .fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        assert!((folded.sbm_total() - expected).abs() < 1e-12);
        assert!(folded.sbm_total() > base.sbm_total());
    }
}
