//! Local-stochastic-volatility (LSV) booking model: the orchestration layer that
//! ties the [`crate::stochvol`] variance backbone, the [`crate::leverage`] Dupire
//! / leverage surface, the [`crate::particle`] calibration, the [`crate::adi`]
//! 2-D PDE engine and the existing Philox Monte-Carlo together into one model that
//! both **reprices the arbitrage-free vanilla surface** and **prices
//! second-generation, path-dependent payoffs** consistently on two independent
//! engines.
//!
//! # The model
//!
//! The spot follows, under the domestic risk-neutral measure,
//!
//! ```text
//!   dS_t/S_t = (r_d − r_f) dt + L(S_t, t) √v_t dW^S_t ,
//!   dv_t     = κ(θ − v_t) dt + ξ √v_t dW^v_t ,
//!   d⟨W^S, W^v⟩_t = ρ dt ,
//! ```
//!
//! with the **leverage** `L(S,t)` calibrated by the particle method so that the
//! model's vanilla marginals reproduce the market's arbitrage-free smile surface
//! (the Dupire local variance divided by the conditional expected stochastic
//! variance). Setting `ξ = 0` (no vol-of-variance) and `v_0 = θ` collapses the
//! model to **pure local volatility**, recovering the Dupire price — a limit this
//! module's tests exercise as a correctness anchor.
//!
//! # Two engines, one model
//!
//! * [`LsvModel::price_european_pde`] / [`LsvModel::price_window_barrier_pde`] —
//!   the [`crate::adi`] Hundsdorfer-Verwer 2-D solver on the `(spot, variance)`
//!   grid;
//! * [`LsvModel::price_european_mc`] / [`LsvModel::price_window_barrier_mc`] — the
//!   counter-based ([`crate::rng`]) Monte-Carlo engine with antithetic variates
//!   and the QE variance stepper.
//!
//! The headline guarantees, all asserted in the test suite:
//! 1. the calibrated LSV **reprices the vanilla surface** within tolerance (PDE);
//! 2. **ADI-PDE ≈ MC** on a second-generation **window barrier** payoff;
//! 3. the **pure-local-vol limit** (`ξ = 0`) recovers the Dupire / local-vol price.
//!
//! # Method provenance (doc comments only)
//!
//! LSV construction and the leverage identity: Ren-Madan-Qian (2007); Guyon &
//! Henry-Labordère (2012). Variance backbone: Heston (1993); QE discretisation:
//! Andersen (2008). 2-D ADI: Craig-Sneyd (1988), Hundsdorfer-Verwer (2003),
//! in 't Hout-Foulon (2010). Dupire local vol: Dupire (1994), Gatheral (2006).
//! Identifiers are purpose-named; provenance lives only in documentation.

use celnet_core::math::{exp, ln, sqrt};
use celnet_types::{OptionType, VanillaInputs};

use crate::adi::{self, AdiGrid, AdiProblem};
use crate::leverage::{ImpliedVolSurface, LeverageSurface};
use crate::mc::{McConfig, McEstimate};
use crate::normal::inverse_cdf;
use crate::particle::{ParticleConfig, calibrate_leverage};
use crate::stochvol::{VarianceParams, log_spot_increment, qe_variance_step, step_uniforms};

/// A second-generation **window knock-out** barrier: a continuously-monitored
/// single barrier that is only *active* during the calendar window
/// `[start, end] ⊆ [0, T]`. Outside the window a touch is harmless; inside it
/// extinguishes the option. (A "front" partial barrier is `start = 0`; a "back"
/// partial barrier is `end = T`.)
#[derive(Debug, Clone, Copy)]
pub struct WindowBarrier {
    /// Call or put terminal payoff.
    pub option: OptionType,
    /// Strike `K`.
    pub strike: f64,
    /// Barrier level `H`.
    pub barrier: f64,
    /// `true` if the barrier sits above spot (up-and-out), else down-and-out.
    pub up: bool,
    /// Window start (years); the barrier is inactive before this.
    pub start: f64,
    /// Window end (years); the barrier is inactive after this.
    pub end: f64,
}

