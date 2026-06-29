//! Convexity-adjusted short-end curve bootstrap from a strip of STIR futures.
//!
//! A short-term-rate future is margined daily, so its quoted rate is a **biased**
//! estimate of the curve's forward fixing — biased high by the convexity
//! adjustment (see [`crate::futures`]). The standard short-end build therefore
//! debiases each contract before constraining the curve:
//!
//! ```text
//! forward(T1, T2) = futures_rate − convexity(T1, T2, σ)
//! DF(T2)         = DF(T1) / ( 1 + forward(T1, T2) · (T2 − T1) )
//! ```
//!
//! Walking a contiguous strip of futures from a known front discount factor
//! yields a discount curve whose simply-compounded forward over each contract
//! window is the **unbiased** forward, not the futures rate. Because the per-step
//! discount factor is a closed-form division (no root solve), the strip
//! bootstrap is exact and allocation-light.
//!
//! The convexity term reuses [`crate::futures::convexity_adjustment`], the
//! one-factor Gaussian (constant normal vol) deterministic adjustment
//! `½·σ²·T1·T2`. With `σ = 0` the implied forward collapses onto the futures
//! rate (no debiasing). Method provenance lives in prose only (CLAUDE.md §8).

use crate::curve::{Curve, CurveError};
use crate::futures::{StirFuture, convexity_adjustment};
use celnet_types::{Df, Rate, Time};

/// A quoted STIR future for the short-end strip: the contract window, its quoted
/// rate, and the convexity volatility used to debias it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StirFuturesQuote {
    /// The contract's reference-rate fixing window `[T1, T2]`.
    pub future: StirFuture,
    /// The quoted futures rate `(100 − price) / 100` over the window.
    pub futures_rate: Rate,
    /// The convexity volatility `σ` (absolute/normal). Zero ⇒ no adjustment.
    pub convexity_vol: f64,
}

/// Failure modes of the futures-strip bootstrap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FuturesStripError {
    /// No futures were supplied.
    EmptyStrip,
    /// The strip's first contract does not fix strictly after the curve origin
    /// (a short-end strip is anchored by a front discount factor at `T1 > 0`).
    FrontFixesAtOrigin,
    /// Consecutive contracts are not contiguous (`next.T1 != prev.T2`).
    NonContiguousStrip,
    /// A supplied convexity volatility was negative.
    NegativeVol,
    /// The front discount factor was not strictly positive.
    NonPositiveFrontDf,
    /// A debiased forward implied a non-positive discount factor
    /// (`1 + forward·(T2 − T1) ≤ 0`).
    NonPositiveImpliedDf,
    /// The assembled curve was rejected.
    Curve(CurveError),
}

impl core::fmt::Display for FuturesStripError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyStrip => f.write_str("the futures strip is empty"),
            Self::FrontFixesAtOrigin => {
                f.write_str("the strip's first contract must fix strictly after the curve origin")
            }
            Self::NonContiguousStrip => {
                f.write_str("consecutive futures must be contiguous (next.T1 == prev.T2)")
            }
            Self::NegativeVol => f.write_str("convexity volatility must be non-negative"),
            Self::NonPositiveFrontDf => f.write_str("the front discount factor must be positive"),
            Self::NonPositiveImpliedDf => {
                f.write_str("a debiased forward implied a non-positive discount factor")
            }
            Self::Curve(_) => f.write_str("the assembled short-end curve was rejected"),
        }
    }
}

impl core::error::Error for FuturesStripError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Curve(e) => Some(e),
            _ => None,
        }
    }
}

/// The convexity-debiased forward implied by a single futures quote.
///
/// `futures_rate − ½·σ²·T1·T2`. Equals the futures rate when `σ = 0`; strictly
/// below it for any `σ > 0`, which is the correct sign of the futures bias.
#[must_use]
pub fn implied_forward_rate(quote: &StirFuturesQuote) -> Rate {
    Rate(quote.futures_rate.0 - convexity_adjustment(&quote.future, quote.convexity_vol))
}

