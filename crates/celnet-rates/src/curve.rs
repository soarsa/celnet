//! The immutable discount/forward curve snapshot and its term-structure interpolation.
//!
//! A [`Curve`] stores its pillars in **log-discount-factor space** and supports two interpolation
//! schemes selected at construction ([`Interpolation`]):
//!
//! - **Log-linear-on-log-DF** — the shipping default (`FI-CURVES-SPEC.md` §4, Q10): linear in
//!   `ln DF(t)` against continuous year-fraction time `t`. Local, cheap, exact at the pillars, and
//!   arbitrage-free in discount-factor space, with piecewise-constant continuously-compounded
//!   instantaneous forwards. This is what an independent oracle (QuantLib's
//!   `InterpolatedDiscountCurve` with the `LogLinear` traits) computes, so the closed-form identity
//!   tests below double as the cross-engine agreement check (`FI-VERIFICATION-CONTRACT.md`).
//! - **Monotone-convex-on-forwards** — the smooth view (`FI-CURVES-SPEC.md` §4): a
//!   piecewise-quadratic instantaneous forward that reproduces every pillar discount factor exactly,
//!   stays continuous across pillars (no log-linear sawtooth), and is monotonicity- and
//!   convexity-preserving, so a monotone sequence of discrete forwards yields a monotone forward
//!   curve with no spurious overshoot.
//!
//! Both schemes keep queries allocation-free; the monotone-convex knot forwards are precomputed once
//! at construction. Method/paper provenance lives in prose only — never in identifiers (GUIDE.md §8).

use std::sync::Arc;

use celnet_types::{Df, DiscountCurve, Rate, Time};

/// Absolute tolerance used when validating the curve origin pillar `(t = 0, DF = 1)`.
const ORIGIN_TOL: f64 = 1e-12;

