//! Local-stochastic-volatility (LSV) booking-model pricing route.
//!
//! When a wire [`celnet_proto::Instrument`] selects
//! [`celnet_proto::PricingModel::LocalStochVol`] (rule 9: a pricing *directive*,
//! not an API version), the supported products are priced on the
//! [`celnet_exotics::lsv`] engine instead of the default analytic / closed-form
//! path: a particle-calibrated leverage surface over a mean-reverting square-root
//! variance backbone, solved on the 2-D ADI PDE (and the counter-based
//! Monte-Carlo engine, which reports a standard error, for a window barrier when
//! the request asks for it).
//!
//! # Supported products (the honest list)
//!
//! The LSV engine ([`celnet_exotics::lsv::LsvModel`]) prices exactly:
//!
//! * **vanilla** — European call/put, priced on the ADI PDE
//!   ([`LsvModel::price_european_pde`]);
//! * **single-barrier knock-out** — a continuously-monitored, full-life
//!   knock-out, priced on the ADI PDE ([`LsvModel::price_barrier_pde`]). A
//!   knock-in is priced by in-out parity (`knock-in = vanilla − knock-out` under
//!   the same model);
//! * **window barrier** — a continuously-monitored knock-out active only inside a
//!   calendar window, priced on the ADI PDE
//!   ([`LsvModel::price_window_barrier_pde`]) or, when the request sets
//!   `mc_pairs > 0`, on the Monte-Carlo engine
//!   ([`LsvModel::price_window_barrier_mc`], which carries a standard error).
//!
//! Selecting LSV for **any other product** (strategy, double-barrier, digital,
//! touch, variance/vol swap, Asian, forward-start, cliquet, quanto, TARF,
//! accumulator, lookback) is a hard [`PriceError::UnsupportedModel`] →
//! `INVALID_ARGUMENT` — never a silent fallback to the default engine, and never
//! a fabricated price.
//!
//! # Calibration target
//!
//! The wire market context carries a single (pinned-or-live) at-forward Black
//! vol, so the LSV is calibrated to a **flat implied-vol surface** anchored at
//! that vol: `v0 = θ = σ²`, with the canonical server stochastic-variance
//! parameters ([`LSV_MEAN_REVERSION`], [`LSV_VOL_OF_VAR`], [`LSV_CORRELATION`]).
//! The particle method then fits the leverage so the model's marginals reproduce
//! that surface. This is a genuine LSV model (non-zero vol-of-variance); the
//! single-vol calibration anchor is exactly what the uniform `MarketContext`
//! exposes across every flow. Richer smile-surface calibration (the
//! [`celnet_exotics::lsv::SurfaceTarget`] seam against a calibrated
//! [`celnet_surface::VolSurface`]) is a drop-in once the wire carries a full
//! smile, and changes nothing about the route below.
//!
//! # Risk (the Greek strip)
//!
//! The model is calibrated **once**; the full finite-difference Greek strip is
//! then taken by repricing under spot / vol / rate / expiry bumps **reusing the
//! same calibrated leverage surface** ([`LsvModel::from_leverage`]). Holding the
//! leverage fixed under a bump is the correct risk convention — sensitivities are
//! measured under a *fixed* model, not a re-calibrated one — and it keeps a price
//! request to one calibration plus a bounded set of ADI solves on the coarser
//! [`LsvGrids::greek`] grid. The headline price uses the finer
//! [`LsvGrids::price`] grid.
//!
//! # Method provenance (doc comments only)
//!
//! LSV / leverage identity: Ren-Madan-Qian (2007), Guyon & Henry-Labordère
//! (2012). Variance backbone: Heston (1993); QE: Andersen (2008); 2-D ADI:
//! Hundsdorfer-Verwer (2003), in 't Hout-Foulon (2010). Identifiers are
//! purpose-named; provenance lives only in documentation (GUIDE.md rule 8).

