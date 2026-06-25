//! One-dimensional root-finding for curve calibration.
//!
//! [`brent_root`] implements Brent's method (the derivative-free
//! van Wijngaarden–Dekker–Brent algorithm): given a sign-changing bracket it combines
//! inverse-quadratic interpolation, the secant step, and bisection, so it converges
//! superlinearly while retaining bisection's guaranteed bracketing. Each pillar of the
//! sequential bootstrap is a monotone repricing in the unknown discount factor, so a bracket
//! always exists and Brent is the robust workhorse (`FI-CURVES-SPEC.md` §5.3). A
//! safeguarded-Newton acceleration is a later optimisation and is intentionally not added here.

/// Failure modes of the one-dimensional root-finder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolverError {
    /// The supplied endpoints do not bracket a root (`f(a)` and `f(b)` share a sign).
    NoBracket,
    /// The iteration limit was reached before the abscissa tolerance was met.
    NoConvergence,
}

impl core::fmt::Display for SolverError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let m = match self {
            Self::NoBracket => "endpoints do not bracket a root (f(a) and f(b) share a sign)",
            Self::NoConvergence => "root-finder hit its iteration limit before converging",
        };
        f.write_str(m)
    }
}

impl core::error::Error for SolverError {}

/// Find a root of `f` in `[a, b]` by Brent's method.
///
/// `f(a)` and `f(b)` must straddle zero (opposite signs). The search stops once the bracket
/// width is at most `x_tol`, returning the better endpoint.
///
/// # Errors
///
/// Returns [`SolverError::NoBracket`] if the endpoints share a sign, or
/// [`SolverError::NoConvergence`] if `max_iter` iterations elapse before the tolerance is met.
pub fn brent_root<F>(
    mut f: F,
    mut a: f64,
    mut b: f64,
    x_tol: f64,
    max_iter: usize,
) -> Result<f64, SolverError>
where
    F: FnMut(f64) -> f64,
{
    let mut fa = f(a);
    let mut fb = f(b);
    if (fa > 0.0) == (fb > 0.0) {
        return Err(SolverError::NoBracket);
    }
    // Keep `b` as the better current estimate (`|f(b)| <= |f(a)|`).
    if fa.abs() < fb.abs() {
        core::mem::swap(&mut a, &mut b);
        core::mem::swap(&mut fa, &mut fb);
    }

    let mut c_prev = a;
    let mut fc_prev = fa;
    let mut d_prev = a; // the contrapoint from two steps ago; only read once `mflag` is false
    let mut mflag = true;

    for _ in 0..max_iter {
        if (b - a).abs() <= x_tol {
            return Ok(b);
        }

        let interp = (fa - fc_prev).abs() > f64::EPSILON && (fb - fc_prev).abs() > f64::EPSILON;
        let step = if interp {
            // inverse quadratic interpolation through (a, b, c_prev)
            a * fb * fc_prev / ((fa - fb) * (fa - fc_prev))
                + b * fa * fc_prev / ((fb - fa) * (fb - fc_prev))
                + c_prev * fa * fb / ((fc_prev - fa) * (fc_prev - fb))
        } else {
            // secant step
            b - fb * (b - a) / (fb - fa)
        };

        // Accept `step` only if it falls strictly inside ((3a+b)/4, b) and makes adequate
        // progress; otherwise fall back to a bisection step (the Brent safeguards).
        let bound = (3.0 * a + b) / 4.0;
        let inside = (step - bound) * (step - b) < 0.0;
        let reject = !inside
            || (mflag && (step - b).abs() >= (b - c_prev).abs() / 2.0)
            || (!mflag && (step - b).abs() >= (c_prev - d_prev).abs() / 2.0)
            || (mflag && (b - c_prev).abs() < x_tol)
            || (!mflag && (c_prev - d_prev).abs() < x_tol);
        let step = if reject {
            mflag = true;
            (a + b) / 2.0
        } else {
            mflag = false;
            step
        };

        let f_step = f(step);
        d_prev = c_prev;
        c_prev = b;
        fc_prev = fb;
        if (fa > 0.0) != (f_step > 0.0) {
            b = step;
            fb = f_step;
        } else {
            a = step;
            fa = f_step;
        }
        if fa.abs() < fb.abs() {
            core::mem::swap(&mut a, &mut b);
            core::mem::swap(&mut fa, &mut fb);
        }
    }
    Err(SolverError::NoConvergence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(f: impl FnMut(f64) -> f64, a: f64, b: f64) -> f64 {
        brent_root(f, a, b, 1e-13, 100).expect("bracketed root converges")
    }

    #[test]
    fn solves_square_root_of_two() {
        let r = root(|x| x * x - 2.0, 0.0, 2.0);
        assert!((r - 2.0_f64.sqrt()).abs() < 1e-10, "got {r}");
    }

    #[test]
    fn solves_cosine_fixed_point() {
        // cos(x) = x has the Dottie-number root ≈ 0.739085133215.
        let r = root(|x| x.cos() - x, 0.0, 1.0);
        assert!((r - 0.739_085_133_215_160_6).abs() < 1e-10, "got {r}");
    }

    #[test]
    fn solves_a_discount_factor_pillar() {
        // The shape a single OIS pillar takes: solve exp(-z·t) = df for the zero rate z.
        let (t, df) = (2.0_f64, 0.91_f64);
        let z = root(|z| (-z * t).exp() - df, -0.5, 1.0);
        assert!((z - (-df.ln() / t)).abs() < 1e-10, "got {z}");
    }

    #[test]
    fn solves_linear_exactly() {
        let r = root(|x| 2.0 * x - 3.0, 0.0, 5.0);
        assert!((r - 1.5).abs() < 1e-12, "got {r}");
    }

    #[test]
    fn rejects_unbracketed_endpoints() {
        // x² + 1 is strictly positive — no sign change to bracket.
        assert_eq!(
            brent_root(|x| x * x + 1.0, -1.0, 1.0, 1e-12, 100).unwrap_err(),
            SolverError::NoBracket
        );
    }

    #[test]
    fn reports_non_convergence_when_starved_of_iterations() {
        assert_eq!(
            brent_root(|x| x * x - 2.0, 0.0, 2.0, 1e-15, 1).unwrap_err(),
            SolverError::NoConvergence
        );
    }
}
