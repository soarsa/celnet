//! The `surface` subcommand: calibrate a single smile slice from broker quotes
//! (ATM vol + 25Δ risk-reversal and butterfly) and print the calibrated vol
//! across a strike grid plus the static-arbitrage report.
//!
//! Calibration and arbitrage diagnostics route entirely through `celnet-surface`
//! (the Vanna-Volga broker→smile pipeline and the Breeden-Litzenberger / vertical
//! checks); this module marshals the resolved convention context, builds the
//! evaluation grid, and formats output.

use celnet_core::Smile;
use celnet_surface::{
    ArbitrageReport, CalibrationError, MarketContext, MarketHedgeSmile, MarketQuotes, build_smile,
    check_slice,
};
use celnet_types::CcyPair;

/// The full set of inputs to a `surface` run: market state, broker quotes, and
/// the evaluation-grid shape. Grouped into one struct to keep [`run`] cohesive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SurfaceRequest {
    /// The currency pair (for convention resolution).
    pub(crate) pair: CcyPair,
    /// The tenor (for convention resolution).
    pub(crate) tenor: celnet_types::Tenor,
    /// Spot FX rate.
    pub(crate) spot: f64,
    /// Continuously-compounded domestic rate.
    pub(crate) r_dom: f64,
    /// Continuously-compounded foreign rate.
    pub(crate) r_for: f64,
    /// Vol-time to expiry in years.
    pub(crate) t: f64,
    /// ATM volatility (absolute).
    pub(crate) atm_vol: f64,
    /// 25Δ risk reversal (absolute vol).
    pub(crate) rr_25: f64,
    /// 25Δ butterfly (absolute vol).
    pub(crate) bf_25: f64,
    /// Lower strike band as a multiple of the forward.
    pub(crate) lo: f64,
    /// Upper strike band as a multiple of the forward.
    pub(crate) hi: f64,
    /// Number of grid strikes (`≥ 3`).
    pub(crate) points: usize,
}

/// One evaluated grid point: a strike and the smile's vol there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SmilePoint {
    /// Strike (quote per 1 unit of base).
    pub(crate) strike: f64,
    /// Calibrated Black vol at the strike (absolute, 0.10 = 10 vol).
    pub(crate) vol: f64,
}

/// The full result of a `surface` run: the calibrated smile context, the
/// evaluated grid, and the arbitrage report over that grid.
#[derive(Debug, Clone)]
pub(crate) struct SurfaceResult {
    /// Outright forward implied by the market context.
    pub(crate) forward: f64,
    /// Vol-time to expiry (years) the smile was calibrated/evaluated at.
    pub(crate) t: f64,
    /// ATM strike for the slice's conventions.
    pub(crate) atm_strike: f64,
    /// The evaluated `(strike, vol)` grid (ascending in strike).
    pub(crate) grid: Vec<SmilePoint>,
    /// The static-arbitrage diagnostics over the grid.
    pub(crate) arbitrage: ArbitrageReport,
    /// The calibrated smile (kept so callers can evaluate further points).
    pub(crate) smile: MarketHedgeSmile,
}

/// Calibrate the smile and evaluate it across `points` strikes spanning the
/// `[lo·forward, hi·forward]` band, then run the arbitrage report.
///
/// `lo` / `hi` are multiplicative bounds around the forward (e.g. `0.85` / `1.15`
/// for a ±15% strike band). `points` is the number of grid strikes (`≥ 3`, the
/// arbitrage check's minimum). The grid is uniform in strike and strictly
/// ascending.
///
/// # Errors
///
/// [`CalibrationError`] if the broker→smile calibration fails.
///
/// # Panics
///
/// Panics if `points < 3` or the band is non-ascending (`lo >= hi`) — these are
/// caller (CLI-validation) contract violations, surfaced loudly rather than
/// producing a degenerate grid.
pub(crate) fn run(req: SurfaceRequest) -> Result<SurfaceResult, CalibrationError> {
    assert!(req.points >= 3, "surface grid needs at least 3 strikes");
    assert!(req.lo < req.hi, "strike band must be ascending (lo < hi)");
    assert!(req.lo > 0.0, "strike band lower bound must be positive");

    let conv = celnet_conventions::resolve(req.pair, req.tenor).record;
    let ctx = MarketContext::new(req.spot, req.r_dom, req.r_for, req.t, conv);
    let quotes = MarketQuotes::three_point(req.atm_vol, req.rr_25, req.bf_25);
    let smile = build_smile(&ctx, &quotes)?;

    let forward = ctx.forward();
    let atm_strike = ctx.atm_strike(req.atm_vol);

    // Uniform ascending strike grid across the forward-relative band.
    let k_lo = req.lo * forward;
    let k_hi = req.hi * forward;
    let step = (k_hi - k_lo) / ((req.points - 1) as f64);
    let mut grid = Vec::with_capacity(req.points);
    let mut strikes = Vec::with_capacity(req.points);
    for i in 0..req.points {
        let strike = k_lo + step * (i as f64);
        let vol = smile.implied_vol(strike, forward, req.t).0;
        grid.push(SmilePoint { strike, vol });
        strikes.push(strike);
    }

    // Density spacing for the Breeden-Litzenberger second difference: a small
    // fraction of the grid step, kept strictly below the lowest strike.
    let h = (step * 0.25).min(k_lo * 0.5);
    let arbitrage = check_slice(&smile, &strikes, forward, req.t, h);

    Ok(SurfaceResult {
        forward,
        t: req.t,
        atm_strike,
        grid,
        arbitrage,
        smile,
    })
}