use celnet_exotics::{
    AdiGrid, ImpliedVolSurface, LeverageSurface, LsvModel, McConfig, ParticleConfig,
    VarianceParams, WindowBarrier as ExWindowBarrier,
};
use celnet_proto::{
    BarrierSide as WireBarrierSide, MarketContext as WireMarketContext,
    OptionType as WireOptionType, Vanilla, WindowBarrier as WireWindowBarrier,
};
use celnet_types::{Greeks, OptionType, VanillaInputs};

use crate::pricer::{PriceError, Priced};

/// The canonical mean-reversion speed `κ` (per year) of the server's
/// stochastic-variance backbone. A moderate, Feller-respecting value with the
/// vol-of-variance below.
pub const LSV_MEAN_REVERSION: f64 = 2.0;
/// The canonical vol-of-variance `ξ`. Non-zero — this is a genuine
/// *stochastic*-vol model, not a degenerate local-vol limit — and small enough
/// that `2κθ ≥ ξ²` (Feller) holds at a representative FX vol (`σ = 10% ⇒
/// θ = 0.01 ⇒ 2κθ = 0.04 ≥ ξ² = 0.0324`), so the variance stays strictly
/// positive a.s. (the full-truncation scheme handles any violation regardless).
pub const LSV_VOL_OF_VAR: f64 = 0.18;
/// The canonical spot/variance correlation `ρ` (the FX leverage/skew sign).
pub const LSV_CORRELATION: f64 = -0.30;

/// The grid resolutions the server uses for an LSV price. The headline price is
/// solved on `price`; the finite-difference Greek strip is solved on the coarser
/// `greek` grid (one calibration, many bounded solves). `mc_steps` is the
/// Monte-Carlo time-step count when a window barrier is priced on the MC engine.
#[derive(Debug, Clone, Copy)]
pub struct LsvGrids {
    /// Particle-calibration configuration (run once per request).
    pub particle: ParticleConfig,
    /// The fine ADI grid for the headline price.
    pub price: AdiGrid,
    /// The coarse ADI grid for the finite-difference Greek strip.
    pub greek: AdiGrid,
    /// The number of leverage spot nodes spanning ±~5 stdev around spot.
    pub leverage_nodes: usize,
    /// Default Monte-Carlo time steps for the window-barrier MC engine.
    pub mc_steps: usize,
}

impl Default for LsvGrids {
    fn default() -> Self {
        Self {
            particle: ParticleConfig {
                particles: 30_000,
                steps: 40,
                seed: 0x0001_0CA1,
                ..ParticleConfig::default()
            },
            price: AdiGrid {
                x_steps: 160,
                v_steps: 48,
                time_steps: 100,
                ..AdiGrid::default()
            },
            greek: AdiGrid {
                x_steps: 64,
                v_steps: 20,
                time_steps: 36,
                ..AdiGrid::default()
            },
            leverage_nodes: 41,
            mc_steps: 96,
        }
    }
}

/// Default Monte-Carlo antithetic path pairs for a window-barrier priced on the
/// LSV MC engine when the wire request leaves `mc_pairs` unset but selects MC.
pub const DEFAULT_LSV_WINDOW_MC_PAIRS: usize = 100_000;

/// A flat implied-vol surface anchored at the market's at-forward vol — the
/// calibration target the uniform [`WireMarketContext`] exposes.
struct FlatIv {
    sigma: f64,
    spot: f64,
    carry: f64,
}

impl ImpliedVolSurface for FlatIv {
    fn implied_vol(&self, _strike: f64, _t: f64) -> f64 {
        self.sigma
    }
    fn forward(&self, t: f64) -> f64 {
        self.spot * celnet_core::math::exp(self.carry * t)
    }
}

/// Build the canonical variance parameters anchored at a flat vol `sigma`.
fn variance_params(sigma: f64) -> VarianceParams {
    let v = sigma * sigma;
    VarianceParams::new(v, LSV_MEAN_REVERSION, v, LSV_VOL_OF_VAR, LSV_CORRELATION)
}

/// The log-spaced leverage spot grid spanning ±~5 stdev around `spot`.
fn leverage_spot_grid(spot: f64, nodes: usize) -> Vec<f64> {
    let n = nodes.max(2);
    let lo = -0.6_f64;
    let span = 1.2_f64;
    (0..n)
        .map(|k| {
            let x = lo + span * (k as f64) / ((n - 1) as f64);
            spot * celnet_core::math::exp(x)
        })
        .collect()
}