/// A single curve pillar held in log-discount-factor space (`t`, `ln DF(t)`).
#[derive(Clone, Copy, Debug)]
struct Node {
    /// Year-fraction time from the curve reference date (strictly increasing across nodes).
    t: f64,
    /// Natural log of the discount factor at `t` (`<= 0` for non-negative zero rates).
    ln_df: f64,
    /// Precomputed slope `(ln_df - prev.ln_df) / (t - prev.t)` for the segment `[t_{i-1}, t_i]`.
    slope: f64,
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

/// The interpolation scheme a [`Curve`] evaluates between its pillars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    /// Linear in `ln DF` against time: piecewise-constant instantaneous forwards (the default).
    LogLinearDf,
    /// Piecewise-quadratic, continuous, monotonicity-preserving instantaneous forwards (smooth view).
    MonotoneConvexForward,
}

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
    /// Instantaneous forward at each node, precomputed for [`Interpolation::MonotoneConvexForward`];
    /// empty for the log-linear scheme, which needs no per-node state.
    knot_fwds: Arc<[f64]>,
    /// The interpolation scheme evaluated between pillars.
    scheme: Interpolation,
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
        Ok(Self::from_nodes(
            Self::parse_df_pillars(pillars)?,
            Interpolation::LogLinearDf,
        ))
    }

    /// Build a curve from `(time, discount factor)` pillars under monotone-convex-on-forwards
    /// interpolation — the smooth view (`FI-CURVES-SPEC.md` §4).
    ///
    /// The pillar contract is identical to [`Curve::from_log_linear_dfs`]; only the interpolation
    /// differs. The scheme fits a piecewise-quadratic instantaneous forward that (a) reproduces every
    /// pillar discount factor exactly, (b) is continuous across pillars (no log-linear sawtooth), and
    /// (c) is monotonicity- and convexity-preserving, so a monotone sequence of discrete forwards
    /// yields a monotone forward curve with no spurious overshoot. Construction is `O(n)`; queries
    /// stay allocation-free, evaluating one quadratic on the bracketing segment. With a single
    /// segment the scheme coincides with log-linear (a constant forward).
    ///
    /// # Errors
    ///
    /// Returns a [`CurveError`] on the same malformed-pillar conditions as
    /// [`Curve::from_log_linear_dfs`].
    pub fn from_monotone_convex_dfs(pillars: &[(Time, Df)]) -> Result<Self, CurveError> {
        Ok(Self::from_nodes(
            Self::parse_df_pillars(pillars)?,
            Interpolation::MonotoneConvexForward,
        ))
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
        Ok(Self::from_nodes(
            Self::parse_zero_rate_pillars(pillars)?,
            Interpolation::LogLinearDf,
        ))
    }

    /// Build a curve from `(time, continuously-compounded zero rate)` pillars under
    /// monotone-convex-on-forwards interpolation.
    ///
    /// The origin `(0, DF = 1)` is prepended; see [`Curve::from_monotone_convex_dfs`] for the scheme
    /// and [`Curve::from_zero_rates`] for the pillar contract.
    ///
    /// # Errors
    ///
    /// Returns [`CurveError::TooFewPillars`] if empty, [`CurveError::NonPositiveZeroTime`] if any
    /// pillar time is `<= 0`, or [`CurveError::NonMonotonicTime`] if times are not increasing.
    pub fn from_monotone_convex_zero_rates(pillars: &[(Time, Rate)]) -> Result<Self, CurveError> {
        Ok(Self::from_nodes(
            Self::parse_zero_rate_pillars(pillars)?,
            Interpolation::MonotoneConvexForward,
        ))
    }

    /// Validate `(time, DF)` pillars and convert them to ascending log-DF nodes.
    fn parse_df_pillars(pillars: &[(Time, Df)]) -> Result<Vec<Node>, CurveError> {
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
                slope: 0.0,
            });
        }
        for i in 1..nodes.len() {
            let dt = nodes[i].t - nodes[i - 1].t;
            nodes[i].slope = (nodes[i].ln_df - nodes[i - 1].ln_df) / dt;
        }
        Ok(nodes)
    }

    /// Prepend the origin pillar and convert `(time, zero rate)` pillars directly to nodes,
    /// avoiding intermediate vector allocation and redundant `exp()`/`ln()` transcendental calls.
    fn parse_zero_rate_pillars(pillars: &[(Time, Rate)]) -> Result<Vec<Node>, CurveError> {
        if pillars.is_empty() {
            return Err(CurveError::TooFewPillars);
        }
        let mut nodes: Vec<Node> = Vec::with_capacity(pillars.len() + 1);
        nodes.push(Node {
            t: 0.0,
            ln_df: 0.0,
            slope: 0.0,
        });
        let mut prev_t = 0.0;
        for &(t, z) in pillars {
            if t.0 <= 0.0 {
                return Err(CurveError::NonPositiveZeroTime);
            }
            if t.0 <= prev_t {
                return Err(CurveError::NonMonotonicTime);
            }
            prev_t = t.0;
            nodes.push(Node {
                t: t.0,
                ln_df: -z.0 * t.0,
                slope: 0.0,
            });
        }
        for i in 1..nodes.len() {
            let dt = nodes[i].t - nodes[i - 1].t;
            nodes[i].slope = (nodes[i].ln_df - nodes[i - 1].ln_df) / dt;
        }
        Ok(nodes)
    }

    /// Assemble a curve from validated nodes, precomputing knot forwards for the smooth scheme.
    fn from_nodes(nodes: Vec<Node>, scheme: Interpolation) -> Self {
        let knot_fwds: Vec<f64> = match scheme {
            Interpolation::LogLinearDf => Vec::new(),
            Interpolation::MonotoneConvexForward => knot_forwards(&nodes),
        };
        Self {
            nodes: Arc::from(nodes),
            knot_fwds: Arc::from(knot_fwds),
            scheme,
        }
    }

    /// The interpolation scheme this curve evaluates between its pillars.
    #[must_use]
    pub fn interpolation(&self) -> Interpolation {
        self.scheme
    }

    /// The latest pillar time on the curve (its calibrated horizon); queries past this point
    /// extrapolate at the final segment's constant instantaneous forward.
    #[must_use]
    pub fn max_time(&self) -> Time {
        Time(self.nodes[self.nodes.len() - 1].t)
    }

    /// Index `hi` of the first pillar with time strictly greater than `t`, clamped so that
    /// `[hi - 1, hi]` is a real segment (binary search). Below the first or above the last pillar the
    /// nearest segment is returned, giving the standard flat-forward extrapolation.
    fn bracket(&self, t: f64) -> usize {
        let n = self.nodes.len();
        self.nodes.partition_point(|nd| nd.t <= t).clamp(1, n - 1)
    }

    /// `ln DF(t)` under the active interpolation scheme, with flat-forward extrapolation at both ends.
    fn ln_df(&self, t: f64) -> f64 {
        match self.scheme {
            Interpolation::LogLinearDf => self.ln_df_log_linear(t),
            Interpolation::MonotoneConvexForward => self.ln_df_monotone_convex(t),
        }
    }

    /// `ln DF(t)` for log-linear-on-log-DF: linear in `ln DF` with flat-forward extrapolation.
    fn ln_df_log_linear(&self, t: f64) -> f64 {
        let hi = self.bracket(t);
        let a = self.nodes[hi - 1];
        let slope = self.nodes[hi].slope;
        a.ln_df + slope * (t - a.t)
    }

    /// `ln DF(t)` for monotone-convex-on-forwards: integrate the piecewise-quadratic forward.
    ///
    /// On the bracketing segment `[t_{i-1}, t_i]` the instantaneous forward is `f^d_i + g(x)`, where
    /// `x = (t − t_{i-1}) / Δt` and `g` is the region quadratic with `∫_0^1 g = 0` (so the segment
    /// reprices its far pillar exactly). Hence
    /// `ln DF(t) = ln DF(t_{i-1}) − [f^d_i·(t − t_{i-1}) + Δt·∫_0^x g]`. Beyond the last pillar the
    /// forward is held flat at the final knot forward.
    fn ln_df_monotone_convex(&self, t: f64) -> f64 {
        let n = self.nodes.len();
        let last = self.nodes[n - 1];
        if t >= last.t {
            return last.ln_df - self.knot_fwds[n - 1] * (t - last.t);
        }
        let hi = self.bracket(t);
        let a = self.nodes[hi - 1];
        let b = self.nodes[hi];
        let dt = b.t - a.t;
        let fdisc = -self.nodes[hi].slope;
        let g0 = self.knot_fwds[hi - 1] - fdisc;
        let g1 = self.knot_fwds[hi] - fdisc;
        let x = (t - a.t) / dt;
        let (_, integral) = mc_segment(g0, g1, x);
        a.ln_df - (fdisc * (t - a.t) + dt * integral)
    }

    /// The negated slope of the segment bracketing `t` — the (continuously-compounded) instantaneous
    /// forward, which is piecewise-constant for the log-linear scheme.
    fn segment_forward(&self, t: f64) -> f64 {
        let hi = self.bracket(t);
        -self.nodes[hi].slope
    }

    /// The continuous instantaneous forward under monotone-convex interpolation: `f^d_i + g(x)` on
    /// the bracketing segment, held flat at the boundary knot forwards outside the calibrated range.
    fn monotone_convex_forward(&self, t: f64) -> f64 {
        let n = self.nodes.len();
        if t <= 0.0 {
            return self.knot_fwds[0];
        }
        if t >= self.nodes[n - 1].t {
            return self.knot_fwds[n - 1];
        }
        let hi = self.bracket(t);
        let a = self.nodes[hi - 1];
        let b = self.nodes[hi];
        let dt = b.t - a.t;
        let fdisc = (a.ln_df - b.ln_df) / dt;
        let g0 = self.knot_fwds[hi - 1] - fdisc;
        let g1 = self.knot_fwds[hi] - fdisc;
        let x = (t - a.t) / dt;
        let (g, _) = mc_segment(g0, g1, x);
        fdisc + g
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
            return self.instantaneous_forward(Time(0.0));
        }
        Rate(-self.ln_df(t.0) / t.0)
    }

    /// The continuously-compounded instantaneous forward `f(t) = -d ln DF / dt`.
    ///
    /// Under log-linear-on-log-DF this is piecewise-constant within each pillar segment (the
    /// characteristic "sawtooth"); under monotone-convex-on-forwards it is a continuous,
    /// monotonicity-preserving piecewise quadratic.
    #[must_use]
    pub fn instantaneous_forward(&self, t: Time) -> Rate {
        match self.scheme {
            Interpolation::LogLinearDf => Rate(self.segment_forward(t.0)),
            Interpolation::MonotoneConvexForward => Rate(self.monotone_convex_forward(t.0)),
        }
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

/// The general **term-structure** case of the [`DiscountCurve`] contract (ADR-0010 §2.1):
/// a [`Curve`] is the bootstrapped many-pillar curve of which [`celnet_types::Carry`] is
/// the degenerate one-pillar special case. The trait method is a **zero-cost delegation**
/// to the inherent [`Curve::discount_factor`] — it merely bridges the newtype boundary
/// (`f64 → Time`, `Df → f64`) so the same discount contract reads uniformly over both the
/// flat carry and the full curve. `forward_factor` inherits the trait default `1/DF(0,t)`
/// — the single-curve capitalization factor — which is the correct outright growth for a
/// bare discount curve (a `Curve` carries no separate asset/foreign leg of its own).
impl DiscountCurve for Curve {
    #[inline]
    fn discount_factor(&self, t: f64) -> f64 {
        // Explicit inherent-method path selects `Curve::discount_factor(&self, Time) -> Df`
        // (never the trait method of the same name) — unwrap the `Df` newtype to `f64`.
        Curve::discount_factor(self, Time(t)).0
    }
}

/// Instantaneous forwards at the pillar knots for the monotone-convex scheme.
///
/// Each interval `i` carries the discrete (continuously-compounded) forward
/// `f^d_i = (ln DF(t_{i-1}) − ln DF(t_i)) / Δt_i`. An interior knot forward is the
/// distance-weighted blend of its two neighbouring discrete forwards (so the closer interval gets
/// the larger weight); the two boundary knots reflect the first/last interior knot through the
/// adjacent discrete forward. A lone segment degenerates to a constant forward, where the scheme
/// coincides with log-linear.
fn knot_forwards(nodes: &[Node]) -> Vec<f64> {
    let n = nodes.len();
    let fdisc = |k: usize| -nodes[k].slope;
    let mut f = vec![0.0_f64; n];
    for j in 1..n - 1 {
        let span = nodes[j + 1].t - nodes[j - 1].t;
        let weight_right = (nodes[j].t - nodes[j - 1].t) / span; // closer to interval j+1's forward
        let weight_left = (nodes[j + 1].t - nodes[j].t) / span; // closer to interval j's forward
        f[j] = weight_right * fdisc(j + 1) + weight_left * fdisc(j);
    }
    if n >= 3 {
        f[0] = fdisc(1) - 0.5 * (f[1] - fdisc(1));
        f[n - 1] = fdisc(n - 1) - 0.5 * (f[n - 2] - fdisc(n - 1));
    } else {
        let flat = fdisc(1);
        f[0] = flat;
        f[1] = flat;
    }
    f
}

/// Evaluate the monotone-convex segment deviation `g(x)` and its integral `∫_0^x g(u) du` for a
/// normalized position `x ∈ [0, 1]`, given the endpoint deviations `g0 = g(0)` and `g1 = g(1)` of
/// the instantaneous forward from the interval's discrete forward.
///
/// Each of the four `(g0, g1)`-plane regions selects a quadratic (region 1) or a pair of
/// flat/quadratic pieces (regions 2–4) chosen so the result is monotonicity- and
/// convexity-preserving while satisfying `∫_0^1 g = 0` — hence every segment reprices its far
/// pillar exactly, for any region. The all-flat case `g0 = g1 = 0` returns `g ≡ 0`. The integrals
/// here are the exact antiderivatives of the same `g`, so the forward and `ln DF` stay consistent.
fn mc_segment(g0: f64, g1: f64, x: f64) -> (f64, f64) {
    // A flat segment (both endpoints already on the discrete forward) contributes nothing.
    if g0.abs() <= 0.0 && g1.abs() <= 0.0 {
        return (0.0, 0.0);
    }

    let g1p2g0 = g1 + 2.0 * g0; // boundary line A
    let g0p2g1 = g0 + 2.0 * g1; // boundary line B

    let region1 = (g1p2g0 < 0.0 && g0p2g1 >= 0.0) || (g1p2g0 > 0.0 && g0p2g1 <= 0.0);
    let region2 = (g0 < 0.0 && g1p2g0 >= 0.0) || (g0 > 0.0 && g1p2g0 <= 0.0);
    let region3 = (g1 <= 0.0 && g0p2g1 > 0.0) || (g1 >= 0.0 && g0p2g1 < 0.0);

    if region1 {
        // g(x) = 3(g0+g1)x² − 2(g1+2g0)x + g0;  G(x) = (g0+g1)x³ − (g1+2g0)x² + g0·x.
        let g = (3.0 * (g0 + g1) * x - 2.0 * g1p2g0) * x + g0;
        let big_g = ((g0 + g1) * x - g1p2g0) * x * x + g0 * x;
        (g, big_g)
    } else if region2 {
        // Flat at g0 over [0, eta], then quadratic to g1.
        let eta = g1p2g0 / (g1 - g0);
        if x > eta {
            let w = 1.0 - eta;
            let r = (x - eta) / w;
            let g = g0 + (g1 - g0) * r * r;
            let big_g = g0 * x + (g1 - g0) * (x - eta).powi(3) / (3.0 * w * w);
            (g, big_g)
        } else {
            (g0, g0 * x)
        }
    } else if region3 {
        // Quadratic from g0 over [0, eta], then flat at g1.
        let eta = 3.0 * g1 / (g1 - g0);
        if eta > 0.0 {
            if x <= eta {
                let r = (eta - x) / eta;
                let g = g1 + (g0 - g1) * r * r;
                let big_g =
                    g1 * x + (g0 - g1) * (eta.powi(3) - (eta - x).powi(3)) / (3.0 * eta * eta);
                (g, big_g)
            } else {
                (g1, g1 * x + (g0 - g1) * eta / 3.0)
            }
        } else {
            // Degenerate eta = 0 (g1 = 0): the leading quadratic region has zero width.
            (g1, g1 * x)
        }
    } else {
        // Region 4: g0 and g1 share a sign; two quadratics meet at the shifted level `shift`.
        let eta = g1 / (g0 + g1);
        let shift = -0.5 * (eta * g0 + (1.0 - eta) * g1);
        if x <= eta {
            let r = (eta - x) / eta;
            let g = shift + (g0 - shift) * r * r;
            let big_g =
                shift * x + (g0 - shift) * (eta.powi(3) - (eta - x).powi(3)) / (3.0 * eta * eta);
            (g, big_g)
        } else {
            let w = 1.0 - eta;
            let r = (x - eta) / w;
            let g = shift + (g1 - shift) * r * r;
            let big_g = shift * x
                + (g0 - shift) * eta / 3.0
                + (g1 - shift) * (x - eta).powi(3) / (3.0 * w * w);
            (g, big_g)
        }
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

    /// Phase-0 delegation gate (ADR-0010 §2.1): the `DiscountCurve` trait method must be a
    /// zero-cost delegation to the inherent `Curve::discount_factor`, bit-for-bit, after
    /// bridging the newtype boundary (`f64 → Time`, `Df → f64`). Exercised across the
    /// pillars, between pillars, at the origin, and before the reference date.
    #[test]
    fn discount_curve_trait_delegates_to_inherent() {
        let c = sample();
        for &t in &[-1.0, 0.0, 0.25, 0.5, 1.3, 2.0, 4.7, 10.0, 15.0] {
            assert_eq!(
                <Curve as DiscountCurve>::discount_factor(&c, t).to_bits(),
                Curve::discount_factor(&c, Time(t)).0.to_bits(),
                "DiscountCurve::discount_factor must delegate byte-identically at t={t}"
            );
        }
    }

    /// The whole point of Phase 0: a flat [`celnet_types::Carry`] viewed as a
    /// [`DiscountCurve`] is the **degenerate one-pillar case** of the general term-structure
    /// [`Curve`]. A single-rate `Curve` built from the same continuously-compounded rate `r`
    /// discounts to `e^{−r·t}` at every horizon; the flat carry discounts to `e^{−r·t}` too,
    /// so the two agree to within floating-point noise (`Carry` uses `libm::exp`, the curve
    /// uses `f64::exp`), well inside 1e-12. Both are read *through the one `DiscountCurve`
    /// contract* — proving the flat carry is the general curve's single-point special case.
    #[test]
    fn flat_carry_equals_single_rate_curve_through_the_trait() {
        for &r in &[0.0, 0.011, 0.025, 0.05, -0.004] {
            // Flat term structure: two pillars at the SAME zero rate ⇒ DF(t) = e^{−r·t}
            // everywhere (log-linear is exact on a straight ln-DF line; constant-forward
            // extrapolation holds it flat beyond the last pillar).
            let flat: Curve =
                Curve::from_zero_rates(&[(Time(1.0), Rate(r)), (Time(30.0), Rate(r))])
                    .expect("valid flat pillars");
            // Discounting depends only on the numeraire rate `r`; `b` (here 0) is irrelevant.
            let carry = celnet_types::Carry::CostOfCarry { r, b: 0.0 };
            for &t in &[0.0, 0.25, 1.0, 2.5, 7.0, 20.0, 30.0] {
                let via_curve = <Curve as DiscountCurve>::discount_factor(&flat, t);
                let via_carry = <celnet_types::Carry as DiscountCurve>::discount_factor(&carry, t);
                close(via_curve, via_carry, 1e-12);
            }
        }
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

    // ----------------------------------------------------------------------------------------------
    // Monotone-convex-on-forwards (the smooth view).
    // ----------------------------------------------------------------------------------------------

    /// The same downward-sloping pillars as [`sample`], under monotone-convex interpolation.
    fn sample_mc() -> Curve {
        Curve::from_monotone_convex_dfs(&[
            (Time(0.0), Df(1.0)),
            (Time(0.5), Df(0.978_0)),
            (Time(1.0), Df(0.955_0)),
            (Time(2.0), Df(0.910_0)),
            (Time(5.0), Df(0.790_0)),
            (Time(10.0), Df(0.620_0)),
        ])
        .expect("valid pillars")
    }

    #[test]
    fn interpolation_accessor_reports_the_scheme() {
        assert_eq!(sample().interpolation(), Interpolation::LogLinearDf);
        assert_eq!(
            sample_mc().interpolation(),
            Interpolation::MonotoneConvexForward
        );
    }

    #[test]
    fn monotone_convex_reproduces_pillar_discount_factors() {
        // The defining property: every pillar DF is reproduced exactly, just like log-linear.
        let c = sample_mc();
        for &(t, df) in &[
            (0.5, 0.978_0),
            (1.0, 0.955_0),
            (2.0, 0.910_0),
            (5.0, 0.790_0),
            (10.0, 0.620_0),
        ] {
            close(c.discount_factor(Time(t)).0, df, 1e-12);
        }
        close(c.discount_factor(Time(0.0)).0, 1.0, 0.0);
    }

    #[test]
    fn monotone_convex_forward_is_continuous_across_pillars() {
        // The smooth view's headline win over log-linear: no forward jump at the pillars.
        let c = sample_mc();
        let h = 1e-7;
        for &t in &[0.5, 1.0, 2.0, 5.0] {
            let left = c.instantaneous_forward(Time(t - h)).0;
            let right = c.instantaneous_forward(Time(t + h)).0;
            let at = c.instantaneous_forward(Time(t)).0;
            close(left, right, 1e-4);
            close(at, right, 1e-4);
        }
    }

    #[test]
    fn monotone_convex_forward_matches_the_log_discount_derivative() {
        // Internal consistency: the instantaneous forward is exactly -d ln DF / dt, i.e. the stored
        // integral is the antiderivative of the stored forward. Sample mid-segment to avoid kinks.
        let c = sample_mc();
        let h = 1e-6;
        for &t in &[0.3, 0.7, 1.4, 3.2, 7.5] {
            let ln_lo = c.discount_factor(Time(t - h)).0.ln();
            let ln_hi = c.discount_factor(Time(t + h)).0.ln();
            let fd = -(ln_hi - ln_lo) / (2.0 * h);
            close(c.instantaneous_forward(Time(t)).0, fd, 1e-5);
        }
    }

    #[test]
    fn monotone_convex_continuous_forward_reprices_the_far_discount_factor() {
        let c = sample_mc();
        let (t1, t2) = (Time(1.0), Time(5.0));
        let f = c.forward_rate_continuous(t1, t2).0;
        let df1 = c.discount_factor(t1).0;
        let df2 = c.discount_factor(t2).0;
        close(df1 * (-f * (t2.0 - t1.0)).exp(), df2, 1e-12);
    }

    #[test]
    fn monotone_convex_preserves_monotone_forwards() {
        // Hagan-West guarantee: monotone discrete forwards yield a monotone forward curve (no
        // spurious overshoot). Build a curve from strictly increasing per-segment forwards.
        let fwds = [0.01_f64, 0.02, 0.03, 0.04, 0.05];
        let mut ln_df = 0.0_f64;
        let mut pillars = vec![(Time(0.0), Df(1.0))];
        for (k, f) in fwds.iter().enumerate() {
            ln_df -= f; // each segment has unit length, so Δln DF = -f
            pillars.push((Time((k + 1) as f64), Df(ln_df.exp())));
        }
        let c = Curve::from_monotone_convex_dfs(&pillars).expect("valid pillars");

        let mut prev = c.instantaneous_forward(Time(0.0)).0;
        let mut t = 0.0;
        while t <= 5.0 {
            let f = c.instantaneous_forward(Time(t)).0;
            assert!(f >= prev - 1e-9, "forward decreased at t={t}: {f} < {prev}");
            prev = f;
            t += 0.02;
        }
    }

    #[test]
    fn monotone_convex_coincides_with_log_linear_on_a_single_segment() {
        // One interval has a constant forward under both schemes, so they must agree everywhere.
        let pillars = [(Time(0.0), Df(1.0)), (Time(1.0), Df(0.955))];
        let ll = Curve::from_log_linear_dfs(&pillars).expect("valid");
        let mc = Curve::from_monotone_convex_dfs(&pillars).expect("valid");
        for t in [0.1, 0.25, 0.5, 0.75, 0.9] {
            close(
                mc.discount_factor(Time(t)).0,
                ll.discount_factor(Time(t)).0,
                1e-14,
            );
            close(
                mc.instantaneous_forward(Time(t)).0,
                ll.instantaneous_forward(Time(t)).0,
                1e-12,
            );
        }
    }

    #[test]
    fn monotone_convex_zero_rates_round_trip() {
        let pillars = [
            (Time(1.0), Rate(0.043)),
            (Time(2.0), Rate(0.040)),
            (Time(5.0), Rate(0.041)),
        ];
        let c = Curve::from_monotone_convex_zero_rates(&pillars).expect("valid zero pillars");
        for &(t, z) in &pillars {
            close(c.zero_rate(t).0, z.0, 1e-12);
            close(c.discount_factor(t).0, (-z.0 * t.0).exp(), 1e-12);
        }
        assert_eq!(c.interpolation(), Interpolation::MonotoneConvexForward);
    }

    #[test]
    fn monotone_convex_allows_negative_rates() {
        let c = Curve::from_monotone_convex_dfs(&[
            (Time(0.0), Df(1.0)),
            (Time(1.0), Df(1.004)),
            (Time(2.0), Df(1.006)),
        ])
        .expect("negative-rate curve is valid");
        assert!(c.zero_rate(Time(1.0)).0 < 0.0);
        close(c.discount_factor(Time(1.0)).0, 1.004, 1e-12);
    }
}
