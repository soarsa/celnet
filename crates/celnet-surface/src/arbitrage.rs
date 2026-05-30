//! Static (per-slice) no-arbitrage checks on a smile.
//!
//! A single smile slice must satisfy two static no-arbitrage laws
//! (`docs/ANALYTICS-SPEC.md` §3.4):
//!
//! 1. **Butterfly / density ≥ 0.** The risk-neutral density implied by the slice
//!    is `g(K) = e^{r_d t}·∂²C/∂K²` (Breeden-Litzenberger). Non-negativity of the
//!    density is exactly **convexity of the call price in strike**: for any three
//!    increasing strikes the butterfly `C(K−h) − 2C(K) + C(K+h) ≥ 0`.
//! 2. **Vertical-spread monotonicity.** The undiscounted call price is
//!    non-increasing in strike (`∂C/∂K ∈ [−1, 0]` undiscounted), i.e. a call
//!    spread has non-negative value: `C(K) ≥ C(K+h)`.
//!
//! Both are evaluated by **re-striking the smile**: the call at strike `K` is
//! priced with the smile's *own* volatility `σ(K)` at that strike (the
//! definition of a smile-consistent price), so the checks see the genuine smile
//! curvature, not a flat-vol curve. Vanna-volga is **not** guaranteed
//! arbitrage-free in the wings, so these checks are the gate the surface layer
//! uses to detect a bad slice and fall back to an arbitrage-free model.
//!
//! The pricing here uses the **undiscounted forward call**
//! `c(K) = e^{r_d t}·C(K) = F·Φ(d₁) − K·Φ(d₂)` so the checks are independent of
//! the discount factor (a positive constant that cannot change the sign of a
//! second difference). This keeps the density/monotonicity tests purely about
//! the smile shape.

use celnet_core::Smile;
use celnet_core::math::{ln, norm_cdf, norm_pdf, sqrt};

/// The outcome of running the static arbitrage checks on a strike grid.
#[derive(Debug, Clone, PartialEq)]
pub struct ArbitrageReport {
    /// The smallest (most negative, scaled) butterfly second-difference seen.
    /// Non-negative (within tolerance) means the implied density stayed ≥ 0.
    pub min_butterfly: f64,
    /// The largest forward call-price *increase* between adjacent strikes
    /// (`c(K+h) − c(K)`). Non-positive (within tolerance) means the call price is
    /// monotone non-increasing in strike (vertical-spread no-arbitrage).
    pub max_vertical_increase: f64,
    /// The smallest implied density value sampled (`≥ 0` when arbitrage-free).
    pub min_density: f64,
}

impl ArbitrageReport {
    /// Whether both static no-arbitrage laws hold within `tol` (absolute, on the
    /// scaled quantities). A clean slice has `min_butterfly ≥ −tol`,
    /// `max_vertical_increase ≤ tol` and `min_density ≥ −tol`.
    #[must_use]
    pub fn is_arbitrage_free(&self, tol: f64) -> bool {
        self.min_butterfly >= -tol && self.max_vertical_increase <= tol && self.min_density >= -tol
    }
}

/// The undiscounted forward call price `c(K) = F·Φ(d₁) − K·Φ(d₂)` at the smile's
/// own vol `σ(K)`, for forward `F`, vol-time `t`.
#[inline]
fn forward_call<S: Smile>(smile: &S, strike: f64, forward: f64, t: f64) -> f64 {
    let vol = smile.implied_vol(strike, forward, t).0;
    let vsqt = vol * sqrt(t);
    let d1 = (ln(forward / strike) + 0.5 * vol * vol * t) / vsqt;
    let d2 = d1 - vsqt;
    forward * norm_cdf(d1) - strike * norm_cdf(d2)
}

/// The Breeden-Litzenberger risk-neutral density (undiscounted, in forward
/// measure) implied by the smile at strike `K`, via a central second difference
/// of the forward call price with spacing `h`:
/// `g(K) ≈ [c(K−h) − 2c(K) + c(K+h)] / h²`.
///
/// `h` should be small relative to `K` (a few tenths of a percent) for an
/// accurate second-difference density; the surface layer picks `h` from the
/// strike scale.
#[must_use]
pub fn implied_density<S: Smile>(smile: &S, strike: f64, forward: f64, t: f64, h: f64) -> f64 {
    debug_assert!(
        h > 0.0 && h < strike,
        "density spacing must satisfy 0 < h < K"
    );
    let down = forward_call(smile, strike - h, forward, t);
    let mid = forward_call(smile, strike, forward, t);
    let up = forward_call(smile, strike + h, forward, t);
    (down - 2.0 * mid + up) / (h * h)
}

