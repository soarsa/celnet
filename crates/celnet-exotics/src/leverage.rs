//! Local-volatility (Dupire) extraction and the leverage function `L(S,t)`.
//!
//! # Two objects
//!
//! 1. [`LocalVolSurface`] — the **Dupire** local volatility `σ_loc(K, t)` implied
//!    by an arbitrage-free Black-implied-volatility surface. It is computed in the
//!    *implied-volatility* parameterisation (no fragile second derivative of an
//!    option price): given the Black total variance `w(y, t) = σ²(K,t)·t` as a
//!    function of log-moneyness `y = ln(K/F_t)` and maturity `t`, the local
//!    variance is the closed form
//!
//!    ```text
//!      σ_loc² = (∂w/∂t)
//!               ───────────────────────────────────────────────────────────
//!               1 − (y/w)(∂w/∂y) + ¼(−¼ − 1/w + y²/w²)(∂w/∂y)² + ½ ∂²w/∂y²
//!    ```
//!
//!    which is numerically stable and manifestly the right object to feed the LSV
//!    leverage. Carry (`r_d − r_f`) enters only through the forward `F_t`, so the
//!    surface is quoted in moneyness and is carry-consistent by construction.
//!
//! 2. [`LeverageSurface`] — the LSV **leverage** `L(S, t)`, a tabulated bilinear
//!    surface on a `(spot, time)` grid. In the local-stochastic-volatility model
//!    the spot diffuses with instantaneous volatility `L(S,t)·√v_t`; matching the
//!    model's marginal to the market's requires
//!
//!    ```text
//!      L(S, t)² = σ_loc²(S, t) / E[ v_t | S_t = S ] .
//!    ```
//!
//!    The conditional expectation in the denominator is what the
//!    [`crate::particle`] calibrator estimates; this module owns the *container*
//!    and its interpolation, and the pure-local-vol fallback `L² = σ_loc²/v0`
//!    used to seed the calibration's first step.
//!
//! # Method provenance (doc comments only)
//!
//! Dupire local volatility: Dupire (1994). The implied-variance ("`w`-form")
//! parameterisation of the Dupire formula: Gatheral (2006, *The Volatility
//! Surface*). The leverage / conditional-expectation identity for LSV:
//! Ren, Madan & Qian (2007); Guyon & Henry-Labordère (2012). Identifiers are
//! purpose-named; provenance lives only in documentation.

use celnet_core::math::{exp, ln, sqrt};

/// A callable Black implied-volatility surface in `(strike, t)` together with its
/// forward curve `t ↦ F_t`. This is the minimal seam the Dupire extraction needs;
/// [`crate::lsv`] adapts a [`celnet_surface::VolSurface`] (or any
/// [`celnet_core::Smile`] term structure) to it.
pub trait ImpliedVolSurface {
    /// Black implied volatility at absolute `strike` and time-to-expiry `t`.
    fn implied_vol(&self, strike: f64, t: f64) -> f64;
    /// Outright forward `F_t` at time-to-expiry `t`.
    fn forward(&self, t: f64) -> f64;
}

/// The Dupire local-volatility surface implied by an [`ImpliedVolSurface`].
///
/// Holds a reference to the implied surface and the finite-difference spacings
/// used for the moneyness/maturity derivatives of total variance. Evaluating
/// `local_vol(S, t)` returns `σ_loc(S, t)`.
#[derive(Debug, Clone, Copy)]
pub struct LocalVolSurface<'a, S: ImpliedVolSurface> {
    iv: &'a S,
    /// Relative bump in log-moneyness for `∂w/∂y`, `∂²w/∂y²` (e.g. `1e-2`).
    dy: f64,
    /// Absolute time bump (years) for `∂w/∂t` (e.g. `1e-3`).
    dt: f64,
    /// Floor on local variance, keeping `σ_loc` real and the LSV well-posed.
    floor_var: f64,
}

impl<'a, S: ImpliedVolSurface> LocalVolSurface<'a, S> {
    /// Build a Dupire surface over an implied surface with the default robust
    /// finite-difference spacings.
    #[must_use]
    pub fn new(iv: &'a S) -> Self {
        Self {
            iv,
            dy: 1e-2,
            dt: 1e-3,
            floor_var: 1e-6,
        }
    }

    /// Override the finite-difference spacings (log-moneyness and time) and the
    /// variance floor.
    #[must_use]
    pub fn with_spacings(mut self, dy: f64, dt: f64, floor_var: f64) -> Self {
        assert!(dy > 0.0 && dt > 0.0 && floor_var >= 0.0);
        self.dy = dy;
        self.dt = dt;
        self.floor_var = floor_var;
        self
    }

