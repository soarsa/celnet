//! Celnet FX exotics — first-generation analytic layer (digitals,
//! one-touch / no-touch, double-no-touch, single / double barriers) priced with
//! the closed-form Black-Scholes-Merton / Garman-Kohlhagen barrier & touch
//! formulae, made **smile-consistent** by a Vanna-Volga overlay with
//! survival-probability (first-exit) weighting (work-stream WS-D, gate G3).
//!
//! # Two layers
//!
//! 1. **Analytic core** — exact closed forms for the lognormal (flat-vol)
//!    Garman-Kohlhagen world:
//!    * [`digital`] — European cash-or-nothing / asset-or-nothing binaries
//!      (price + closed-form delta/gamma/vega), each cross-checked against the
//!      `−∂(vanilla)/∂K` strike-derivative limit;
//!    * [`touch`] — one-touch / no-touch (with deferred or at-hit rebate),
//!      double-no-touch and the double-touch it complements, via the
//!      reflection-principle (image) construction;
//!    * [`barrier`] — the eight standard single-barrier knock-in/knock-out
//!      flavours with rebate, and the double-barrier knock-out via the
//!      method-of-images series; in/out parity (`KI + KO = vanilla`) holds by
//!      construction.
//!
//! 2. **Smile overlay** — [`market_hedge_overlay`] adds the FX-market-standard
//!    Vanna-Volga *cost* of the static vega/vanna/volga hedge, **scaled by the
//!    survival (no-touch / first-exit) probability** of the exotic, so a barrier
//!    or touch priced at flat ATM vol is shifted toward the value implied by the
//!    arbitrage-free smile from [`celnet_surface`]
//!    ([`celnet_core::Smile`] / `VolSurface` / `SmileModel`). Touch and
//!    double-no-touch values are clamped to `[0, notional]`.
//!
//! 3. **Numerical engines** — the shared machinery for path-dependent and
//!    second-generation exotics that have no usable closed form:
//!    * [`pde`] — a 1-D Crank-Nicolson finite-difference solver with Rannacher
//!      start-up smoothing on a log-spot grid with barrier-aligned nodes;
//!    * [`mc`] — a Monte-Carlo engine over a [`rng::CounterRng`] counter-based
//!      generator (seeded by `(stream, path, step)` for bit-reproducibility),
//!      [`normal`] inverse-CDF / Box-Muller normals, antithetic variates, a
//!      geometric-Asian control variate, and a Brownian-bridge construction with
//!      the Broadie-Glasserman-Kou continuity correction for discretely-monitored
//!      barriers;
//!    * [`payoff`] — the engine-agnostic path-dependent payoff vocabulary.
//!
//!    Both engines are cross-validated against the analytic layer (and each
//!    other) on the overlap — see the crate-level `cross_validation` tests.
//!
//! 4. **Local-stochastic-volatility (LSV) booking model** — a full second-
//!    generation model that prices on *both* numerical engines from one calibrated
//!    state:
//!    * [`stochvol`] — the mean-reverting square-root **variance backbone** with
//!      the quadratic-exponential discretisation and the full-truncation
//!      log-spot integration;
//!    * [`leverage`] — the **Dupire local-volatility** extraction (implied-total-
//!      variance form) and the tabulated **leverage** surface `L(S,t)`;
//!    * [`particle`] — the **interacting-particle** calibration that fills the
//!      leverage so the model reprices the arbitrage-free vanilla surface
//!      (`L² = σ_Dupire² / E[v | S]`);
//!    * [`adi`] — a 2-D **Hundsdorfer-Verwer ADI** finite-difference solver for the
//!      `(spot, variance)` PDE (mixed-term-explicit, direction-implicit);
//!    * [`lsv`] — the [`LsvModel`] orchestration: calibrate once, then price
//!      European and second-generation **window-barrier** payoffs on the ADI PDE
//!      and on the [`mc`] Monte-Carlo engine, cross-validated against each other
//!      and against the pure-local-vol (`ξ=0`) Dupire limit.
//!
//! 5. **Structured & path-dependent breadth** — the products a flow-options desk
//!    quotes alongside the second-generation barriers, each priced on the engines
//!    above and cross-validated against an independent reference:
//!    * [`quanto`] — correlation-adjusted (quanto-drift) vanilla and
//!      cash-or-nothing digital, with **exact closed forms** (the carry shifted by
//!      `−ρ σ_S σ_Z`) cross-validated against settlement-measure Monte-Carlo;
//!    * [`lookback`] — fixed- and floating-strike lookbacks on the running
//!      extremum, with the Goldman-Sosin-Gatto / Conze-Viswanathan continuous
//!      closed forms cross-validated against a **Brownian-bridge extremum**
//!      Monte-Carlo (the discrete path samples the continuous min/max exactly);
//!    * [`tarf`] — the **Target-Redemption Forward**: a strip of fixings with a
//!      cumulative-gain redemption (knock-out on target), downside gearing, and
//!      explicit [`tarf::RedemptionStyle`] **gap-risk** handling at the breaching
//!      fixing, priced by Monte-Carlo;
//!    * [`accumulator`] — periodic accumulation at a discounted pivot with an
//!      up-and-out knock-out barrier (discrete or Brownian-bridge continuous) and
//!      below-pivot gearing, priced by Monte-Carlo;
//!    * [`pivot`] — the **pivot Target-Redemption Accumulator**: a two-level
//!      (pivot/strike) piecewise-linear fixing strip with cumulative-target
//!      redemption and gap-risk handling, collapsing to [`tarf::Tarf`] exactly when
//!      `pivot == strike` (gated to 1e-12), priced by Monte-Carlo with a forward-
//!      strip control variate and cross-validated against a code-disjoint oracle;
//!    * [`forward_start`] — **forward-start** vanillas (strike reset to
//!      `m·S(t₁)` at a future date, with the exact Rubinstein (1990) FX
//!      dual-carry closed form) and **cliquet / ratchet** strips (the plain
//!      ratchet as the exact sum of forward-start legs, the locally-capped /
//!      -floored variant by Monte-Carlo, cross-validated);
//!    * [`perpetual`] — **perpetual (no-expiry) American** call/put on the
//!      carry seam, with the exact stationary-ODE closed form (free boundary
//!      by value matching + smooth pasting) and fully analytic Greeks,
//!      cross-validated against an independent bisection re-derivation of the
//!      characteristic root and hand-pinned offline references.
//!
//! 6. **Variance & volatility swaps** — the model-free volatility products,
//!    priced directly off an arbitrage-free [`celnet_surface`] smile:
//!    * [`var_swap`] — the **variance-swap fair strike** by log-contract static
//!      replication (the `1/K²`-weighted strip of OTM option forward values),
//!      which recovers `σ²` exactly for a flat smile and lifts above ATM variance
//!      for a positive-butterfly smile;
//!    * [`vol_swap`] — the **volatility-swap fair strike** by the Carr–Lee
//!      convexity (Jensen) adjustment `K_vol = √K_var − Var(v)/(8·K_var^{3/2})`,
//!      strictly below `√K_var` for any non-degenerate smile and widening with
//!      smile convexity.
//!
//! # Method provenance (doc comments only)
//!
//! Reflection-principle / image closed forms for barriers and touches:
//! Reiner-Rubinstein (1991); Rubinstein-Reiner (1991); the unified
//! generalised-BSM presentation in Haug (2007). Double-barrier method of images:
//! Kunitomo-Ikeda (1992); Geman-Yor (1996). Vanna-Volga exotic overlay with
//! survival weighting: Bossens, Rayée, Skantzos & Deelstra (2010); Wystup (2017,
//! *FX Options and Structured Products*), Castagna-Mercurio (2007). All
//! identifiers here are purpose-named and vendor/research-neutral; provenance
//! lives only in documentation.
//!
//! # Determinism
//!
//! Every transcendental routes through [`celnet_core::math`] (the deterministic
//! `rust-lang/libm` software implementation), with **one documented exception**:
//! [`gaussian_pair_from_uniforms`] calls `libm::sin`/`libm::cos` directly (the `celnet_core::math`
//! seam exposes `exp`/`ln`/`sqrt`/`erfc` but not trig). This is still the same
//! `rust-lang/libm` software backend — bit-identical across OS/arch — so the
//! determinism contract holds; the dependency on `libm` is declared solely for
//! that trig pair and is the only transcendental not behind the `celnet_core`
//! seam. No float is compared with `==`; all validation uses
//! [`celnet_core::is_close`] / [`celnet_core::assert_close`]. The analytic layer
//! is allocation-free on the hot path.

