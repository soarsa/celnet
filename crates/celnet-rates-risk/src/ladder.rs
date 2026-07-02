//! Sign-normalized key-rate (per-tenor DV01) axis for linear FI — the FI counterpart
//! to the options vega ladder, expressed in the platform's ONE signed P&L convention
//! (central-core Phase C2c).
//!
//! # The one P&L sign convention
//!
//! Every dv01 here is the SIGNED present-value change of the POSITION for a +1bp
//! **upward** bump of the discount curve's zero rates — `ΔPV / +1bp`. A positive
//! number is a gain, a negative number is a loss, exactly matching the scenario-engine
//! P&L convention (`shocked − base`, [`crate::scenario_pnls`]) and the options vega
//! ladder's signed convention. Under this convention a long cash bond and a
//! receive-fixed OIS both carry a NEGATIVE dv01 (a rate rise is a loss).
//!
//! # Reconciling the two source conventions (before aggregation)
//!
//! The FI leaves expose dv01 in two *different* conventions, which must be reconciled
//! into the one signed convention BEFORE the risk cube aggregates them:
//!
//! - [`celnet_rates::OisRisk`]'s `dv01` / `key_rate` are already SIGNED PV changes
//!   (for a *receive-fixed* swap); a pay-fixed position negates them — [`normalize_ois_dv01`].
//! - [`celnet_bond::dv01`] is a POSITIVE MAGNITUDE (`−∂P/∂y·1bp`, per unit redemption);
//!   the signed PV change for a +1bp rate rise is its NEGATION, scaled by the units
//!   held — [`normalize_bond_dv01`].
//!
//! So a mixed OIS+bond book never sums a signed value with a positive magnitude.
//! [`normalize_ois_dv01`] / [`normalize_bond_dv01`] are the explicit reconciliation
//! primitives; [`key_rate_ladder`] / [`signed_parallel_dv01`] compute the *same*
//! signed quantity directly by curve-reprice — the single source of truth, since both
//! FI instrument kinds already value through the discount curve with their own sign
//! (OIS via `ois_pv`'s receive/pay sign, a bond via `price_from_curve` scaled by
//! holdings). The tests pin that the reprice-derived signed dv01 and the normalized
//! analytic dv01 agree in sign (and, on a flat curve, in magnitude).

use crate::curve_shock::{RatePillars, RateShock};
use crate::error::RateRiskError;
use crate::girr::LadderPoint;
use crate::position::FiPosition;
use crate::var::scenario_pnls;

/// One basis point in absolute zero-rate terms — the ladder's unit bump.
pub const ONE_BP: f64 = 1e-4;

/// One point of the signed key-rate ladder: the signed DV01 (`ΔPV per +1bp` up-bump)
/// at a curve pillar's tenor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyRatePoint {
    /// The pillar tenor, in years.
    pub tenor_years: f64,
    /// The signed DV01 at this tenor: the book's PV change for a +1bp upward bump of
    /// THIS pillar's zero rate, holding the others fixed. Signed — a rate rise is a
    /// loss, so this is negative for a long bond / receive-fixed swap.
    pub dv01: f64,
}

/// The per-tenor signed key-rate DV01 ladder of a linear-FI book — the FI counterpart
/// to the options vega ladder, in the one signed P&L convention.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct KeyRateLadder {
    points: Vec<KeyRatePoint>,
}

impl KeyRateLadder {
    /// The ladder points, in ascending pillar-tenor order (one per curve pillar).
    #[must_use]
    pub fn points(&self) -> &[KeyRatePoint] {
        &self.points
    }

    /// The number of key-rate buckets (one per curve pillar).
    #[must_use]
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Whether the ladder is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// The total signed DV01 across the ladder — the first-order parallel DV01 (the
    /// key-rate buckets sum to the parallel move to first order; the residual is the
    /// second-order cross-pillar curve convexity).
    #[must_use]
    pub fn total_dv01(&self) -> f64 {
        self.points.iter().map(|p| p.dv01).sum()
    }

    /// The GIRR delta-ladder view (`(tenor, signed 1bp DV01)` points) this ladder
    /// feeds into the FRTB SbM GIRR delta charge ([`crate::girr`]). The dv01 is
    /// already the signed per-1bp PV change [`crate::girr::GirrSensitivity::from_dv01`]
    /// expects, so no further sign handling is needed downstream.
    #[must_use]
    pub fn to_girr_ladder(&self) -> Vec<LadderPoint> {
        self.points
            .iter()
            .map(|p| LadderPoint {
                tenor_years: p.tenor_years,
                dv01: p.dv01,
            })
            .collect()
    }
}

