//! Market (broker) strangle → smile-strangle calibration.
//!
//! The single most expensive convention error in an FX-options surface is to
//! treat the quoted butterfly as the arithmetic smile-strangle
//! (`docs/ANALYTICS-SPEC.md` §1.4 — "the #1 production bug"). What the broker
//! actually trades is the **market (broker) strangle**: a *single* volatility
//!
//! ```text
//!   σ_strangle = σ_ATM + BF_broker
//! ```
//!
//! applied to **both** wing strikes. Those two strikes are the strikes whose
//! `±dΔ` delta equals the pillar delta *at that single strangle vol*:
//!
//! ```text
//!   K_call = strike( +dΔ ; σ_strangle ),   K_put = strike( −dΔ ; σ_strangle ).
//! ```
//!
//! The **market strangle price** is the sum of those two vanillas priced with
//! `σ_strangle`. A correctly-built smile must reprice exactly this number while
//! also reproducing the risk-reversal `σ_call − σ_put = RR`. The two *smile* wing
//! volatilities `(σ_call, σ_put)` that achieve this are generally **not**
//! `σ_ATM ± ½RR + BF` — that arithmetic identity only holds when the broker and
//! smile strangles coincide, which they do not for high-RR / EM pairs.
//!
//! This module solves for the smile wing volatilities by the
//! Reiswich-Wystup (2010) / Clark (2011) fixed point: parametrise the smile by a
//! single **smile-strangle** `σ_ss` so that
//!
//! ```text
//!   σ_call = σ_ATM + σ_ss + ½RR,    σ_put = σ_ATM + σ_ss − ½RR,
//! ```
//!
//! and choose `σ_ss` such that the smile (a [`crate::vannavolga`] model built
//! from `σ_ATM`, `RR`, `σ_ss`) reprices the market strangle. The arithmetic
//! butterfly is the natural initial guess `σ_ss ≈ BF_broker`; the calibration is
//! the correction. The residual is monotone in `σ_ss`, so a guarded
//! secant/bisection converges quickly and deterministically.

use celnet_core::math::sqrt;
use celnet_types::OptionType;
use celnet_vanilla::price;

use crate::quotes::{DeltaPillar, MarketContext, RiskReversalButterfly};

/// The calibrated smile-wing volatilities at one delta pillar, together with the
/// fully-resolved wing strikes and the broker-strangle target they reprice.
///
/// `call_vol`/`put_vol` are the *smile* volatilities at the *smile* wing strikes
/// `call_strike`/`put_strike` — the points the smile model must pass through.
/// `smile_strangle` is the calibrated convexity `σ_ss` (the correction to the
/// arithmetic butterfly). `market_strangle_price` is the broker target the smile
/// reprices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibratedPillar {
    /// The delta pillar this calibration is for.
    pub pillar: DeltaPillar,
    /// Smile volatility at the smile call-wing strike.
    pub call_vol: f64,
    /// Smile volatility at the smile put-wing strike.
    pub put_vol: f64,
    /// Smile call-wing strike (delta pillar at `call_vol`).
    pub call_strike: f64,
    /// Smile put-wing strike (delta pillar at `put_vol`).
    pub put_strike: f64,
    /// Calibrated smile-strangle `σ_ss` (the corrected convexity).
    pub smile_strangle: f64,
    /// The market (broker) strangle price the smile reprices.
    pub market_strangle_price: f64,
}

/// Why a strangle calibration could not be completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationError {
    /// A delta→strike inversion failed inside the calibration (wing unreachable
    /// in the convention, or solver non-convergence).
    StrikeInversion,
    /// The smile-strangle fixed point did not converge within the budget.
    NoConvergence,
}

impl From<celnet_vanilla::DeltaSolveError> for CalibrationError {
    fn from(_: celnet_vanilla::DeltaSolveError) -> Self {
        CalibrationError::StrikeInversion
    }
}

/// Absolute convergence tolerance on the strangle-price residual, scaled by the
/// market strangle price inside [`calibrate_pillar`].
const PRICE_REL_TOL: f64 = 1e-12;
/// Maximum fixed-point iterations (the residual is smooth and monotone in `σ_ss`).
const MAX_ITERS: usize = 100;

/// The market (broker) strangle for one pillar: the two wing strikes (delta
/// pillar at the single strangle vol) and the summed vanilla price at that vol.
///
/// Returned separately from the calibration so callers and tests can inspect the
/// broker target directly and confirm the broker↔smile distinction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketStrangle {
    /// The single strangle volatility `σ_ATM + BF_broker`.
    pub strangle_vol: f64,
    /// Broker call-wing strike (delta pillar at `strangle_vol`).
    pub call_strike: f64,
    /// Broker put-wing strike (delta pillar at `strangle_vol`).
    pub put_strike: f64,
    /// Summed price of the two wings at `strangle_vol` (the reprice target).
    pub price: f64,
}

