//! Term-structure interpolation in **total variance / business time**.
//!
//! `docs/ANALYTICS-SPEC.md` §3.6: interpolate across tenors in total-variance
//! terms, never in raw vols. Calendar-spread no-arbitrage is the statement that
//! total variance `w` is **non-decreasing in `t` at fixed strike `K`**.
//! Interpolating vols directly (or interpolating RR/BF independently per tenor)
//! is a known cause of crossed total-variance curves.
//!
//! This module interpolates total variance **linearly in business time at fixed
//! log-moneyness `k = ln(K/F_t)`**, which keeps `w(k, ·)` monotone in `t`
//! whenever the per-pillar `w(k)` values are monotone. **Caveat:** fixed-`k` is
//! the same point in strike space as fixed-`K` only when the pillar forwards
//! coincide. When the pillar forwards differ, a fixed `k` maps to a *different*
//! strike at each pillar (`K = F_t·eᵏ`), so monotone-in-`k` total variance does
//! **not** by itself certify fixed-strike calendar no-arbitrage. The
//! fixed-log-moneyness guarantee is therefore exact only for equal pillar
//! forwards; for differing forwards the calendar check must be read in strike
//! space — see [`TermStructure::min_calendar_increment`], which evaluates the
//! increment at a fixed strike by converting that strike to each pillar's own
//! log-moneyness.
//!
//! # Business time
//!
//! "Business time" `τ(t)` is a non-decreasing clock that can place extra weight
//! on event days (central-bank meetings, fixings) and discount weekends; the
//! simplest clock is calendar time `τ(t) = t`. The interpolator is written
//! against an arbitrary monotone clock so the engine can plug in an
//! event-weighted clock without changing the math.

use celnet_core::Smile;
use celnet_core::math::{ln, sqrt};
use celnet_types::Vol;

/// A monotone non-decreasing business-time clock `τ(t)`.
///
/// The default [`CalendarClock`] is the identity `τ(t) = t`; an event-weighted
/// clock can over-weight scheduled-event days. Implementations must be
/// non-decreasing and return `0` at `t = 0`.
pub trait BusinessClock {
    /// Accumulated business time at calendar time `t` (years).
    fn business_time(&self, t: f64) -> f64;
}

/// The trivial calendar clock `τ(t) = t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CalendarClock;

impl BusinessClock for CalendarClock {
    #[inline]
    fn business_time(&self, t: f64) -> f64 {
        t
    }
}

/// One calibrated maturity pillar: a [`Smile`] slice plus its forward and
/// calendar expiry. The slice is queried in **strike space** at this pillar's
/// own forward; the term-structure layer converts to/from log-moneyness so the
/// interpolation happens at fixed log-moneyness across maturities.
#[derive(Debug, Clone, Copy)]
pub struct TenorPillar<S: Smile> {
    /// The calibrated smile slice for this maturity.
    pub smile: S,
    /// Outright forward at this maturity.
    pub forward: f64,
    /// Calendar time to expiry (years), strictly increasing across pillars.
    pub t: f64,
}

impl<S: Smile> TenorPillar<S> {
    /// Construct a pillar.
    #[must_use]
    pub fn new(smile: S, forward: f64, t: f64) -> Self {
        Self { smile, forward, t }
    }

    /// Total implied variance `w(k) = σ²·t` at log-moneyness `k` for this pillar.
    #[must_use]
    pub fn total_variance(&self, k: f64) -> f64 {
        let strike = self.forward * crate::mathx::powf(core::f64::consts::E, k);
        let sigma = self.smile.implied_vol(strike, self.forward, self.t).0;
        sigma * sigma * self.t
    }
}

/// A full volatility surface assembled from per-tenor smile pillars, continuously
/// re-strikable in `(strike, t)` via total-variance interpolation on a business
/// clock.
///
/// The pillars are stored sorted by maturity. Queries at a maturity between two
/// pillars interpolate the total variance **linearly in business time** at the
/// query's log-moneyness; queries before the first / after the last pillar use
/// the nearest pillar's total-variance *rate* (flat-forward-variance
/// extrapolation), which preserves monotonicity of `w` in `t`.
#[derive(Debug, Clone)]
pub struct TermStructure<S: Smile, C: BusinessClock = CalendarClock> {
    pillars: Vec<TenorPillar<S>>,
    clock: C,
}