#![forbid(unsafe_code)]

pub mod accumulator;
pub mod adi;
pub mod american;
pub mod asian;
pub mod barrier;
pub mod digital;
pub mod forward_start;
pub mod inputs;
pub mod leverage;
pub mod lookback;
pub mod lsv;
pub mod market_hedge_overlay;
pub mod mc;
pub mod multiasset;
pub mod normal;
pub mod particle;
pub mod payoff;
pub mod pde;
pub mod perpetual;
pub mod pivot;
pub mod quanto;
pub mod rng;
pub mod stochvol;
pub mod tarf;
pub mod touch;
pub mod var_swap;
pub mod vol_swap;

pub use accumulator::{
    Accumulator, AccumulatorMcConfig, AccumulatorResult, Monitoring, accumulator_price,
};
pub use adi::{AdiGrid, AdiProblem, WindowSpec, solve as adi_solve, solve_window};
pub use american::{
    AmericanGrid, AmericanOption, ExerciseStyle, LsmConfig, LsmEstimate, american_fd,
    american_fd_greeks, american_lsm,
};
pub use asian::{
    AnalyticAsian, AveragingSchedule, curran_price, geometric_average_price, turnbull_wakeman_price,
};
pub use barrier::{
    BarrierKind, BarrierStyle, DoubleBarrierKnockOut, SingleBarrier, double_knock_out_price,
    single_barrier_price,
};
pub use digital::{DigitalKind, DigitalStyle, digital_greeks, digital_price};
pub use forward_start::{
    Cliquet, CliquetEstimate, CliquetMcConfig, CliquetSchedule, ForwardStart,
    cliquet_price_capped_mc, cliquet_price_plain, cliquet_price_plain_mc, forward_start_price,
};
pub use inputs::ExoticInputs;
pub use leverage::{ImpliedVolSurface, LeverageSurface, LocalVolSurface};
pub use lookback::{
    Lookback, LookbackEstimate, LookbackMcConfig, LookbackStyle, fixed_lookback_price,
    floating_lookback_price, lookback_mc,
};
pub use lsv::{LsvModel, SurfaceTarget, WindowBarrier};
pub use market_hedge_overlay::{
    ExoticSensitivities, MarketCrossPrices, OverlayResult, SurvivalWeight, exotic_sensitivities_fd,
    hedge_smile_cost, hedge_smile_overlay, market_price_of_hedge_smile,
};
pub use mc::{
    BGK_BETA, McConfig, McEstimate, geometric_asian_price, price_asian, price_barrier,
    price_barrier_bgk_shifted,
};
pub use multiasset::{
    BasketEstimate, BasketKind, BasketLeg, BasketMcConfig, BasketSensitivities, BasketSpec,
    CholeskyFactor, CorrelationError, cholesky, price_basket, price_basket_with_sensitivities,
};
pub use normal::{gaussian_pair_from_uniforms, inverse_cdf};
pub use particle::{CalibrationResult, ParticleConfig, calibrate_leverage};
pub use payoff::{ArithmeticAsian, DiscreteBarrier, vanilla_intrinsic};
pub use pde::{PdeGrid, PdeProblem, solve as pde_solve};
pub use perpetual::{
    PerpetualError, PerpetualGreeks, PerpetualInputs, perpetual_exercise_boundary,
    perpetual_greeks, perpetual_price,
};
pub use pivot::{PivotTra, PivotTraMcConfig, PivotTraResult, pivot_tra_price, pivot_tra_price_cv};
pub use quanto::{
    QuantoEstimate, QuantoMcConfig, QuantoParams, quanto_digital_mc, quanto_digital_price,
    quanto_vanilla_mc, quanto_vanilla_price,
};
pub use rng::CounterRng;
pub use stochvol::{
    QE_SWITCH, VarianceParams, log_spot_increment, qe_variance_step, step_uniforms,
};
pub use tarf::{RedemptionStyle, Tarf, TarfMcConfig, TarfResult, tarf_price};
pub use touch::{
    DoubleNoTouch, RebateTiming, TouchSide, double_no_touch_price, double_touch_price,
    no_touch_price, one_touch_price,
};
pub use var_swap::{
    VarSwapContext, VarSwapResult, VarSwapStrip, fair_variance, fair_variance_with,
};
pub use vol_swap::{VolSwapResult, fair_volatility, fair_volatility_with};