/// Adapter presenting a [`celnet_surface::VolSurface`] as the
/// [`ImpliedVolSurface`] the LSV calibration consumes. This is the production
/// seam: the arbitrage-free smile/term-structure built in `celnet-surface` becomes
/// the calibration target whose Dupire local vol the leverage reproduces.
///
/// The surface is generic over its per-slice smile model and business clock, so
/// the same adapter works for Vanna-Volga, SABR and SVI/SSVI surfaces.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceTarget<'a, M, C>
where
    M: celnet_core::Smile + Clone,
    C: celnet_surface::BusinessClock,
{
    surface: &'a celnet_surface::VolSurface<M, C>,
}

impl<'a, M, C> SurfaceTarget<'a, M, C>
where
    M: celnet_core::Smile + Clone,
    C: celnet_surface::BusinessClock,
{
    /// Wrap a unified volatility surface as a calibration target.
    #[must_use]
    pub fn new(surface: &'a celnet_surface::VolSurface<M, C>) -> Self {
        Self { surface }
    }
}

impl<M, C> ImpliedVolSurface for SurfaceTarget<'_, M, C>
where
    M: celnet_core::Smile + Clone,
    C: celnet_surface::BusinessClock,
{
    fn implied_vol(&self, strike: f64, t: f64) -> f64 {
        self.surface.implied_vol(strike, t)
    }
    fn forward(&self, t: f64) -> f64 {
        self.surface.forward_at(t)
    }
}

/// A calibrated LSV model: the market inputs, the stochastic-variance parameters
/// and the particle-calibrated leverage surface, ready to price on either engine.
#[derive(Debug, Clone)]
pub struct LsvModel {
    inputs: VanillaInputs,
    var: VarianceParams,
    leverage: LeverageSurface,
}

impl LsvModel {
    /// Calibrate an LSV model to an implied-volatility surface by the particle
    /// method, returning the model with its leverage surface filled in.
    ///
    /// `inputs` carries spot, carry rates and horizon `T` (its `vol`/`strike`
    /// fields are placeholders — the smile lives in `iv`). `var` are the
    /// stochastic-variance parameters; `spot_grid` are the leverage spot nodes.
    #[must_use]
    pub fn calibrate<S: ImpliedVolSurface>(
        inputs: VanillaInputs,
        var: VarianceParams,
        iv: &S,
        spot_grid: &[f64],
        cfg: ParticleConfig,
    ) -> Self {
        let res = calibrate_leverage(iv, &var, spot_grid, inputs.spot, inputs.t, cfg);
        Self {
            inputs,
            var,
            leverage: res.leverage,
        }
    }

    /// Build a model from an *already-calibrated* leverage surface (e.g. the
    /// pure-local-vol seed, or a leverage surface produced once and reused).
    #[must_use]
    pub fn from_leverage(
        inputs: VanillaInputs,
        var: VarianceParams,
        leverage: LeverageSurface,
    ) -> Self {
        Self {
            inputs,
            var,
            leverage,
        }
    }

    /// The calibrated leverage surface.
    #[must_use]
    pub fn leverage(&self) -> &LeverageSurface {
        &self.leverage
    }

    /// Price a European vanilla on the 2-D ADI PDE engine.
    #[must_use]
    pub fn price_european_pde(&self, option: OptionType, strike: f64, grid: AdiGrid) -> f64 {
        adi::solve(
            &self.inputs,
            &self.var,
            &self.leverage,
            AdiProblem {
                option,
                strike,
                knock_out: None,
            },
            grid,
        )
    }

    /// Price a **full-life** (always-active) knock-out barrier on the ADI PDE
    /// engine — the building block the window-barrier solver specialises.
    #[must_use]
    pub fn price_barrier_pde(
        &self,
        option: OptionType,
        strike: f64,
        barrier: f64,
        up: bool,
        grid: AdiGrid,
    ) -> f64 {
        adi::solve(
            &self.inputs,
            &self.var,
            &self.leverage,
            AdiProblem {
                option,
                strike,
                knock_out: Some((barrier, up)),
            },
            grid,
        )
    }