/// Calibrate the LSV leverage surface for a market context (run once per
/// request). The leverage is the model's fixed parameter; risk bumps reuse it.
fn calibrate_leverage_for(
    market: &WireMarketContext,
    expiry: f64,
    grids: &LsvGrids,
) -> (VanillaInputs, VarianceParams, LeverageSurface) {
    let carry = market.r_dom() - market.r_for();
    let iv = FlatIv {
        sigma: market.vol,
        spot: market.spot,
        carry,
    };
    let var = variance_params(market.vol);
    // The vol/strike fields of `inputs` are placeholders for the LSV — the smile
    // lives in `iv`; the spot, carry rates and horizon are authoritative.
    let inputs = VanillaInputs::new(
        market.spot,
        market.spot,
        market.vol,
        expiry,
        market.r_dom(),
        market.r_for(),
    );
    let spot_grid = leverage_spot_grid(market.spot, grids.leverage_nodes);
    let model = LsvModel::calibrate((&inputs).into(), var, &iv, &spot_grid, grids.particle);
    (inputs, var, model.leverage().clone())
}

/// Decode the wire option type into the engine enum.
fn decode_option_type(tag: i32) -> Result<OptionType, PriceError> {
    match WireOptionType::try_from(tag) {
        Ok(WireOptionType::Call) => Ok(OptionType::Call),
        Ok(WireOptionType::Put) => Ok(OptionType::Put),
        Err(_) => Err(PriceError::UnknownEnum {
            kind: "OptionType",
            tag,
        }),
    }
}

/// Extract the (option-type, absolute strike) from a wire [`Vanilla`] leg. LSV
/// products carry an absolute strike (no delta inversion under the LSV measure).
fn vanilla_strike(v: &Vanilla, field: &'static str) -> Result<(OptionType, f64), PriceError> {
    let option = decode_option_type(v.option_type)?;
    let strike = match v
        .strike
        .as_ref()
        .and_then(|s| s.spec.as_ref())
        .ok_or(PriceError::MissingField(field))?
    {
        celnet_proto::strike_or_delta::Spec::Strike(k) => *k,
        celnet_proto::strike_or_delta::Spec::Delta(_) => {
            return Err(PriceError::Domain(
                "the LSV model requires an absolute strike (delta-keyed strikes \
                 are resolved under the analytic measure only)",
            ));
        }
    };
    if !(strike.is_finite() && strike > 0.0) {
        return Err(PriceError::Domain("strike must be positive and finite"));
    }
    Ok((option, strike))
}

/// What the LSV reprice closure prices, parameterised so the Greek finite
/// differences reuse the one calibrated leverage surface.
enum LsvPayoff {
    /// European vanilla.
    Vanilla { option: OptionType, strike: f64 },
    /// Full-life single knock-out (in-out parity gives knock-in).
    Knockout {
        option: OptionType,
        strike: f64,
        barrier: f64,
        up: bool,
        knock_in: bool,
    },
    /// Window knock-out, PDE-priced.
    Window(ExWindowBarrier),
}

/// Reprice an [`LsvPayoff`] under a (possibly bumped) market context and expiry,
/// reusing the supplied calibrated `leverage`, on the given ADI `grid`. This is
/// the FD reprice primitive for the Greek strip; the headline price uses it too
/// (with the fine grid).
fn reprice(
    payoff: &LsvPayoff,
    market: &WireMarketContext,
    expiry: f64,
    leverage: &LeverageSurface,
    grid: AdiGrid,
) -> f64 {
    let var = variance_params(market.vol);
    let inputs = VanillaInputs::new(
        market.spot,
        market.spot,
        market.vol,
        expiry,
        market.r_dom(),
        market.r_for(),
    );
    let model = LsvModel::from_leverage((&inputs).into(), var, leverage.clone());
    match *payoff {
        LsvPayoff::Vanilla { option, strike } => model.price_european_pde(option, strike, grid),
        LsvPayoff::Knockout {
            option,
            strike,
            barrier,
            up,
            knock_in,
        } => {
            let ko = model.price_barrier_pde(option, strike, barrier, up, grid);
            if knock_in {
                // In-out parity under the SAME model: KI = vanilla − KO.
                let vanilla = model.price_european_pde(option, strike, grid);
                vanilla - ko
            } else {
                ko
            }
        }
        LsvPayoff::Window(spec) => model.price_window_barrier_pde(
            ExWindowBarrier {
                option: spec.option,
                strike: spec.strike,
                barrier: spec.barrier,
                up: spec.up,
                start: spec.start.min(expiry),
                end: spec.end.min(expiry),
            },
            grid,
        ),
    }
}

