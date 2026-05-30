//! Vanna-Volga overlay with survival-probability (first-exit) weighting.
//!
//! The analytic cores in [`crate::touch`] and [`crate::barrier`] price exotics in
//! a **flat-volatility** Garman-Kohlhagen world (one ATM vol). Real FX exotics are
//! quoted off the **smile**, so a smile-consistent overlay is required. The
//! FX-market-standard light correction is **Vanna-Volga**: the exotic inherits the
//! market cost of the static vega / vanna / volga hedge that makes a flat-vol
//! Black-Scholes book smile-neutral.
//!
//! # Construction
//!
//! At the ATM vol `σ₀`, build the unique portfolio of three liquid benchmark
//! vanillas (ATM, a `25Δ` risk-reversal, a `25Δ` butterfly) that matches the
//! exotic's Black-Scholes **vega, vanna and volga**. The market over-/under-prices
//! that hedge relative to flat vol because the wings carry the quoted smile; the
//! difference — the **Vanna-Volga cost** — added to the flat-vol exotic price
//! gives the smile-consistent price.
//!
//! Because vanna and volga of the benchmark RR/BF are (to first order)
//! proportional to the smile's risk-reversal and butterfly, the cost collapses to
//!
//! ```text
//!   cost ≈ vanna_X · price_of_one_vanna_unit  +  volga_X · price_of_one_volga_unit
//! ```
//!
//! where `price_of_one_vanna_unit` and `price_of_one_volga_unit` are read off the
//! smile vs the flat-vol benchmark at the `25Δ` pillars (the "market price of
//! vanna / volga"). `vanna_X`, `volga_X` are the exotic's Black-Scholes vanna and
//! volga.
//!
//! # Survival weighting (first-exit)
//!
//! For **path-dependent** exotics (touches, knock-outs) the full Vanna-Volga cost
//! over-states the correction, because once the spot exits the live region the
//! option no longer carries vega/vanna/volga risk. The standard FX desk damper is
//! to **scale the cost by the survival (no-touch) probability** `p` — the
//! probability the exotic is still alive — equivalently the first-exit-time
//! adjustment. [`SurvivalWeight`] carries `p ∈ [0, 1]`; European digitals use
//! `p = 1` (no early exit).
//!
//! Provenance (doc-only): Castagna-Mercurio (2007); Bossens, Rayée, Skantzos &
//! Deelstra (2010) "Vanna-Volga methods applied to FX derivatives"; Wystup (2017).
//! Identifiers are purpose-named and vendor/research-neutral.

use celnet_core::Smile;
use celnet_core::math::{exp, ln, norm_pdf, sqrt};
use celnet_types::VanillaInputs;

/// The survival (no-early-exit) weight applied to the Vanna-Volga cost.
///
/// `probability` is the chance the exotic is still alive at expiry (no barrier /
/// touch event). A European payoff that cannot exit early uses
/// [`SurvivalWeight::EUROPEAN`] (`p = 1`); a path-dependent exotic passes its
/// analytic survival / no-touch probability so the smile correction is damped by
/// the time the option actually carries smile risk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurvivalWeight {
    /// Survival probability `p ∈ [0, 1]`.
    pub probability: f64,
}

impl SurvivalWeight {
    /// No early exit (European): full Vanna-Volga cost.
    pub const EUROPEAN: SurvivalWeight = SurvivalWeight { probability: 1.0 };

    /// Construct a survival weight, clamped to `[0, 1]`.
    #[must_use]
    pub fn new(probability: f64) -> Self {
        Self {
            probability: probability.clamp(0.0, 1.0),
        }
    }
}

/// The result of applying the Vanna-Volga overlay to a flat-vol exotic price.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayResult {
    /// The flat-vol (ATM) analytic price the overlay started from.
    pub flat_vol_price: f64,
    /// The (survival-weighted) Vanna-Volga cost added to it.
    pub hedge_smile_cost: f64,
    /// The smile-consistent price `flat_vol_price + hedge_smile_cost`.
    pub smile_price: f64,
}

/// The exotic's Black-Scholes vanna and volga at the ATM vol — the sensitivities
/// the overlay weights the market price of vanna / volga by.
///
/// The caller supplies these for the specific exotic (computed analytically for
/// digitals, by finite difference of the closed form for touches / barriers). The
/// overlay is exotic-agnostic: it only needs the two cross-Greeks and the
/// reference market state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExoticSensitivities {
    /// Black-Scholes vanna `∂²V/∂S∂σ` of the exotic at ATM vol.
    pub vanna: f64,
    /// Black-Scholes volga `∂²V/∂σ²` of the exotic at ATM vol.
    pub volga: f64,
}