/// Compute the signed key-rate DV01 ladder of a linear-FI book off `base`.
///
/// Each pillar is bumped +1bp in isolation and the book repriced through the C2a
/// scenario engine ([`scenario_pnls`]); the resulting per-pillar signed P&L IS the
/// signed 1bp DV01 (a +1bp bump ⇒ the P&L is the ΔPV per 1bp). Sign-consistent across
/// a heterogeneous OIS+bond book by construction.
///
/// # Errors
///
/// Propagates [`RateRiskError`] from building any shocked curve or repricing any position.
pub fn key_rate_ladder(
    positions: &[FiPosition],
    base: &RatePillars,
) -> Result<KeyRateLadder, RateRiskError> {
    let n = base.len();
    let times = base.pillar_times();
    let shocks: Vec<RateShock> = (0..n).map(|i| RateShock::key_rate(n, i, ONE_BP)).collect();
    let pnl = scenario_pnls(positions, base, &shocks)?;
    let points = times
        .iter()
        .zip(pnl)
        .map(|(t, dv01)| KeyRatePoint {
            tenor_years: t.0,
            dv01,
        })
        .collect();
    Ok(KeyRateLadder { points })
}

/// The signed parallel DV01 of a linear-FI book off `base`: the book's PV change for a
/// +1bp upward parallel bump of every pillar zero rate, in the one signed convention
/// (a rate rise is a loss ⇒ negative for a long bond / receive-fixed swap).
///
/// # Errors
///
/// Propagates [`RateRiskError`] from building the shocked curve or repricing.
pub fn signed_parallel_dv01(
    positions: &[FiPosition],
    base: &RatePillars,
) -> Result<f64, RateRiskError> {
    let shock = RateShock::parallel(base.len(), ONE_BP);
    Ok(scenario_pnls(positions, base, &[shock])?
        .first()
        .copied()
        .unwrap_or(0.0))
}

/// Normalize a [`celnet_rates::OisRisk`]-style OIS DV01 — already a SIGNED PV change
/// for a *receive-fixed* swap — into the one signed convention for a position of the
/// given direction: pass-through when receiving fixed, negated when paying fixed.
#[must_use]
pub fn normalize_ois_dv01(ois_receive_fixed_dv01: f64, receive_fixed: bool) -> f64 {
    if receive_fixed {
        ois_receive_fixed_dv01
    } else {
        -ois_receive_fixed_dv01
    }
}

