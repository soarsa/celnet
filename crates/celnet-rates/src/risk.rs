//! Curve risk for an OIS — PV01, DV01, and the key-rate ladder.
//!
//! Three trader-facing sensitivities (`FI-ARCHITECTURE.md` §1 `risk/`, findings §2.5/§2.6):
//!
//! - **PV01** — the analytic annuity risk: the PV change of a one-basis-point move in the fixed
//!   rate, `N · annuity · 1bp`. Exact (the swap PV is linear in the fixed rate).
//! - **DV01** — the PV change for a one-basis-point **parallel** bump of the calibrating par
//!   quotes, by re-bootstrapping and re-pricing.
//! - **Key-rate ladder** — the instrument Jacobian the desk hedges on: bump each calibrating
//!   quote individually, re-bootstrap, re-price; the resulting vector sums (to first order) to the
//!   parallel DV01.
//!
//! DV01 and the ladder use the calibration-instrument bump rather than an opaque internal curve
//! shift, so each ladder bucket maps directly to a tradeable hedge instrument.

use crate::bootstrap::{BootstrapError, OisQuote, bootstrap_ois};
use crate::curve::Curve;
use crate::ois::{OisSchedule, ois_annuity, ois_pv};
use celnet_types::Rate;

/// One basis point, in absolute rate terms.
pub const ONE_BP: f64 = 1e-4;

/// Analytic PV01: the PV change of a one-basis-point fixed-rate move, `N · annuity · 1bp`.
#[must_use]
pub fn pv01(curve: &Curve, schedule: &OisSchedule, notional: f64) -> f64 {
    notional * ois_annuity(curve, schedule) * ONE_BP
}

/// A bumped-curve risk report for an OIS priced off a curve bootstrapped from `quotes`.
#[derive(Clone, Debug)]
pub struct OisRisk {
    /// Present value of the swap on the base (unbumped) curve.
    pub pv: f64,
    /// Analytic PV01 (`N · annuity · 1bp`).
    pub pv01: f64,
    /// PV change for a parallel one-basis-point bump of every calibrating quote.
    pub dv01: f64,
    /// Per-instrument key-rate DV01s, in calibrating-quote order; sums to `dv01` to first order.
    pub key_rate: Vec<f64>,
}

/// Bump a single quote's par rate by `delta` (cloning its schedule).
fn bump(quote: &OisQuote, delta: f64) -> OisQuote {
    OisQuote {
        schedule: quote.schedule.clone(),
        par_rate: Rate(quote.par_rate.0 + delta),
    }
}

/// Compute PV, PV01, DV01, and the key-rate ladder for a receive-fixed OIS.
///
/// The curve is bootstrapped from `quotes`; `schedule`/`fixed_rate`/`notional` define the priced
/// swap. DV01 and the ladder re-bootstrap the curve under bumped quotes (the instrument Jacobian).
///
/// # Errors
///
/// Propagates any [`BootstrapError`] from the base or bumped curve builds.
pub fn ois_risk(
    quotes: &[OisQuote],
    schedule: &OisSchedule,
    fixed_rate: Rate,
    notional: f64,
) -> Result<OisRisk, BootstrapError> {
    let base = bootstrap_ois(quotes)?;
    let pv = ois_pv(&base, schedule, fixed_rate, notional);
    let pv01 = pv01(&base, schedule, notional);

    // Parallel DV01: every calibrating quote bumped +1bp.
    let parallel: Vec<OisQuote> = quotes.iter().map(|q| bump(q, ONE_BP)).collect();
    let bumped_curve = bootstrap_ois(&parallel)?;
    let dv01 = ois_pv(&bumped_curve, schedule, fixed_rate, notional) - pv;

    // Key-rate ladder: each calibrating quote bumped +1bp in isolation.
    let mut key_rate = Vec::with_capacity(quotes.len());
    for (target, _) in quotes.iter().enumerate() {
        let single: Vec<OisQuote> = quotes
            .iter()
            .enumerate()
            .map(|(j, q)| {
                if j == target {
                    bump(q, ONE_BP)
                } else {
                    q.clone()
                }
            })
            .collect();
        let curve = bootstrap_ois(&single)?;
        key_rate.push(ois_pv(&curve, schedule, fixed_rate, notional) - pv);
    }

    Ok(OisRisk {
        pv,
        pv01,
        dv01,
        key_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ois::{FixedPeriod, ois_par_rate};
    use celnet_types::Time;

    fn annual_schedule(n: usize) -> OisSchedule {
        let periods = (1..=n)
            .map(|i| FixedPeriod {
                pay: Time(i as f64),
                accrual: Time(1.0),
            })
            .collect();
        OisSchedule::new(Time(0.0), periods).expect("schedule")
    }

    fn quote(n: usize, par: f64) -> OisQuote {
        OisQuote {
            schedule: annual_schedule(n),
            par_rate: Rate(par),
        }
    }

    fn ladder() -> Vec<OisQuote> {
        vec![
            quote(1, 0.0420),
            quote(2, 0.0410),
            quote(5, 0.0405),
            quote(10, 0.0415),
        ]
    }

    #[test]
    fn pv01_equals_the_fixed_rate_finite_difference() {
        let curve = bootstrap_ois(&ladder()).unwrap();
        let sched = annual_schedule(5);
        let (k, n) = (0.04, 100.0);
        let diff = ois_pv(&curve, &sched, Rate(k + ONE_BP), n) - ois_pv(&curve, &sched, Rate(k), n);
        // The swap PV is linear in the fixed rate, so the analytic PV01 is exact.
        assert!((diff - pv01(&curve, &sched, n)).abs() < 1e-12);
    }

    #[test]
    fn key_rate_ladder_sums_to_parallel_dv01() {
        let quotes = ladder();
        let sched = annual_schedule(7);
        let risk = ois_risk(&quotes, &sched, Rate(0.041), 100.0).unwrap();
        assert_eq!(risk.key_rate.len(), quotes.len());
        let summed: f64 = risk.key_rate.iter().sum();
        // The single-bump ladder agrees with the simultaneous parallel bump to first order; the
        // residual is the second-order curve non-additivity (cross-gamma between pillars), ~0.04%.
        assert!(
            (summed - risk.dv01).abs() / risk.dv01.abs() < 5e-3,
            "ladder sum {summed} vs dv01 {}",
            risk.dv01
        );
    }

    #[test]
    fn par_swap_dv01_offsets_pv01() {
        // A par receive-fixed swap loses ~N·annuity·1bp when rates rise 1bp, so DV01 ≈ −PV01.
        let quotes = ladder();
        let curve = bootstrap_ois(&quotes).unwrap();
        let sched = annual_schedule(5);
        let par = ois_par_rate(&curve, &sched);
        let risk = ois_risk(&quotes, &sched, par, 100.0).unwrap();
        assert!(
            risk.pv.abs() < 1e-9,
            "par swap PV should be ~0, got {}",
            risk.pv
        );
        assert!(
            (risk.dv01 + risk.pv01).abs() / risk.pv01 < 1e-3,
            "dv01 {} pv01 {}",
            risk.dv01,
            risk.pv01
        );
    }

    #[test]
    fn propagates_bootstrap_errors() {
        let sched = annual_schedule(2);
        assert_eq!(
            ois_risk(&[], &sched, Rate(0.04), 100.0).unwrap_err(),
            BootstrapError::NoQuotes
        );
    }
}