use celnet_core::math::{exp, ln, sqrt};
use celnet_types::Carry;

/// Shared lognormal Garman-Kohlhagen scaffolding reused by every analytic
/// exotic in this crate.
///
/// These are the carry / forward / drift quantities the reflection-principle
/// closed forms are written in. Holding them in one place keeps the touch,
/// barrier and digital modules consistent and avoids re-deriving `μ`, `λ` and the
/// power exponents in each formula.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Lognormal {
    /// Annualised Black volatility `σ`.
    pub vol: f64,
    /// Time to expiry in years `T`.
    pub t: f64,
    /// The cost-of-carry producer behind the forward and discounting. For FX this
    /// is [`Carry::FxRates`], so every accessor below is byte-identical to the FX
    /// two-rate form.
    pub carry: Carry,
}

impl Lognormal {
    /// Build the scaffolding from the agnostic [`ExoticInputs`] (the `strike`
    /// field is unused — touches and barriers are parameterised by their own
    /// barrier/strike levels).
    #[inline]
    pub(crate) fn from_inputs(i: &ExoticInputs) -> Self {
        Self {
            vol: i.vol,
            t: i.t,
            carry: i.carry,
        }
    }

    /// Cost of carry `b` (= `r_d − r_f` for FX): the drift of the spot under the
    /// numeraire risk-neutral measure. Read through the agnostic carry seam.
    #[inline]
    pub(crate) fn carry(&self) -> f64 {
        self.carry.carry_rate()
    }