/// Build the market (broker) strangle for a quoted RR/BF pair.
///
/// Uses the broker convexity `BF_broker = quote.butterfly`: a single vol
/// `σ_ATM + BF_broker` applied to both wings, whose strikes are the delta-pillar
/// strikes *at that vol*. The price is the sum of the two vanillas at that vol.
///
/// # Errors
///
/// Returns [`CalibrationError::StrikeInversion`] if either delta→strike
/// inversion fails.
pub fn market_strangle(
    ctx: &MarketContext,
    atm_vol: f64,
    quote: RiskReversalButterfly,
) -> Result<MarketStrangle, CalibrationError> {
    let strangle_vol = atm_vol + quote.butterfly;
    let call_strike = ctx.strike_at_delta(OptionType::Call, quote.pillar, strangle_vol)?;
    let put_strike = ctx.strike_at_delta(OptionType::Put, quote.pillar, strangle_vol)?;
    let call = price(OptionType::Call, &ctx.template(call_strike, strangle_vol));
    let put = price(OptionType::Put, &ctx.template(put_strike, strangle_vol));
    Ok(MarketStrangle {
        strangle_vol,
        call_strike,
        put_strike,
        price: call + put,
    })
}

/// Given a trial smile-strangle `σ_ss`, resolve the two smile wings: their smile
/// volatilities and the strikes whose delta equals the pillar *at their own smile
/// vol*. The wing vols are `σ_ATM + σ_ss ± ½RR`.
fn smile_wings(
    ctx: &MarketContext,
    atm_vol: f64,
    pillar: DeltaPillar,
    rr: f64,
    smile_strangle: f64,
) -> Result<(f64, f64, f64, f64), CalibrationError> {
    let call_vol = atm_vol + smile_strangle + 0.5 * rr;
    let put_vol = atm_vol + smile_strangle - 0.5 * rr;
    let call_strike = ctx.strike_at_delta(OptionType::Call, pillar, call_vol)?;
    let put_strike = ctx.strike_at_delta(OptionType::Put, pillar, put_vol)?;
    Ok((call_vol, put_vol, call_strike, put_strike))
}

/// The reprice residual at a trial smile-strangle: (smile-wing price summed at
/// the smile wing vols) − (market strangle price). A correctly calibrated smile
/// drives this to zero (Reiswich-Wystup 2010, §5; Clark 2011, §3.5).
fn residual(
    ctx: &MarketContext,
    atm_vol: f64,
    pillar: DeltaPillar,
    rr: f64,
    smile_strangle: f64,
    target_price: f64,
) -> Result<f64, CalibrationError> {
    let (call_vol, put_vol, call_strike, put_strike) =
        smile_wings(ctx, atm_vol, pillar, rr, smile_strangle)?;
    let call = price(OptionType::Call, &ctx.template(call_strike, call_vol));
    let put = price(OptionType::Put, &ctx.template(put_strike, put_vol));
    Ok(call + put - target_price)
}

