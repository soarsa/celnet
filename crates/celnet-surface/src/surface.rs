//! Unified volatility surface: model selection + re-strikable evaluation + the
//! consolidated static-arbitrage report.
//!
//! `VolSurface` is the single object a consumer holds. It owns a maturity term
//! structure of smile slices (one of the supported smile **models**) on a
//! business clock, and exposes the two operations the rest of the platform needs
//! (`docs/ANALYTICS-SPEC.md` §3):
//!
//! * [`VolSurface::implied_vol`] — Black implied vol at any `(strike, t)`, with
//!   the surface continuously re-strikable in total variance across maturities
//!   (no calendar arbitrage by construction when the pillars are monotone), and
//! * [`VolSurface::arbitrage_report`] — the consolidated static no-arbitrage
//!   diagnostics (per-slice butterfly / vertical via [`crate::arbitrage`], plus
//!   the cross-slice calendar monotonicity from [`crate::termstructure`]).
//!
//! # Model selection
//!
//! The surface is generic over the per-slice smile type `S: Smile`, so it works
//! uniformly with the [`crate::market_hedge::MarketHedgeSmile`] baseline, a
//! [`crate::stochvol::StochasticVolSmile`], or an [`crate::parametric::ParametricSlice`] / SSVI-derived
//! slice. [`SmileModel`] names the selected family for diagnostics and so the
//! engine can record which model produced a mark; the evaluation path is the same
//! trait call regardless.

use celnet_core::Smile;

use crate::arbitrage::check_slice;
use crate::termstructure::{BusinessClock, CalendarClock, TenorPillar, TermStructure};

/// The smile-model family backing a surface, recorded for diagnostics / audit.
///
/// Variant names describe the modelling *purpose*; mathematical-method
/// provenance lives in the doc comments only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmileModel {
    /// Market-hedge interpolation (the FX broker baseline; vanna-volga method).
    MarketHedge,
    /// Stochastic-volatility smile with arbitrage-free wing density (SABR method).
    StochasticVol,
    /// Single parametric total-variance slice (SVI method).
    Parametric,
    /// Surface-level parametric family, closed-form arbitrage-free (SSVI method).
    ParametricSurface,
}

/// The consolidated static no-arbitrage report for a whole surface: the
/// worst-case per-slice butterfly/vertical diagnostics across the sampled
/// maturities, and the worst-case cross-slice calendar (total-variance
/// monotonicity) increment.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceArbitrageReport {
    /// Worst (smallest) per-slice implied density seen across maturities.
    pub min_density: f64,
    /// Worst (largest) per-slice forward-call vertical increase across maturities.
    pub max_vertical_increase: f64,
    /// Worst (smallest) cross-maturity total-variance increment at the sampled
    /// log-moneyness levels. Non-negative ⇒ no calendar arbitrage.
    pub min_calendar_increment: f64,
}

impl SurfaceArbitrageReport {
    /// Whether the surface is free of all three static arbitrages within `tol`.
    #[must_use]
    pub fn is_arbitrage_free(&self, tol: f64) -> bool {
        self.min_density >= -tol
            && self.max_vertical_increase <= tol
            && self.min_calendar_increment >= -tol
    }
}

/// A unified, re-strikable volatility surface over a chosen smile model.
#[derive(Debug, Clone)]
pub struct VolSurface<S: Smile + Clone, C: BusinessClock = CalendarClock> {
    model: SmileModel,
    term: TermStructure<S, C>,
}

impl<S: Smile + Clone> VolSurface<S, CalendarClock> {
    /// Build a surface from per-maturity smile pillars on the calendar clock,
    /// recording the smile-model family.
    ///
    /// # Panics
    ///
    /// Panics if the pillar set is empty or maturities are not strictly
    /// increasing (delegated to [`TermStructure`]).
    #[must_use]
    pub fn new(model: SmileModel, pillars: Vec<TenorPillar<S>>) -> Self {
        Self {
            model,
            term: TermStructure::new(pillars),
        }
    }
}

impl<S: Smile + Clone, C: BusinessClock> VolSurface<S, C> {
    /// Build a surface on a custom business clock.
    ///
    /// # Panics
    ///
    /// Panics if the pillar set is empty or maturities are not strictly
    /// increasing.
    #[must_use]
    pub fn with_clock(model: SmileModel, pillars: Vec<TenorPillar<S>>, clock: C) -> Self {
        Self {
            model,
            term: TermStructure::with_clock(pillars, clock),
        }
    }

    /// The smile-model family backing this surface.
    #[must_use]
    pub fn model(&self) -> SmileModel {
        self.model
    }

    /// The underlying term structure (for advanced callers that need the
    /// total-variance machinery directly).
    #[must_use]
    pub fn term_structure(&self) -> &TermStructure<S, C> {
        &self.term
    }

    /// Implied Black volatility at `strike` and time-to-expiry `t` (years), the
    /// surface coordinate. The surface interpolates total variance across
    /// maturities and re-strikes against the interpolated forward — so it is
    /// continuously re-strikable in `(strike, t)`.
    #[must_use]
    pub fn implied_vol(&self, strike: f64, t: f64) -> f64 {
        self.term.implied_vol(strike, t).0
    }

    /// The interpolated outright forward at time-to-expiry `t`.
    #[must_use]
    pub fn forward_at(&self, t: f64) -> f64 {
        self.term.forward_at(t)
    }