impl<S: Smile + Clone> TermStructure<S, CalendarClock> {
    /// Build a term structure from maturity pillars on the trivial calendar
    /// clock. See [`TermStructure::with_clock`] for an event-weighted clock.
    ///
    /// # Panics
    ///
    /// Panics if `pillars` is empty or the maturities are not strictly
    /// increasing.
    #[must_use]
    pub fn new(pillars: Vec<TenorPillar<S>>) -> Self {
        Self::with_clock(pillars, CalendarClock)
    }
}

impl<S: Smile + Clone, C: BusinessClock> TermStructure<S, C> {
    /// Build a term structure on a custom business clock.
    ///
    /// # Panics
    ///
    /// Panics if `pillars` is empty or the maturities are not strictly
    /// increasing.
    #[must_use]
    pub fn with_clock(pillars: Vec<TenorPillar<S>>, clock: C) -> Self {
        assert!(!pillars.is_empty(), "term structure needs ≥ 1 pillar");
        assert!(
            pillars.windows(2).all(|w| w[0].t < w[1].t),
            "term-structure pillars must have strictly increasing maturities"
        );
        assert!(
            pillars.iter().all(|p| p.t > 0.0 && p.forward > 0.0),
            "term-structure pillars need positive maturity and forward"
        );
        Self { pillars, clock }
    }

    /// The pillar maturities, ascending.
    #[must_use]
    pub fn maturities(&self) -> Vec<f64> {
        self.pillars.iter().map(|p| p.t).collect()
    }

    /// Interpolated total implied variance `w(k, t)` at log-moneyness `k` and
    /// calendar time `t`, linear in business time `τ` across the bracketing
    /// pillars (flat-forward-variance extrapolation outside the pillar range).
    #[must_use]
    pub fn total_variance(&self, k: f64, t: f64) -> f64 {
        let tau = self.clock.business_time(t);
        let n = self.pillars.len();

        // Before the first pillar: scale the first pillar's total variance by the
        // business-time ratio (constant forward variance back to 0).
        let first = &self.pillars[0];
        let tau1 = self.clock.business_time(first.t);
        if tau <= tau1 {
            let w1 = first.total_variance(k);
            return if tau1 > 0.0 { w1 * (tau / tau1) } else { w1 };
        }

        // After the last pillar: extend at the last pillar's forward-variance rate.
        let last = &self.pillars[n - 1];
        let tau_last = self.clock.business_time(last.t);
        if tau >= tau_last {
            let w_last = last.total_variance(k);
            // Forward-variance rate over the last inter-pillar gap (or 0→last).
            let (w_prev, tau_prev) = if n >= 2 {
                (
                    self.pillars[n - 2].total_variance(k),
                    self.clock.business_time(self.pillars[n - 2].t),
                )
            } else {
                (0.0, 0.0)
            };
            let rate = ((w_last - w_prev) / (tau_last - tau_prev)).max(0.0);
            return w_last + rate * (tau - tau_last);
        }

        // Interior: locate the bracketing pillars and interpolate linearly in τ.
        let mut hi = 1;
        while hi < n && self.clock.business_time(self.pillars[hi].t) < tau {
            hi += 1;
        }
        let lo = hi - 1;
        let pl = &self.pillars[lo];
        let ph = &self.pillars[hi];
        let tl = self.clock.business_time(pl.t);
        let th = self.clock.business_time(ph.t);
        let wl = pl.total_variance(k);
        let wh = ph.total_variance(k);
        let frac = (tau - tl) / (th - tl);
        wl + frac * (wh - wl)
    }

    /// Implied Black volatility at strike `K` and calendar time `t`:
    /// `σ = √(w(k, t)/t)` with `k = ln(K/F_t)`. The forward `F_t` is interpolated
    /// log-linearly between the bracketing pillar forwards (the standard
    /// forward-curve interpolation), so the surface is re-strikable at any `t`.
    #[must_use]
    pub fn implied_vol(&self, strike: f64, t: f64) -> Vol {
        let f = self.forward_at(t);
        let k = ln(strike / f);
        let w = self.total_variance(k, t);
        Vol(sqrt((w / t).max(0.0)))
    }