    /// Numeraire discount factor `e^{−r·T}` (= `e^{−r_d·T}` for FX).
    #[inline]
    pub(crate) fn df_dom(&self) -> f64 {
        self.carry.discount_df(self.t)
    }

    /// Yield/foreign discount factor `e^{−q·T}` (= `e^{−r_f·T}` for FX), reading
    /// the stored yield rate verbatim via [`Carry::yield_rate`].
    #[inline]
    pub(crate) fn df_for(&self) -> f64 {
        exp(-self.carry.yield_rate() * self.t)
    }

    /// The numeraire discount rate `r` (= `r_d` for FX).
    #[inline]
    pub(crate) fn discount_rate(&self) -> f64 {
        self.carry.discount_rate()
    }

    /// `σ·√T`, the total Black standard deviation.
    #[inline]
    pub(crate) fn sigma_sqrt_t(&self) -> f64 {
        self.vol * sqrt(self.t)
    }

    /// The reflection-principle drift parameter `μ = b/σ² − ½`.
    ///
    /// In `x = ln(S)` coordinates the log-spot drifts at `(b − ½σ²)`; written per
    /// unit `σ²` (the form the image formulae use) this is `μ`.
    #[inline]
    pub(crate) fn mu(&self) -> f64 {
        self.carry() / (self.vol * self.vol) - 0.5
    }

    /// `√(μ² + 2·r/σ²)`, the discounted-hit exponent (the `λ` of the
    /// Reiner-Rubinstein touch formulae).
    #[inline]
    pub(crate) fn lambda(&self) -> f64 {
        let m = self.mu();
        sqrt(m * m + 2.0 * self.discount_rate() / (self.vol * self.vol))
    }
}