/// Bootstrap a short-end discount curve from a contiguous, convexity-adjusted
/// futures strip.
///
/// `front_df` is the discount factor to the first contract's fixing start
/// (`strip[0].future.fixing_start()`), supplied by the cash/deposit stub that
/// precedes the strip. The returned curve carries the origin `(0, 1)`, the front
/// pillar, and one pillar per contract end; its
/// [`Curve::forward_rate_simple`] over each window equals
/// [`implied_forward_rate`] (the unbiased forward), not the futures rate.
///
/// # Errors
///
/// Returns a [`FuturesStripError`] if the strip is empty, fixes at the origin,
/// is non-contiguous, carries a negative volatility or non-positive front
/// discount factor, implies a non-positive discount factor, or yields a curve
/// the constructor rejects.
pub fn bootstrap_futures_strip(
    front_df: Df,
    strip: &[StirFuturesQuote],
) -> Result<Curve, FuturesStripError> {
    let first = strip.first().ok_or(FuturesStripError::EmptyStrip)?;
    if front_df.0 <= 0.0 {
        return Err(FuturesStripError::NonPositiveFrontDf);
    }
    let t_start = first.future.fixing_start();
    if t_start.0 <= 0.0 {
        return Err(FuturesStripError::FrontFixesAtOrigin);
    }
    for q in strip {
        if q.convexity_vol < 0.0 {
            return Err(FuturesStripError::NegativeVol);
        }
    }
    for pair in strip.windows(2) {
        let gap = pair[1].future.fixing_start().0 - pair[0].future.fixing_end().0;
        if gap.abs() > CONTIGUITY_TOL {
            return Err(FuturesStripError::NonContiguousStrip);
        }
    }

    let mut pillars: Vec<(Time, Df)> = Vec::with_capacity(strip.len() + 2);
    pillars.push((Time(0.0), Df(1.0)));
    pillars.push((t_start, front_df));

    let mut df_prev = front_df.0;
    for q in strip {
        let t1 = q.future.fixing_start().0;
        let t2 = q.future.fixing_end().0;
        let tau = t2 - t1;
        let forward = implied_forward_rate(q).0;
        let growth = 1.0 + forward * tau;
        if growth <= 0.0 {
            return Err(FuturesStripError::NonPositiveImpliedDf);
        }
        df_prev /= growth;
        pillars.push((Time(t2), Df(df_prev)));
    }

    Curve::from_log_linear_dfs(&pillars).map_err(FuturesStripError::Curve)
}

/// Tolerance (in curve-time years) for the contract-contiguity check.
const CONTIGUITY_TOL: f64 = 1e-12;

#[cfg(test)]
mod tests {
    use super::*;

    fn future(t1: f64, t2: f64) -> StirFuture {
        StirFuture::new(Time(t1), Time(t2)).expect("valid future")
    }

    /// A four-contract contiguous quarterly strip starting one year out.
    fn strip(vol: f64) -> Vec<StirFuturesQuote> {
        [
            (1.00, 1.25, 0.0430),
            (1.25, 1.50, 0.0440),
            (1.50, 1.75, 0.0450),
            (1.75, 2.00, 0.0460),
        ]
        .into_iter()
        .map(|(t1, t2, rate)| StirFuturesQuote {
            future: future(t1, t2),
            futures_rate: Rate(rate),
            convexity_vol: vol,
        })
        .collect()
    }

    const FRONT_DF: Df = Df(0.9576); // ~ exp(-0.0433 * 1.0): a representative 1y stub DF.

    /// With zero volatility the curve reprices each window's forward to the
    /// futures rate exactly — no debiasing.
    #[test]
    fn zero_vol_recovers_the_futures_rate() {
        let quotes = strip(0.0);
        let curve = bootstrap_futures_strip(FRONT_DF, &quotes).expect("bootstraps");
        for q in &quotes {
            let fwd = curve
                .forward_rate_simple(q.future.fixing_start(), q.future.fixing_end())
                .0;
            assert!(
                (fwd - q.futures_rate.0).abs() < 1e-12,
                "fwd {fwd} vs futures {}",
                q.futures_rate.0
            );
        }
    }