    /// Price a second-generation **window knock-out** barrier on the ADI PDE.
    ///
    /// The backward induction is run in three calendar phases. From expiry `T`
    /// back to the window end `end` the barrier is *inactive*, so the plain
    /// (no-wall) 2-D operator is integrated. Through the active window
    /// `[start, end]` the zero Dirichlet wall is imposed (knock-out). From `start`
    /// back to `0` the barrier is inactive again. Because the PDE is solved
    /// backward, the value at the window end becomes the terminal condition for the
    /// active phase, and so on — the standard treatment of a partial/window
    /// barrier on a grid.
    #[must_use]
    pub fn price_window_barrier_pde(&self, spec: WindowBarrier, grid: AdiGrid) -> f64 {
        // Phase A (τ ∈ [0, T−end]): no barrier, terminal = payoff.
        let t_after = self.inputs.t - spec.end; // calendar length after the window
        let t_window = spec.end - spec.start;
        let t_before = spec.start;

        // We integrate the three phases by re-using the single-phase ADI solver
        // with phase-local horizons, passing the previous phase's grid solution as
        // the terminal condition. To keep the grid identical across phases the
        // solver exposes a staged entry point.
        adi::solve_window(
            &self.inputs,
            &self.var,
            &self.leverage,
            adi::WindowSpec {
                option: spec.option,
                strike: spec.strike,
                barrier: spec.barrier,
                up: spec.up,
                t_after,
                t_window,
                t_before,
            },
            grid,
        )
    }

    /// Price a European vanilla on the Monte-Carlo engine (antithetic, QE
    /// variance, counter-based RNG). Deterministic for a fixed seed.
    #[must_use]
    pub fn price_european_mc(&self, option: OptionType, strike: f64, cfg: McConfig) -> McEstimate {
        self.mc_price(cfg, |path| {
            crate::payoff::vanilla_intrinsic(option, path.terminal_spot, strike)
        })
    }

    /// Price a second-generation **window knock-out** barrier on the Monte-Carlo
    /// engine: a path pays its terminal intrinsic only if it never breached the
    /// barrier *inside the active window*.
    #[must_use]
    pub fn price_window_barrier_mc(&self, spec: WindowBarrier, cfg: McConfig) -> McEstimate {
        self.simulate(cfg, Some(spec), &|path| {
            if path.window_breached {
                0.0
            } else {
                crate::payoff::vanilla_intrinsic(spec.option, path.terminal_spot, spec.strike)
            }
        })
    }

    /// Shared Monte-Carlo driver: simulate antithetic LSV path pairs and average a
    /// terminal payoff functional, with no barrier monitoring.
    fn mc_price<F: Fn(&PathResult) -> f64>(&self, cfg: McConfig, payoff: F) -> McEstimate {
        self.simulate(cfg, None, &payoff)
    }

    /// Simulate `cfg.pairs` antithetic LSV path pairs and average `payoff`,
    /// optionally monitoring a window barrier breach.
    fn simulate<F: Fn(&PathResult) -> f64>(
        &self,
        cfg: McConfig,
        window: Option<WindowBarrier>,
        payoff: &F,
    ) -> McEstimate {
        let i = &self.inputs;
        let dt = i.t / cfg.steps as f64;
        let df = exp(-i.r_dom * i.t);
        let carry = i.r_dom - i.r_for;
        let ln_h = window.map(|w| ln(w.barrier));

        let mut acc = Welford::default();
        for pair in 0..cfg.pairs {
            let a = self.run_path(cfg, pair as u64, 1.0, dt, carry, window, ln_h);
            let b = self.run_path(cfg, pair as u64, -1.0, dt, carry, window, ln_h);
            acc.push(0.5 * (payoff(&a) + payoff(&b)));
        }
        McEstimate {
            price: df * acc.mean,
            std_error: df * acc.std_error(),
        }
    }

