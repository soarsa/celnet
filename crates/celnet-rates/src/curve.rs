//! The immutable discount/forward curve snapshot and its log-linear-on-log-DF interpolation.
//!
//! A [`Curve`] stores its pillars in **log-discount-factor space** and interpolates linearly in
//! `ln DF(t)` against continuous year-fraction time `t`. This is the shipping default scheme
//! (`FI-CURVES-SPEC.md` §4, Q10): it is local, cheap, exact at the pillars, and arbitrage-free in
//! discount-factor space, with piecewise-constant continuously-compounded instantaneous forwards.
//!
//! The same construction is what an independent oracle (QuantLib's `InterpolatedDiscountCurve`
//! with the `LogLinear` traits) computes, so the closed-form identity tests below double as the
//! cross-engine agreement check for this scheme (`FI-VERIFICATION-CONTRACT.md`).

use std::sync::Arc;

use celnet_types::{Df, Rate, Time};

/// Absolute tolerance used when validating the curve origin pillar `(t = 0, DF = 1)`.
const ORIGIN_TOL: f64 = 1e-12;

/// A single curve pillar held in log-discount-factor space (`t`, `ln DF(t)`).
#[derive(Clone, Copy, Debug)]
struct Node {
    /// Year-fraction time from the curve reference date (strictly increasing across nodes).
    t: f64,
    /// Natural log of the discount factor at `t` (`<= 0` for non-negative zero rates).
    ln_df: f64,
}

/// Construction error for a [`Curve`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveError {
    /// Fewer than two pillars were supplied (an origin plus at least one dated pillar are required).
    TooFewPillars,
    /// The first pillar was not the curve origin `(t = 0, DF = 1)`.
    MissingOrigin,
    /// Pillar times were not strictly increasing.
    NonMonotonicTime,
    /// A discount factor was not strictly positive (negative rates are allowed; `DF` may exceed 1).
    NonPositiveDiscountFactor,
    /// A supplied zero-rate pillar had a non-positive time (zero rates are only defined for `t > 0`).
    NonPositiveZeroTime,
}

impl core::fmt::Display for CurveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            Self::TooFewPillars => "curve needs an origin pillar plus at least one dated pillar",
            Self::MissingOrigin => "first pillar must be the curve origin (t = 0, DF = 1)",
            Self::NonMonotonicTime => "pillar times must be strictly increasing",
            Self::NonPositiveDiscountFactor => "discount factors must be strictly positive",
            Self::NonPositiveZeroTime => "zero-rate pillars require strictly positive time",
        };
        f.write_str(msg)
    }
}

impl core::error::Error for CurveError {}

/// An immutable, cheaply-cloned term-structure snapshot in discount-factor space.
///
/// The curve interpolates linearly in `ln DF` against year-fraction time (log-linear-on-log-DF).
/// Queries are allocation-free and the pillars are shared behind an [`Arc`], so cloning a snapshot
/// — e.g. to fan scenarios out across threads — is a single reference-count bump.
///
/// All query times are continuous year fractions measured from the curve's reference date; the
/// date→time mapping (day-count + calendar) is supplied by a later slice and is intentionally not
/// part of this numeric core.
#[derive(Clone, Debug)]
pub struct Curve {
    /// Pillars in ascending time order; `nodes[0]` is always the origin `(0, ln 1 = 0)`.
    nodes: Arc<[Node]>,
}

impl Curve {
    /// Build a curve from `(time, discount factor)` pillars under log-linear-on-log-DF interpolation.
    ///
    /// The first pillar must be the curve origin `(t = 0, DF = 1)`; subsequent pillars must have
    /// strictly increasing times and strictly positive discount factors (a `DF` above 1 is allowed,
    /// representing a negative zero rate). At least two pillars are required.
    ///
    /// # Errors
    ///
    /// Returns a [`CurveError`] if the origin is missing, fewer than two pillars are supplied, times
    /// are not strictly increasing, or a discount factor is not strictly positive.
    pub fn from_log_linear_dfs(pillars: &[(Time, Df)]) -> Result<Self, CurveError> {
        if pillars.len() < 2 {
            return Err(CurveError::TooFewPillars);
        }
        let (t0, df0) = (pillars[0].0.0, pillars[0].1.0);
        if t0.abs() > ORIGIN_TOL || (df0 - 1.0).abs() > ORIGIN_TOL {
            return Err(CurveError::MissingOrigin);
        }

        let mut nodes: Vec<Node> = Vec::with_capacity(pillars.len());
        let mut prev_t = f64::NEG_INFINITY;
        for &(t, df) in pillars {
            if df.0 <= 0.0 {
                return Err(CurveError::NonPositiveDiscountFactor);
            }
            if t.0 <= prev_t {
                return Err(CurveError::NonMonotonicTime);
            }
            prev_t = t.0;
            nodes.push(Node {
                t: t.0,
                ln_df: df.0.ln(),
            });
        }
        Ok(Self {
            nodes: Arc::from(nodes),
        })
    }

