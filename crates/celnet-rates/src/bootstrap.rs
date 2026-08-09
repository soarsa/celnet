//! Sequential bootstrap of a self-discounting discount curve from a mixed instrument ladder.
//!
//! When discounting and projection share one curve the calibration dependency graph is **acyclic**
//! and the curve is built instrument-by-instrument, short to long (`FI-CURVES-SPEC.md` §5.1): each
//! new maturity contributes exactly one unknown discount factor, solved so the corresponding
//! instrument reprices to its quote. The per-pillar solve is a one-dimensional monotone root-find
//! (`solver::brent_root`) in the pillar's continuously-compounded zero rate — except for the
//! closed-form short-end pillars (deposits, convexity-adjusted futures) whose discount factor is a
//! direct simple-interest division.
//!
//! [`bootstrap_curve`] is the generalised entry point: it walks a heterogeneous
//! [`CalibrationInstrument`] ladder (cash deposits at the front, a STIR-futures strip or FRAs in the
//! middle, OIS and vanilla IRS to the long end), ordering by maturity and adding exactly one pillar
//! per instrument. [`bootstrap_ois`] is the OIS-only special case, kept as a thin wrapper that maps
//! its quotes onto [`CalibrationInstrument::Ois`] and delegates — byte-identical to the original
//! self-discounting OIS build.

use crate::curve::{Curve, CurveError, Interpolation};
use crate::deposit::Deposit;
use crate::fra::{Fra, fra_par_rate};
use crate::futures_strip::{StirFuturesQuote, implied_forward_rate};
use crate::ois::{OisSchedule, is_spot_starting, ois_par_rate};
use crate::solver::{SolverError, brent_root};
use crate::vanilla_swap::{SwapLeg, fixed_annuity, float_leg_value};
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
    /// No calibrating instruments were supplied to the generalised bootstrap.
    NoInstruments,
    /// A quote's schedule does not start at the curve origin (the spot-starting bootstrap case).
    ForwardStartingQuote,
    /// Quote maturities were not strictly increasing (after ordering by maturity).
    NonIncreasingMaturity,
    /// A closed-form pillar (deposit / convexity-adjusted future) implied a non-positive discount
    /// factor — a degenerate quote the curve cannot carry.
    UnsolvablePillar,
    /// A pillar root-solve failed.
    Solve(SolverError),
    /// The assembled curve was rejected.
    Curve(CurveError),
    /// The monotone-convex refinement did not reach a repricing fixed point within the sweep cap.
    ///
    /// Monotone-convex-on-forwards interpolation is non-local — placing a later pillar shifts the
    /// preceding segment's shape — so the sequential build is refined with Gauss-Seidel sweeps until
    /// every calibrating instrument reprices to par simultaneously. A curve so pathological that the
    /// sweep never contracts within [`REFINE_MAX_SWEEPS`] surfaces here rather than returning a curve
    /// that silently fails to reprice its own quotes. (The log-linear scheme is local and single-pass,
    /// so it never raises this.)
    DidNotConverge,
}

impl core::fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoQuotes => f.write_str("no calibrating OIS quotes supplied"),
            Self::NoInstruments => f.write_str("no calibrating instruments supplied"),
            Self::ForwardStartingQuote => {
                f.write_str("a quote schedule does not start at the curve origin (spot)")
            }
            Self::NonIncreasingMaturity => {
                f.write_str("instrument maturities must be strictly increasing")
            }
            Self::UnsolvablePillar => {
                f.write_str("a closed-form pillar implied a non-positive discount factor")
            }
            Self::Solve(e) => write!(f, "pillar root-solve failed: {e}"),
            Self::Curve(e) => write!(f, "assembled curve rejected: {e}"),
            Self::DidNotConverge => {
                f.write_str("monotone-convex refinement did not converge to a repricing curve")
            }
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

/// Convergence threshold (sup relative pillar-DF change across a sweep) for the monotone-convex
/// Gauss-Seidel refinement. Chosen just above the per-pillar solver's own abscissa noise so the
/// sweep declares convergence once the fixed point is stable to the inner solver's precision.
const REFINE_TOL: f64 = 1e-13;
/// Sweep cap for the monotone-convex refinement (well-behaved ladders contract in a handful of
/// sweeps; the cap only guards a pathological non-contracting curve — see [`BootstrapError::DidNotConverge`]).
const REFINE_MAX_SWEEPS: usize = 256;

/// Build a curve from `(time, DF)` pillars under the selected interpolation scheme.
///
/// The single seam that routes the bootstrap's intermediate-cashflow interpolation and its final
/// assembly through the chosen [`Interpolation`]. For [`Interpolation::LogLinearDf`] this is exactly
/// [`Curve::from_log_linear_dfs`] — so the log-linear build stays byte-identical to the original.
fn build_curve(pillars: &[(Time, Df)], interp: Interpolation) -> Result<Curve, CurveError> {
    match interp {
        Interpolation::LogLinearDf => Curve::from_log_linear_dfs(pillars),
        Interpolation::MonotoneConvexForward => Curve::from_monotone_convex_dfs(pillars),
    }
}

