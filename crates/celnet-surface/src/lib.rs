//! Celnet FX volatility surface — arbitrage-aware delta-space smile construction
//! from broker market quotes (work-stream WS-C, gate G2).
//!
//! # What this crate builds
//!
//! An FX smile is quoted in **delta space** under a sticky-delta assumption
//! (`docs/ANALYTICS-SPEC.md` §3): per `(pair, tenor)` the market gives an
//! at-the-money volatility plus risk-reversal / butterfly pairs at the `25Δ`
//! (and, for liquid pairs, `10Δ`) pillars. This crate turns those broker quotes
//! into a smile object that:
//!
//! 1. is **convention-aware** — every delta↔strike conversion goes through the
//!    resolved [`celnet_conventions::ConventionRecord`] and the convention-aware
//!    solver in [`celnet_vanilla`] (the four [`celnet_types::DeltaConvention`]
//!    variants and both ATM conventions);
//! 2. **reprices the broker quotes exactly** via the market(broker)-strangle →
//!    smile-strangle calibration ([`strangle`]) — an *iterative* fixed point, not
//!    the naive arithmetic average that silently biases the wings (the "#1
//!    production bug", `docs/ANALYTICS-SPEC.md` §1.4);
//! 3. evaluates as a market-hedge smile ([`market_hedge`]) implementing
//!    [`celnet_core::Smile`], the FX-market-standard light interpolation (the
//!    vanna-volga method) in its second-order, exact-repricing form;
//! 4. is checked against **static no-arbitrage laws** ([`arbitrage`]) — butterfly
//!    (implied density ≥ 0) and vertical-spread monotonicity — which the surface
//!    layer uses to detect when the market-hedge baseline must fall back to an
//!    arbitrage-free model in the wings.
//!
//! # Pipeline
//!
//! [`build_smile`] is the end-to-end entry point: resolve conventions →
//! locate the ATM strike → calibrate the inner (`25Δ`) pillar against its broker
//! strangle → assemble the three anchor pillars (put wing, ATM, call wing) →
//! return a [`market_hedge::MarketHedgeSmile`]. The lower-level modules are public so
//! the engine layer can compose them differently (e.g. five-point smiles, or a
//! different smile model anchored on the same calibrated pillars).
//!
//! # Beyond the broker baseline (S2)
//!
//! On top of the market-hedge baseline this crate provides the production smile
//! models and the term-structure machinery that tie tenor slices into a full,
//! re-strikable, arbitrage-aware surface:
//!
//! * [`stochvol`] — a stochastic-volatility smile (the singular-perturbation
//!   lognormal expansion plus an arbitrage-free density-PDE refinement for the
//!   wings, the SABR method) implementing [`celnet_core::Smile`];
//! * [`parametric`] / [`parametric_surface`] / [`extended_surface`] — the
//!   parametric total-variance **slice** and the **surface** forms (the SVI /
//!   SSVI / eSSVI methods); the surface forms carry closed-form static
//!   no-arbitrage (butterfly + calendar) conditions, and the eSSVI extension lets
//!   the correlation be maturity-dependent (`ρ → ρ(θ)`) while keeping them;
//! * [`termstructure`] — interpolation in **total variance / business time**
//!   (calendar-arbitrage-free: total variance non-decreasing in maturity), tying
//!   slices into a continuously re-strikable surface;
//! * [`surface`] — the unified [`VolSurface`] that selects the smile model and
//!   exposes `implied_vol(strike, t)` plus a consolidated arbitrage report.
//!
//! # Method provenance (doc-only)
//!
//! The market-hedge baseline uses the vanna-volga method (Castagna & Mercurio,
//! 2007). Market→smile strangle calibration: Reiswich & Wystup (2010); Clark
//! (2011). The stochastic-volatility smile uses the SABR expansion and an
//! arbitrage-free density: Hagan, Kumar, Lesniewski & Woodward (2002, 2014). The
//! parametric slice / surface use the SVI / SSVI methods and their closed-form
//! no-arbitrage conditions: Gatheral (2004), Gatheral & Jacquier (2014); the
//! extended (maturity-dependent-`ρ`) surface uses the eSSVI method and its
//! closed-form conditions: Hendriks & Martini (2019). Density
//! / arbitrage conditions: Breeden-Litzenberger; Durrleman. All such names are
//! provenance only — every public identifier is purpose-named and
//! vendor/research-neutral.