/// Finite-difference bump sizes (mirroring the analytic exotic FD set).
const FD_SPOT_REL: f64 = 1e-3;
const FD_VOL_ABS: f64 = 1e-3;
const FD_RATE_ABS: f64 = 1e-4;
const FD_TIME_REL: f64 = 1e-3;

fn bump_spot(m: &WireMarketContext, rel: f64) -> WireMarketContext {
    WireMarketContext {
        spot: m.spot * (1.0 + rel),
        ..*m
    }
}
fn bump_vol(m: &WireMarketContext, d: f64) -> WireMarketContext {
    WireMarketContext {
        vol: m.vol + d,
        ..*m
    }
}

/// Build the 13-Greek strip of an LSV product by central finite differences on
/// the coarse Greek grid, reusing the single calibrated `leverage`. The headline
/// `base` price (computed on the fine grid by the caller) is substituted for the
/// FD base so the reported `price` is the fine-grid value while the
/// sensitivities are the (consistent) coarse-grid differences.
fn lsv_greeks(
    payoff: &LsvPayoff,
    market: &WireMarketContext,
    expiry: f64,
    leverage: &LeverageSurface,
    grids: &LsvGrids,
    headline_price: f64,
) -> Greeks {
    let g = grids.greek;
    let p = |m: &WireMarketContext, t: f64| reprice(payoff, m, t, leverage, g);

    let base = p(market, expiry);

    // Spot Greeks.
    let h_s = market.spot * FD_SPOT_REL;
    let m_up = bump_spot(market, FD_SPOT_REL);
    let m_dn = bump_spot(market, -FD_SPOT_REL);
    let p_up = p(&m_up, expiry);
    let p_dn = p(&m_dn, expiry);
    let delta_spot = (p_up - p_dn) / (2.0 * h_s);
    let gamma = (p_up - 2.0 * base + p_dn) / (h_s * h_s);
    let m_up2 = bump_spot(market, 2.0 * FD_SPOT_REL);
    let m_dn2 = bump_spot(market, -2.0 * FD_SPOT_REL);
    let speed =
        (p(&m_up2, expiry) - 2.0 * p_up + 2.0 * p_dn - p(&m_dn2, expiry)) / (2.0 * h_s * h_s * h_s);

    // Vol Greeks.
    let h_v = FD_VOL_ABS;
    let v_up = p(&bump_vol(market, h_v), expiry);
    let v_dn = p(&bump_vol(market, -h_v), expiry);
    let vega = (v_up - v_dn) / (2.0 * h_v);
    let volga = (v_up - 2.0 * base + v_dn) / (h_v * h_v);

    // Vanna and zomma (cross spot/vol).
    let p_su_vu = p(&bump_vol(&m_up, h_v), expiry);
    let p_su_vd = p(&bump_vol(&m_up, -h_v), expiry);
    let p_sd_vu = p(&bump_vol(&m_dn, h_v), expiry);
    let p_sd_vd = p(&bump_vol(&m_dn, -h_v), expiry);
    let vanna = (p_su_vu - p_su_vd - p_sd_vu + p_sd_vd) / (4.0 * h_s * h_v);
    let gamma_vu = (p_su_vu - 2.0 * v_up + p_sd_vu) / (h_s * h_s);
    let gamma_vd = (p_su_vd - 2.0 * v_dn + p_sd_vd) / (h_s * h_s);
    let zomma = (gamma_vu - gamma_vd) / (2.0 * h_v);

    // Rate Greeks.
    let h_r = FD_RATE_ABS;
    let rho_dom = {
        let up = p(&market.with_r_dom(market.r_dom() + h_r), expiry);
        let dn = p(&market.with_r_dom(market.r_dom() - h_r), expiry);
        (up - dn) / (2.0 * h_r)
    };
    let rho_for = {
        let up = p(&market.with_r_for(market.r_for() + h_r), expiry);
        let dn = p(&market.with_r_for(market.r_for() - h_r), expiry);
        (up - dn) / (2.0 * h_r)
    };

    // Time Greeks.
    let h_t = expiry * FD_TIME_REL;
    let t_up = expiry + h_t;
    let t_dn = (expiry - h_t).max(f64::MIN_POSITIVE);
    let theta = -(p(market, t_up) - p(market, t_dn)) / (2.0 * h_t);
    let charm = {
        let d_up = (p(&m_up, t_up) - p(&m_dn, t_up)) / (2.0 * h_s);
        let d_dn = (p(&m_up, t_dn) - p(&m_dn, t_dn)) / (2.0 * h_s);
        (d_up - d_dn) / (2.0 * h_t)
    };
    let color = {
        let g_up = (p(&m_up, t_up) - 2.0 * p(market, t_up) + p(&m_dn, t_up)) / (h_s * h_s);
        let g_dn = (p(&m_up, t_dn) - 2.0 * p(market, t_dn) + p(&m_dn, t_dn)) / (h_s * h_s);
        (g_up - g_dn) / (2.0 * h_t)
    };

    let delta_forward = delta_spot * celnet_core::math::exp(market.r_for() * expiry);

    Greeks {
        price: headline_price,
        delta_spot,
        delta_forward,
        gamma,
        vega,
        theta,
        rho_dom,
        rho_for,
        vanna,
        volga,
        charm,
        speed,
        zomma,
        color,
    }
}

