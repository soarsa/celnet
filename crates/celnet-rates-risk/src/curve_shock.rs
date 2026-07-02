//! Rate-shock scenario generation over a base discount curve's zero-rate pillars.
//!
//! # The shock seam: a per-pillar zero-rate perturbation
//!
//! A scenario is an **absolute additive shift** applied to the continuously-compounded zero rate
//! at each dated pillar of the base curve, re-built into a shocked [`Curve`] via
//! [`celnet_rates::Curve::from_zero_rates`]. This one seam is **asset-class-agnostic**: any linear
//! FI position that prices off a `&Curve` — an OIS via [`celnet_rates::ois_pv`], a cash bond via
//! [`celnet_bond::price_from_curve`] — reprices under the *same* shocked curve, so the scenario
//! generator never branches on the instrument. (A par-quote bump + re-bootstrap, as
//! [`celnet_rates::ois_risk`] uses for its DV01, is OIS-specific and has no analogue for a bond
//! priced directly off a curve; the zero-rate-pillar shift is the unifying choice.)
//!
//! # Exact shift identity (log-linear-on-log-DF)
//!
//! The base curve is the shipping-default log-linear-on-log-DF scheme. Because interpolation is
//! **linear in `ln DF` and additive over pillar values**, shifting every pillar zero rate `zᵢ` by
//! `sᵢ` moves the pillar log-discount by `−sᵢ·tᵢ`, and the interpolated log-discount at *any*
//! query time `t ∈ [0, t_n]` becomes
//!
//! ```text
//! ln DF_shocked(t) = ln DF_base(t) − S(t),   DF_shocked(t) = DF_base(t)·exp(−S(t)),
//! ```
//!
//! where `S(t)` is the log-linear interpolation, over the pillar times, of the values `{sᵢ·tᵢ}`
//! (with the un-shocked curve origin contributing the knot `(0, 0)`). For a **parallel** shock
//! `sᵢ ≡ δ` this collapses to `S(t) = δ·t`, i.e. `DF_shocked(t) = DF_base(t)·exp(−δ·t)`,
//! the exact analytic parallel-rate move — used as the independent repricing oracle. This identity
//! is what makes the reprice-under-shock validate to ≤1e-12 against a hand computation that never
//! touches the shocked-curve construction (see `tests/oracle.rs`).

use celnet_rates::{Curve, CurveError};
use celnet_types::{Rate, Time};

use crate::error::RateRiskError;

/// A base discount curve described by its dated zero-rate pillars, ready to shock.
///
/// Holds the `(time, continuously-compounded zero rate)` pillars and the pre-built base [`Curve`]
/// (a cheaply-cloned `Arc` snapshot). Because a [`Curve`] does not expose its pillars, the explicit
/// grid is retained here so a shock can be re-applied to the underlying rates. The curve origin
/// `(t = 0, DF = 1)` is implicit and never shocked.
#[derive(Clone, Debug)]
pub struct RatePillars {
    /// Dated pillars `(tᵢ, zᵢ)`, strictly increasing in time, `tᵢ > 0`.
    pillars: Vec<(Time, Rate)>,
    /// The base curve `Curve::from_zero_rates(&pillars)`, built once at construction.
    base: Curve,
}

impl RatePillars {
    /// Build from `(time, zero rate)` pillars, validating them by constructing the base curve.
    ///
    /// # Errors
    ///
    /// Propagates any [`CurveError`] from the base build (empty, non-positive pillar time, or
    /// non-increasing times).
    pub fn new(pillars: Vec<(Time, Rate)>) -> Result<Self, CurveError> {
        let base = Curve::from_zero_rates(&pillars)?;
        Ok(Self { pillars, base })
    }

    /// The number of dated pillars (the required [`RateShock`] length).
    #[must_use]
    pub fn len(&self) -> usize {
        self.pillars.len()
    }

    /// True when there are no dated pillars. Always false for a curve built by [`RatePillars::new`]
    /// (which rejects an empty pillar set), retained for API completeness.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pillars.is_empty()
    }

    /// The dated pillar times, in ascending order.
    #[must_use]
    pub fn pillar_times(&self) -> Vec<Time> {
        self.pillars.iter().map(|&(t, _)| t).collect()
    }

    /// The base (un-shocked) discount curve.
    #[must_use]
    pub fn base_curve(&self) -> &Curve {
        &self.base
    }

    /// The shocked discount curve for one scenario: every pillar zero rate shifted by the shock's
    /// corresponding entry, re-built via [`Curve::from_zero_rates`].
    ///
    /// # Errors
    ///
    /// [`RateRiskError::ShockLength`] if the shock's length differs from the pillar count, or
    /// [`RateRiskError::Curve`] if the shocked pillars are rejected by the curve builder.
    pub fn shocked_curve(&self, shock: &RateShock) -> Result<Curve, RateRiskError> {
        if shock.len() != self.pillars.len() {
            return Err(RateRiskError::ShockLength {
                expected: self.pillars.len(),
                got: shock.len(),
            });
        }
        let shifted: Vec<(Time, Rate)> = self
            .pillars
            .iter()
            .zip(shock.shifts())
            .map(|(&(t, z), &s)| (t, Rate(z.0 + s)))
            .collect();
        Ok(Curve::from_zero_rates(&shifted)?)
    }
}