    /// The interpolated outright forward at calendar time `t` (log-linear in
    /// calendar time between pillars, flat outside).
    #[must_use]
    pub fn forward_at(&self, t: f64) -> f64 {
        let n = self.pillars.len();
        if t <= self.pillars[0].t {
            return self.pillars[0].forward;
        }
        if t >= self.pillars[n - 1].t {
            return self.pillars[n - 1].forward;
        }
        let mut hi = 1;
        while hi < n && self.pillars[hi].t < t {
            hi += 1;
        }
        let lo = hi - 1;
        let (tl, th) = (self.pillars[lo].t, self.pillars[hi].t);
        let (fl, fh) = (self.pillars[lo].forward, self.pillars[hi].forward);
        let frac = (t - tl) / (th - tl);
        // Log-linear forward interpolation.
        crate::mathx::powf(core::f64::consts::E, ln(fl) + frac * (ln(fh) - ln(fl)))
    }

    /// Whether the surface is **calendar-arbitrage-free** along the *fixed strike*
    /// pinned by log-moneyness `k` at the first pillar's forward
    /// (`K = F₀·eᵏ`): total variance non-decreasing across a dense maturity grid
    /// spanning the pillars, measured at that **fixed strike** `K`. Returns the
    /// smallest forward-variance increment seen (≥ −tol when arbitrage-free).
    ///
    /// Calendar no-arbitrage is a fixed-strike statement, so the increment is
    /// evaluated by holding `K` constant and converting it to each maturity's own
    /// log-moneyness `k(t) = ln(K / F_t)` before reading the interpolated total
    /// variance. When all pillar forwards coincide this is identical to a
    /// fixed-`k` scan; when they differ it is the correct strike-space quantity.
    #[must_use]
    pub fn min_calendar_increment(&self, k: f64, samples: usize) -> f64 {
        assert!(samples >= 2, "need ≥ 2 maturity samples");
        let t0 = self.pillars[0].t;
        let t1 = self.pillars[self.pillars.len() - 1].t;
        // Pin the strike from the near-pillar log-moneyness, then walk that fixed
        // strike across maturities (converting to each maturity's own k).
        let strike = self.pillars[0].forward * crate::mathx::powf(core::f64::consts::E, k);
        let w_at = |t: f64| {
            let k_t = ln(strike / self.forward_at(t));
            self.total_variance(k_t, t)
        };
        let mut prev_w = w_at(t0);
        let mut min_inc = f64::INFINITY;
        for i in 1..samples {
            let t = t0 + (t1 - t0) * (i as f64) / ((samples - 1) as f64);
            let w = w_at(t);
            min_inc = min_inc.min(w - prev_w);
            prev_w = w;
        }
        min_inc
    }