/// A vanilla fixed-vs-float IRS calibration quote: its two coterminal leg schedules and the quoted
/// par (fair fixed) rate the bootstrapped curve must reprice.
///
/// Mirrors [`OisQuote`] (a schedule paired with a target rate) for the two-leg swap: the legs carry
/// the conventionful day-count accruals and payment coordinates, and `par_rate` is the calibration
/// target. The swap's notional and traded fixed rate are irrelevant to calibration — the par rate
/// `float_leg_value / fixed_annuity` depends only on the curve and the two schedules.
#[derive(Clone, Debug)]
pub struct VanillaIrsQuote {
    /// The fixed-leg schedule (carries the fixed-leg day-count accruals).
    pub fixed_leg: SwapLeg,
    /// The float-leg schedule (coterminal with the fixed leg).
    pub float_leg: SwapLeg,
    /// The quoted par (fair fixed) rate the bootstrapped curve must reprice exactly.
    pub par_rate: Rate,
}

/// A single calibrating instrument of a mixed curve-build ladder.
///
/// Each variant carries its family's schedule/conventions plus its quote, and contributes exactly
/// one pillar at its maturity to the sequential bootstrap ([`bootstrap_curve`]). Adding a sixth
/// family is one new variant plus one arm in [`CalibrationInstrument::maturity`] and the pillar
/// solver — the asset-class-agnostic dispatch seam.
#[derive(Clone, Debug)]
pub enum CalibrationInstrument {
    /// A money-market cash deposit (closed-form front pillar `1/(1+r·τ)`).
    Deposit(Deposit),
    /// A convexity-adjusted STIR future (closed-form forward pillar off the curve so far).
    StirFuture(StirFuturesQuote),
    /// A single-period forward rate agreement (par fixed rate root-solved).
    Fra(Fra),
    /// A spot-starting overnight-index swap (par fixed rate root-solved).
    Ois(OisQuote),
    /// A coterminal vanilla fixed-vs-float IRS (par fixed rate root-solved).
    VanillaIrs(VanillaIrsQuote),
}

impl CalibrationInstrument {
    /// The instrument's final pay/settle time on the curve year-fraction axis — its pillar
    /// maturity, and the key that orders the ladder short → long.
    #[must_use]
    pub fn maturity(&self) -> Time {
        match self {
            Self::Deposit(d) => d.maturity,
            Self::StirFuture(q) => q.future.fixing_end(),
            Self::Fra(f) => f.maturity,
            Self::Ois(q) => q.schedule.maturity(),
            Self::VanillaIrs(q) => q.fixed_leg.maturity(),
        }
    }

    /// Solve this instrument's pillar discount factor into `pillars[idx]`, under interpolation
    /// scheme `interp`, holding every other pillar in `pillars` fixed.
    ///
    /// `pillars[idx].0` is the instrument's maturity; the slot's discount factor is (re)solved so
    /// the instrument reprices to its quote. Deposits and convexity-adjusted futures are closed-form
    /// divisions (the future reads its fixing-start discount factor off the pillars *before* `idx`,
    /// interpolated under `interp`); FRAs, OIS, and IRS invert their par-rate identity with a
    /// one-dimensional Brent root-find, building the trial curve from the full pillar set with the
    /// `idx` slot overwritten.
    ///
    /// The sequential build calls this with `idx` the newest (largest-time) slot and only shorter
    /// pillars populated — recovering the classic short→long single sweep. The monotone-convex
    /// refinement calls it repeatedly with later pillars also present, so the non-local knot-forward
    /// coupling is captured (Gauss-Seidel). For [`Interpolation::LogLinearDf`] the interpolation is
    /// local, so the sequential sweep already reprices exactly and no refinement runs.
    fn solve_into(
        &self,
        pillars: &mut [(Time, Df)],
        idx: usize,
        interp: Interpolation,
    ) -> Result<(), BootstrapError> {
        let maturity = pillars[idx].0.0;
        let df = match self {
            Self::Deposit(d) => {
                let growth = 1.0 + d.rate.0 * d.accrual;
                if growth <= 0.0 {
                    return Err(BootstrapError::UnsolvablePillar);
                }
                Df(1.0 / growth)
            }
            Self::StirFuture(q) => {
                // Anchor the future off the discount factor to its fixing start, interpolated from
                // the pillars before this one; debias the quoted rate by the convexity adjustment.
                let curve = build_curve(&pillars[..idx], interp).map_err(BootstrapError::Curve)?;
                let t1 = q.future.fixing_start();
                let t2 = q.future.fixing_end();
                let df_start = curve.discount_factor(t1).0;
                let tau = t2.0 - t1.0;
                let forward = implied_forward_rate(q).0;
                let growth = 1.0 + forward * tau;
                if growth <= 0.0 {
                    return Err(BootstrapError::UnsolvablePillar);
                }
                Df(df_start / growth)
            }
            Self::Fra(f) => {
                let target = f.fixed_rate.0;
                let z = solve_pillar_zero(pillars, idx, maturity, interp, |curve| {
                    fra_par_rate(curve, f).0 - target
                })?;
                Df((-z * maturity).exp())
            }
            Self::Ois(q) => {
                let target = q.par_rate.0;
                let z = solve_pillar_zero(pillars, idx, maturity, interp, |curve| {
                    ois_par_rate(curve, &q.schedule).0 - target
                })?;
                Df((-z * maturity).exp())
            }
            Self::VanillaIrs(q) => {
                let target = q.par_rate.0;
                let z = solve_pillar_zero(pillars, idx, maturity, interp, |curve| {
                    float_leg_value(curve, &q.float_leg) / fixed_annuity(curve, &q.fixed_leg)
                        - target
                })?;
                Df((-z * maturity).exp())
            }
        };
        pillars[idx].1 = df;
        Ok(())
    }
}