/// One rate scenario: a per-pillar **absolute additive shift** (in zero-rate units, e.g. `1e-4`
/// for one basis point) applied to the base curve's dated pillars, in pillar order.
///
/// A *parallel* shift has all entries equal ([`RateShock::parallel`]); a *key-rate* shift has a
/// single non-zero entry ([`RateShock::key_rate`]); a *historical / prescribed* scenario is an
/// arbitrary vector ([`RateShock::new`]). The length must equal the base curve's dated-pillar
/// count when applied.
#[derive(Clone, Debug, PartialEq)]
pub struct RateShock {
    /// Per-pillar zero-rate shifts, aligned with the curve's dated pillars.
    shifts: Vec<f64>,
}

impl RateShock {
    /// A scenario from an explicit per-pillar shift vector (a historical or prescribed scenario).
    #[must_use]
    pub fn new(shifts: Vec<f64>) -> Self {
        Self { shifts }
    }

    /// A parallel shift of `size` applied to all `n` pillars (`size` in zero-rate units).
    #[must_use]
    pub fn parallel(n: usize, size: f64) -> Self {
        Self {
            shifts: vec![size; n],
        }
    }

    /// A key-rate shift: `size` at pillar `pillar`, zero elsewhere, over `n` pillars.
    ///
    /// # Panics
    ///
    /// Panics if `pillar >= n` (the bucket index is out of range for the pillar grid).
    #[must_use]
    pub fn key_rate(n: usize, pillar: usize, size: f64) -> Self {
        assert!(
            pillar < n,
            "key-rate pillar {pillar} out of range for {n} pillars"
        );
        let mut shifts = vec![0.0; n];
        shifts[pillar] = size;
        Self { shifts }
    }

    /// The per-pillar shifts.
    #[must_use]
    pub fn shifts(&self) -> &[f64] {
        &self.shifts
    }

    /// The number of per-pillar shifts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.shifts.len()
    }

    /// True when the shock carries no shifts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shifts.is_empty()
    }
}

/// The standard prescribed sensitivity-scenario grid for an `n`-pillar curve at shock magnitude
/// `size`: the parallel shift up and down, then each pillar's key-rate shift up and down.
///
/// Returns `2 + 2n` scenarios in a deterministic order: `[parallel +, parallel −, pillar 0 +,
/// pillar 0 −, …, pillar n−1 +, pillar n−1 −]`. This is the deterministic bump set for a
/// scenario/bump-and-revalue VaR; a caller with a historical return window supplies its own
/// [`RateShock`] set instead.
#[must_use]
pub fn standard_bump_scenarios(n: usize, size: f64) -> Vec<RateShock> {
    let mut out = Vec::with_capacity(2 + 2 * n);
    out.push(RateShock::parallel(n, size));
    out.push(RateShock::parallel(n, -size));
    for i in 0..n {
        out.push(RateShock::key_rate(n, i, size));
        out.push(RateShock::key_rate(n, i, -size));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pillars() -> RatePillars {
        RatePillars::new(vec![
            (Time(1.0), Rate(0.043)),
            (Time(2.0), Rate(0.041)),
            (Time(5.0), Rate(0.040)),
            (Time(10.0), Rate(0.042)),
        ])
        .expect("valid pillars")
    }

    #[test]
    fn parallel_shock_matches_analytic_discount_factor_move() {
        // The headline identity: a +δ parallel zero-rate shift moves DF(t) to DF_base(t)·e^(−δ·t)
        // at EVERY time, including between pillars. Computed here from the BASE curve + the analytic
        // factor (never from the shocked curve), so agreement validates the shock construction.
        let p = pillars();
        let delta = 25e-4; // 25 bp
        let shocked = p.shocked_curve(&RateShock::parallel(4, delta)).unwrap();
        for &t in &[0.0, 0.3, 1.0, 1.5, 2.0, 3.7, 5.0, 8.2, 10.0] {
            let base_df = p.base_curve().discount_factor(Time(t)).0;
            let expected = base_df * (-delta * t).exp();
            let got = shocked.discount_factor(Time(t)).0;
            assert!(
                (got - expected).abs() <= 1e-12,
                "parallel shift at t={t}: {got} vs {expected}"
            );
        }
    }

    #[test]
    fn shock_length_mismatch_is_rejected() {
        let p = pillars();
        let err = p.shocked_curve(&RateShock::parallel(3, 1e-4)).unwrap_err();
        assert_eq!(
            err,
            RateRiskError::ShockLength {
                expected: 4,
                got: 3
            }
        );
    }

    #[test]
    fn standard_grid_has_two_plus_two_n_scenarios() {
        let grid = standard_bump_scenarios(4, 1e-4);
        assert_eq!(grid.len(), 2 + 2 * 4);
        // First two are parallel up/down.
        assert_eq!(grid[0].shifts(), &[1e-4, 1e-4, 1e-4, 1e-4]);
        assert_eq!(grid[1].shifts(), &[-1e-4, -1e-4, -1e-4, -1e-4]);
        // Then key-rate up/down at pillar 0.
        assert_eq!(grid[2].shifts(), &[1e-4, 0.0, 0.0, 0.0]);
        assert_eq!(grid[3].shifts(), &[-1e-4, 0.0, 0.0, 0.0]);
    }
}