/// Run both static no-arbitrage checks across a strike grid.
///
/// `grid` is the (ascending, ≥ 3 points) strike grid to test; `forward`/`t` the
/// slice's forward and vol-time; `h` the second-difference spacing for the
/// density (relative to the local strike — pass an absolute value sized to the
/// grid). The butterfly second-difference is scaled by `1/h²` so it reads as a
/// density-comparable quantity.
///
/// # Panics
///
/// Panics if `grid` has fewer than three points or is not strictly ascending,
/// or if `h` is not a valid spacing.
#[must_use]
pub fn check_slice<S: Smile>(
    smile: &S,
    grid: &[f64],
    forward: f64,
    t: f64,
    h: f64,
) -> ArbitrageReport {
    assert!(grid.len() >= 3, "arbitrage grid needs ≥ 3 strikes");
    assert!(h > 0.0, "density spacing must be positive");
    assert!(
        grid.windows(2).all(|w| w[0] < w[1]),
        "arbitrage grid must be strictly ascending"
    );
    assert!(grid[0] > h, "grid must stay above the density spacing h");

    let mut min_butterfly = f64::INFINITY;
    let mut max_vertical_increase = f64::NEG_INFINITY;
    let mut min_density = f64::INFINITY;

    let mut prev_call = forward_call(smile, grid[0], forward, t);
    for &k in grid {
        // Density (butterfly) at k.
        let dens = implied_density(smile, k, forward, t, h);
        min_density = min_density.min(dens);
        // The scaled butterfly equals h²·density; reporting the density itself is
        // the cleaner, h-independent quantity, but we also track the raw
        // (h²-scaled) butterfly for callers that asked for the spread number.
        min_butterfly = min_butterfly.min(dens * h * h);

        // Vertical-spread monotonicity: call must not rise from the previous
        // (lower) strike to this one.
        let call_k = forward_call(smile, k, forward, t);
        let increase = call_k - prev_call;
        if k != grid[0] {
            max_vertical_increase = max_vertical_increase.max(increase);
        }
        prev_call = call_k;
    }

    ArbitrageReport {
        min_butterfly,
        max_vertical_increase,
        min_density,
    }
}

/// The slope-bound diagnostic: the undiscounted-call strike slope
/// `∂c/∂K = −Φ(d₂(K))` lies in `[−1, 0]`. Returned for one strike so callers can
/// assert the digital-call price `−∂c/∂K = Φ(d₂)` is a valid probability.
#[must_use]
pub fn forward_call_strike_slope<S: Smile>(smile: &S, strike: f64, forward: f64, t: f64) -> f64 {
    let vol = smile.implied_vol(strike, forward, t).0;
    let vsqt = vol * sqrt(t);
    let d1 = (ln(forward / strike) + 0.5 * vol * vol * t) / vsqt;
    let d2 = d1 - vsqt;
    // Note: with a vol that varies in K this is the *sticky* slope ignoring
    // ∂σ/∂K; the density check above captures the full curvature. φ(d1) appears
    // only via the smile term, which the second-difference density subsumes.
    let _ = norm_pdf(d1);
    -norm_cdf(d2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_hedge::MarketHedgeSmile;
    use celnet_core::FlatSmile;

    fn grid(lo: f64, hi: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| lo + (hi - lo) * (i as f64) / ((n - 1) as f64))
            .collect()
    }

    /// A flat smile is arbitrage-free: density ≥ 0 everywhere, call monotone.
    #[test]
    fn flat_smile_is_arbitrage_free() {
        let s = FlatSmile::new(0.12);
        let f = 1.11;
        let g = grid(0.7, 1.7, 41);
        let rep = check_slice(&s, &g, f, 1.0, 1e-3);
        assert!(
            rep.is_arbitrage_free(1e-6),
            "flat smile must be arbitrage-free: {rep:?}"
        );
        assert!(rep.min_density >= -1e-6);
    }

    /// A mild, well-behaved vanna-volga smile is arbitrage-free across a grid
    /// spanning the wings.
    #[test]
    fn mild_vanna_volga_is_arbitrage_free() {
        let s = MarketHedgeSmile::new([1.02, 1.11, 1.20], [0.118, 0.11, 0.114], 1.11, 1.0);
        let g = grid(0.85, 1.5, 61);
        let rep = check_slice(&s, &g, s.forward(), s.reference_t(), 1e-3);
        assert!(
            rep.is_arbitrage_free(1e-5),
            "mild VV smile must be arbitrage-free: {rep:?}"
        );
    }

    /// The strike-slope of the undiscounted call is in [−1, 0] (digital-call
    /// price Φ(d2) is a probability).
    #[test]
    fn call_strike_slope_is_bounded() {
        let s = MarketHedgeSmile::new([1.02, 1.11, 1.20], [0.118, 0.11, 0.114], 1.11, 1.0);
        for k in grid(0.9, 1.4, 21) {
            let slope = forward_call_strike_slope(&s, k, s.forward(), s.reference_t());
            assert!(
                (-1.0..=0.0).contains(&slope),
                "slope {slope} out of [-1,0] at K={k}"
            );
        }
    }

    /// A pathological smile (huge convexity in a tiny wing) trips the density
    /// check — proving the detector actually fires on arbitrage.
    #[test]
    fn detects_negative_density() {
        // A V-shaped "smile" with an extreme central dip and high wings creates a
        // concave region in the call price ⇒ negative density.
        let s = MarketHedgeSmile::new([1.05, 1.11, 1.17], [0.40, 0.08, 0.40], 1.11, 0.10);
        let g = grid(0.95, 1.30, 81);
        let rep = check_slice(&s, &g, s.forward(), s.reference_t(), 5e-4);
        assert!(
            !rep.is_arbitrage_free(1e-6),
            "extreme-convexity smile must be flagged: {rep:?}"
        );
        assert!(
            rep.min_density < -1e-6,
            "expected a negative density: {rep:?}"
        );
    }
}