    /// Build a curve from `(time, continuously-compounded zero rate)` pillars, `t > 0`.
    ///
    /// The origin `(0, DF = 1)` is prepended automatically; each pillar's discount factor is
    /// `DF(t) = exp(-z·t)`. This is the convenient constructor for tests and for the bootstrap's
    /// output, and produces exactly the same curve as passing the equivalent discount factors to
    /// [`Curve::from_log_linear_dfs`].
    ///
    /// # Errors
    ///
    /// Returns [`CurveError::TooFewPillars`] if empty, [`CurveError::NonPositiveZeroTime`] if any
    /// pillar time is `<= 0`, or [`CurveError::NonMonotonicTime`] if times are not increasing.
    pub fn from_zero_rates(pillars: &[(Time, Rate)]) -> Result<Self, CurveError> {
        if pillars.is_empty() {
            return Err(CurveError::TooFewPillars);
        }
        let mut dfs: Vec<(Time, Df)> = Vec::with_capacity(pillars.len() + 1);
        dfs.push((Time(0.0), Df(1.0)));
        for &(t, z) in pillars {
            if t.0 <= 0.0 {
                return Err(CurveError::NonPositiveZeroTime);
            }
            dfs.push((t, Df((-z.0 * t.0).exp())));
        }
        Self::from_log_linear_dfs(&dfs)
    }

    /// The latest pillar time on the curve (its calibrated horizon); queries past this point
    /// extrapolate at the final segment's constant instantaneous forward.
    #[must_use]
    pub fn max_time(&self) -> Time {
        Time(self.nodes[self.nodes.len() - 1].t)
    }

    /// `ln DF(t)` under log-linear interpolation, with flat-forward extrapolation at both ends.
    ///
    /// The bracketing segment is found by binary search; for `t` below the first or above the last
    /// pillar the nearest segment's slope is extended (a constant instantaneous forward), which is
    /// the standard, arbitrage-free extrapolation for this scheme.
    fn ln_df(&self, t: f64) -> f64 {
        let n = self.nodes.len();
        // First index whose time is strictly greater than `t`; clamp so [lo, hi] is a real segment.
        let hi = self.nodes.partition_point(|nd| nd.t <= t).clamp(1, n - 1);
        let lo = hi - 1;
        let a = self.nodes[lo];
        let b = self.nodes[hi];
        let slope = (b.ln_df - a.ln_df) / (b.t - a.t);
        a.ln_df + slope * (t - a.t)
    }

    /// The negated slope of the segment bracketing `t` — the (continuously-compounded) instantaneous
    /// forward, which is piecewise-constant for this scheme.
    fn segment_forward(&self, t: f64) -> f64 {
        let n = self.nodes.len();
        let hi = self.nodes.partition_point(|nd| nd.t <= t).clamp(1, n - 1);
        let lo = hi - 1;
        let a = self.nodes[lo];
        let b = self.nodes[hi];
        -(b.ln_df - a.ln_df) / (b.t - a.t)
    }

    /// The discount factor `DF(t)`. For `t <= 0` this is exactly 1 (the curve reference date).
    #[must_use]
    pub fn discount_factor(&self, t: Time) -> Df {
        if t.0 <= 0.0 {
            return Df(1.0);
        }
        Df(self.ln_df(t.0).exp())
    }

    /// The continuously-compounded zero rate `z(t) = -ln DF(t) / t`.
    ///
    /// At and below the origin the zero rate is undefined as a ratio, so the instantaneous short
    /// rate (the first segment's forward) is returned as its continuous limit.
    #[must_use]
    pub fn zero_rate(&self, t: Time) -> Rate {
        if t.0 <= ORIGIN_TOL {
            return Rate(self.segment_forward(0.0));
        }
        Rate(-self.ln_df(t.0) / t.0)
    }