    /// With positive volatility the implied forward sits **below** the futures
    /// rate by exactly the convexity adjustment (correct sign and magnitude).
    #[test]
    fn convexity_debiases_the_forward_downward() {
        let vol = 0.0075;
        let quotes = strip(vol);
        let curve = bootstrap_futures_strip(FRONT_DF, &quotes).expect("bootstraps");
        for q in &quotes {
            let fwd = curve
                .forward_rate_simple(q.future.fixing_start(), q.future.fixing_end())
                .0;
            let conv = convexity_adjustment(&q.future, vol);
            assert!(conv > 0.0);
            assert!(
                fwd < q.futures_rate.0,
                "forward must be below the futures rate"
            );
            assert!(
                (fwd - (q.futures_rate.0 - conv)).abs() < 1e-12,
                "fwd {fwd} vs futures-conv {}",
                q.futures_rate.0 - conv
            );
        }
    }

    /// Independent published-oracle check of the convexity magnitude used by the
    /// strip. Hull, *Options, Futures, and Other Derivatives* gives, for the
    /// one-factor Gaussian adjustment `½·σ²·t₁·t₂` with σ = 0.012, t₁ = 8,
    /// t₂ = 8.25 years, a convexity of 0.004752 (47.52 bp). The strip must move
    /// the implied forward by exactly that amount for that contract.
    #[test]
    fn convexity_magnitude_matches_published_reference() {
        let hull = future(8.0, 8.25);
        let conv = convexity_adjustment(&hull, 0.012);
        assert!(
            (conv - 0.004752).abs() < 1e-9,
            "convexity {conv} vs Hull 0.004752"
        );

        // And smaller-tenor Hull figures, re-derived from the same formula.
        assert!((convexity_adjustment(&future(2.0, 2.25), 0.012) - 0.000324).abs() < 1e-9);
        assert!((convexity_adjustment(&future(1.0, 1.25), 0.012) - 0.00009).abs() < 1e-9);
    }

    /// The closed-form discount-factor recursion is internally consistent: each
    /// pillar discount factor equals the previous one divided by `1 + fwd·tau`.
    #[test]
    fn discount_factors_compound_through_the_strip() {
        let vol = 0.0075;
        let quotes = strip(vol);
        let curve = bootstrap_futures_strip(FRONT_DF, &quotes).expect("bootstraps");
        let mut df_prev = FRONT_DF.0;
        for q in &quotes {
            let tau = q.future.fixing_end().0 - q.future.fixing_start().0;
            let fwd = q.futures_rate.0 - convexity_adjustment(&q.future, vol);
            let expected = df_prev / (1.0 + fwd * tau);
            let actual = curve.discount_factor(q.future.fixing_end()).0;
            assert!(
                (actual - expected).abs() < 1e-12,
                "df {actual} vs {expected}"
            );
            df_prev = expected;
        }
    }

    #[test]
    fn rejects_malformed_strips() {
        assert_eq!(
            bootstrap_futures_strip(FRONT_DF, &[]).unwrap_err(),
            FuturesStripError::EmptyStrip
        );
        // Non-contiguous: a gap between the first two contracts.
        let mut gapped = strip(0.0);
        gapped[1].future = future(1.30, 1.55);
        assert_eq!(
            bootstrap_futures_strip(FRONT_DF, &gapped).unwrap_err(),
            FuturesStripError::NonContiguousStrip
        );
        // Negative vol.
        let mut neg = strip(0.0);
        neg[0].convexity_vol = -0.001;
        assert_eq!(
            bootstrap_futures_strip(FRONT_DF, &neg).unwrap_err(),
            FuturesStripError::NegativeVol
        );
        // Non-positive front DF.
        assert_eq!(
            bootstrap_futures_strip(Df(0.0), &strip(0.0)).unwrap_err(),
            FuturesStripError::NonPositiveFrontDf
        );
    }
}