/// Market price of one unit of vanna and one unit of volga, read off the smile at
/// the `25Δ` pillars relative to the flat ATM-vol benchmark.
///
/// These are the "cost of a unit of vanna / volga" the Vanna-Volga method extracts
/// from the smile: how much the market pays, per unit of benchmark vanna / volga,
/// over the flat-vol Black-Scholes value. They are derived once per `(pair, tenor)`
/// from the calibrated smile via [`market_price_of_hedge_smile`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketCrossPrices {
    /// Market price per unit of vanna (skew / risk-reversal driven).
    pub vanna_price: f64,
    /// Market price per unit of volga (convexity / butterfly driven).
    pub volga_price: f64,
}

/// Black-Scholes vega, vanna and volga of a single benchmark vanilla at the
/// flat ATM vol, in the forward measure (the quantities the smile correction is
/// projected onto).
struct BenchGreeks {
    vega: f64,
    vanna: f64,
    volga: f64,
}

/// Compute the forward-measure vega/vanna/volga of a vanilla benchmark at strike
/// `strike`, forward `forward`, vol `vol`, time `t`.
fn bench_greeks(forward: f64, strike: f64, vol: f64, t: f64, df_dom: f64) -> BenchGreeks {
    let sqt = sqrt(t);
    let vsqt = vol * sqt;
    let d1 = (ln(forward / strike) + 0.5 * vol * vol * t) / vsqt;
    let d2 = d1 - vsqt;
    let pdf = norm_pdf(d1);
    // Forward (Black) vega of a vanilla on the forward: F·DF_dom·φ(d1)·√t.
    let vega = forward * df_dom * pdf * sqt;
    let vanna = -df_dom * pdf * d2 / vol;
    let volga = vega * d1 * d2 / vol;
    BenchGreeks { vega, vanna, volga }
}

/// Extract the **market price of vanna and volga** from a smile at the `25Δ`
/// pillars, relative to the flat ATM-vol benchmark.
///
/// `i` carries spot/rates/time and the ATM vol in `i.vol`; `put_strike` and
/// `call_strike` are the `25Δ` smile wings (from [`celnet_surface`]'s calibrated
/// pillars). `smile` supplies the wing vols. The construction matches the
/// benchmark vanna / volga of the risk-reversal and butterfly to their market
/// surplus over flat vol, then solves the 2×2 system for the per-unit prices.
///
/// The result is independent of the exotic — compute it once per slice and reuse.
#[must_use]
pub fn market_price_of_hedge_smile<S: Smile>(
    smile: &S,
    i: &VanillaInputs,
    put_strike: f64,
    call_strike: f64,
) -> MarketCrossPrices {
    let f = i.forward();
    let t = i.t;
    let s0 = i.vol; // ATM vol
    let df_dom = exp(-i.r_dom * t);

    // Wing smile vols.
    let sig_put = smile.implied_vol(put_strike, f, t).0;
    let sig_call = smile.implied_vol(call_strike, f, t).0;

    // Benchmark Greeks at the two wings (computed at flat ATM vol — the VV
    // construction matches the FLAT-vol vega/vanna/volga of the hedge).
    let put = bench_greeks(f, put_strike, s0, t, df_dom);
    let call = bench_greeks(f, call_strike, s0, t, df_dom);

    // Market surplus of each wing vanilla over its flat-vol value, to first order
    // in the vol difference: ΔV ≈ vega·(σ_wing − σ₀). This is the price the
    // market pays for that wing's risk above flat vol.
    let surplus_put = put.vega * (sig_put - s0);
    let surplus_call = call.vega * (sig_call - s0);

    // The risk-reversal (long call wing, short put wing) isolates VANNA; the
    // butterfly (long both wings, short 2× ATM) isolates VOLGA. Project:
    //   rr_surplus = surplus_call − surplus_put   ↔   rr_vanna = call.vanna − put.vanna
    //   bf_surplus = surplus_call + surplus_put   ↔   bf_volga = call.volga + put.volga
    // (ATM vanna ≈ 0, ATM volga ≈ 0 at the forward, so they drop out.)
    let rr_surplus = surplus_call - surplus_put;
    let bf_surplus = surplus_call + surplus_put;
    let rr_vanna = call.vanna - put.vanna;
    let bf_volga = call.volga + put.volga;

    let vanna_price = if rr_vanna.abs() > 1e-300 {
        rr_surplus / rr_vanna
    } else {
        0.0
    };
    let volga_price = if bf_volga.abs() > 1e-300 {
        bf_surplus / bf_volga
    } else {
        0.0
    };

    MarketCrossPrices {
        vanna_price,
        volga_price,
    }
}

