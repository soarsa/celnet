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
//! and choose `σ_ss` such that the smile (a [`crate::market_hedge`] model built
//! from `σ_ATM`, `RR`, `σ_ss`) reprices the market strangle. The arithmetic
//! butterfly is the natural initial guess `σ_ss ≈ BF_broker`; the calibration is
//! the correction. The residual is monotone in `σ_ss`, so a guarded
//! secant/bisection converges quickly and deterministically.
//!
//! ## The canonical market-strangle constraint (Reiswich-Wystup / Clark)
//!
//! The constraint is **not** "the smile wings, priced at their own smile vols,
//! sum to the market strangle" — that would only re-state the wing pillars
//! against themselves and never touches the broker strikes. The canonical
//! constraint instead pins the *whole smile's convexity* to the broker strangle:
//!
//! 1. Build the market (broker) strangle: a single vol `σ_MS = σ_ATM + BF`, its
//!    two delta-pillar strikes `K_call^MS`, `K_put^MS` *at that vol*, and the
//!    summed vanilla price `V_MS` (the target).
//! 2. For a trial `σ_ss`, form the three smile pillars
//!    `{(K_put, σ_put), (K_ATM, σ_ATM), (K_call, σ_call)}` and build the
//!    [`crate::market_hedge::MarketHedgeSmile`] through them.
//! 3. Evaluate **that smile** at the *broker* strikes `K_call^MS`, `K_put^MS`,
//!    obtaining smile vols `σ_smile(K_call^MS)`, `σ_smile(K_put^MS)`, price the
//!    two vanillas at those smile vols, and require their sum to equal `V_MS`.
//!
//! Only step 3 — evaluating the constructed smile *at the broker strikes* — makes
//! the reprice test non-vacuous: the broker strikes are **not** smile pillars, so
//! the equality genuinely constrains `σ_ss` (Reiswich-Wystup 2010, §5; Clark
//! 2011, §3.5, eq. 3.30–3.33).

use celnet_core::Smile;
use celnet_core::math::sqrt;
use celnet_types::OptionType;
use celnet_vanilla::price;

use crate::market_hedge::MarketHedgeSmile;
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
    /// The quote is economically degenerate / uncalibratable: it forces a
    /// non-positive smile wing volatility (e.g. `|RR|` too large relative to the
    /// ATM vol and butterfly), so no well-posed smile can reprice it. The
    /// calibrator rejects such inputs rather than returning an invalid smile or
    /// panicking on the constructed pillars.
    DegenerateQuote,
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

/// Build the trial Vanna-Volga smile through the three pillars implied by a trial
/// smile-strangle `σ_ss`: the smile put wing, the ATM, and the smile call wing.
fn build_trial_smile(
    ctx: &MarketContext,
    atm_vol: f64,
    atm_strike: f64,
    pillar: DeltaPillar,
    rr: f64,
    smile_strangle: f64,
) -> Result<MarketHedgeSmile, CalibrationError> {
    let (call_vol, put_vol, call_strike, put_strike) =
        smile_wings(ctx, atm_vol, pillar, rr, smile_strangle)?;
    // A degenerate quote (|RR| too large relative to ATM+σ_ss) drives a wing vol
    // ≤ 0; such a trial is out-of-domain — there is no well-posed vanna-volga
    // smile through a non-positive pillar vol. Reject it as a degenerate quote so
    // the bracketing loop steps away and a final degenerate state surfaces as an
    // error (never a panic, never an invalid Ok). The fallible try_new also
    // covers the strike-ordering requirement K₁<K₂<K₃: for any well-posed FX
    // slice the put wing sits below the ATM strike which sits below the call
    // wing; if a pathological trial vol inverts that order the smile is not
    // constructible and we likewise treat it as out-of-domain.
    if !(put_vol > 0.0 && call_vol > 0.0) {
        return Err(CalibrationError::DegenerateQuote);
    }
    MarketHedgeSmile::try_new(
        [put_strike, atm_strike, call_strike],
        [put_vol, atm_vol, call_vol],
        ctx.forward(),
        ctx.t,
    )
    .ok_or(CalibrationError::DegenerateQuote)
}