    /// Black total variance `w(y, t) = σ²(K,t)·t` at log-moneyness `y = ln(K/F_t)`.
    #[inline]
    fn total_var(&self, y: f64, t: f64) -> f64 {
        let f = self.iv.forward(t);
        let k = f * exp(y);
        let s = self.iv.implied_vol(k, t);
        s * s * t
    }

    /// Dupire local volatility `σ_loc(S, t)` at absolute spot level `S`.
    ///
    /// Uses the implied-total-variance form of the Dupire equation, with central
    /// differences in `y` and a forward (or central, when `t − dt > 0`) difference
    /// in `t`. The result is floored at `√floor_var` so a tiny negative numerator
    /// from finite-difference noise can never produce a NaN.
    #[must_use]
    pub fn local_vol(&self, spot: f64, t: f64) -> f64 {
        sqrt(self.local_var(spot, t))
    }

    /// Dupire local **variance** `σ_loc²(S, t)`.
    #[must_use]
    pub fn local_var(&self, spot: f64, t: f64) -> f64 {
        let tt = t.max(self.dt);
        let f = self.iv.forward(tt);
        let y = ln(spot / f);

        // Spatial (log-moneyness) derivatives of total variance, central.
        let w = self.total_var(y, tt);
        let w_up = self.total_var(y + self.dy, tt);
        let w_dn = self.total_var(y - self.dy, tt);
        let wy = (w_up - w_dn) / (2.0 * self.dy);
        let wyy = (w_up - 2.0 * w + w_dn) / (self.dy * self.dy);

        // Maturity derivative of total variance at *fixed log-moneyness*.
        // (The forward at t±dt is re-evaluated inside `total_var`, so `y` is held
        //  in moneyness — the carry-consistent way to take ∂w/∂t.)
        let wt = if tt - self.dt > 0.0 {
            (self.total_var(y, tt + self.dt) - self.total_var(y, tt - self.dt)) / (2.0 * self.dt)
        } else {
            (self.total_var(y, tt + self.dt) - w) / self.dt
        };

        if w <= 0.0 {
            return self.floor_var;
        }
        // Gatheral denominator g(y) = 1 − (y/w) w_y + ¼(−¼ − 1/w + y²/w²) w_y²
        //                              + ½ w_yy.
        let term1 = 1.0 - (y / w) * wy;
        let term2 = 0.25 * (-0.25 - 1.0 / w + (y * y) / (w * w)) * wy * wy;
        let term3 = 0.5 * wyy;
        let denom = term1 + term2 + term3;

        let lv = if denom > 1e-8 { wt / denom } else { wt / 1e-8 };
        lv.max(self.floor_var)
    }
}

/// A tabulated leverage function `L(S, t)` on a `(spot, time)` grid, bilinearly
/// interpolated and flat-extrapolated at the grid edges.
///
/// The grid is the calibration grid the [`crate::particle`] method fills; the ADI
/// PDE and the Monte-Carlo engine both read the leverage through
/// [`LeverageSurface::leverage`], so the two engines are guaranteed to use an
/// identical model.
#[derive(Debug, Clone)]
pub struct LeverageSurface {
    /// Ascending spot grid `S_0 < S_1 < … < S_{m−1}`.
    spots: Vec<f64>,
    /// Ascending time grid `t_0 < t_1 < … < t_{n−1}` (years).
    times: Vec<f64>,
    /// Row-major `L[j·m + i] = L(S_i, t_j)`.
    values: Vec<f64>,
}

impl LeverageSurface {
    /// Build an (initially unit) leverage surface over the given grids. The grids
    /// must be strictly ascending and non-empty.
    ///
    /// # Panics
    ///
    /// Panics if a grid is empty or not strictly increasing.
    #[must_use]
    pub fn new(spots: Vec<f64>, times: Vec<f64>) -> Self {
        assert!(
            !spots.is_empty() && !times.is_empty(),
            "empty leverage grid"
        );
        assert!(
            spots.windows(2).all(|w| w[1] > w[0]),
            "spot grid must be strictly ascending"
        );
        assert!(
            times.windows(2).all(|w| w[1] > w[0]),
            "time grid must be strictly ascending"
        );
        let values = vec![1.0; spots.len() * times.len()];
        Self {
            spots,
            times,
            values,
        }
    }

    /// The number of spot nodes.
    #[must_use]
    pub fn spot_len(&self) -> usize {
        self.spots.len()
    }