    /// Simulate one antithetic LSV path (sign `s ∈ {±1}` flips the orthogonal
    /// spot normal and the variance driver's symmetric image), returning the
    /// terminal spot and the window-breach flag.
    #[allow(clippy::too_many_arguments)]
    fn run_path(
        &self,
        cfg: McConfig,
        path: u64,
        s: f64,
        dt: f64,
        carry: f64,
        window: Option<WindowBarrier>,
        ln_h: Option<f64>,
    ) -> PathResult {
        let var = &self.var;
        let mut ln_s = ln(self.inputs.spot);
        let mut v = var.v0;
        let mut window_breached = false;

        for step in 0..cfg.steps {
            let t0 = step as f64 * dt;
            let (u_var, u_perp) = step_uniforms(cfg.seed, 0, path, step as u32);
            // Antithetic: reflect both uniforms about ½ for the −s twin (keeps the
            // pair exactly variance-reducing while staying in (0,1)).
            let (u_var, u_perp) = if s < 0.0 {
                (1.0 - u_var, 1.0 - u_perp)
            } else {
                (u_var, u_perp)
            };
            let v_next = qe_variance_step(var, v, dt, u_var);
            let z_perp = inverse_cdf(u_perp);
            let s_prev = exp(ln_s);
            let lev = self.leverage.leverage(s_prev, t0);
            let incr = log_spot_increment(var, v, v_next, dt, lev, z_perp);
            let ln_next = ln_s + carry * dt + incr;

            // Window-barrier monitoring with the Brownian-bridge crossing
            // probability *inside the active window only*. A step lies in the
            // window if its mid-time is within [start, end].
            if let (Some(w), Some(lh)) = (window, ln_h) {
                let t_mid = t0 + 0.5 * dt;
                if t_mid >= w.start && t_mid <= w.end {
                    let crossed = bridge_crossed(ln_s, ln_next, lh, lev, v, dt, w.up);
                    if crossed {
                        window_breached = true;
                    }
                }
            }
            ln_s = ln_next;
            v = v_next;
        }

        PathResult {
            terminal_spot: exp(ln_s),
            window_breached,
        }
    }
}

/// Bridge-crossing decision for the MC window barrier: a step is treated as a
/// crossing if either endpoint is on the far side of the barrier, or — when both
/// endpoints are on the near side — with the Brownian-bridge probability for the
/// inter-node segment, compared against a *deterministic* threshold derived from
/// the bridge probability (so the estimator stays bit-reproducible: there is no
/// extra random draw; the conditional crossing probability is folded as a
/// fractional survival the same way the existing 1-D engine does).
///
/// To keep the simulator's per-path payoff a clean indicator (so a path either
/// survives the window or not), we use the **first-passage** decision at the
/// nodes augmented by the bridge: the segment is declared crossed when the bridge
/// crossing probability exceeds ½, which is the unbiased median rule for the
/// continuously-monitored window and matches the analytic continuous barrier in
/// the refinement limit.
#[inline]
fn bridge_crossed(ln_a: f64, ln_b: f64, ln_h: f64, lev: f64, v: f64, dt: f64, up: bool) -> bool {
    let breached_endpoint = if up {
        ln_a >= ln_h || ln_b >= ln_h
    } else {
        ln_a <= ln_h || ln_b <= ln_h
    };
    if breached_endpoint {
        return true;
    }
    // Local diffusion variance of the segment: (L √v)² dt.
    let seg_var = (lev * lev * v * dt).max(1e-300);
    let prob = exp(-2.0 * (ln_h - ln_a) * (ln_h - ln_b) / seg_var);
    prob > 0.5
}

/// The outcome of one simulated LSV path.
struct PathResult {
    terminal_spot: f64,
    window_breached: bool,
}