/// Price a wire vanilla under the LSV engine.
pub fn price_vanilla_lsv(
    v: &Vanilla,
    market: &WireMarketContext,
    expiry: f64,
    grids: &LsvGrids,
) -> Result<Priced, PriceError> {
    let (option, strike) = vanilla_strike(v, "vanilla.strike")?;
    let (_, _, leverage) = calibrate_leverage_for(market, expiry, grids);
    let payoff = LsvPayoff::Vanilla { option, strike };
    let headline = reprice(&payoff, market, expiry, &leverage, grids.price);
    let greeks = lsv_greeks(&payoff, market, expiry, &leverage, grids, headline);
    Ok(Priced {
        greeks,
        resolved_strike: strike,
        vol: market.vol,
        std_error: None,
    })
}

/// Price a wire single-barrier under the LSV engine. Only continuously-monitored
/// knock-out / knock-in is supported (the LSV ADI engine monitors continuously);
/// a discrete-monitoring single barrier under LSV is rejected as unsupported.
pub fn price_single_barrier_lsv(
    b: &celnet_proto::SingleBarrier,
    market: &WireMarketContext,
    expiry: f64,
    grids: &LsvGrids,
) -> Result<Priced, PriceError> {
    let v = b
        .vanilla
        .as_ref()
        .ok_or(PriceError::MissingField("single_barrier.vanilla"))?;
    let (option, strike) = vanilla_strike(v, "single_barrier.vanilla.strike")?;

    let side = WireBarrierSide::try_from(b.side).map_err(|_| PriceError::UnknownEnum {
        kind: "BarrierSide",
        tag: b.side,
    })?;
    let up = matches!(side, WireBarrierSide::Up);

    let kind =
        celnet_proto::BarrierKind::try_from(b.kind).map_err(|_| PriceError::UnknownEnum {
            kind: "BarrierKind",
            tag: b.kind,
        })?;
    let knock_in = matches!(kind, celnet_proto::BarrierKind::KnockIn);

    // The LSV ADI engine imposes a continuously-monitored Dirichlet wall. A
    // discrete-monitoring single barrier is a different product the LSV ADI
    // engine does not represent — reject rather than misprice.
    if let Ok(celnet_proto::MonitoringStyle::Discrete) =
        celnet_proto::MonitoringStyle::try_from(b.monitoring)
    {
        return Err(PriceError::UnsupportedModel {
            model: "LOCAL_STOCH_VOL",
            product: "single_barrier (DISCRETE monitoring)",
        });
    }
    if b.rebate != 0.0 {
        return Err(PriceError::UnsupportedModel {
            model: "LOCAL_STOCH_VOL",
            product: "single_barrier (non-zero rebate)",
        });
    }
    if !(b.barrier.is_finite() && b.barrier > 0.0) {
        return Err(PriceError::Domain("barrier must be positive and finite"));
    }

    let (_, _, leverage) = calibrate_leverage_for(market, expiry, grids);
    let payoff = LsvPayoff::Knockout {
        option,
        strike,
        barrier: b.barrier,
        up,
        knock_in,
    };
    let headline = reprice(&payoff, market, expiry, &leverage, grids.price);
    let greeks = lsv_greeks(&payoff, market, expiry, &leverage, grids, headline);
    Ok(Priced {
        greeks,
        resolved_strike: strike,
        vol: market.vol,
        std_error: None,
    })
}

