//! Sequential bootstrap of a self-discounting OIS (USD-SOFR) discount curve.
//!
//! When discounting and projection share one curve, the OIS dependency graph is **acyclic** and the
//! curve is built instrument-by-instrument, short to long (`FI-CURVES-SPEC.md` §5.1): each new
//! maturity contributes exactly one unknown discount factor, solved so the corresponding OIS
//! reprices to its quoted par rate. The per-pillar solve is a one-dimensional monotone root-find
//! (`solver::brent_root`) in the pillar's continuously-compounded zero rate.

use crate::curve::{Curve, CurveError};
use crate::ois::{OisSchedule, is_spot_starting, ois_par_rate};
use crate::solver::{SolverError, brent_root};
use celnet_types::{Df, Rate, Time};

/// A calibrating OIS quote: its fixed-leg schedule and the quoted par rate.
#[derive(Clone, Debug)]
pub struct OisQuote {
    /// The swap's fixed-leg schedule (must start at the curve origin / spot).
    pub schedule: OisSchedule,
    /// The quoted par (fair fixed) rate the bootstrapped curve must reprice exactly.
    pub par_rate: Rate,
}

/// Failure modes of the sequential bootstrap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootstrapError {
    /// No calibrating quotes were supplied.
    NoQuotes,
    /// A quote's schedule does not start at the curve origin (the spot-starting bootstrap case).
    ForwardStartingQuote,
    /// Quote maturities were not strictly increasing.
    NonIncreasingMaturity,
    /// A pillar root-solve failed.
    Solve(SolverError),
    /// The assembled curve was rejected.
    Curve(CurveError),
}

impl core::fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoQuotes => f.write_str("no calibrating OIS quotes supplied"),
            Self::ForwardStartingQuote => {
                f.write_str("a quote schedule does not start at the curve origin (spot)")
            }
            Self::NonIncreasingMaturity => {
                f.write_str("quote maturities must be strictly increasing")
            }
            Self::Solve(e) => write!(f, "pillar root-solve failed: {e}"),
            Self::Curve(e) => write!(f, "assembled curve rejected: {e}"),
        }
    }
}

impl core::error::Error for BootstrapError {}

/// Lower zero-rate bracket for a pillar solve (−50% continuously compounded).
const Z_LO: f64 = -0.5;
/// Upper zero-rate bracket for a pillar solve (+100% continuously compounded).
const Z_HI: f64 = 1.0;
/// Abscissa tolerance for the per-pillar root-find.
const SOLVE_TOL: f64 = 1e-13;
/// Iteration cap for the per-pillar root-find.
const SOLVE_MAX_ITER: usize = 200;

/// Bootstrap a self-discounting curve from spot-starting OIS quotes ordered by increasing maturity.
///
/// Each quote adds one pillar at its maturity, solved so the OIS reprices to its quoted par rate;
/// intermediate fixed-leg payments are interpolated from the curve built so far (log-linear-DF).
///
/// # Errors
///
/// Returns a [`BootstrapError`] if no quotes are supplied, a quote is forward-starting, maturities
/// are not strictly increasing, a pillar solve fails to converge/bracket, or the final curve is
/// rejected.
pub fn bootstrap_ois(quotes: &[OisQuote]) -> Result<Curve, BootstrapError> {
    if quotes.is_empty() {
        return Err(BootstrapError::NoQuotes);
    }

    let mut pillars: Vec<(Time, Df)> = Vec::with_capacity(quotes.len() + 1);
    pillars.push((Time(0.0), Df(1.0)));
    let mut prev_maturity = 0.0;

    for quote in quotes {
        if !is_spot_starting(&quote.schedule) {
            return Err(BootstrapError::ForwardStartingQuote);
        }
        let maturity = quote.schedule.maturity().0;
        if maturity <= prev_maturity {
            return Err(BootstrapError::NonIncreasingMaturity);
        }
        let target = quote.par_rate.0;

        let residual = |z: f64| {
            let df = (-z * maturity).exp();
            let mut candidate = pillars.clone();
            candidate.push((Time(maturity), Df(df)));
            // Always well-formed: origin present, strictly increasing times (maturity > prev), DF > 0.
            let curve = Curve::from_log_linear_dfs(&candidate)
                .expect("bootstrap candidate pillars are well-formed by construction");
            ois_par_rate(&curve, &quote.schedule).0 - target
        };
        let z = brent_root(residual, Z_LO, Z_HI, SOLVE_TOL, SOLVE_MAX_ITER)
            .map_err(BootstrapError::Solve)?;

        pillars.push((Time(maturity), Df((-z * maturity).exp())));
        prev_maturity = maturity;
    }

    Curve::from_log_linear_dfs(&pillars).map_err(BootstrapError::Curve)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ois::{FixedPeriod, ois_par_rate, ois_pv};

    /// A spot-starting annual OIS quote maturing in `n` years with unit-year accruals.
    fn quote(n: usize, par: f64) -> OisQuote {
        let periods = (1..=n)
            .map(|i| FixedPeriod {
                pay: Time(i as f64),
                accrual: Time(1.0),
            })
            .collect();
        OisQuote {
            schedule: OisSchedule::new(Time(0.0), periods).expect("valid schedule"),
            par_rate: Rate(par),
        }
    }

    #[test]
    fn single_pillar_matches_closed_form() {
        // One annual period: par = (1 - DF) / DF  ⇒  DF = 1 / (1 + par).
        let curve = bootstrap_ois(&[quote(1, 0.04)]).expect("bootstraps");
        let expected = 1.0 / 1.04;
        assert!((curve.discount_factor(Time(1.0)).0 - expected).abs() < 1e-10);
    }

    #[test]
    fn reprices_every_quote_to_par() {
        let quotes = [
            quote(1, 0.0420),
            quote(2, 0.0405),
            quote(3, 0.0398),
            quote(4, 0.0402),
            quote(5, 0.0411),
        ];
        let curve = bootstrap_ois(&quotes).expect("bootstraps");
        for q in &quotes {
            let repriced = ois_par_rate(&curve, &q.schedule).0;
            assert!(
                (repriced - q.par_rate.0).abs() < 1e-10,
                "maturity {} repriced {repriced} vs {}",
                q.schedule.maturity().0,
                q.par_rate.0
            );
            assert!(ois_pv(&curve, &q.schedule, q.par_rate, 100.0).abs() < 1e-9);
        }
    }

    #[test]
    fn discount_factors_decrease_for_positive_rates() {
        let curve = bootstrap_ois(&[quote(1, 0.04), quote(2, 0.04), quote(5, 0.04)]).expect("ok");
        let d1 = curve.discount_factor(Time(1.0)).0;
        let d2 = curve.discount_factor(Time(2.0)).0;
        let d5 = curve.discount_factor(Time(5.0)).0;
        assert!(d1 > d2 && d2 > d5 && d5 > 0.0);
    }

    #[test]
    fn rejects_bad_inputs() {
        assert_eq!(bootstrap_ois(&[]).unwrap_err(), BootstrapError::NoQuotes);

        let forward = OisQuote {
            schedule: OisSchedule::new(
                Time(0.5),
                vec![FixedPeriod {
                    pay: Time(1.5),
                    accrual: Time(1.0),
                }],
            )
            .unwrap(),
            par_rate: Rate(0.04),
        };
        assert_eq!(
            bootstrap_ois(&[forward]).unwrap_err(),
            BootstrapError::ForwardStartingQuote
        );

        assert_eq!(
            bootstrap_ois(&[quote(2, 0.04), quote(1, 0.04)]).unwrap_err(),
            BootstrapError::NonIncreasingMaturity
        );
    }
}