    /// The consolidated static-arbitrage report.
    ///
    /// Samples `maturity_samples` maturities across the pillar range; at each it
    /// runs the per-slice butterfly/vertical [`check_slice`] over a strike grid of
    /// `±span` log-moneyness (`strike_samples` points) built around the
    /// interpolated forward, and accumulates the worst case. The cross-slice
    /// calendar increment is the worst total-variance step over the same maturity
    /// grid at the central and wing log-moneyness levels.
    ///
    /// `density_h` is the second-difference spacing relative to the forward used
    /// for the implied-density estimate (e.g. `1e-3`).
    ///
    /// # Panics
    ///
    /// Panics if the sample counts are too small or `span`/`density_h` invalid.
    #[must_use]
    pub fn arbitrage_report(
        &self,
        span: f64,
        strike_samples: usize,
        maturity_samples: usize,
        density_h: f64,
    ) -> SurfaceArbitrageReport {
        assert!(strike_samples >= 3, "need ≥ 3 strike samples");
        assert!(maturity_samples >= 2, "need ≥ 2 maturity samples");
        assert!(
            span > 0.0 && density_h > 0.0,
            "span and density_h must be positive"
        );

        let mats = self.term.maturities();
        let (t0, t1) = (mats[0], mats[mats.len() - 1]);

        let mut min_density = f64::INFINITY;
        let mut max_vertical_increase = f64::NEG_INFINITY;

        for j in 0..maturity_samples {
            let t = if maturity_samples == 1 {
                t0
            } else {
                t0 + (t1 - t0) * (j as f64) / ((maturity_samples - 1) as f64)
            };
            let f = self.term.forward_at(t);
            // Build a strike grid around the forward in log-moneyness ±span.
            let grid: Vec<f64> = (0..strike_samples)
                .map(|i| {
                    let k = -span + 2.0 * span * (i as f64) / ((strike_samples - 1) as f64);
                    f * crate::mathx::powf(core::f64::consts::E, k)
                })
                .collect();
            // A transient slice view at this maturity: the surface evaluated at
            // fixed t is itself a `Smile` in strike (see `MaturitySlice`).
            let slice = MaturitySlice { surface: self, t };
            let h = density_h * f;
            let rep = check_slice(&slice, &grid, f, t, h);
            min_density = min_density.min(rep.min_density);
            max_vertical_increase = max_vertical_increase.max(rep.max_vertical_increase);
        }

        // Cross-slice calendar monotonicity at the central + wing log-moneyness.
        let mut min_calendar_increment = f64::INFINITY;
        for &k in &[-span, 0.0, span] {
            min_calendar_increment =
                min_calendar_increment.min(self.term.min_calendar_increment(k, 256));
        }

        SurfaceArbitrageReport {
            min_density,
            max_vertical_increase,
            min_calendar_increment,
        }
    }
}

/// A view of the surface frozen at one maturity `t`, presented as a strike-space
/// [`Smile`] so the per-slice arbitrage checks (which consume any `Smile`) see
/// the genuine interpolated curvature at that maturity.
struct MaturitySlice<'a, S: Smile + Clone, C: BusinessClock> {
    surface: &'a VolSurface<S, C>,
    t: f64,
}

impl<S: Smile + Clone, C: BusinessClock> Smile for MaturitySlice<'_, S, C> {
    fn implied_vol(&self, strike: f64, _forward: f64, _t: f64) -> celnet_types::Vol {
        celnet_types::Vol(self.surface.implied_vol(strike, self.t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parametric::ParametricSlice;
    use crate::termstructure::TenorPillar;
    use celnet_core::is_close;

    fn svi_surface() -> VolSurface<ParametricSlice, CalendarClock> {
        // Two mild, monotone-in-total-variance SVI pillars.
        let s0 = ParametricSlice::new(0.004, 0.02, -0.15, 0.0, 0.10, 1.10, 0.5);
        let s1 = ParametricSlice::new(0.010, 0.03, -0.15, 0.0, 0.12, 1.11, 1.5);
        let p0 = TenorPillar::new(s0, 1.10, 0.5);
        let p1 = TenorPillar::new(s1, 1.11, 1.5);
        VolSurface::new(SmileModel::Parametric, vec![p0, p1])
    }

    /// The unified surface evaluates and re-strikes; at a pillar it reproduces the
    /// pillar slice's vol.
    #[test]
    fn evaluates_and_restrikes() {
        let s = svi_surface();
        let v = s.implied_vol(1.10, 0.5);
        assert!(v > 0.0 && v.is_finite());
        assert_eq!(s.model(), SmileModel::Parametric);
        // Mid-maturity is finite & positive (re-strikable continuity).
        let mid = s.implied_vol(1.05, 1.0);
        assert!(mid.is_finite() && mid > 0.0);
    }

    /// The consolidated arbitrage report is clean for a mild monotone surface.
    #[test]
    fn mild_surface_is_arbitrage_free() {
        let s = svi_surface();
        let rep = s.arbitrage_report(0.5, 81, 8, 1e-3);
        assert!(
            rep.is_arbitrage_free(1e-4),
            "mild surface must be arbitrage-free: {rep:?}"
        );
        assert!(rep.min_calendar_increment >= -1e-9);
    }

    /// At a pillar maturity, the surface vol equals the pillar slice's own vol.
    #[test]
    fn pillar_maturity_matches_slice() {
        let s = svi_surface();
        let slice = ParametricSlice::new(0.004, 0.02, -0.15, 0.0, 0.10, 1.10, 0.5);
        let strike = 1.07;
        let direct = slice.vol_at(strike);
        let via_surface = s.implied_vol(strike, 0.5);
        assert!(
            is_close(direct, via_surface, 1e-9, 1e-11),
            "direct {direct} vs surface {via_surface}"
        );
    }
}