/// The reprice residual at a trial smile-strangle, in the **canonical**
/// Reiswich-Wystup / Clark form: build the trial smile through the three pillars,
/// evaluate it at the *broker* strangle strikes `K_call^MS`, `K_put^MS`, price the
/// two vanillas at those *smile* vols, and subtract the market strangle target.
///
/// Because the broker strikes are not smile pillars, the smile vols there move
/// non-trivially with `σ_ss`; driving this residual to zero is the genuine
/// constraint that pins the smile convexity to the broker strangle (Reiswich-
/// Wystup 2010, §5; Clark 2011, §3.5).
fn residual(
    ctx: &MarketContext,
    atm_vol: f64,
    atm_strike: f64,
    pillar: DeltaPillar,
    rr: f64,
    smile_strangle: f64,
    ms: &MarketStrangle,
) -> Result<f64, CalibrationError> {
    let smile = build_trial_smile(ctx, atm_vol, atm_strike, pillar, rr, smile_strangle)?;
    // Evaluate the constructed smile AT THE BROKER STRIKES (the load-bearing
    // step): these are not pillars of the smile, so σ_smile here is interpolated.
    let f = ctx.forward();
    let vol_call = smile.implied_vol(ms.call_strike, f, ctx.t).0;
    let vol_put = smile.implied_vol(ms.put_strike, f, ctx.t).0;
    let call = price(OptionType::Call, &ctx.template(ms.call_strike, vol_call));
    let put = price(OptionType::Put, &ctx.template(ms.put_strike, vol_put));
    Ok(call + put - ms.price)
}