/// Solve a pillar's continuously-compounded zero rate so that `par_residual` (model par minus quote)
/// vanishes, overwriting the `idx` slot of `pillars` with the trial pillar `(maturity, exp(−z·maturity))`.
///
/// The trial discount factor keeps the candidate pillars well-formed for every `z` (origin present,
/// strictly increasing times since `maturity` equals `pillars[idx].0` which strictly separates its
/// neighbours, DF strictly positive), so the curve constructor cannot fail mid-solve under either
/// interpolation scheme.
fn solve_pillar_zero<F>(
    pillars: &[(Time, Df)],
    idx: usize,
    maturity: f64,
    interp: Interpolation,
    par_residual: F,
) -> Result<f64, BootstrapError>
where
    F: Fn(&Curve) -> f64,
{
    let residual = |z: f64| {
        let df = (-z * maturity).exp();
        let mut candidate = pillars.to_vec();
        candidate[idx] = (Time(maturity), Df(df));
        let curve = build_curve(&candidate, interp)
            .expect("bootstrap candidate pillars are well-formed by construction");
        par_residual(&curve)
    };
    brent_root(residual, Z_LO, Z_HI, SOLVE_TOL, SOLVE_MAX_ITER).map_err(BootstrapError::Solve)
}

/// Refine a sequentially-built pillar set to the monotone-convex repricing fixed point.
///
/// Monotone-convex-on-forwards interpolation is non-local: adding a pillar reshapes the *preceding*
/// segment (its far knot forward gains a right-neighbour), perturbing the intermediate-cashflow
/// discount factors of an already-calibrated instrument. A single short→long sweep therefore leaves
/// residual repricing error at instruments with off-pillar cashflows. Re-solving every pillar
/// against the *full* current curve, sweep after sweep (Gauss-Seidel), drives all residuals to zero
/// simultaneously; the coupling is one-pillar-local and contractive, so the sweep converges quickly.
///
/// `ordered` is the maturity-sorted instrument list aligned with `pillars[1..]` (pillar `j+1` is
/// owned by `ordered[j]`); `pillars` already carries the sequential first guess.
fn refine_monotone_convex(
    ordered: &[&CalibrationInstrument],
    pillars: &mut [(Time, Df)],
) -> Result<(), BootstrapError> {
    for _ in 0..REFINE_MAX_SWEEPS {
        let mut max_rel_change = 0.0_f64;
        for (j, instrument) in ordered.iter().enumerate() {
            let idx = j + 1;
            let before = pillars[idx].1.0;
            instrument.solve_into(pillars, idx, Interpolation::MonotoneConvexForward)?;
            let after = pillars[idx].1.0;
            let rel = ((after - before) / before).abs();
            if rel > max_rel_change {
                max_rel_change = rel;
            }
        }
        if max_rel_change <= REFINE_TOL {
            return Ok(());
        }
    }
    Err(BootstrapError::DidNotConverge)
}

/// Bootstrap a self-discounting curve from a mixed ladder of calibrating instruments.
///
/// Instruments are ordered by maturity (final pay/settle time) and walked short → long; each adds
/// exactly one pillar at its maturity, built incrementally on the curve so far (log-linear-DF):
/// deposits and convexity-adjusted futures are closed-form pillars, while FRAs, OIS, and IRS invert
/// their par-rate identity with a Brent root-find. Intermediate cashflow discount factors are
/// interpolated from the curve built so far, so the build is a single short-to-long sweep.
///
/// # Errors
///
/// Returns a [`BootstrapError`] if no instruments are supplied, two instruments share a maturity
/// (non-strictly-increasing after ordering), a closed-form pillar is degenerate
/// ([`BootstrapError::UnsolvablePillar`]), a root-solve fails to converge/bracket, or the assembled
/// curve is rejected.
pub fn bootstrap_curve(instruments: &[CalibrationInstrument]) -> Result<Curve, BootstrapError> {
    bootstrap_curve_with(instruments, Interpolation::LogLinearDf)
}