/// Price a wire window barrier under the LSV engine. PDE by default; Monte-Carlo
/// (carrying a standard error) when `mc_pairs > 0`.
pub fn price_window_barrier_lsv(
    w: &WireWindowBarrier,
    market: &WireMarketContext,
    expiry: f64,
    grids: &LsvGrids,
) -> Result<Priced, PriceError> {
    let v = w
        .vanilla
        .as_ref()
        .ok_or(PriceError::MissingField("window_barrier.vanilla"))?;
    let (option, strike) = vanilla_strike(v, "window_barrier.vanilla.strike")?;

    let side = WireBarrierSide::try_from(w.side).map_err(|_| PriceError::UnknownEnum {
        kind: "BarrierSide",
        tag: w.side,
    })?;
    let up = matches!(side, WireBarrierSide::Up);

    if !(w.barrier.is_finite() && w.barrier > 0.0) {
        return Err(PriceError::Domain("barrier must be positive and finite"));
    }
    if !(w.window_start.is_finite() && w.window_end.is_finite()) {
        return Err(PriceError::Domain("window bounds must be finite"));
    }
    if !(w.window_start >= 0.0 && w.window_start < w.window_end && w.window_end <= expiry) {
        return Err(PriceError::Domain(
            "window must satisfy 0 <= window_start < window_end <= expiry_years",
        ));
    }

    let spec = ExWindowBarrier {
        option,
        strike,
        barrier: w.barrier,
        up,
        start: w.window_start,
        end: w.window_end,
    };

    let (inputs, var, leverage) = calibrate_leverage_for(market, expiry, grids);

    if w.mc_pairs > 0 {
        // Monte-Carlo engine: price + honest standard error.
        let model = LsvModel::from_leverage((&inputs).into(), var, leverage.clone());
        let steps = if w.mc_steps > 0 {
            w.mc_steps as usize
        } else {
            grids.mc_steps
        };
        let cfg = McConfig {
            pairs: w.mc_pairs as usize,
            steps,
            seed: w.mc_seed,
        };
        let est = model.price_window_barrier_mc(spec, cfg);
        // Greeks from the (deterministic) PDE engine under the same model — the MC
        // headline price is the value, the PDE strip is the consistent risk.
        let payoff = LsvPayoff::Window(spec);
        let greeks = lsv_greeks(&payoff, market, expiry, &leverage, grids, est.price);
        Ok(Priced {
            greeks,
            resolved_strike: strike,
            vol: market.vol,
            std_error: Some(est.std_error),
        })
    } else {
        // ADI PDE engine: exact grid price, no standard error.
        let payoff = LsvPayoff::Window(spec);
        let headline = reprice(&payoff, market, expiry, &leverage, grids.price);
        let greeks = lsv_greeks(&payoff, market, expiry, &leverage, grids, headline);
        Ok(Priced {
            greeks,
            resolved_strike: strike,
            vol: market.vol,
            std_error: None,
        })
    }
}