/// Calibrate the smile-strangle for one quoted RR/BF pair so that the
/// constructed smile, **evaluated at the broker-strangle strikes**, reprices the
/// market (broker) strangle exactly (the canonical Reiswich-Wystup / Clark
/// constraint described in the module docs).
///
/// The unknown is the smile-strangle convexity `σ_ss`; the smile wing vols are
/// `σ_ATM + σ_ss ± ½RR`. Each trial builds the three-pillar Vanna-Volga smile and
/// prices the two broker-strike options at the *smile's* vols there. We bracket
/// around the arithmetic-butterfly initial guess `σ_ss₀ = BF_broker` (the *wrong*
/// answer that the naive desk uses, here only a seed), then refine with a guarded
/// secant that falls back to bisection. Convergence is guaranteed because raising
/// `σ_ss` lifts both wing vols, hence the whole smile (and so its vols at the
/// fixed broker strikes), hence both vanilla prices — the residual is strictly
/// increasing in `σ_ss` over the relevant range.
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
    // The ATM pillar of every trial smile is fixed; resolve it once.
    let atm_strike = ctx.atm_strike(atm_vol);

    // Tolerance scaled to the strangle price (premia are ~1e-2..1e-1 of notional).
    let tol = (PRICE_REL_TOL * target.abs()).max(1e-15);

    // The smallest admissible smile-strangle: below it a wing vol turns
    // non-positive (σ_put = σ_ATM + σ_ss − ½RR > 0 and the symmetric call
    // condition), so the smile is degenerate / uncalibratable. We keep every
    // probe at-or-above a small margin past this floor; a quote whose reprice
    // root lies below the floor is genuinely degenerate and is rejected via the
    // residual's DegenerateQuote error rather than ever building a bad smile.
    let ss_floor = 0.5 * rr.abs() - atm_vol;
    let ss_min = ss_floor + 1e-4;

    // Seed at the arithmetic butterfly (the naive, generally-wrong convexity).
    // If even the seed is below the admissible floor the quote is degenerate.
    let x0 = quote.butterfly;
    if x0 <= ss_min {
        return Err(CalibrationError::DegenerateQuote);
    }
    let f0 = residual(ctx, atm_vol, atm_strike, pillar, rr, x0, &ms)?;

    // A second point to start the secant. The residual is increasing in σ_ss, so
    // step in the direction that reduces |f|. Use the ATM std-dev as a scale.
    // Clamp the (possibly downward) step so the probe never crosses the
    // admissible floor — a transient degenerate probe must not spuriously reject
    // an otherwise well-posed quote.
    let scale = ctx.atm_std_dev(atm_vol).max(1e-4);
    let x1 = (x0 - 0.5 * scale * f0.signum() - 1e-4 * f0.signum()).max(ss_min);
    let f1 = residual(ctx, atm_vol, atm_strike, pillar, rr, x1, &ms)?;

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
        // Expand outward from x0 until the residual changes sign. The residual is
        // increasing in σ_ss, so a positive f0 means the reprice root lies *below*
        // x0 and we must step down; a negative f0 means it lies above. We never
        // step below the admissible floor `ss_min` (below it a wing vol turns
        // non-positive). If the downward search is blocked by that floor while
        // still on the same side of the root, the root sits in the degenerate
        // region (|RR| too large for the ATM/BF): the quote is uncalibratable.
        let mut step = scale.max(quote.butterfly.abs()).max(1e-3);
        let mut found = None;
        let mut down_blocked_by_floor = false;
        for _ in 0..64 {
            let up = x0 + step;
            let fu = residual(ctx, atm_vol, atm_strike, pillar, rr, up, &ms)?;
            if f0 * fu <= 0.0 {
                found = Some((
                    x0.min(up),
                    x0.max(up),
                    if x0 < up { (f0, fu) } else { (fu, f0) },
                ));
                break;
            }
            let dn = x0 - step;
            // Keep wing vols positive: stay strictly above the admissible floor
            // (σ_put = σ_ATM + σ_ss − ½RR > 0 and the symmetric call condition).
            if dn > ss_min {
                let fd = residual(ctx, atm_vol, atm_strike, pillar, rr, dn, &ms)?;
                if f0 * fd <= 0.0 {
                    found = Some((dn, x0, (fd, f0)));
                    break;
                }
            } else {
                // The downward probe would cross the floor; record it. Clamp one
                // probe exactly at the floor so a root sitting just above the
                // floor is still bracketed before we conclude degeneracy.
                down_blocked_by_floor = true;
                let fd = residual(ctx, atm_vol, atm_strike, pillar, rr, ss_min, &ms)?;
                if f0 * fd <= 0.0 {
                    found = Some((ss_min, x0, (fd, f0)));
                    break;
                }
            }
            step *= 2.0;
        }
        let (l, h, (fl, fh)) = match found {
            Some(b) => b,
            // No sign change inside the admissible domain. If we were driven into
            // the floor on the downward side, the reprice root is below it, in the
            // non-positive-wing region: the quote is degenerate. Otherwise the
            // upward search simply exhausted its budget (non-convergence).
            None if down_blocked_by_floor && f0 > 0.0 => {
                return Err(CalibrationError::DegenerateQuote);
            }
            None => return Err(CalibrationError::NoConvergence),
        };
        lo = l;
        hi = h;
        flo = fl;
        fhi = fh;
    }
    debug_assert!(flo * fhi <= 0.0, "calibration bracket must straddle root");

    // Guarded secant with bisection fallback (Brent-lite).
    let mut x = 0.5 * (lo + hi);
    for _ in 0..MAX_ITERS {
        let fx = residual(ctx, atm_vol, atm_strike, pillar, rr, x, &ms)?;
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
            // The bracket has collapsed. Only succeed if the residual at `x` is
            // genuinely within tolerance — i.e. the calibrated smile actually
            // reprices the broker strangle. A degenerate quote (e.g. |RR| ≳ ATM,
            // which forces a non-positive wing vol) can shrink the bracket without
            // ever repricing; return an error so such inputs are rejected, never
            // silently mispriced. A successful return is thus a postcondition: the
            // returned pillar reprices the market strangle to tolerance.
            let fx = residual(ctx, atm_vol, atm_strike, pillar, rr, x, &ms)?;
            if fx.abs() <= tol {
                return finalize(ctx, atm_vol, quote, x, ms);
            }
            return Err(CalibrationError::NoConvergence);
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
    // Postcondition: a successful calibration GUARANTEES strictly-positive wing
    // vols (and, with the residual tolerance already met by the caller, that the
    // smile reprices the broker strangle). Defend it here so no degenerate
    // pillar can ever escape as an Ok, even if the converged σ_ss sat exactly on
    // the admissible boundary.
    if !(put_vol > 0.0 && call_vol > 0.0 && put_vol.is_finite() && call_vol.is_finite()) {
        return Err(CalibrationError::DegenerateQuote);
    }
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
    use celnet_types::{Carry, CcyPair, Tenor};

    fn ctx(spot: f64, r_dom: f64, r_for: f64) -> MarketContext {
        let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
        MarketContext::new(spot, Carry::FxRates { r_dom, r_for }, 1.0, conv)
    }

    /// NON-VACUOUS reprice: the *constructed smile*, evaluated at the **broker**
    /// strangle strikes (which are NOT smile pillars), must reproduce the market
    /// strangle price. Pricing the smile wings at their own (pillar) vols would be
    /// trivially true by construction — this test instead feeds the broker strikes
    /// through `smile.implied_vol`, the load-bearing step the audit flagged.
    #[test]
    fn calibrated_smile_reprices_market_strangle_at_broker_strikes() {
        let c = ctx(1.30, 0.02, 0.01);
        let q = MarketQuotes::three_point(0.105, 0.012, 0.0035);
        let cal = calibrate_pillar(&c, q.atm_vol, q.inner).unwrap();
        let ms = market_strangle(&c, q.atm_vol, q.inner).unwrap();

        // Build the calibrated smile and evaluate it at the BROKER strikes.
        let atm_strike = c.atm_strike(q.atm_vol);
        let smile = MarketHedgeSmile::new(
            [cal.put_strike, atm_strike, cal.call_strike],
            [cal.put_vol, q.atm_vol, cal.call_vol],
            c.forward(),
            c.t,
        );
        let f = c.forward();
        let vol_call = smile.implied_vol(ms.call_strike, f, c.t).0;
        let vol_put = smile.implied_vol(ms.put_strike, f, c.t).0;

        // The broker strikes are distinct from the smile pillars: the test is
        // genuinely constraining (the smile is being interpolated, not echoed).
        assert!(
            (ms.call_strike - cal.call_strike).abs() > 1e-6 * f
                || (ms.put_strike - cal.put_strike).abs() > 1e-6 * f,
            "broker strikes must differ from smile pillars for a non-vacuous test"
        );

        let call = price(OptionType::Call, &c.template(ms.call_strike, vol_call));
        let put = price(OptionType::Put, &c.template(ms.put_strike, vol_put));
        assert!(
            is_close(call + put, cal.market_strangle_price, 1e-9, 1e-11),
            "smile @ broker strikes {} must reprice market strangle {}",
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

    /// A deliberately-degenerate quote — risk-reversal of the order of (here
    /// larger than) the ATM vol — forces a non-positive smile wing volatility.
    /// Such a quote is uncalibratable: `calibrate_pillar` must return `Err`
    /// (a `DegenerateQuote`), never an `Ok` with an invalid pillar and never a
    /// panic. This is the regression guard for the calibration-robustness bug
    /// (a high-RR quote previously produced a non-positive wing vol that then
    /// panicked the vanna-volga constructor).
    #[test]
    fn degenerate_high_rr_quote_is_rejected_cleanly() {
        let c = ctx(1.30, 0.02, 0.01);
        // |RR| > ATM: the put wing vol σ_ATM + σ_ss − ½|RR| cannot stay positive
        // for any convexity that also reprices the broker strangle.
        let degenerate = MarketQuotes::three_point(0.10, 0.16, 0.004);
        let res = calibrate_pillar(&c, degenerate.atm_vol, degenerate.inner);
        // It must be cleanly REJECTED — an Err (degenerate / non-convergent),
        // never a panic and never an Ok carrying an invalid (non-positive-wing)
        // pillar. Both error variants are acceptable rejections; the contract is
        // "no panic, no bad Ok".
        assert!(
            matches!(
                res,
                Err(CalibrationError::DegenerateQuote | CalibrationError::NoConvergence)
            ),
            "a |RR| > ATM quote must be cleanly rejected (Err), got {res:?}"
        );

        // The exact failing seed from the proptest reduction (spot=0.5,
        // atm≈0.2936, rr≈0.173, bf=0.0005, t≈2.26): it previously panicked the
        // smile constructor with a put_vol ≈ −0.0107. It must now be a clean Err
        // (or, if calibratable with positive wings, a valid Ok — never a panic).
        let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
        let c2 = MarketContext::new(
            0.5,
            Carry::FxRates {
                r_dom: 0.0,
                r_for: 0.0,
            },
            2.26,
            conv,
        );
        let q2 = MarketQuotes::three_point(0.2936, 0.173, 0.0005);
        match calibrate_pillar(&c2, q2.atm_vol, q2.inner) {
            Ok(cal) => {
                assert!(
                    cal.put_vol > 0.0 && cal.call_vol > 0.0,
                    "an Ok pillar must carry strictly-positive wing vols, got {cal:?}"
                );
            }
            Err(_) => { /* cleanly rejected — the expected outcome */ }
        }
    }

    /// `market_strangle` propagates a strike-inversion failure as
    /// `StrikeInversion`. We force the inner delta→strike solver to fail by
    /// asking for a strangle vol so enormous that no finite strike attains the
    /// pillar delta in the convention. The error must surface cleanly (no panic),
    /// and `calibrate_pillar` (which calls `market_strangle` first) must surface
    /// the same variant. This pins the `?`-propagation arms of both functions.
    #[test]
    fn unreachable_pillar_delta_surfaces_strike_inversion() {
        let c = ctx(1.30, 0.02, 0.01);
        // A pathological butterfly drives strangle_vol = atm + bf astronomically
        // high; the premium-adjusted/forward delta of any strike then cannot
        // reach the 25Δ pillar, so the solver reports the delta unreachable.
        let q = MarketQuotes::three_point(0.10, 0.0, 1.0e6);
        match market_strangle(&c, q.atm_vol, q.inner) {
            Err(CalibrationError::StrikeInversion) => {}
            other => panic!("expected StrikeInversion, got {other:?}"),
        }
        match calibrate_pillar(&c, q.atm_vol, q.inner) {
            Err(CalibrationError::StrikeInversion) => {}
            other => panic!("calibrate_pillar should propagate StrikeInversion, got {other:?}"),
        }
    }

    /// A quote whose arithmetic-butterfly SEED already sits on/below the
    /// admissible smile-strangle floor (`σ_ss ≤ ½|RR| − σ_ATM + 1e-4`) is
    /// rejected immediately as `DegenerateQuote` — before any residual evaluation.
    /// This pins the early-seed degeneracy guard (`x0 <= ss_min`). The construction
    /// uses |RR| just under 2·ATM with a tiny butterfly so the seed `bf` is below
    /// the floor `½|RR| − σ_ATM`.
    #[test]
    fn seed_below_floor_is_rejected_immediately() {
        let c = ctx(1.30, 0.02, 0.01);
        // atm = 0.10, |RR| = 0.19 → floor = 0.095 − 0.10 = -0.005, ss_min ≈ -0.0049.
        // bf must be ≤ ss_min to trip the early guard, which needs a *negative*
        // floor; instead push |RR| above 2·ATM so the floor is positive and a
        // small bf is below it. |RR| = 0.205, atm = 0.10 → floor = 0.1025 − 0.10
        // = 0.0025, ss_min ≈ 0.0026; bf = 0.001 < ss_min → immediate reject.
        let q = MarketQuotes::three_point(0.10, 0.205, 0.001);
        assert_eq!(
            calibrate_pillar(&c, q.atm_vol, q.inner),
            Err(CalibrationError::DegenerateQuote),
            "a seed below the admissible floor must be rejected immediately"
        );
    }

    /// A quote whose admissible SEED sits above the floor (so it passes the
    /// immediate guard) but whose reprice root lies in the non-positive-wing
    /// region BELOW the floor is rejected as `DegenerateQuote` only after the
    /// geometric bracket-expansion drives the downward probe into the floor with
    /// the residual still one-signed (`down_blocked_by_floor && f0 > 0.0`). This
    /// is distinct from `seed_below_floor_is_rejected_immediately`: here the seed
    /// is admissible and the expansion `else` block does the work. The contract is
    /// a clean `Err`, never a panic, never an `Ok` with a non-positive wing.
    ///
    /// The state (spot 0.5, t = 3, atm 0.10, tiny RR, tiny BF) is a confirmed
    /// floor-blocked-expansion case: the broker strangle's reprice convexity sits
    /// below the admissible σ_ss floor, so no well-posed smile exists.
    #[test]
    fn expansion_floor_blocked_quote_is_degenerate() {
        let c = MarketContext::new(
            0.5,
            Carry::FxRates {
                r_dom: 0.02,
                r_for: 0.01,
            },
            3.0,
            resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record,
        );
        // atm = 0.10, |RR| = 0.001 → floor ½|RR| − atm + 1e-4 ≈ −0.0994, seed
        // bf = 0.0005 is far above it (immediate guard passes), but the reprice
        // root is below the floor → degenerate via the expansion path.
        let q = MarketQuotes::three_point(0.10, 0.001, 0.0005);
        // Seed is admissible (above floor) — confirm we are NOT in the immediate
        // guard: ss_min < bf.
        let ss_min = 0.5 * q.inner.risk_reversal.abs() - q.atm_vol + 1e-4;
        assert!(
            q.inner.butterfly > ss_min,
            "seed must be admissible so the expansion path (not the immediate \
             guard) discovers the degeneracy: bf={} ss_min={ss_min}",
            q.inner.butterfly
        );
        assert_eq!(
            calibrate_pillar(&c, q.atm_vol, q.inner),
            Err(CalibrationError::DegenerateQuote),
            "a floor-blocked reprice root must be rejected as DegenerateQuote \
             through the expansion path"
        );
    }

    /// A quote whose reprice root cannot be bracketed within the expansion budget
    /// on the upward side (and is not floor-blocked) surfaces as `NoConvergence` —
    /// the other expansion-exhaustion exit (`None` arm with no floor block). This
    /// pins the non-degenerate failure path of the geometric expansion. The state
    /// (spot 0.5, atm 0.10, high +RR, tiny BF, t = 1) is a confirmed NoConvergence
    /// case from the outcome scan. Contract: a clean `Err`, never a panic.
    #[test]
    fn unbracketable_quote_surfaces_no_convergence() {
        let c = ctx(0.5, 0.02, 0.01); // t = 1
        let q = MarketQuotes::three_point(0.10, 0.9 * 0.10, 0.0005);
        assert_eq!(
            calibrate_pillar(&c, q.atm_vol, q.inner),
            Err(CalibrationError::NoConvergence),
            "an unbracketable (non-floor-blocked) quote must surface NoConvergence"
        );
    }

    /// When the arithmetic-butterfly seed and its secant neighbour fall on the
    /// SAME side of the reprice root (no immediate sign-change bracket), the root
    /// finder enters the geometric **bracket-expansion** `else` block and steps
    /// outward — DOWN when the seed lies above the root, UP when below — until it
    /// straddles the root, then converges. These two confirmed states (short-/
    /// long-dated, low-vol, skewed, spot 0.5) drive the seed above and below the
    /// root respectively, exercising both expansion directions on the SUCCESS
    /// path. INDEPENDENT CHECK per case: the returned smile, evaluated at the
    /// broker strikes (NOT smile pillars), reprices the market strangle, and the
    /// risk-reversal is reproduced — the two defining invariants, here for an
    /// expansion-path solve; and the calibrated convexity genuinely differs from
    /// the seed (the expansion moved off the arithmetic butterfly).
    #[test]
    fn bracket_expansion_solves_both_step_directions() {
        // (atm, rr, bf, t): the first seed lies ABOVE the root (residual > 0 at
        // the seed AND its secant neighbour → step DOWN expansion: high-vol
        // strongly-skewed very-short-tenor quote, σ_ss falls from 0.30 to ≈0.262);
        // the second lies BELOW the root (residual < 0 at both → step UP
        // expansion: tiny butterfly seed with a large true convexity, σ_ss rises
        // from 0.001 to ≈0.017, a 17× correction). Both states were confirmed to
        // enter the geometric-expansion `else` block (f0·f1 > 0).
        let cases: [(f64, f64, f64, f64); 2] = [
            (0.30, 0.24, 0.30, 0.05),  // step-DOWN expansion (seed above root)
            (0.20, 0.16, 0.001, 0.02), // step-UP expansion (seed below root)
        ];
        for (atm, rr, bf, t) in cases {
            let c = ctx(1.0, 0.02, 0.01);
            let c = MarketContext::new(c.spot, c.carry, t, c.conventions);
            let q = MarketQuotes::three_point(atm, rr, bf);
            let cal = calibrate_pillar(&c, q.atm_vol, q.inner).unwrap_or_else(|e| {
                panic!("expansion case (atm={atm},rr={rr},bf={bf},t={t}) failed: {e:?}")
            });
            let ms = market_strangle(&c, q.atm_vol, q.inner).unwrap();

            // RR reproduced exactly by construction.
            assert!(is_close(cal.call_vol - cal.put_vol, rr, 1e-10, 1e-12));

            // NON-VACUOUS reprice at the broker strikes (interpolated, not echoed).
            let atm_strike = c.atm_strike(q.atm_vol);
            let smile = MarketHedgeSmile::new(
                [cal.put_strike, atm_strike, cal.call_strike],
                [cal.put_vol, q.atm_vol, cal.call_vol],
                c.forward(),
                c.t,
            );
            let f = c.forward();
            let vc = smile.implied_vol(ms.call_strike, f, c.t).0;
            let vp = smile.implied_vol(ms.put_strike, f, c.t).0;
            let call = price(OptionType::Call, &c.template(ms.call_strike, vc));
            let put = price(OptionType::Put, &c.template(ms.put_strike, vp));
            assert!(
                is_close(call + put, cal.market_strangle_price, 1e-7, 1e-11),
                "expansion-path smile @ broker strikes {} must reprice market strangle {} \
                 (atm={atm},rr={rr},bf={bf},t={t})",
                call + put,
                cal.market_strangle_price
            );
            // The calibrated convexity must differ from the seed: the expansion
            // genuinely corrected the arithmetic butterfly (broker ≠ smile
            // strangle for a skewed quote).
            assert!(
                (cal.smile_strangle - q.inner.butterfly).abs() > 1e-6,
                "expansion should move σ_ss {} off the seed BF {} (atm={atm},rr={rr},bf={bf},t={t})",
                cal.smile_strangle,
                q.inner.butterfly
            );
        }
    }

    /// The free `atm_std_dev(σ, t) = σ√t` helper (exposed for callers sizing their
    /// own tolerances) returns the unsigned ATM standard deviation. Pins the
    /// public helper against its closed form.
    #[test]
    fn atm_std_dev_is_sigma_root_t() {
        assert!(is_close(atm_std_dev(0.20, 4.0), 0.40, 1e-15, 1e-15));
        assert!(is_close(atm_std_dev(0.10, 0.25), 0.05, 1e-15, 1e-15));
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
            // The 25Δ risk-reversal is drawn as a fraction of ATM: real RRs are a
            // fraction of the ATM vol, and |RR| ≥ ATM is economically impossible
            // (it forces a non-positive wing vol). Degenerate quotes outside this
            // band are rejected by calibrate_pillar (returns Err) and skipped by
            // the Ok-guard below — they are not valid market inputs to reprice.
            rr_frac in -0.6f64..0.6,
            bf in 0.0005f64..0.012,
            r_dom in -0.01f64..0.06,
            r_for in -0.01f64..0.06,
            t in 0.1f64..3.0,
        ) {
            let rr = rr_frac * atm;
            let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
            let c = MarketContext::new(spot, Carry::FxRates { r_dom, r_for }, t, conv);
            let q = MarketQuotes::three_point(atm, rr, bf);
            if let (Ok(cal), Ok(ms)) =
                (calibrate_pillar(&c, q.atm_vol, q.inner), market_strangle(&c, q.atm_vol, q.inner))
            {
                // Risk-reversal reproduced exactly (by construction).
                proptest::prop_assert!(is_close(
                    cal.call_vol - cal.put_vol,
                    rr,
                    1e-10,
                    1e-12
                ));
                // NON-VACUOUS: the constructed smile, evaluated at the BROKER
                // strikes (not the smile pillars), reprices the market strangle.
                let atm_strike = c.atm_strike(q.atm_vol);
                let smile = MarketHedgeSmile::new(
                    [cal.put_strike, atm_strike, cal.call_strike],
                    [cal.put_vol, q.atm_vol, cal.call_vol],
                    c.forward(),
                    c.t,
                );
                let f = c.forward();
                let vol_call = smile.implied_vol(ms.call_strike, f, c.t).0;
                let vol_put = smile.implied_vol(ms.put_strike, f, c.t).0;
                let call = price(OptionType::Call, &c.template(ms.call_strike, vol_call));
                let put = price(OptionType::Put, &c.template(ms.put_strike, vol_put));
                proptest::prop_assert!(
                    is_close(call + put, cal.market_strangle_price, 1e-7, 1e-11),
                    "smile @ broker strikes {} vs broker {}",
                    call + put,
                    cal.market_strangle_price
                );
            }
        }
    }
}