    /// The number of time nodes.
    #[must_use]
    pub fn time_len(&self) -> usize {
        self.times.len()
    }

    /// The spot grid.
    #[must_use]
    pub fn spots(&self) -> &[f64] {
        &self.spots
    }

    /// The time grid.
    #[must_use]
    pub fn times(&self) -> &[f64] {
        &self.times
    }

    /// Set the leverage at grid node `(i, j)` = `(spot_i, time_j)`.
    ///
    /// # Panics
    ///
    /// Panics if the indices are out of range.
    pub fn set(&mut self, i: usize, j: usize, value: f64) {
        let m = self.spots.len();
        self.values[j * m + i] = value;
    }

    /// The leverage value stored at grid node `(i, j)` (no interpolation).
    #[must_use]
    pub fn at(&self, i: usize, j: usize) -> f64 {
        let m = self.spots.len();
        self.values[j * m + i]
    }

    /// Bilinearly-interpolated leverage `L(S, t)`, flat-extrapolated beyond the
    /// grid edges (the leverage is well-defined on the calibration domain and held
    /// constant outside it, which is the standard production convention).
    #[must_use]
    pub fn leverage(&self, spot: f64, t: f64) -> f64 {
        let (i0, i1, fx) = bracket(&self.spots, spot);
        let (j0, j1, ft) = bracket(&self.times, t);
        let m = self.spots.len();
        let v00 = self.values[j0 * m + i0];
        let v10 = self.values[j0 * m + i1];
        let v01 = self.values[j1 * m + i0];
        let v11 = self.values[j1 * m + i1];
        let bottom = v00 + (v10 - v00) * fx;
        let top = v01 + (v11 - v01) * fx;
        bottom + (top - bottom) * ft
    }
}

/// Locate `x` in an ascending grid, returning `(lo_index, hi_index, frac)` for
/// linear interpolation, clamped (flat-extrapolated) at the edges.
#[inline]
fn bracket(grid: &[f64], x: f64) -> (usize, usize, f64) {
    let n = grid.len();
    if n == 1 || x <= grid[0] {
        return (0, 0, 0.0);
    }
    if x >= grid[n - 1] {
        return (n - 1, n - 1, 0.0);
    }
    // Binary search for the bracketing interval.
    let mut lo = 0usize;
    let mut hi = n - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if grid[mid] <= x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let frac = (x - grid[lo]) / (grid[hi] - grid[lo]);
    (lo, hi, frac)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    /// A flat implied-vol surface (constant `σ`) at zero carry has Dupire local
    /// vol equal to that same constant everywhere — the defining sanity check of
    /// the extraction (flat implied ⇒ flat local).
    #[test]
    fn flat_implied_gives_flat_local() {
        struct Flat {
            sigma: f64,
        }
        impl ImpliedVolSurface for Flat {
            fn implied_vol(&self, _k: f64, _t: f64) -> f64 {
                self.sigma
            }
            fn forward(&self, _t: f64) -> f64 {
                100.0
            }
        }
        let iv = Flat { sigma: 0.2 };
        let lv = LocalVolSurface::new(&iv);
        for s in [80.0, 100.0, 125.0] {
            for t in [0.25, 1.0, 2.0] {
                assert_close!(lv.local_vol(s, t), 0.2, 1e-3, 1e-4);
            }
        }
    }

    /// Bilinear interpolation reproduces grid nodes exactly and interpolates
    /// linearly between them; flat-extrapolates outside.
    #[test]
    fn leverage_interpolation() {
        let mut lev = LeverageSurface::new(vec![90.0, 100.0, 110.0], vec![0.5, 1.0]);
        lev.set(0, 0, 1.1);
        lev.set(1, 0, 1.0);
        lev.set(2, 0, 0.9);
        lev.set(0, 1, 1.2);
        lev.set(1, 1, 1.05);
        lev.set(2, 1, 0.95);
        // Exact node reads.
        assert_close!(lev.leverage(100.0, 0.5), 1.0, 1e-12, 1e-12);
        assert_close!(lev.leverage(110.0, 1.0), 0.95, 1e-12, 1e-12);
        // Midpoint in spot at t=0.5.
        assert_close!(lev.leverage(95.0, 0.5), 1.05, 1e-12, 1e-12);
        // Flat extrapolation beyond edges.
        assert_close!(lev.leverage(70.0, 0.5), 1.1, 1e-12, 1e-12);
        assert_close!(lev.leverage(130.0, 2.0), 0.95, 1e-12, 1e-12);
    }
}