/// The raw (un-weighted) Vanna-Volga cost for an exotic with the given
/// sensitivities against the market cross-prices.
///
/// `cost = vanna_X · vanna_price + volga_X · volga_price`.
#[must_use]
pub fn hedge_smile_cost(x: ExoticSensitivities, market: MarketCrossPrices) -> f64 {
    x.vanna * market.vanna_price + x.volga * market.volga_price
}

/// Compute an exotic's Black-Scholes vanna and volga by central finite
/// differences of its flat-vol closed-form price.
///
/// The analytic touch / barrier cores in this crate are closed forms in the flat
/// ATM vol; their cross-Greeks have no compact closed form, so the overlay reads
/// them off the same closed form by differencing. `flat_price(spot, vol)` is the
/// exotic's flat-vol price as a function of bumped spot and vol — the caller
/// closes over the fixed contract terms. Both bumps default to a stable fraction
/// of the level.
///
/// * `vanna = ∂²V/∂S∂σ` via a 4-point mixed central difference;
/// * `volga = ∂²V/∂σ²` via a 3-point central second difference.
#[must_use]
pub fn exotic_sensitivities_fd<F: Fn(f64, f64) -> f64>(
    flat_price: F,
    spot: f64,
    vol: f64,
) -> ExoticSensitivities {
    let hs = 1e-4 * spot;
    let hv = 1e-4;

    // Mixed second derivative ∂²V/∂S∂σ (4-point stencil).
    let vanna = (flat_price(spot + hs, vol + hv)
        - flat_price(spot + hs, vol - hv)
        - flat_price(spot - hs, vol + hv)
        + flat_price(spot - hs, vol - hv))
        / (4.0 * hs * hv);

    // Second derivative in vol ∂²V/∂σ² (3-point stencil).
    let v0 = flat_price(spot, vol);
    let volga = (flat_price(spot, vol + hv) - 2.0 * v0 + flat_price(spot, vol - hv)) / (hv * hv);

    ExoticSensitivities { vanna, volga }
}