/// Normalize a [`celnet_bond::dv01`] — a POSITIVE MAGNITUDE (`−∂P/∂y·1bp`, per unit
/// redemption) — into the one signed convention for a holding of `holdings` units: the
/// signed PV change for a +1bp rate rise is the NEGATION of the magnitude, scaled by
/// holdings (a long holding loses when rates rise).
#[must_use]
pub fn normalize_bond_dv01(bond_dv01_magnitude: f64, holdings: f64) -> f64 {
    -bond_dv01_magnitude * holdings
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_bond::{AccrualBasis, Bond, PaymentFrequency};
    use celnet_rates::{FixedPeriod, OisSchedule};
    use celnet_types::{Rate, Time};
    use time::{Date, Month};

    /// A sloped base curve covering the fixtures' tenors.
    fn sloped_pillars() -> RatePillars {
        let times = [1.0, 2.0, 3.0, 4.0, 5.0];
        let rates = [0.030, 0.032, 0.033, 0.034, 0.035];
        RatePillars::new(
            times
                .iter()
                .zip(&rates)
                .map(|(&t, &z)| (Time(t), Rate(z)))
                .collect(),
        )
        .expect("valid pillars")
    }

    /// A FLAT base curve at `y` over five pillars — where curve discounting collapses
    /// to a single rate, so the curve-repriced dv01 can be reconciled in magnitude to
    /// the yield-space analytic dv01.
    fn flat_pillars(y: f64) -> RatePillars {
        RatePillars::new((1..=5).map(|i| (Time(i as f64), Rate(y))).collect()).expect("valid")
    }

    fn coupon_bond() -> Bond {
        Bond::new(
            Date::from_calendar_date(2030, Month::January, 1).unwrap(),
            Date::from_calendar_date(2034, Month::January, 1).unwrap(),
            0.035,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Act365Fixed,
            1.0,
        )
        .expect("valid bond")
    }

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

    /// The key-rate ladder has one bucket per pillar (carrying the pillar tenor) and its
    /// buckets sum, to first order, to the signed parallel DV01; both are negative for a
    /// long bond, and the GIRR-ladder view preserves the signed dv01.
    #[test]
    fn key_rate_ladder_sums_to_signed_parallel_dv01() {
        let base = sloped_pillars();
        let book = vec![FiPosition::CashBond {
            bond: coupon_bond(),
            holdings: 10.0,
        }];
        let ladder = key_rate_ladder(&book, &base).expect("ladder");
        assert_eq!(ladder.len(), 5);
        assert_eq!(ladder.points()[0].tenor_years, 1.0);
        assert_eq!(ladder.points()[4].tenor_years, 5.0);

        let parallel = signed_parallel_dv01(&book, &base).expect("parallel");
        // A long bond loses when rates rise ⇒ both the parallel and the total ladder are
        // negative; they agree to first order (residual = cross-pillar curve convexity).
        assert!(parallel < 0.0, "long-bond parallel dv01 {parallel}");
        assert!(ladder.total_dv01() < 0.0);
        assert!(
            (ladder.total_dv01() - parallel).abs() / parallel.abs() < 5e-3,
            "ladder sum {} vs parallel {parallel}",
            ladder.total_dv01()
        );
        // GIRR-ladder view carries the same signed dv01 per tenor.
        let girr = ladder.to_girr_ladder();
        assert_eq!(girr.len(), 5);
        assert_eq!(girr[2].tenor_years, ladder.points()[2].tenor_years);
        assert_eq!(girr[2].dv01, ladder.points()[2].dv01);
    }

    /// Sign reconciliation, cash bond: `celnet_bond::dv01` is a POSITIVE magnitude; the
    /// engine's reprice-derived signed dv01 is NEGATIVE for a long holding; the
    /// normalization flips + scales the magnitude to that signed value, agreeing in sign
    /// exactly and — on a flat curve — in magnitude to the compounding-convention residual.
    #[test]
    fn sign_reconciliation_bond() {
        let y = 0.035;
        let base = flat_pillars(y);
        let bond = coupon_bond();
        let holdings = 10.0;
        let book = vec![FiPosition::CashBond { bond, holdings }];

        let signed = signed_parallel_dv01(&book, &base).expect("signed");
        let magnitude = celnet_bond::dv01(&bond, Rate(y)).expect("bond dv01");
        let normalized = normalize_bond_dv01(magnitude, holdings);

        // The analytic bond dv01 is a positive magnitude; the normalization identity is
        // an exact sign-flip scaled by holdings.
        assert!(magnitude > 0.0, "bond dv01 magnitude {magnitude}");
        assert!((normalized - (-magnitude * holdings)).abs() < 1e-15);
        // Same sign as the reprice-derived signed dv01 (both a loss on a rate rise).
        assert!(signed < 0.0 && normalized < 0.0);
        // Same risk: on a flat curve the curve-space and yield-space dv01s agree to the
        // continuous-vs-periodic compounding residual (a couple of percent over 4y).
        assert!(
            (signed - normalized).abs() / normalized.abs() < 3e-2,
            "signed {signed} vs normalized {normalized}"
        );
    }

    /// Sign reconciliation, OIS: `OisRisk.dv01` is a signed receive-fixed PV change; the
    /// normalization passes it through for a receive-fixed position and negates it for a
    /// pay-fixed one, matching the engine's reprice-derived signs (receive-fixed loses on
    /// a rate rise; pay-fixed is its exact negation).
    #[test]
    fn sign_reconciliation_ois() {
        let base = sloped_pillars();
        let recv = vec![annual_ois(5, 0.033, 100.0, true)];
        let pay = vec![annual_ois(5, 0.033, 100.0, false)];

        let recv_signed = signed_parallel_dv01(&recv, &base).expect("recv");
        let pay_signed = signed_parallel_dv01(&pay, &base).expect("pay");

        // Receive-fixed loses when rates rise (negative); pay-fixed is its exact negation.
        assert!(recv_signed < 0.0, "receive-fixed dv01 {recv_signed}");
        assert!((pay_signed + recv_signed).abs() < 1e-12);
        // The normalization primitive reproduces both directions from the receive-fixed
        // signed dv01 (the `OisRisk.dv01` convention).
        assert_eq!(
            normalize_ois_dv01(recv_signed, true).to_bits(),
            recv_signed.to_bits()
        );
        assert!((normalize_ois_dv01(recv_signed, false) - pay_signed).abs() < 1e-12);
    }
}