    /// The continuously-compounded instantaneous forward `f(t) = -d ln DF / dt`.
    ///
    /// Under log-linear-on-log-DF this is piecewise-constant within each pillar segment (the
    /// characteristic "sawtooth" forward curve).
    #[must_use]
    pub fn instantaneous_forward(&self, t: Time) -> Rate {
        Rate(self.segment_forward(t.0))
    }

    /// The continuously-compounded forward rate over `[t1, t2]`:
    /// `f = (ln DF(t1) - ln DF(t2)) / (t2 - t1)`.
    ///
    /// `t1` and `t2` must satisfy `t2 > t1`; the result is the rate `f` for which
    /// `DF(t2) = DF(t1)·exp(-f·(t2 - t1))`.
    #[must_use]
    pub fn forward_rate_continuous(&self, t1: Time, t2: Time) -> Rate {
        debug_assert!(t2.0 > t1.0, "forward_rate_continuous requires t2 > t1");
        Rate((self.ln_df(t1.0) - self.ln_df(t2.0)) / (t2.0 - t1.0))
    }

    /// The simple (linearly-compounded) forward rate over `[t1, t2]`:
    /// `f = (DF(t1) / DF(t2) - 1) / (t2 - t1)`.
    ///
    /// This is the money-market forward fixing convention used by FRAs and floating coupons; `t2`
    /// must exceed `t1`.
    #[must_use]
    pub fn forward_rate_simple(&self, t1: Time, t2: Time) -> Rate {
        debug_assert!(t2.0 > t1.0, "forward_rate_simple requires t2 > t1");
        let df1 = self.discount_factor(t1).0;
        let df2 = self.discount_factor(t2).0;
        Rate((df1 / df2 - 1.0) / (t2.0 - t1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A representative downward-sloping (positive-rate) USD-style discount curve.
    fn sample() -> Curve {
        Curve::from_log_linear_dfs(&[
            (Time(0.0), Df(1.0)),
            (Time(0.5), Df(0.978_0)),
            (Time(1.0), Df(0.955_0)),
            (Time(2.0), Df(0.910_0)),
            (Time(5.0), Df(0.790_0)),
            (Time(10.0), Df(0.620_0)),
        ])
        .expect("valid pillars")
    }

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "expected {b}, got {a} (tol {tol})");
    }

    #[test]
    fn origin_discount_factor_is_one() {
        let c = sample();
        close(c.discount_factor(Time(0.0)).0, 1.0, 0.0);
        // Times at or before the reference date also discount to one.
        close(c.discount_factor(Time(-1.0)).0, 1.0, 0.0);
    }

    #[test]
    fn reproduces_pillar_discount_factors() {
        let c = sample();
        for &(t, df) in &[
            (0.5, 0.978_0),
            (1.0, 0.955_0),
            (2.0, 0.910_0),
            (5.0, 0.790_0),
            (10.0, 0.620_0),
        ] {
            close(c.discount_factor(Time(t)).0, df, 1e-12);
        }
    }

    #[test]
    fn log_linear_midpoint_is_geometric_mean() {
        // Between t=1 (0.955) and t=2 (0.910), the t=1.5 DF is exp of the mean log-DF,
        // i.e. the geometric mean of the bracketing discount factors.
        let c = sample();
        let expected = (0.955_0f64 * 0.910_0).sqrt();
        close(c.discount_factor(Time(1.5)).0, expected, 1e-12);
    }

    #[test]
    fn instantaneous_forward_is_piecewise_constant() {
        let c = sample();
        // On the [1, 2] segment the forward equals the segment's continuous forward.
        let seg_fwd = (0.955_0f64.ln() - 0.910_0f64.ln()) / (2.0 - 1.0);
        for t in [1.001, 1.25, 1.5, 1.999] {
            close(c.instantaneous_forward(Time(t)).0, seg_fwd, 1e-12);
        }
    }

    #[test]
    fn zero_rate_matches_definition() {
        let c = sample();
        let t = 2.0;
        let expected = -0.910_0f64.ln() / t;
        close(c.zero_rate(Time(t)).0, expected, 1e-12);
        // The limit at the origin is the first-segment instantaneous forward (short rate).
        close(
            c.zero_rate(Time(0.0)).0,
            c.instantaneous_forward(Time(0.0)).0,
            1e-12,
        );
    }

    #[test]
    fn continuous_forward_reprices_the_far_discount_factor() {
        let c = sample();
        let (t1, t2) = (Time(1.0), Time(5.0));
        let f = c.forward_rate_continuous(t1, t2).0;
        let df1 = c.discount_factor(t1).0;
        let df2 = c.discount_factor(t2).0;
        close(df1 * (-f * (t2.0 - t1.0)).exp(), df2, 1e-12);
    }

    #[test]
    fn simple_forward_matches_money_market_definition() {
        let c = sample();
        let (t1, t2) = (Time(1.0), Time(2.0));
        let df1 = c.discount_factor(t1).0;
        let df2 = c.discount_factor(t2).0;
        let expected = (df1 / df2 - 1.0) / (t2.0 - t1.0);
        close(c.forward_rate_simple(t1, t2).0, expected, 1e-12);
    }

    #[test]
    fn extrapolates_at_constant_final_forward() {
        let c = sample();
        // Beyond the last pillar (10y) the forward is the final segment's constant forward.
        let final_fwd = (0.790_0f64.ln() - 0.620_0f64.ln()) / (10.0 - 5.0);
        close(c.instantaneous_forward(Time(15.0)).0, final_fwd, 1e-12);
        // ...and the discount factor follows that flat-forward extension exactly.
        let df10 = 0.620_0f64;
        let expected = df10 * (-final_fwd * (15.0 - 10.0)).exp();
        close(c.discount_factor(Time(15.0)).0, expected, 1e-12);
    }

    #[test]
    fn from_zero_rates_round_trips() {
        let pillars = [
            (Time(1.0), Rate(0.043)),
            (Time(2.0), Rate(0.040)),
            (Time(5.0), Rate(0.041)),
        ];
        let c = Curve::from_zero_rates(&pillars).expect("valid zero pillars");
        for &(t, z) in &pillars {
            close(c.zero_rate(t).0, z.0, 1e-12);
            close(c.discount_factor(t).0, (-z.0 * t.0).exp(), 1e-12);
        }
        close(c.discount_factor(Time(0.0)).0, 1.0, 0.0);
    }

    #[test]
    fn negative_rates_are_allowed() {
        // A discount factor above 1 encodes a negative zero rate and must be accepted.
        let c = Curve::from_log_linear_dfs(&[
            (Time(0.0), Df(1.0)),
            (Time(1.0), Df(1.004)),
            (Time(2.0), Df(1.006)),
        ])
        .expect("negative-rate curve is valid");
        assert!(c.zero_rate(Time(1.0)).0 < 0.0);
    }

    #[test]
    fn rejects_malformed_pillars() {
        assert_eq!(
            Curve::from_log_linear_dfs(&[(Time(0.0), Df(1.0))]).unwrap_err(),
            CurveError::TooFewPillars
        );
        assert_eq!(
            Curve::from_log_linear_dfs(&[(Time(0.1), Df(1.0)), (Time(1.0), Df(0.95))]).unwrap_err(),
            CurveError::MissingOrigin
        );
        assert_eq!(
            Curve::from_log_linear_dfs(&[(Time(0.0), Df(0.9)), (Time(1.0), Df(0.95))]).unwrap_err(),
            CurveError::MissingOrigin
        );
        assert_eq!(
            Curve::from_log_linear_dfs(&[
                (Time(0.0), Df(1.0)),
                (Time(2.0), Df(0.95)),
                (Time(1.0), Df(0.90)),
            ])
            .unwrap_err(),
            CurveError::NonMonotonicTime
        );
        assert_eq!(
            Curve::from_log_linear_dfs(&[(Time(0.0), Df(1.0)), (Time(1.0), Df(0.0))]).unwrap_err(),
            CurveError::NonPositiveDiscountFactor
        );
        assert_eq!(
            Curve::from_zero_rates(&[(Time(0.0), Rate(0.04))]).unwrap_err(),
            CurveError::NonPositiveZeroTime
        );
    }

    #[test]
    fn clone_is_shared_snapshot() {
        let c = sample();
        let d = c.clone();
        // Both observe identical state; the clone shares the pillar storage.
        close(
            d.discount_factor(Time(3.0)).0,
            c.discount_factor(Time(3.0)).0,
            0.0,
        );
        assert_eq!(Arc::strong_count(&c.nodes), 2);
    }
}