#![forbid(unsafe_code)]

pub mod arbitrage;
pub mod calibrate;
pub mod extended_surface;
pub mod market_hedge;
pub mod parametric;
pub mod parametric_surface;
pub mod quotes;
pub mod stochvol;
pub mod strangle;
pub mod surface;
pub mod termstructure;

mod mathx;

pub use arbitrage::{ArbitrageReport, check_slice, implied_density};
pub use calibrate::{CalibratedSmile, build_model_smile};
pub use extended_surface::{ExtendedSlice, ExtendedSurface};
pub use market_hedge::MarketHedgeSmile;
pub use parametric::ParametricSlice;
pub use parametric_surface::ParametricSurface;
pub use quotes::{DeltaPillar, MarketContext, MarketQuotes, RiskReversalButterfly};
pub use stochvol::{StochasticVolParams, StochasticVolSmile, WingDensity};
pub use strangle::{
    CalibratedPillar, CalibrationError, MarketStrangle, calibrate_pillar, market_strangle,
};
pub use surface::{SmileModel, SurfaceArbitrageReport, VolSurface};
pub use termstructure::{BusinessClock, CalendarClock, TenorPillar, TermStructure};

/// Build a calibrated Vanna-Volga smile from broker market quotes.
///
/// End-to-end: takes the per-slice [`MarketContext`] (market state + resolved
/// conventions) and the broker [`MarketQuotes`] (ATM + `25Δ` RR/BF, optionally
/// `10Δ`), runs the market(broker)-strangle → smile-strangle calibration on the
/// **inner (`25Δ`)** pillar so the smile reprices the broker strangle exactly,
/// and assembles the three anchor pillars (put wing, ATM, call wing) into a
/// [`MarketHedgeSmile`] implementing [`celnet_core::Smile`].
///
/// The `10Δ` pillar, when present, is used only to *validate* the constructed
/// smile (the three-point Vanna-Volga anchors on the `25Δ` wings; the `10Δ` wing
/// is a downstream consistency check / SVI-SSVI anchor rather than a fourth
/// Vanna-Volga benchmark). See [`build_smile_and_outer`] when the caller needs
/// the calibrated outer pillar too.
///
/// # Errors
///
/// Returns [`CalibrationError`] if the broker-strangle calibration or any
/// delta→strike inversion fails.
pub fn build_smile(
    ctx: &MarketContext,
    quotes: &MarketQuotes,
) -> Result<MarketHedgeSmile, CalibrationError> {
    Ok(build_smile_and_outer(ctx, quotes)?.0)
}