/// Calibrate the smile-strangle for one quoted RR/BF pair so the resulting smile
/// wings reprice the market (broker) strangle exactly.
///
/// The unknown is the smile-strangle convexity `σ_ss`; the smile wing vols are
/// `σ_ATM + σ_ss ± ½RR`. We bracket around the arithmetic-butterfly initial
/// guess `σ_ss₀ = BF_broker` (the *wrong* answer that the naive desk uses, here
/// only a seed), then refine with a guarded secant that falls back to bisection,
/// guaranteeing convergence because the strangle price is strictly increasing in
/// `σ_ss` over the relevant range (raising both wing vols raises both vanilla
/// prices).
///
/// # Errors
///
/// Returns [`CalibrationError`] if a wing strike inversion fails or the fixed
/// point does not converge in the iteration budget.
pub fn calibrate_pillar(
    ctx: &MarketContext,
    atm_vol: f64,
    quote: RiskReversalButterfly,
) -> Result<CalibratedPillar, CalibrationError> {
    let ms = market_strangle(ctx, atm_vol, quote)?;
    let target = ms.price;
    let pillar = quote.pillar;
    let rr = quote.risk_reversal;

    // Tolerance scaled to the strangle price (premia are ~1e-2..1e-1 of notional).
    let tol = (PRICE_REL_TOL * target.abs()).max(1e-15);

    // Seed at the arithmetic butterfly (the naive, generally-wrong convexity).
    let x0 = quote.butterfly;
    let f0 = residual(ctx, atm_vol, pillar, rr, x0, target)?;

    // A second point to start the secant. The residual is increasing in σ_ss, so
    // step in the direction that reduces |f|. Use the ATM std-dev as a scale.
    let scale = ctx.atm_std_dev(atm_vol).max(1e-4);
    let x1 = x0 - 0.5 * scale * f0.signum() - 1e-4 * f0.signum();
    let f1 = residual(ctx, atm_vol, pillar, rr, x1, target)?;

    // Establish a sign-change bracket [lo, hi] by geometric expansion.
    let (mut lo, mut hi, mut flo, mut fhi);
    if f0 * f1 <= 0.0 {
        if x0 <= x1 {
            lo = x0;
            flo = f0;
            hi = x1;
            fhi = f1;
        } else {
            lo = x1;
            flo = f1;
            hi = x0;
            fhi = f0;
        }
    } else {
        // Expand outward from x0 until the residual changes sign.
        let mut step = scale.max(quote.butterfly.abs()).max(1e-3);
        let mut found = None;
        for _ in 0..64 {
            let up = x0 + step;
            let fu = residual(ctx, atm_vol, pillar, rr, up, target)?;
            if f0 * fu <= 0.0 {
                found = Some((
                    x0.min(up),
                    x0.max(up),
                    if x0 < up { (f0, fu) } else { (fu, f0) },
                ));
                break;
            }
            let dn = x0 - step;
            // Keep wing vols positive: σ_put = σ_ATM + σ_ss − ½RR > 0.
            if atm_vol + dn - 0.5 * rr.abs() > 1e-4 {
                let fd = residual(ctx, atm_vol, pillar, rr, dn, target)?;
                if f0 * fd <= 0.0 {
                    found = Some((dn, x0, (fd, f0)));
                    break;
                }
            }
            step *= 2.0;
        }
        let (l, h, (fl, fh)) = found.ok_or(CalibrationError::NoConvergence)?;
        lo = l;
        hi = h;
        flo = fl;
        fhi = fh;
    }
    debug_assert!(flo * fhi <= 0.0, "calibration bracket must straddle root");

    // Guarded secant with bisection fallback (Brent-lite).
    let mut x = 0.5 * (lo + hi);
    for _ in 0..MAX_ITERS {
        let fx = residual(ctx, atm_vol, pillar, rr, x, target)?;
        if fx.abs() <= tol {
            return finalize(ctx, atm_vol, quote, x, ms);
        }
        if flo * fx <= 0.0 {
            hi = x;
            fhi = fx;
        } else {
            lo = x;
            flo = fx;
        }
        // Secant step from the bracket endpoints.
        let denom = fhi - flo;
        let secant = if denom.abs() > f64::MIN_POSITIVE {
            hi - fhi * (hi - lo) / denom
        } else {
            f64::NAN
        };
        x = if secant.is_finite() && secant > lo && secant < hi {
            secant
        } else {
            0.5 * (lo + hi)
        };
        if (hi - lo) <= tol_on_strangle(scale) {
            return finalize(ctx, atm_vol, quote, x, ms);
        }
    }
    Err(CalibrationError::NoConvergence)
}

/// Convergence width on the smile-strangle parameter itself (a vol), scaled to
/// the ATM std-dev so very-short-dated slices still converge crisply.
#[inline]
fn tol_on_strangle(scale: f64) -> f64 {
    1e-13 * (1.0 + scale)
}

/// Build the [`CalibratedPillar`] result for the converged smile-strangle.
fn finalize(
    ctx: &MarketContext,
    atm_vol: f64,
    quote: RiskReversalButterfly,
    smile_strangle: f64,
    ms: MarketStrangle,
) -> Result<CalibratedPillar, CalibrationError> {
    let (call_vol, put_vol, call_strike, put_strike) = smile_wings(
        ctx,
        atm_vol,
        quote.pillar,
        quote.risk_reversal,
        smile_strangle,
    )?;
    Ok(CalibratedPillar {
        pillar: quote.pillar,
        call_vol,
        put_vol,
        call_strike,
        put_strike,
        smile_strangle,
        market_strangle_price: ms.price,
    })
}