/// Render a [`SurfaceResult`] as a strike/vol table plus the arbitrage verdict.
#[must_use]
pub(crate) fn format_report(r: &SurfaceResult, arb_tol: f64) -> String {
    let mut out = String::new();
    out.push_str(&format!("forward         {:.10}\n", r.forward));
    out.push_str(&format!("atm_strike      {:.10}\n", r.atm_strike));
    // The smile's own vol at the ATM strike (recovers the quoted ATM vol; a quick
    // self-consistency read of the calibrated smile).
    let atm_fwd_vol = r.smile.implied_vol(r.atm_strike, r.forward, r.t).0;
    out.push_str(&format!("atm_vol         {atm_fwd_vol:.10}\n"));
    out.push_str("smile (strike → vol)\n");
    for p in &r.grid {
        out.push_str(&format!("  {:.8}   {:.10}\n", p.strike, p.vol));
    }
    out.push_str("arbitrage\n");
    out.push_str(&format!(
        "  min_butterfly        {:.3e}\n",
        r.arbitrage.min_butterfly
    ));
    out.push_str(&format!(
        "  max_vertical_increase {:.3e}\n",
        r.arbitrage.max_vertical_increase
    ));
    out.push_str(&format!(
        "  min_density          {:.3e}\n",
        r.arbitrage.min_density
    ));
    let verdict = if r.arbitrage.is_arbitrage_free(arb_tol) {
        "arbitrage-free"
    } else {
        "ARBITRAGE DETECTED"
    };
    out.push_str(&format!(
        "  verdict              {verdict} (tol {arb_tol:.1e})\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;
    use celnet_types::Tenor;

    fn eurusd_1y() -> SurfaceResult {
        run(SurfaceRequest {
            pair: CcyPair::parse("EURUSD").unwrap(),
            tenor: Tenor::Years(1),
            spot: 1.10,
            r_dom: 0.02,
            r_for: 0.01,
            t: 1.0,
            atm_vol: 0.105,
            rr_25: 0.015,
            bf_25: 0.0035,
            lo: 0.85,
            hi: 1.15,
            points: 11,
        })
        .unwrap()
    }

    #[test]
    fn grid_vols_match_the_calibrated_smile() {
        let r = eurusd_1y();
        // Every reported grid vol equals a fresh smile evaluation at that strike.
        for p in &r.grid {
            let v = r.smile.implied_vol(p.strike, r.forward, 1.0).0;
            assert!(
                is_close(p.vol, v, 1e-14, 1e-14),
                "grid vol drift at {}",
                p.strike
            );
        }
    }

    #[test]
    fn calibrated_slice_is_arbitrage_free() {
        let r = eurusd_1y();
        assert!(
            r.arbitrage.is_arbitrage_free(1e-6),
            "a benign 25Δ broker slice must be arbitrage-free: {:?}",
            r.arbitrage
        );
    }

    #[test]
    fn atm_vol_recovered_at_atm_strike() {
        let r = eurusd_1y();
        // The smile reprices the ATM vol at the ATM strike (pipeline invariant).
        let v = r.smile.implied_vol(r.atm_strike, r.forward, 1.0).0;
        assert!(is_close(v, 0.105, 1e-9, 1e-9), "ATM vol drift: {v}");
    }
}