/// As [`build_smile`], but also returns the calibrated outer (`10Δ`) pillar when
/// the quotes carry one (so the caller can validate `10Δ` reproduction or anchor
/// a richer model).
///
/// # Errors
///
/// Returns [`CalibrationError`] if any pillar calibration or strike inversion
/// fails.
pub fn build_smile_and_outer(
    ctx: &MarketContext,
    quotes: &MarketQuotes,
) -> Result<(MarketHedgeSmile, Option<CalibratedPillar>), CalibrationError> {
    let atm_vol = quotes.atm_vol;

    // Calibrate the inner (25Δ) pillar to its broker strangle.
    let inner = calibrate_pillar(ctx, atm_vol, quotes.inner)?;

    // Optional outer (10Δ) calibration for validation / richer anchoring.
    let outer = match quotes.outer {
        Some(pair) => Some(calibrate_pillar(ctx, atm_vol, pair)?),
        None => None,
    };

    // Assemble the three Vanna-Volga anchor pillars: put wing, ATM, call wing.
    // The pillar came from `calibrate_pillar`, whose Ok already guarantees
    // strictly-positive wing vols and a well-ordered slice; we still build the
    // smile through the fallible constructor so the assembly is *robust* — a
    // library smile must never panic on quote-derived inputs. A degenerate set
    // surfaces as a calibration error, never a panic.
    let atm_strike = ctx.atm_strike(atm_vol);
    let strikes = [inner.put_strike, atm_strike, inner.call_strike];
    let vols = [inner.put_vol, atm_vol, inner.call_vol];
    let smile = MarketHedgeSmile::try_new(strikes, vols, ctx.forward(), ctx.t)
        .ok_or(CalibrationError::DegenerateQuote)?;

    Ok((smile, outer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_conventions::resolve;
    use celnet_core::{Smile, is_close};
    use celnet_types::{CcyPair, OptionType, Tenor};
    use celnet_vanilla::price;

    fn ctx(pair: &str, tenor: Tenor, spot: f64, r_dom: f64, r_for: f64, t: f64) -> MarketContext {
        let conv = resolve(CcyPair::parse(pair).unwrap(), tenor).record;
        MarketContext::new(spot, r_dom, r_for, t, conv)
    }

    /// The full pipeline produces a smile that reprices ATM, both 25Δ wings, and
    /// the broker (market) strangle — to tolerance.
    #[test]
    fn pipeline_reprices_atm_rr_bf() {
        let c = ctx("EURUSD", Tenor::Years(1), 1.10, 0.02, 0.01, 1.0);
        let q = MarketQuotes::three_point(0.105, 0.015, 0.0035);
        let (smile, _) = build_smile_and_outer(&c, &q).unwrap();

        // ATM repriced exactly.
        let k_atm = c.atm_strike(q.atm_vol);
        assert!(
            is_close(
                smile.implied_vol(k_atm, c.forward(), c.t).0,
                q.atm_vol,
                1e-9,
                1e-11
            ),
            "ATM vol must be repriced"
        );

        // Each calibrated 25Δ wing is a benchmark of the smile ⇒ repriced exactly.
        let cal = calibrate_pillar(&c, q.atm_vol, q.inner).unwrap();
        assert!(is_close(
            smile.implied_vol(cal.call_strike, c.forward(), c.t).0,
            cal.call_vol,
            1e-9,
            1e-11
        ));
        assert!(is_close(
            smile.implied_vol(cal.put_strike, c.forward(), c.t).0,
            cal.put_vol,
            1e-9,
            1e-11
        ));

        // Risk-reversal reproduced.
        assert!(is_close(
            cal.call_vol - cal.put_vol,
            q.inner.risk_reversal,
            1e-11,
            1e-12
        ));

        // NON-VACUOUS broker reprice: evaluate the constructed smile AT THE
        // BROKER strangle strikes (which are not smile pillars) and require the
        // two options, priced at the smile's interpolated vols there, to sum to
        // the market strangle. This exercises smile.implied_vol() at the broker
        // strikes — the step the audit found missing.
        let ms = market_strangle(&c, q.atm_vol, q.inner).unwrap();
        let f = c.forward();
        let vol_call = smile.implied_vol(ms.call_strike, f, c.t).0;
        let vol_put = smile.implied_vol(ms.put_strike, f, c.t).0;
        let call = price(OptionType::Call, &c.template(ms.call_strike, vol_call));
        let put = price(OptionType::Put, &c.template(ms.put_strike, vol_put));
        assert!(
            is_close(call + put, cal.market_strangle_price, 1e-9, 1e-11),
            "smile @ broker strikes {} must reprice market strangle {}",
            call + put,
            cal.market_strangle_price
        );
    }

    /// The constructed mild-quote smile passes the static arbitrage checks.
    #[test]
    fn pipeline_smile_is_arbitrage_free_for_mild_quotes() {
        let c = ctx("EURUSD", Tenor::Years(1), 1.10, 0.02, 0.01, 1.0);
        let q = MarketQuotes::three_point(0.10, 0.01, 0.0025);
        let smile = build_smile(&c, &q).unwrap();

        let f = c.forward();
        let grid: Vec<f64> = (0..81)
            .map(|i| 0.80 + (1.60 - 0.80) * (i as f64) / 80.0)
            .collect();
        let rep = check_slice(&smile, &grid, f, c.t, 1e-3);
        assert!(
            rep.is_arbitrage_free(1e-5),
            "mild-quote smile must be arbitrage-free across the grid: {rep:?}"
        );
    }

    /// High-RR / EM case: the broker-vs-smile distinction is material — the
    /// calibrated smile-strangle differs from the quoted (arithmetic) butterfly,
    /// and the naive arithmetic smile would *not* reprice the broker strangle.
    #[test]
    fn high_rr_em_broker_vs_smile_distinction() {
        // USDKRW-like EM slice: high vol, large negative-skew RR.
        let c = ctx("USDKRW", Tenor::Months(3), 1330.0, 0.035, 0.05, 0.25);
        let q = MarketQuotes::three_point(0.13, 0.045, 0.007);
        let cal = calibrate_pillar(&c, q.atm_vol, q.inner).unwrap();

        // Calibrated convexity differs from the quoted broker butterfly.
        assert!(
            (cal.smile_strangle - q.inner.butterfly).abs() > 1e-4,
            "EM smile-strangle {} must differ from broker BF {}",
            cal.smile_strangle,
            q.inner.butterfly
        );

        // The NAIVE arithmetic smile (σ_call = ATM + ½RR + BF, σ_put = ATM − ½RR + BF
        // at the broker-strangle strikes) does NOT reprice the broker strangle —
        // this is precisely the production bug the calibration avoids.
        let ms = market_strangle(&c, q.atm_vol, q.inner).unwrap();
        let naive_call_vol = q.atm_vol + 0.5 * q.inner.risk_reversal + q.inner.butterfly;
        let naive_put_vol = q.atm_vol - 0.5 * q.inner.risk_reversal + q.inner.butterfly;
        let naive = price(
            OptionType::Call,
            &c.template(ms.call_strike, naive_call_vol),
        ) + price(OptionType::Put, &c.template(ms.put_strike, naive_put_vol));
        assert!(
            !is_close(naive, ms.price, 1e-6, 1e-8),
            "naive arithmetic smile would mis-reprice the broker strangle \
             (naive={naive}, broker={})",
            ms.price
        );

        // The calibrated smile DOES reprice it — evaluated at the BROKER strikes
        // `ms.call_strike`/`ms.put_strike` (the same strikes the naive smile
        // mis-prices above), not at the smile's own pillars.
        let smile = build_smile(&c, &q).unwrap();
        let smile_call = price(
            OptionType::Call,
            &c.template(
                ms.call_strike,
                smile.implied_vol(ms.call_strike, c.forward(), c.t).0,
            ),
        );
        let smile_put = price(
            OptionType::Put,
            &c.template(
                ms.put_strike,
                smile.implied_vol(ms.put_strike, c.forward(), c.t).0,
            ),
        );
        assert!(
            is_close(
                smile_call + smile_put,
                cal.market_strangle_price,
                1e-9,
                1e-11
            ),
            "calibrated smile @ broker strikes {} must reprice market strangle {}",
            smile_call + smile_put,
            cal.market_strangle_price
        );
    }

    /// Works for a long-tenor forward-delta convention (the convention-aware
    /// requirement): the pipeline builds, reprices ATM, and calibrates a 10Δ
    /// pillar.
    #[test]
    fn pipeline_handles_long_tenor_forward_delta() {
        // 2Y EURUSD resolves to a forward-delta convention; pipeline must work.
        let c = ctx("EURUSD", Tenor::Years(2), 1.10, 0.02, 0.01, 2.0);
        let q = MarketQuotes::five_point(0.11, 0.012, 0.003, 0.02, 0.006);
        let (smile, outer) = build_smile_and_outer(&c, &q).unwrap();
        let k_atm = c.atm_strike(q.atm_vol);
        assert!(is_close(
            smile.implied_vol(k_atm, c.forward(), c.t).0,
            q.atm_vol,
            1e-9,
            1e-11
        ));
        // The outer (10Δ) pillar calibrated and reproduces its risk-reversal.
        let outer = outer.expect("five-point quotes carry a 10Δ pillar");
        assert!(is_close(outer.call_vol - outer.put_vol, 0.02, 1e-11, 1e-12));
    }
}