/// The unsigned ATM std-dev `σ√t` exposed for callers that want to size their
/// own tolerances relative to a slice (kept here so the calibration module is
/// self-contained for its scale needs).
#[must_use]
pub fn atm_std_dev(atm_vol: f64, t: f64) -> f64 {
    atm_vol * sqrt(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quotes::MarketQuotes;
    use celnet_conventions::resolve;
    use celnet_core::is_close;
    use celnet_types::{CcyPair, Tenor};

    fn ctx(spot: f64, r_dom: f64, r_for: f64) -> MarketContext {
        let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
        MarketContext::new(spot, r_dom, r_for, 1.0, conv)
    }

    /// The calibrated smile wings reprice the market (broker) strangle exactly.
    #[test]
    fn calibrated_wings_reprice_market_strangle() {
        let c = ctx(1.30, 0.02, 0.01);
        let q = MarketQuotes::three_point(0.105, 0.012, 0.0035);
        let cal = calibrate_pillar(&c, q.atm_vol, q.inner).unwrap();

        let call = price(OptionType::Call, &c.template(cal.call_strike, cal.call_vol));
        let put = price(OptionType::Put, &c.template(cal.put_strike, cal.put_vol));
        assert!(
            is_close(call + put, cal.market_strangle_price, 1e-10, 1e-12),
            "smile strangle {} must reprice market strangle {}",
            call + put,
            cal.market_strangle_price
        );
    }

    /// The risk-reversal is reproduced: σ_call − σ_put = RR exactly by construction.
    #[test]
    fn calibration_reproduces_risk_reversal() {
        let c = ctx(1.30, 0.02, 0.01);
        let q = MarketQuotes::three_point(0.105, 0.02, 0.004);
        let cal = calibrate_pillar(&c, q.atm_vol, q.inner).unwrap();
        assert!(is_close(
            cal.call_vol - cal.put_vol,
            q.inner.risk_reversal,
            1e-12,
            1e-13
        ));
    }

    /// For a non-zero risk-reversal the calibrated smile-strangle differs from
    /// the quoted (arithmetic) butterfly — the broker-vs-smile distinction. With
    /// zero RR the two coincide (a symmetric smile).
    #[test]
    fn smile_strangle_differs_from_broker_butterfly_when_skewed() {
        let c = ctx(1.30, 0.02, 0.01);
        // High RR (EM-like skew): smile-strangle must move off the arithmetic BF.
        let skewed = MarketQuotes::three_point(0.12, 0.05, 0.006);
        let cal_s = calibrate_pillar(&c, skewed.atm_vol, skewed.inner).unwrap();
        assert!(
            (cal_s.smile_strangle - skewed.inner.butterfly).abs() > 1e-5,
            "skewed smile-strangle {} should differ from broker BF {}",
            cal_s.smile_strangle,
            skewed.inner.butterfly
        );

        // Symmetric smile (RR = 0): broker and smile strangles coincide.
        let sym = MarketQuotes::three_point(0.12, 0.0, 0.006);
        let cal_z = calibrate_pillar(&c, sym.atm_vol, sym.inner).unwrap();
        assert!(
            is_close(cal_z.smile_strangle, sym.inner.butterfly, 1e-7, 1e-9),
            "symmetric smile-strangle {} should equal broker BF {}",
            cal_z.smile_strangle,
            sym.inner.butterfly
        );
    }

    /// The market strangle uses a single vol on both wings, and its strikes
    /// straddle the forward.
    #[test]
    fn market_strangle_single_vol_both_wings() {
        let c = ctx(1.30, 0.02, 0.01);
        let q = MarketQuotes::three_point(0.10, 0.01, 0.003);
        let ms = market_strangle(&c, q.atm_vol, q.inner).unwrap();
        assert!(is_close(ms.strangle_vol, 0.103, 1e-15, 1e-15));
        let f = c.forward();
        assert!(ms.put_strike < f && f < ms.call_strike);
        assert!(ms.price > 0.0);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(192))]
        /// Across a broad band of market states and mild-to-strong quotes, the
        /// calibrated smile wings always reprice the broker (market) strangle and
        /// reproduce the risk-reversal — the two defining calibration invariants.
        #[test]
        fn calibration_invariants_hold(
            spot in 0.5f64..200.0,
            atm in 0.05f64..0.30,
            rr in -0.06f64..0.06,
            bf in 0.0005f64..0.012,
            r_dom in -0.01f64..0.06,
            r_for in -0.01f64..0.06,
            t in 0.1f64..3.0,
        ) {
            let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
            let c = MarketContext::new(spot, r_dom, r_for, t, conv);
            let q = MarketQuotes::three_point(atm, rr, bf);
            if let Ok(cal) = calibrate_pillar(&c, q.atm_vol, q.inner) {
                // Risk-reversal reproduced exactly (by construction).
                proptest::prop_assert!(is_close(
                    cal.call_vol - cal.put_vol,
                    rr,
                    1e-10,
                    1e-12
                ));
                // Smile wings reprice the broker strangle.
                let call = price(OptionType::Call, &c.template(cal.call_strike, cal.call_vol));
                let put = price(OptionType::Put, &c.template(cal.put_strike, cal.put_vol));
                proptest::prop_assert!(
                    is_close(call + put, cal.market_strangle_price, 1e-8, 1e-12),
                    "smile strangle {} vs broker {}",
                    call + put,
                    cal.market_strangle_price
                );
            }
        }
    }
}