/// Bootstrap a self-discounting curve from a mixed instrument ladder under a selected interpolation
/// scheme, so the produced curve reprices every calibrating instrument to its quote *under that
/// scheme* (intermediate-cashflow discount factors interpolated under `interp` during the solve).
///
/// [`bootstrap_curve`] is exactly this with [`Interpolation::LogLinearDf`] — byte-identical to the
/// original single short→long sweep, because log-linear interpolation is local (a later pillar never
/// reshapes an earlier segment) so one sweep reprices exactly. [`Interpolation::MonotoneConvexForward`]
/// is non-local, so the sequential first guess is refined with Gauss-Seidel sweeps
/// ([`refine_monotone_convex`]) until every instrument reprices to par simultaneously.
///
/// # Errors
///
/// Returns a [`BootstrapError`] on the same conditions as [`bootstrap_curve`], plus
/// [`BootstrapError::DidNotConverge`] if the monotone-convex refinement fails to contract within the
/// sweep cap.
pub fn bootstrap_curve_with(
    instruments: &[CalibrationInstrument],
    interp: Interpolation,
) -> Result<Curve, BootstrapError> {
    if instruments.is_empty() {
        return Err(BootstrapError::NoInstruments);
    }

    // Order short → long without disturbing the supplied order of equal-maturity ties (stable).
    let mut ordered: Vec<&CalibrationInstrument> = instruments.iter().collect();
    ordered.sort_by(|a, b| a.maturity().0.total_cmp(&b.maturity().0));

    let mut pillars: Vec<(Time, Df)> = Vec::with_capacity(instruments.len() + 1);
    pillars.push((Time(0.0), Df(1.0)));
    let mut prev_maturity = 0.0;

    // Sequential first guess: one pillar per instrument, short → long, solved off the curve so far.
    for instrument in &ordered {
        let maturity = instrument.maturity();
        if maturity.0 <= prev_maturity {
            return Err(BootstrapError::NonIncreasingMaturity);
        }
        pillars.push((maturity, Df(1.0)));
        let idx = pillars.len() - 1;
        instrument.solve_into(&mut pillars, idx, interp)?;
        prev_maturity = maturity.0;
    }

    // Log-linear is local — the single sweep reprices exactly. Monotone-convex is non-local, so
    // refine the sequential guess to the repricing fixed point before assembling the final curve.
    if interp == Interpolation::MonotoneConvexForward {
        refine_monotone_convex(&ordered, &mut pillars)?;
    }

    build_curve(&pillars, interp).map_err(BootstrapError::Curve)
}

/// Bootstrap a self-discounting curve from spot-starting OIS quotes ordered by increasing maturity.
///
/// The OIS-only special case of [`bootstrap_curve`]: each quote adds one pillar at its maturity,
/// solved so the OIS reprices to its quoted par rate, with intermediate fixed-leg payments
/// interpolated from the curve built so far. Spot-starting and strictly-increasing-maturity are
/// pre-validated here (an OIS calibration requires the telescoping spot anchor) before the quotes
/// are mapped onto [`CalibrationInstrument::Ois`] and delegated — the produced pillars are
/// byte-identical to building each OIS pillar directly.
///
/// # Errors
///
/// Returns a [`BootstrapError`] if no quotes are supplied, a quote is forward-starting, maturities
/// are not strictly increasing, a pillar solve fails to converge/bracket, or the final curve is
/// rejected.
pub fn bootstrap_ois(quotes: &[OisQuote]) -> Result<Curve, BootstrapError> {
    bootstrap_ois_with(quotes, Interpolation::LogLinearDf)
}