/// Natural log helper that routes through the deterministic core math, used by
/// the closed forms' log-moneyness terms.
#[inline]
pub(crate) fn dlog(x: f64) -> f64 {
    ln(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::VanillaInputs;

    #[test]
    fn lognormal_carry_and_discounts() {
        let i: ExoticInputs = VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01).into();
        let l = Lognormal::from_inputs(&i);
        celnet_core::assert_close!(l.carry(), 0.02, 1e-15, 1e-15);
        celnet_core::assert_close!(l.df_dom(), exp(-0.03), 1e-15, 1e-15);
        celnet_core::assert_close!(l.df_for(), exp(-0.01), 1e-15, 1e-15);
        celnet_core::assert_close!(l.sigma_sqrt_t(), 0.10, 1e-15, 1e-15);
    }

    #[test]
    fn lambda_is_non_negative() {
        let i: ExoticInputs = VanillaInputs::new(100.0, 100.0, 0.2, 0.5, 0.05, 0.02).into();
        let l = Lognormal::from_inputs(&i);
        assert!(l.lambda() >= 0.0);
    }

    /// End-to-end smile-consistency check: build a real arbitrage-free smile in
    /// `celnet-surface` (a calibrated [`celnet_surface::MarketHedgeSmile`]) with a
    /// positive butterfly (convex wings), then apply the survival-weighted
    /// Vanna-Volga overlay to a flat-vol double-no-touch. A long-volga product
    /// (the DNT, which is long convexity — it benefits from the corridor staying
    /// quiet) must be shifted in the documented direction by a positive-butterfly
    /// smile, and the shift must be damped by the survival probability.
    #[test]
    fn overlay_consumes_surface_smile_and_shifts_in_documented_direction() {
        use crate::market_hedge_overlay::{
            SurvivalWeight, exotic_sensitivities_fd, hedge_smile_overlay,
            market_price_of_hedge_smile,
        };
        use crate::touch::{DoubleNoTouch, double_no_touch_price};
        use celnet_surface::MarketHedgeSmile;

        // Market: EURUSD-like 1Y, ATM 10 vol.
        let i = VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01);
        let f = i.forward();

        // Build a convex (positive-butterfly), symmetric smile in celnet-surface:
        // wings at 11.5 vol, ATM at 10 vol, strikes log-symmetric around F.
        let (kp, kc) = (f / 1.10, f * 1.10);
        let smile = MarketHedgeSmile::new([kp, f, kc], [0.115, 0.10, 0.115], f, i.t);

        // Market price of vanna/volga read off the surface smile at the wings.
        let market = market_price_of_hedge_smile(&smile, &(&i).into(), kp, kc);
        assert!(
            market.volga_price > 0.0,
            "convex smile ⇒ positive volga price"
        );

        // A double-no-touch corridor around spot; its flat-vol price.
        let dnt = DoubleNoTouch::new(1.18, 1.43, 1.0);
        let flat = double_no_touch_price(&(&i).into(), dnt);

        // Exotic vanna/volga by finite differences of the flat-vol closed form.
        let x = exotic_sensitivities_fd(
            |spot, vol| {
                let bumped = VanillaInputs { spot, vol, ..i };
                double_no_touch_price(&(&bumped).into(), dnt)
            },
            i.spot,
            i.vol,
        );

        // Survival weight = the DNT's own survival (no-touch) probability: the
        // overlay correction is damped by the probability the option is still
        // alive (first-exit weighting). The flat DNT price is e^{−r_d T}·p.
        let survival = SurvivalWeight::new(flat / (i.df_dom() * dnt.rebate));
        let full = hedge_smile_overlay(flat, x, market, SurvivalWeight::EUROPEAN);
        let weighted = hedge_smile_overlay(flat, x, market, survival);

        // The DNT is long volga (positive ∂²V/∂σ²) — a positive-butterfly smile
        // therefore raises its value. The documented direction is an upward shift.
        assert!(x.volga > 0.0, "a DNT is long volga, got {}", x.volga);
        assert!(
            full.smile_price > flat,
            "positive-BF smile must shift the long-volga DNT up: {} > {}",
            full.smile_price,
            flat
        );
        // Survival weighting damps (does not reverse) the correction.
        assert!(
            weighted.hedge_smile_cost.abs() <= full.hedge_smile_cost.abs() + 1e-12
                && weighted.hedge_smile_cost.signum() == full.hedge_smile_cost.signum()
        );
        // Smile-consistent price stays a valid DNT value in [0, notional].
        assert!(weighted.smile_price >= 0.0 && weighted.smile_price <= dnt.rebate);
    }
}