    /// Whether total variance is non-decreasing in `t` (no calendar arbitrage) at
    /// the fixed strike pinned by near-pillar log-moneyness `k`, to tolerance
    /// `tol`. See [`Self::min_calendar_increment`] for the fixed-strike semantics.
    #[must_use]
    pub fn is_calendar_free(&self, k: f64, tol: f64) -> bool {
        self.min_calendar_increment(k, 256) >= -tol
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parametric::ParametricSlice;
    use celnet_core::{FlatSmile, is_close};

    /// Two flat-smile pillars with increasing total variance produce a
    /// calendar-arbitrage-free surface; total variance interpolates linearly in t.
    #[test]
    fn flat_pillars_are_calendar_free() {
        // 6M at 10 vol, 1Y at 12 vol ⇒ w(6M)=0.01·0.5=0.005, w(1Y)=0.0144 — monotone.
        let p0 = TenorPillar::new(FlatSmile::new(0.10), 1.10, 0.5);
        let p1 = TenorPillar::new(FlatSmile::new(0.12), 1.10, 1.0);
        let ts = TermStructure::new(vec![p0, p1]);
        assert!(ts.is_calendar_free(0.0, 1e-12));
        assert!(ts.is_calendar_free(0.2, 1e-12));
        // Interpolated total variance at 9M sits between the pillar variances.
        let w9 = ts.total_variance(0.0, 0.75);
        assert!(w9 > 0.005 && w9 < 0.0144);
    }

    /// Total variance is monotone even when interpolating between SVI pillars.
    #[test]
    fn svi_pillars_calendar_free() {
        let s0 = ParametricSlice::new(0.004, 0.03, -0.2, 0.0, 0.08, 1.10, 0.5);
        let s1 = ParametricSlice::new(0.010, 0.05, -0.2, 0.0, 0.10, 1.11, 1.5);
        let p0 = TenorPillar::new(s0, 1.10, 0.5);
        let p1 = TenorPillar::new(s1, 1.11, 1.5);
        let ts = TermStructure::new(vec![p0, p1]);
        for &k in &[-0.3, -0.1, 0.0, 0.1, 0.3] {
            assert!(
                ts.is_calendar_free(k, 1e-9),
                "k={k}: min increment {}",
                ts.min_calendar_increment(k, 256)
            );
        }
    }

    /// The surface re-strikes: implied_vol at a pillar maturity reproduces the
    /// pillar's own vol at the forward, and is finite everywhere.
    #[test]
    fn restrikable_at_pillar() {
        let p0 = TenorPillar::new(FlatSmile::new(0.10), 1.10, 0.5);
        let p1 = TenorPillar::new(FlatSmile::new(0.12), 1.10, 1.0);
        let ts = TermStructure::new(vec![p0, p1]);
        let v = ts.implied_vol(1.10, 1.0).0;
        assert!(celnet_core::is_close(v, 0.12, 1e-9, 1e-11));
        let mid = ts.implied_vol(1.05, 0.75).0;
        assert!(mid.is_finite() && mid > 0.0);
    }

    /// Interior interpolation, both extrapolation regimes and the exact-pillar
    /// boundaries match hand-computed closed forms (linear-in-τ total variance;
    /// flat-forward-variance extension), with three pillars so the bracketing
    /// search is exercised past the first interval.
    #[test]
    fn total_variance_matches_hand_interpolation() {
        let (v0, v1, v2) = (0.10_f64, 0.12_f64, 0.13_f64);
        let (t0, t1, t2) = (0.5_f64, 1.0_f64, 2.0_f64);
        let ts = TermStructure::new(vec![
            TenorPillar::new(FlatSmile::new(v0), 1.10, t0),
            TenorPillar::new(FlatSmile::new(v1), 1.10, t1),
            TenorPillar::new(FlatSmile::new(v2), 1.10, t2),
        ]);
        let (w0, w1, w2) = (v0 * v0 * t0, v1 * v1 * t1, v2 * v2 * t2);

        // Exact pillar times reproduce the pillar total variances.
        assert!(is_close(ts.total_variance(0.0, t0), w0, 1e-14, 1e-16));
        assert!(is_close(ts.total_variance(0.0, t1), w1, 1e-14, 1e-16));
        assert!(is_close(ts.total_variance(0.0, t2), w2, 1e-14, 1e-16));

        // Interior of the SECOND interval: linear in τ between (t1,w1),(t2,w2).
        let t = 1.6;
        let want = w1 + (t - t1) / (t2 - t1) * (w2 - w1);
        assert!(
            is_close(ts.total_variance(0.0, t), want, 1e-14, 1e-16),
            "interior: got {}, want {want}",
            ts.total_variance(0.0, t)
        );

        // Before the first pillar: constant forward variance back to zero.
        let t = 0.2;
        assert!(is_close(
            ts.total_variance(0.0, t),
            w0 * (t / t0),
            1e-14,
            1e-16
        ));

        // After the last pillar: extend at the last inter-pillar variance rate.
        let t = 3.0;
        let rate = (w2 - w1) / (t2 - t1);
        assert!(is_close(
            ts.total_variance(0.0, t),
            w2 + rate * (t - t2),
            1e-14,
            1e-16
        ));

        // implied_vol is √(w/t) against the interpolated forward.
        let v = ts.implied_vol(1.10, 1.6).0;
        assert!(is_close(v, (want / 1.6).sqrt(), 1e-14, 1e-16));
    }

    /// A DECREASING pillar total variance clamps the beyond-last-pillar
    /// extension rate at zero (flat, never decreasing) — pins the `.max(0.0)`
    /// on the forward-variance rate.
    #[test]
    fn negative_terminal_rate_is_clamped_flat() {
        let p0 = TenorPillar::new(FlatSmile::new(0.15), 1.10, 0.5); // w = 0.01125
        let p1 = TenorPillar::new(FlatSmile::new(0.10), 1.10, 1.0); // w = 0.01
        let ts = TermStructure::new(vec![p0, p1]);
        let w_last = 0.10 * 0.10 * 1.0;
        assert!(
            is_close(ts.total_variance(0.0, 5.0), w_last, 1e-14, 1e-16),
            "a negative terminal variance rate must extend flat: got {}",
            ts.total_variance(0.0, 5.0)
        );
    }

    /// A single-pillar structure extends past the pillar at the rate w/τ
    /// anchored at zero (the `(0,0)` synthetic previous pillar).
    #[test]
    fn single_pillar_extends_at_its_own_rate() {
        let ts = TermStructure::new(vec![TenorPillar::new(FlatSmile::new(0.10), 1.10, 1.0)]);
        let w = 0.01;
        assert!(is_close(ts.total_variance(0.0, 2.5), w * 2.5, 1e-14, 1e-16));
        assert!(is_close(ts.total_variance(0.0, 0.4), w * 0.4, 1e-14, 1e-16));
    }

    /// The interpolated forward is log-linear between pillars and flat outside
    /// — against an in-test closed form, including a mid-point of the second
    /// interval (bracket search past the first interval).
    #[test]
    fn forward_interpolation_matches_log_linear_closed_form() {
        let p0 = TenorPillar::new(FlatSmile::new(0.10), 1.05, 0.5);
        let p1 = TenorPillar::new(FlatSmile::new(0.11), 1.10, 1.0);
        let p2 = TenorPillar::new(FlatSmile::new(0.12), 1.22, 2.0);
        let ts = TermStructure::new(vec![p0, p1, p2]);
        assert!(is_close(ts.forward_at(0.1), 1.05, 0.0, 0.0), "flat before");
        assert!(is_close(ts.forward_at(5.0), 1.22, 0.0, 0.0), "flat after");
        assert!(
            is_close(ts.forward_at(1.0), 1.10, 1e-15, 1e-16),
            "at pillar"
        );
        let t = 1.5;
        let frac = (t - 1.0) / (2.0 - 1.0);
        let want = (1.10_f64.ln() + frac * (1.22_f64.ln() - 1.10_f64.ln())).exp();
        assert!(
            is_close(ts.forward_at(t), want, 1e-14, 1e-16),
            "log-linear mid: got {}, want {want}",
            ts.forward_at(t)
        );
    }

    /// `TenorPillar::total_variance` is `σ(F·eᵏ)²·t` — pinned against a direct
    /// recomputation through the pillar's own smile.
    #[test]
    fn pillar_total_variance_matches_definition() {
        let s = ParametricSlice::new(0.004, 0.03, -0.2, 0.0, 0.08, 1.10, 0.5);
        let p = TenorPillar::new(s, 1.10, 0.5);
        for &k in &[-0.3_f64, 0.0, 0.25] {
            let strike = 1.10 * k.exp();
            let sigma = s.vol_at(strike);
            assert!(
                is_close(p.total_variance(k), sigma * sigma * 0.5, 1e-13, 1e-15),
                "k={k}"
            );
        }
    }

    /// The fixed-strike calendar scan agrees with an in-test re-derivation on a
    /// surface whose pillar forwards DIFFER (the strike↔log-moneyness
    /// conversion per maturity is load-bearing, not an identity).
    #[test]
    fn calendar_increment_matches_in_test_scan_with_differing_forwards() {
        let p0 = TenorPillar::new(FlatSmile::new(0.10), 1.05, 0.5);
        let p1 = TenorPillar::new(FlatSmile::new(0.12), 1.15, 1.5);
        let ts = TermStructure::new(vec![p0, p1]);
        let (k, samples) = (0.1_f64, 16_usize);
        let strike = 1.05 * k.exp();
        let w_at = |t: f64| {
            let k_t = (strike / ts.forward_at(t)).ln();
            ts.total_variance(k_t, t)
        };
        let (t0, t1) = (0.5, 1.5);
        let mut prev = w_at(t0);
        let mut want = f64::INFINITY;
        for i in 1..samples {
            let t = t0 + (t1 - t0) * (i as f64) / ((samples - 1) as f64);
            let w = w_at(t);
            want = want.min(w - prev);
            prev = w;
        }
        let got = ts.min_calendar_increment(k, samples);
        assert!(
            is_close(got, want, 1e-13, 1e-15),
            "calendar scan: got {got}, want {want}"
        );
    }

    /// Decreasing pillar total variance is detected as calendar arbitrage.
    #[test]
    fn decreasing_total_variance_is_arbitrage() {
        // 6M at 15 vol, 1Y at 10 vol ⇒ w(6M)=0.01125 > w(1Y)=0.01 — crossing.
        let p0 = TenorPillar::new(FlatSmile::new(0.15), 1.10, 0.5);
        let p1 = TenorPillar::new(FlatSmile::new(0.10), 1.10, 1.0);
        let ts = TermStructure::new(vec![p0, p1]);
        assert!(!ts.is_calendar_free(0.0, 1e-6));
    }
}