/// Bootstrap a self-discounting curve from spot-starting OIS quotes under a selected interpolation
/// scheme, so the produced curve reprices every calibrating OIS to its quoted par rate *under that
/// scheme*.
///
/// [`bootstrap_ois`] is exactly this with [`Interpolation::LogLinearDf`] — byte-identical to the
/// original self-discounting OIS build. [`Interpolation::MonotoneConvexForward`] bootstraps *with*
/// monotone-convex intermediate-cashflow interpolation and refines to the repricing fixed point, so
/// (unlike naively re-wrapping log-linear pillar discount factors under the smooth scheme) every
/// calibrating OIS — including those with off-pillar annual cashflows — reprices to par.
///
/// # Errors
///
/// Returns a [`BootstrapError`] if no quotes are supplied, a quote is forward-starting, maturities
/// are not strictly increasing, a pillar solve fails to converge/bracket, the final curve is
/// rejected, or (monotone-convex only) the refinement fails to converge.
pub fn bootstrap_ois_with(
    quotes: &[OisQuote],
    interp: Interpolation,
) -> Result<Curve, BootstrapError> {
    if quotes.is_empty() {
        return Err(BootstrapError::NoQuotes);
    }

    let mut prev_maturity = 0.0;
    for quote in quotes {
        if !is_spot_starting(&quote.schedule) {
            return Err(BootstrapError::ForwardStartingQuote);
        }
        let maturity = quote.schedule.maturity().0;
        if maturity <= prev_maturity {
            return Err(BootstrapError::NonIncreasingMaturity);
        }
        prev_maturity = maturity;
    }

    let instruments: Vec<CalibrationInstrument> = quotes
        .iter()
        .map(|q| CalibrationInstrument::Ois(q.clone()))
        .collect();
    bootstrap_curve_with(&instruments, interp)
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

    /// Parity: the OIS wrapper is byte-identical to building the same OIS ladder through the
    /// generalised [`bootstrap_curve`]. Every pillar discount factor must match to the last bit, so
    /// reimplementing `bootstrap_ois` as a thin wrapper introduces zero drift.
    #[test]
    fn ois_wrapper_is_byte_identical_to_generalised_path() {
        let quotes = [
            quote(1, 0.0420),
            quote(2, 0.0405),
            quote(3, 0.0398),
            quote(5, 0.0411),
            quote(10, 0.0415),
        ];
        let via_wrapper = bootstrap_ois(&quotes).expect("wrapper");

        let instruments: Vec<CalibrationInstrument> = quotes
            .iter()
            .map(|q| CalibrationInstrument::Ois(q.clone()))
            .collect();
        let via_general = bootstrap_curve(&instruments).expect("generalised");

        for q in &quotes {
            let t = q.schedule.maturity();
            assert_eq!(
                via_wrapper.discount_factor(t).0.to_bits(),
                via_general.discount_factor(t).0.to_bits(),
                "pillar DF at {} differs between wrapper and generalised path",
                t.0
            );
        }
    }

    /// `bootstrap_curve` orders an unsorted ladder by maturity before walking it short → long.
    #[test]
    fn unordered_ladder_is_sorted_before_walking() {
        let shuffled = vec![
            CalibrationInstrument::Ois(quote(5, 0.0411)),
            CalibrationInstrument::Ois(quote(1, 0.0420)),
            CalibrationInstrument::Ois(quote(2, 0.0405)),
        ];
        let curve = bootstrap_curve(&shuffled).expect("sorts and builds");
        for inst in &shuffled {
            if let CalibrationInstrument::Ois(q) = inst {
                let repriced = ois_par_rate(&curve, &q.schedule).0;
                assert!((repriced - q.par_rate.0).abs() < 1e-10);
            }
        }
    }

    #[test]
    fn generalised_rejects_empty_and_duplicate_maturities() {
        assert_eq!(
            bootstrap_curve(&[]).unwrap_err(),
            BootstrapError::NoInstruments
        );
        // Two instruments at the same maturity ⇒ a non-strictly-increasing pillar.
        let dup = vec![
            CalibrationInstrument::Ois(quote(2, 0.0405)),
            CalibrationInstrument::Ois(quote(2, 0.0410)),
        ];
        assert_eq!(
            bootstrap_curve(&dup).unwrap_err(),
            BootstrapError::NonIncreasingMaturity
        );
    }

    /// The acceptance gate: build a realistic USD mixed ladder — cash deposits (ON/1W/1M/3M) at the
    /// front, a convexity-adjusted STIR-futures strip and FRAs in the middle, OIS and vanilla IRS to
    /// 30Y — and confirm every calibrating instrument reprices to its own quote (model par within
    /// 1e-8) with pillar discount factors consistent within 1e-10.
    #[test]
    fn mixed_usd_ladder_reprices_every_instrument_to_par() {
        use crate::daycount::AccrualBasis;
        use crate::deposit::{Deposit, deposit_discount_factor, deposit_par_rate};
        use crate::fra::{Fra, fra_par_rate};
        use crate::futures::StirFuture;
        use crate::futures_strip::{StirFuturesQuote, implied_forward_rate};
        use crate::schedule::usd_sofr_ois_schedule;
        use crate::vanilla_swap::{
            PaymentFrequency, fixed_annuity, float_leg_value, swap_leg_schedule,
        };
        use celnet_types::DayCount;
        use time::{Date, Month};

        let spot = Date::from_calendar_date(2025, Month::June, 16).expect("spot");
        let date = |y, m, d| Date::from_calendar_date(y, m, d).expect("date");

        // --- Cash deposits (ACT/360), each pinning DF(maturity) = 1/(1+r·τ) from spot. ---
        let deposits = [
            Deposit::from_dates(
                spot,
                date(2025, Month::June, 17),
                AccrualBasis::Act360,
                Rate(0.0431),
            ),
            Deposit::from_dates(
                spot,
                date(2025, Month::June, 23),
                AccrualBasis::Act360,
                Rate(0.0432),
            ),
            Deposit::from_dates(
                spot,
                date(2025, Month::July, 16),
                AccrualBasis::Act360,
                Rate(0.0433),
            ),
            Deposit::from_dates(
                spot,
                date(2025, Month::September, 16),
                AccrualBasis::Act360,
                Rate(0.0435),
            ),
        ]
        .map(|d| d.expect("deposit"));

        // --- STIR-futures strip (convexity-adjusted), quarterly windows above the 3M deposit. ---
        let convexity_vol = 0.0075;
        let future = |t1: f64, t2: f64, rate: f64| StirFuturesQuote {
            future: StirFuture::new(Time(t1), Time(t2)).expect("future"),
            futures_rate: Rate(rate),
            convexity_vol,
        };
        let futures = [
            future(0.25, 0.50, 0.0436),
            future(0.50, 0.75, 0.0438),
            future(0.75, 1.00, 0.0440),
        ];

        // --- FRAs (ACT/360) bridging the 1Y–1.5Y region. ---
        let fra = |fix: Date, mat: Date, rate: f64| {
            Fra::from_dates(spot, fix, mat, AccrualBasis::Act360, Rate(rate), 1.0).expect("fra")
        };
        let fras = [
            fra(
                date(2026, Month::June, 16),
                date(2026, Month::September, 16),
                0.0442,
            ),
            fra(
                date(2026, Month::September, 16),
                date(2026, Month::December, 16),
                0.0444,
            ),
        ];

        // --- OIS (annual SOFR) at 2Y/3Y/5Y. ---
        let ois = |years: u32, par: f64| OisQuote {
            schedule: usd_sofr_ois_schedule(spot, years).expect("ois schedule"),
            par_rate: Rate(par),
        };
        let oises = [ois(2, 0.0420), ois(3, 0.0415), ois(5, 0.0410)];

        // --- Vanilla IRS (semi fixed vs quarterly float, ACT/360) at 10Y/30Y. ---
        let irs = |years: u32, par: f64| VanillaIrsQuote {
            fixed_leg: swap_leg_schedule(
                spot,
                years,
                PaymentFrequency::SemiAnnual,
                DayCount::Act360,
            )
            .expect("fixed leg"),
            float_leg: swap_leg_schedule(
                spot,
                years,
                PaymentFrequency::Quarterly,
                DayCount::Act360,
            )
            .expect("float leg"),
            par_rate: Rate(par),
        };
        let irses = [irs(10, 0.0425), irs(30, 0.0435)];

        // Assemble the ladder in a deliberately mixed (but maturity-ordered) sequence.
        let mut ladder: Vec<CalibrationInstrument> = Vec::new();
        ladder.extend(deposits.iter().map(|d| CalibrationInstrument::Deposit(*d)));
        ladder.extend(
            futures
                .iter()
                .map(|q| CalibrationInstrument::StirFuture(*q)),
        );
        ladder.extend(fras.iter().map(|f| CalibrationInstrument::Fra(*f)));
        ladder.extend(oises.iter().map(|q| CalibrationInstrument::Ois(q.clone())));
        ladder.extend(
            irses
                .iter()
                .map(|q| CalibrationInstrument::VanillaIrs(q.clone())),
        );

        let curve = bootstrap_curve(&ladder).expect("mixed ladder bootstraps");

        let mut worst_rate = 0.0_f64;
        let mut worst_df = 0.0_f64;

        // Deposits: par rate and the closed-form discount-factor identity.
        for d in &deposits {
            worst_rate = worst_rate.max((deposit_par_rate(&curve, d).0 - d.rate.0).abs());
            worst_df = worst_df
                .max((curve.discount_factor(d.maturity).0 - deposit_discount_factor(d).0).abs());
        }
        // Futures: the curve's simple forward over each window equals the convexity-debiased forward.
        for q in &futures {
            let model = curve
                .forward_rate_simple(q.future.fixing_start(), q.future.fixing_end())
                .0;
            worst_rate = worst_rate.max((model - implied_forward_rate(q).0).abs());
        }
        // FRAs: the par (break-even) fixed rate equals the quoted rate.
        for f in &fras {
            worst_rate = worst_rate.max((fra_par_rate(&curve, f).0 - f.fixed_rate.0).abs());
        }
        // OIS: the par fixed rate equals the quote.
        for q in &oises {
            worst_rate = worst_rate.max((ois_par_rate(&curve, &q.schedule).0 - q.par_rate.0).abs());
        }
        // IRS: float_leg_value / fixed_annuity equals the quoted par rate.
        for q in &irses {
            let model = float_leg_value(&curve, &q.float_leg) / fixed_annuity(&curve, &q.fixed_leg);
            worst_rate = worst_rate.max((model - q.par_rate.0).abs());
        }

        eprintln!("mixed-ladder worst reprice residual: rate {worst_rate:.3e}, DF {worst_df:.3e}");
        assert!(
            worst_rate < 1e-8,
            "worst reprice-to-par rate residual {worst_rate:.3e} exceeds 1e-8"
        );
        assert!(
            worst_df < 1e-10,
            "worst pillar DF residual {worst_df:.3e} exceeds 1e-10"
        );

        // Pillar discount factors are a strictly decreasing, positive, continuous term structure.
        let mut prev_df = 1.0;
        for inst in &ladder {
            let df = curve.discount_factor(inst.maturity()).0;
            assert!(df > 0.0 && df <= 1.0, "DF {df} out of (0, 1]");
            assert!(df < prev_df, "discount factors must strictly decrease");
            prev_df = df;
        }
    }

    // ----------------------------------------------------------------------------------------------
    // Interpolation-selectable bootstrap (`bootstrap_ois_with`).
    // ----------------------------------------------------------------------------------------------

    /// The P0 USD-SOFR pillar ladder: pillars at 1/2/3/5/7/10/15/20/30y, every OIS with ANNUAL
    /// fixed cashflows — so the 5y+ swaps have coupons at non-pillar years (4, 6, 8, 9, …) whose
    /// discount factors must be interpolated from the curve during the solve.
    fn p0_ladder() -> Vec<OisQuote> {
        [
            (1, 0.0432),
            (2, 0.0418),
            (3, 0.0410),
            (5, 0.0405),
            (7, 0.0406),
            (10, 0.0411),
            (15, 0.0420),
            (20, 0.0425),
            (30, 0.0428),
        ]
        .into_iter()
        .map(|(n, par)| quote(n, par))
        .collect()
    }

    /// Acceptance criterion for the monotone-convex bootstrap: the curve produced under
    /// [`Interpolation::MonotoneConvexForward`] reprices EVERY calibrating OIS in the P0 ladder to
    /// its quoted par rate — including the 5y+ swaps whose annual coupons fall between pillars. The
    /// reprice check runs the independent `ois_par_rate` engine (NOT the bootstrap), so this is a
    /// genuine par-reproduction gate, not a circular oracle.
    #[test]
    fn monotone_convex_bootstrap_reprices_every_ois_to_par() {
        let quotes = p0_ladder();
        let curve = bootstrap_ois_with(&quotes, Interpolation::MonotoneConvexForward)
            .expect("monotone-convex bootstrap converges");
        assert_eq!(curve.interpolation(), Interpolation::MonotoneConvexForward);

        let mut worst = 0.0_f64;
        for q in &quotes {
            let repriced = ois_par_rate(&curve, &q.schedule).0;
            worst = worst.max((repriced - q.par_rate.0).abs());
        }
        eprintln!("monotone-convex worst OIS reprice-to-par residual: {worst:.3e}");
        assert!(
            worst < 1e-10,
            "monotone-convex reprice-to-par residual {worst:.3e} exceeds 1e-10"
        );
    }

    /// Backward-compat guarantee: `bootstrap_ois_with(_, LogLinearDf)` is byte-identical to the
    /// original `bootstrap_ois` — every pillar discount factor matches to the last bit, so routing
    /// the log-linear path through the interpolation-selectable core introduces zero numeric drift.
    #[test]
    fn log_linear_with_is_byte_identical_to_bootstrap_ois() {
        let quotes = p0_ladder();
        let baseline = bootstrap_ois(&quotes).expect("baseline log-linear");
        let via_with =
            bootstrap_ois_with(&quotes, Interpolation::LogLinearDf).expect("with-variant");

        assert_eq!(via_with.interpolation(), Interpolation::LogLinearDf);
        for q in &quotes {
            let t = q.schedule.maturity();
            assert_eq!(
                baseline.discount_factor(t).0.to_bits(),
                via_with.discount_factor(t).0.to_bits(),
                "pillar DF at {} differs between bootstrap_ois and bootstrap_ois_with(LogLinearDf)",
                t.0
            );
        }
        // Between pillars too: identical scheme + identical pillars ⇒ identical curve everywhere.
        for &t in &[0.5, 1.5, 4.0, 6.5, 8.25, 12.0, 17.5, 26.0] {
            assert_eq!(
                baseline.discount_factor(Time(t)).0.to_bits(),
                via_with.discount_factor(Time(t)).0.to_bits(),
                "off-pillar DF at {t} differs between the two log-linear paths"
            );
        }
    }

    /// Curve-level scheme contrast (task bullet c, the interpolation-only view): given ONE fixed set
    /// of pillar discount factors, the log-linear and monotone-convex curves agree *at* the pillars
    /// (both reproduce every pillar DF exactly) but differ *between* them (piecewise-flat vs smooth
    /// piecewise-quadratic forwards). This isolates the interpolation itself from the solve.
    #[test]
    fn same_pillars_agree_at_knots_but_differ_between_schemes() {
        // A fixed pillar set (the P0 ladder maturities, arbitrary but downward-sloping DFs).
        let pillars: Vec<(Time, Df)> = std::iter::once((Time(0.0), Df(1.0)))
            .chain(
                [
                    (1.0, 0.9585),
                    (2.0, 0.9190),
                    (3.0, 0.8810),
                    (5.0, 0.8100),
                    (7.0, 0.7470),
                    (10.0, 0.6650),
                    (15.0, 0.5450),
                    (20.0, 0.4520),
                    (30.0, 0.3100),
                ]
                .into_iter()
                .map(|(t, d)| (Time(t), Df(d))),
            )
            .collect();
        let ll = Curve::from_log_linear_dfs(&pillars).expect("log-linear");
        let mc = Curve::from_monotone_convex_dfs(&pillars).expect("monotone-convex");

        // Agree AT the pillars — both interpolations reproduce every pillar DF exactly (same input).
        for &(t, _) in &pillars {
            assert_eq!(
                ll.discount_factor(t).0.to_bits(),
                mc.discount_factor(t).0.to_bits(),
                "schemes must reproduce the pillar DF identically at {}",
                t.0
            );
        }
        // Differ BETWEEN the pillars — the smooth forward departs from the piecewise-flat one.
        let mut max_gap = 0.0_f64;
        for &t in &[4.0, 6.0, 8.0, 9.0, 12.5, 17.5, 25.0] {
            max_gap =
                max_gap.max((ll.discount_factor(Time(t)).0 - mc.discount_factor(Time(t)).0).abs());
        }
        assert!(
            max_gap > 1e-6,
            "monotone-convex must differ from log-linear between pillars (max gap {max_gap:.3e})"
        );
    }

    /// Bootstrap-level proof that the scheme selection reaches the SOLVE (not just interpolation):
    /// because monotone-convex reshapes the off-pillar-cashflow discount factors during calibration,
    /// the *solved pillar discount factors themselves* differ from the log-linear build — a
    /// `MonotoneConvexForward` resolve produces a materially different curve than `LogLinearDf` for
    /// the identical market inputs. (First pillar agrees: the 1y OIS has a single on-pillar cashflow,
    /// so its DF is scheme-independent — the divergence begins once off-pillar coupons appear.)
    #[test]
    fn bootstrap_scheme_selection_changes_the_solved_pillars() {
        let quotes = p0_ladder();
        let ll = bootstrap_ois_with(&quotes, Interpolation::LogLinearDf).expect("log-linear");
        let mc =
            bootstrap_ois_with(&quotes, Interpolation::MonotoneConvexForward).expect("monotone");

        // The 1y pillar (single on-pillar cashflow) is scheme-independent: DF = 1/(1+par).
        let t1 = quotes[0].schedule.maturity();
        assert!(
            (ll.discount_factor(t1).0 - mc.discount_factor(t1).0).abs() < 1e-12,
            "the single-cashflow 1y pillar must be scheme-independent"
        );

        // From the 5y pillar onward (off-pillar annual coupons), the solved pillar DFs diverge.
        let mut max_pillar_gap = 0.0_f64;
        for q in quotes.iter().filter(|q| q.schedule.maturity().0 >= 5.0) {
            let t = q.schedule.maturity();
            max_pillar_gap =
                max_pillar_gap.max((ll.discount_factor(t).0 - mc.discount_factor(t).0).abs());
        }
        assert!(
            max_pillar_gap > 1e-6,
            "scheme selection must change the solved pillar DFs (max pillar gap {max_pillar_gap:.3e})"
        );

        // Each curve still reprices its OWN quotes to par under its own scheme (both are valid
        // calibrations — they simply differ), so the divergence is a genuine scheme effect, not error.
        for q in &quotes {
            assert!((ois_par_rate(&ll, &q.schedule).0 - q.par_rate.0).abs() < 1e-10);
            assert!((ois_par_rate(&mc, &q.schedule).0 - q.par_rate.0).abs() < 1e-10);
        }
    }

    /// The whole point of bootstrapping *with* monotone-convex interpolation: naively taking the
    /// log-linear-bootstrapped pillar discount factors and merely re-wrapping them under the smooth
    /// scheme yields a curve that does NOT reprice the off-pillar-cashflow OIS to par, whereas the
    /// proper monotone-convex bootstrap does. This directly validates why a scheme-aware solve is
    /// required rather than a post-hoc re-interpolation.
    #[test]
    fn naive_rewrap_fails_to_reprice_but_proper_monotone_bootstrap_succeeds() {
        let quotes = p0_ladder();

        // Log-linear pillars, re-wrapped (WRONG) under monotone-convex without re-solving.
        let ll = bootstrap_ois_with(&quotes, Interpolation::LogLinearDf).expect("log-linear");
        let pillars: Vec<(Time, Df)> = std::iter::once((Time(0.0), Df(1.0)))
            .chain(quotes.iter().map(|q| {
                (
                    q.schedule.maturity(),
                    ll.discount_factor(q.schedule.maturity()),
                )
            }))
            .collect();
        let rewrapped =
            Curve::from_monotone_convex_dfs(&pillars).expect("re-wrap under monotone-convex");

        let mut worst_rewrap = 0.0_f64;
        for q in &quotes {
            let repriced = ois_par_rate(&rewrapped, &q.schedule).0;
            worst_rewrap = worst_rewrap.max((repriced - q.par_rate.0).abs());
        }
        // The naive re-wrap must visibly break repricing (off-pillar coupons are discounted on the
        // reshaped smooth forward) — far outside the 1e-10 par tolerance.
        assert!(
            worst_rewrap > 1e-6,
            "naive monotone-convex re-wrap unexpectedly repriced to par (residual {worst_rewrap:.3e}); \
             the test market may lack off-pillar cashflows"
        );

        // The proper scheme-aware bootstrap fixes exactly this.
        let proper = bootstrap_ois_with(&quotes, Interpolation::MonotoneConvexForward)
            .expect("monotone-convex bootstrap");
        let mut worst_proper = 0.0_f64;
        for q in &quotes {
            let repriced = ois_par_rate(&proper, &q.schedule).0;
            worst_proper = worst_proper.max((repriced - q.par_rate.0).abs());
        }
        assert!(
            worst_proper < 1e-10,
            "proper monotone-convex bootstrap reprice residual {worst_proper:.3e} exceeds 1e-10"
        );
    }
}