/// Apply the full survival-weighted Vanna-Volga overlay.
///
/// Adds the survival-weighted Vanna-Volga cost to `flat_vol_price`:
/// ```text
///   smile_price = flat_vol_price + p · (vanna_X·vanna_price + volga_X·volga_price)
/// ```
/// where `p` is the survival weight (1 for European, the no-touch / no-knock
/// probability for path-dependent exotics). Returns the breakdown so callers can
/// audit the correction.
#[must_use]
pub fn hedge_smile_overlay(
    flat_vol_price: f64,
    x: ExoticSensitivities,
    market: MarketCrossPrices,
    survival: SurvivalWeight,
) -> OverlayResult {
    let raw = hedge_smile_cost(x, market);
    let cost = survival.probability * raw;
    OverlayResult {
        flat_vol_price,
        hedge_smile_cost: cost,
        smile_price: flat_vol_price + cost,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::{FlatSmile, assert_close};

    fn base() -> VanillaInputs {
        // EURUSD-like 1Y: S=1.30, σ_ATM=10%, r_d=3%, r_f=1%.
        VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01)
    }

    /// A FLAT smile (no skew, no convexity) prices zero vanna and volga cost — the
    /// overlay must leave a flat-vol price unchanged.
    #[test]
    fn flat_smile_zero_cost() {
        let i = base();
        let smile = FlatSmile::new(i.vol);
        let f = i.forward();
        // Symmetric 25Δ-ish wings around the forward in log space.
        let (kp, kc) = (f * 0.92, f * 1.08);
        let market = market_price_of_hedge_smile(&smile, &i, kp, kc);
        assert_close!(market.vanna_price, 0.0, 1e-9, 1e-10);
        assert_close!(market.volga_price, 0.0, 1e-9, 1e-10);

        let x = ExoticSensitivities {
            vanna: 0.7,
            volga: -1.3,
        };
        let r = hedge_smile_overlay(2.5, x, market, SurvivalWeight::EUROPEAN);
        assert_close!(r.smile_price, 2.5, 1e-9, 1e-10);
    }

    /// A positive risk-reversal (call wing richer than put wing) yields a positive
    /// market price of vanna; the overlay then shifts a positive-vanna exotic UP
    /// and a negative-vanna exotic DOWN — the documented, smile-consistent
    /// direction.
    #[test]
    fn risk_reversal_signs_vanna_correction() {
        let i = base();
        let f = i.forward();
        let (kp, kc) = (f * 0.90, f * 1.10);

        // A skewed smile: call wing 12%, put wing 9% (positive RR).
        let skewed = ThreePointSmile {
            put_strike: kp,
            call_strike: kc,
            put_vol: 0.09,
            call_vol: 0.12,
            atm_vol: 0.10,
        };
        let market = market_price_of_hedge_smile(&skewed, &i, kp, kc);
        assert!(
            market.vanna_price > 0.0,
            "positive RR ⇒ positive vanna price, got {}",
            market.vanna_price
        );

        let pos_vanna = ExoticSensitivities {
            vanna: 1.0,
            volga: 0.0,
        };
        let up = hedge_smile_overlay(1.0, pos_vanna, market, SurvivalWeight::EUROPEAN);
        assert!(up.smile_price > 1.0, "positive-vanna exotic must shift up");

        let neg_vanna = ExoticSensitivities {
            vanna: -1.0,
            volga: 0.0,
        };
        let down = hedge_smile_overlay(1.0, neg_vanna, market, SurvivalWeight::EUROPEAN);
        assert!(
            down.smile_price < 1.0,
            "negative-vanna exotic must shift down"
        );
    }

    /// A positive butterfly (both wings above ATM) yields a positive market price
    /// of volga; a positive-volga exotic shifts UP.
    #[test]
    fn butterfly_signs_volga_correction() {
        let i = base();
        let f = i.forward();
        // Log-symmetric wings around the forward so the butterfly carries no skew
        // (kc/f = f/kp) ⇒ the vanna leg cancels cleanly.
        let (kp, kc) = (f / 1.10, f * 1.10);
        // Symmetric convex smile: both wings 11.5%, ATM 10% (positive BF, zero RR).
        let convex = ThreePointSmile {
            put_strike: kp,
            call_strike: kc,
            put_vol: 0.115,
            call_vol: 0.115,
            atm_vol: 0.10,
        };
        let market = market_price_of_hedge_smile(&convex, &i, kp, kc);
        assert!(
            market.volga_price > 0.0,
            "positive BF ⇒ positive volga price, got {}",
            market.volga_price
        );
        // Zero RR + log-symmetric wings ⇒ the vanna leg is negligible relative to
        // the volga leg (only a small residual cross-coupling from the forward
        // weighting survives).
        assert!(
            market.vanna_price.abs() < 0.10 * market.volga_price.abs(),
            "vanna price {} should be negligible vs volga price {}",
            market.vanna_price,
            market.volga_price
        );

        let pos_volga = ExoticSensitivities {
            vanna: 0.0,
            volga: 2.0,
        };
        let r = hedge_smile_overlay(1.0, pos_volga, market, SurvivalWeight::EUROPEAN);
        assert!(r.smile_price > 1.0, "positive-volga exotic must shift up");
    }

    /// The survival weight scales the cost linearly: `p = 0` ⇒ no correction;
    /// `p = ½` ⇒ half the European correction.
    #[test]
    fn survival_weight_scales_cost() {
        let i = base();
        let f = i.forward();
        let (kp, kc) = (f * 0.90, f * 1.10);
        let skewed = ThreePointSmile {
            put_strike: kp,
            call_strike: kc,
            put_vol: 0.09,
            call_vol: 0.12,
            atm_vol: 0.10,
        };
        let market = market_price_of_hedge_smile(&skewed, &i, kp, kc);
        let x = ExoticSensitivities {
            vanna: 1.0,
            volga: 0.5,
        };
        let full = hedge_smile_overlay(1.0, x, market, SurvivalWeight::EUROPEAN);
        let half = hedge_smile_overlay(1.0, x, market, SurvivalWeight::new(0.5));
        let none = hedge_smile_overlay(1.0, x, market, SurvivalWeight::new(0.0));
        assert_close!(none.hedge_smile_cost, 0.0, 1e-12, 1e-12);
        assert_close!(
            half.hedge_smile_cost,
            0.5 * full.hedge_smile_cost,
            1e-12,
            1e-12
        );
    }

    /// Survival weight is clamped into `[0, 1]`.
    #[test]
    fn survival_weight_clamped() {
        assert_close!(SurvivalWeight::new(1.5).probability, 1.0, 1e-15, 1e-15);
        assert_close!(SurvivalWeight::new(-0.3).probability, 0.0, 1e-15, 1e-15);
    }

    /// A minimal three-point smile used only by these unit tests (the production
    /// path consumes `celnet_surface::MarketHedgeSmile` / `VolSurface` through the
    /// same [`Smile`] trait). Linear-in-log-strike interpolation suffices here.
    #[derive(Clone, Copy)]
    struct ThreePointSmile {
        put_strike: f64,
        call_strike: f64,
        put_vol: f64,
        call_vol: f64,
        atm_vol: f64,
    }

    impl Smile for ThreePointSmile {
        fn implied_vol(&self, strike: f64, _forward: f64, _t: f64) -> celnet_types::Vol {
            let v = if (strike - self.put_strike).abs() < 1e-12 {
                self.put_vol
            } else if (strike - self.call_strike).abs() < 1e-12 {
                self.call_vol
            } else {
                self.atm_vol
            };
            celnet_types::Vol(v)
        }
    }
}