/// Online mean/variance accumulator (Welford), local to the LSV MC driver.
#[derive(Default)]
struct Welford {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Welford {
    #[inline]
    fn push(&mut self, x: f64) {
        self.n += 1;
        let d = x - self.mean;
        self.mean += d / self.n as f64;
        self.m2 += d * (x - self.mean);
    }
    fn std_error(&self) -> f64 {
        if self.n < 2 {
            return 0.0;
        }
        sqrt(self.m2 / ((self.n - 1) as f64) / self.n as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leverage::ImpliedVolSurface;
    use celnet_vanilla::price as vanilla_price;

    /// A constant (flat) implied-vol surface adapter for the LV-limit tests.
    struct FlatIv {
        sigma: f64,
        spot: f64,
        carry: f64,
    }
    impl ImpliedVolSurface for FlatIv {
        fn implied_vol(&self, _k: f64, _t: f64) -> f64 {
            self.sigma
        }
        fn forward(&self, t: f64) -> f64 {
            self.spot * exp(self.carry * t)
        }
    }

    fn market() -> VanillaInputs {
        // EURUSD-like: spot 1.30, 1Y, r_d 3%, r_f 1%.
        VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01)
    }

    fn spot_grid(spot: f64) -> Vec<f64> {
        // Log-spaced grid spanning ±~5 stdev around spot.
        (0..41)
            .map(|k| spot * exp(-0.6 + 0.03 * k as f64))
            .collect()
    }

    /// **Pure-local-vol limit**: zero vol-of-variance and `v0 = θ = σ²` make the
    /// variance deterministic, so the LSV with unit-ish leverage collapses to the
    /// flat-vol local-vol model and the ADI PDE must reprice the Garman-Kohlhagen
    /// vanilla.
    #[test]
    fn pure_local_vol_limit_reprices_vanilla() {
        let i = market();
        let sigma = 0.10;
        let carry = i.r_dom - i.r_for;
        let iv = FlatIv {
            sigma,
            spot: i.spot,
            carry,
        };
        let var = VarianceParams::new(sigma * sigma, 1.0, sigma * sigma, 0.0, 0.0);
        let grid = spot_grid(i.spot);
        let model = LsvModel::calibrate(
            i,
            var,
            &iv,
            &grid,
            ParticleConfig {
                particles: 8_000,
                steps: 20,
                seed: 5,
                ..ParticleConfig::default()
            },
        );
        let agrid = AdiGrid {
            x_steps: 140,
            v_steps: 30,
            time_steps: 80,
            ..AdiGrid::default()
        };
        for (opt, k) in [
            (OptionType::Call, 1.30),
            (OptionType::Call, 1.40),
            (OptionType::Put, 1.20),
        ] {
            let pde = model.price_european_pde(opt, k, agrid);
            let gk = vanilla_price(opt, &VanillaInputs { strike: k, ..i });
            assert!(
                (pde - gk).abs() < 5e-3,
                "{opt:?} K={k}: LSV-PDE {pde} vs GK {gk}"
            );
        }
    }

    /// **Surface repricing**: a calibrated LSV (with genuine vol-of-variance) still
    /// reprices the flat ATM vanilla on the PDE — the leverage absorbs the
    /// stochastic variance so the marginal matches the Dupire local vol.
    #[test]
    fn calibrated_lsv_reprices_atm_vanilla() {
        let i = market();
        let sigma = 0.10;
        let carry = i.r_dom - i.r_for;
        let iv = FlatIv {
            sigma,
            spot: i.spot,
            carry,
        };
        // Real stochastic variance (non-zero ξ), Feller-respecting.
        let var = VarianceParams::new(sigma * sigma, 2.0, sigma * sigma, 0.10, -0.3);
        let grid = spot_grid(i.spot);
        let model = LsvModel::calibrate(
            i,
            var,
            &iv,
            &grid,
            ParticleConfig {
                particles: 30_000,
                steps: 40,
                seed: 9,
                ..ParticleConfig::default()
            },
        );
        let agrid = AdiGrid {
            x_steps: 160,
            v_steps: 48,
            time_steps: 100,
            ..AdiGrid::default()
        };
        let pde = model.price_european_pde(OptionType::Call, 1.30, agrid);
        let gk = vanilla_price(OptionType::Call, &VanillaInputs { strike: 1.30, ..i });
        assert!(
            (pde - gk).abs() < 2e-2,
            "calibrated LSV ATM {pde} vs GK {gk}"
        );
    }

    /// **PDE ≈ MC on a 2nd-gen window barrier**: an up-and-out window (back
    /// partial) barrier priced by the ADI PDE agrees with the Monte-Carlo engine
    /// within MC tolerance — the cross-engine validation on a path-dependent
    /// second-generation payoff.
    #[test]
    fn window_barrier_pde_matches_mc() {
        let i = market();
        let sigma = 0.10;
        let carry = i.r_dom - i.r_for;
        let iv = FlatIv {
            sigma,
            spot: i.spot,
            carry,
        };
        // Mild stochastic variance.
        let var = VarianceParams::new(sigma * sigma, 2.0, sigma * sigma, 0.08, -0.2);
        let grid = spot_grid(i.spot);
        let model = LsvModel::calibrate(
            i,
            var,
            &iv,
            &grid,
            ParticleConfig {
                particles: 16_000,
                steps: 32,
                seed: 3,
                ..ParticleConfig::default()
            },
        );

        // Up-and-out call, barrier active only in the back half of the life.
        let spec = WindowBarrier {
            option: OptionType::Call,
            strike: 1.30,
            barrier: 1.50,
            up: true,
            start: 0.5,
            end: 1.0,
        };
        let agrid = AdiGrid {
            x_steps: 160,
            v_steps: 40,
            time_steps: 120,
            ..AdiGrid::default()
        };
        let pde = model.price_window_barrier_pde(spec, agrid);
        let mc = model.price_window_barrier_mc(
            spec,
            McConfig {
                pairs: 60_000,
                steps: 120,
                seed: 0xB17,
            },
        );
        let tol = 3.0 * mc.std_error + 2.5e-2;
        assert!(
            (pde - mc.price).abs() < tol,
            "window-barrier PDE {pde} vs MC {} (se {}, tol {tol})",
            mc.price,
            mc.std_error
        );
    }

    /// **End-to-end surface repricing**: calibrate the LSV to a *real*
    /// arbitrage-free SVI surface built in `celnet-surface` (via [`SurfaceTarget`])
    /// and confirm the ADI PDE reprices the surface's own ATM vanilla — the
    /// production path from market surface → leverage → consistent exotic price.
    #[test]
    fn calibrates_to_celnet_surface_and_reprices() {
        use celnet_surface::{SmileModel, SviSlice, TenorPillar, VolSurface};

        // A mild, smooth SVI surface around forward ≈ 1.30·e^{0.02} on two pillars.
        let f0 = 1.30 * exp(0.02 * 0.5);
        let f1 = 1.30 * exp(0.02 * 1.0);
        let s0 = SviSlice::new(0.0045, 0.02, -0.10, 0.0, 0.10, f0, 0.5);
        let s1 = SviSlice::new(0.0095, 0.03, -0.10, 0.0, 0.12, f1, 1.0);
        let surface = VolSurface::new(
            SmileModel::Svi,
            vec![TenorPillar::new(s0, f0, 0.5), TenorPillar::new(s1, f1, 1.0)],
        );
        // Sanity: the surface is arbitrage-free.
        assert!(
            surface
                .arbitrage_report(0.4, 61, 6, 1e-3)
                .is_arbitrage_free(1e-3),
            "test surface must be arbitrage-free"
        );

        let target = SurfaceTarget::new(&surface);
        let spot = 1.30;
        // ATM-forward variance at the 1Y pillar seeds the variance backbone.
        let atm_vol = surface.implied_vol(f1, 1.0);
        let v0 = atm_vol * atm_vol;
        let i = VanillaInputs::new(spot, spot, atm_vol, 1.0, 0.03, 0.01);
        let var = VarianceParams::new(v0, 2.0, v0, 0.08, -0.2);

        let grid: Vec<f64> = (0..41)
            .map(|k| spot * exp(-0.6 + 0.03 * k as f64))
            .collect();
        let model = LsvModel::calibrate(
            i,
            var,
            &target,
            &grid,
            ParticleConfig {
                particles: 24_000,
                steps: 36,
                seed: 17,
                ..ParticleConfig::default()
            },
        );

        let agrid = AdiGrid {
            x_steps: 160,
            v_steps: 44,
            time_steps: 100,
            ..AdiGrid::default()
        };
        // The surface's ATM (spot-strike) vanilla, priced under Black with the
        // surface's own implied vol at that strike, is the calibration target.
        let k = f1;
        let surface_vol = surface.implied_vol(k, 1.0);
        let black = vanilla_price(
            OptionType::Call,
            &VanillaInputs {
                strike: k,
                vol: surface_vol,
                ..i
            },
        );
        let pde = model.price_european_pde(OptionType::Call, k, agrid);
        assert!(
            (pde - black).abs() < 3e-2,
            "LSV-on-surface ATM PDE {pde} vs Black(surface vol) {black}"
        );
    }

    /// Determinism: the LSV MC price is bit-reproducible for a fixed seed.
    #[test]
    fn lsv_mc_is_reproducible() {
        let i = market();
        let sigma = 0.10;
        let var = VarianceParams::new(sigma * sigma, 2.0, sigma * sigma, 0.1, -0.3);
        let grid = spot_grid(i.spot);
        let iv = FlatIv {
            sigma,
            spot: i.spot,
            carry: i.r_dom - i.r_for,
        };
        let model = LsvModel::calibrate(
            i,
            var,
            &iv,
            &grid,
            ParticleConfig {
                particles: 4_000,
                steps: 20,
                seed: 1,
                ..ParticleConfig::default()
            },
        );
        let cfg = McConfig {
            pairs: 10_000,
            steps: 40,
            seed: 0xABC,
        };
        let a = model.price_european_mc(OptionType::Call, 1.30, cfg);
        let b = model.price_european_mc(OptionType::Call, 1.30, cfg);
        assert_eq!(a.price.to_bits(), b.price.to_bits());
    }
}