/// Three-way cross-validation: the analytic S1 closed form, the PDE engine and
/// the Monte-Carlo engine must agree on the prices they all overlap on, within
/// documented tolerance. This is the headline correctness guarantee for the
/// numerical layer — each engine is an independent implementation of the same
/// Garman-Kohlhagen world, so their agreement is strong evidence that all three
/// are right.
#[cfg(test)]
mod cross_validation {
    use crate::barrier::{BarrierKind, BarrierStyle, SingleBarrier, single_barrier_price};
    use crate::mc::{McConfig, price_barrier};
    use crate::payoff::DiscreteBarrier;
    use crate::pde::{PdeGrid, PdeProblem, solve as pde_solve};
    use celnet_types::{OptionType, VanillaInputs};
    use celnet_vanilla::price as vanilla_price;

    fn base() -> VanillaInputs {
        VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.02)
    }

    /// Vanilla: analytic == PDE == MC (MC of a 1-step "barrier" far from spot
    /// degenerates to a vanilla). The triangle closes for the simplest payoff.
    #[test]
    fn vanilla_triangle() {
        let i = base();
        let analytic = vanilla_price(OptionType::Call, &i);

        let pde = pde_solve(
            &(&i).into(),
            PdeProblem {
                option: OptionType::Call,
                strike: 100.0,
                knock_out: None,
            },
            PdeGrid {
                space_steps: 1000,
                time_steps: 600,
                ..PdeGrid::default()
            },
        );

        // A knock-out with a barrier so far away it never binds ≈ vanilla.
        let mc = price_barrier(
            &(&i).into(),
            DiscreteBarrier {
                option: OptionType::Call,
                strike: 100.0,
                barrier: 1.0e6,
                up: true,
                knock_in: false,
            },
            McConfig {
                pairs: 100_000,
                steps: 50,
                seed: 0x1111,
            },
        );

        assert!(
            (pde - analytic).abs() < 5e-3,
            "PDE {pde} vs analytic {analytic}"
        );
        assert!(
            (mc.price - analytic).abs() < 3.0 * mc.std_error + 5e-3,
            "MC {} vs analytic {analytic} (se {})",
            mc.price,
            mc.std_error
        );
    }

    /// Up-and-out call: analytic ≈ PDE ≈ MC. The three independent engines
    /// (closed form, finite difference, simulation) must agree on the same
    /// continuously-monitored barrier price.
    #[test]
    fn up_and_out_call_triangle() {
        let i = base();
        let (k, h) = (100.0, 130.0);

        let analytic = single_barrier_price(
            &(&i).into(),
            SingleBarrier {
                kind: BarrierKind {
                    up: true,
                    style: BarrierStyle::KnockOut,
                    option: OptionType::Call,
                },
                strike: k,
                barrier: h,
                rebate: 0.0,
            },
        );

        let pde = pde_solve(
            &(&i).into(),
            PdeProblem {
                option: OptionType::Call,
                strike: k,
                knock_out: Some((h, true)),
            },
            PdeGrid {
                space_steps: 1200,
                time_steps: 800,
                ..PdeGrid::default()
            },
        );

        let mc = price_barrier(
            &(&i).into(),
            DiscreteBarrier {
                option: OptionType::Call,
                strike: k,
                barrier: h,
                up: true,
                knock_in: false,
            },
            McConfig {
                pairs: 150_000,
                steps: 120,
                seed: 0x2222,
            },
        );

        // PDE is the tight reference (deterministic, ~1e-3); MC carries O(1/√N).
        assert!(
            (pde - analytic).abs() < 5e-3,
            "PDE {pde} vs analytic {analytic}"
        );
        let mc_tol = 3.0 * mc.std_error + 3e-2;
        assert!(
            (mc.price - analytic).abs() < mc_tol,
            "MC {} vs analytic {analytic} (se {}, tol {mc_tol})",
            mc.price,
            mc.std_error
        );
        assert!(
            (mc.price - pde).abs() < mc_tol,
            "MC {} vs PDE {pde}",
            mc.price
        );
    }
}
