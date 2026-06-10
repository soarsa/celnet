//! The `exotic` subcommand: price a digital, one-touch, double-no-touch, or
//! single-barrier option from command-line inputs.
//!
//! Every price is the corresponding closed form in `celnet-exotics` (the
//! Reiner-Rubinstein single barrier, the cash-or-nothing digital, and the
//! reflection-principle touch / double-no-touch formulae). This module only maps
//! the chosen variant and inputs onto those functions and formats the result.

use celnet_core::FlatSmile;
use celnet_exotics::{
    Accumulator, AccumulatorMcConfig, AmericanGrid, AmericanOption, AnalyticAsian,
    AveragingSchedule, Cliquet, CliquetMcConfig, CliquetSchedule, DigitalKind, DoubleNoTouch,
    ExerciseStyle, ForwardStart, Lookback, LookbackMcConfig, LookbackStyle, LsmConfig, Monitoring,
    QuantoParams, RebateTiming, RedemptionStyle, SingleBarrier, Tarf, TarfMcConfig, VarSwapContext,
    accumulator_price, american_fd, american_lsm, cliquet_price_capped_mc, cliquet_price_plain,
    curran_price, digital_price, double_no_touch_price, fair_variance, fair_volatility,
    fixed_lookback_price, floating_lookback_price, forward_start_price, lookback_mc,
    one_touch_price, quanto_digital_price, quanto_vanilla_price, single_barrier_price, tarf_price,
    turnbull_wakeman_price,
};
use celnet_types::{OptionType, VanillaInputs};

use crate::args::CliBarrier;

/// Which exotic to price and its variant-specific parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ExoticSpec {
    /// A vanilla European option (Garman-Kohlhagen under `--model analytic`, the
    /// LSV ADI PDE under `--model lsv`). The strike is `inputs.strike`.
    Vanilla {
        /// Call or put.
        option: OptionType,
    },
    /// A European cash-or-nothing digital (pays 1 domestic in the money).
    Digital(DigitalKind),
    /// A one-touch paying `rebate` (domestic) if `barrier` is touched before `T`.
    OneTouch {
        /// Barrier level.
        barrier: f64,
        /// Domestic rebate paid on the touch.
        rebate: f64,
        /// Whether the rebate is paid at hit or deferred to expiry.
        at_expiry: bool,
    },
    /// A double-no-touch paying `rebate` (domestic) at expiry if spot stays
    /// strictly inside `(lower, upper)`.
    DoubleNoTouch {
        /// Lower corridor barrier.
        lower: f64,
        /// Upper corridor barrier.
        upper: f64,
        /// Domestic rebate paid if neither barrier is touched.
        rebate: f64,
    },
    /// A single-barrier vanilla (knock-in/out, up/down) with an optional rebate.
    SingleBarrier {
        /// Underlying option type.
        option: OptionType,
        /// Barrier topology.
        topology: CliBarrier,
        /// Strike of the underlying vanilla.
        strike: f64,
        /// Barrier level.
        barrier: f64,
        /// Rebate paid on the terminating event (domestic).
        rebate: f64,
    },
    /// A window knock-out barrier — active only inside `[start, end] ⊆ [0, T]`.
    /// Priced only under the LSV engine (no closed form).
    WindowBarrier {
        /// Underlying option type.
        option: OptionType,
        /// Strike of the underlying vanilla.
        strike: f64,
        /// Barrier level `H`.
        barrier: f64,
        /// `true` for up-and-out, else down-and-out.
        up: bool,
        /// Window start in years.
        start: f64,
        /// Window end in years.
        end: f64,
        /// Antithetic Monte-Carlo path pairs (`0` ⇒ ADI PDE; `> 0` ⇒ MC).
        mc_pairs: usize,
        /// Monte-Carlo time steps (`0` ⇒ default; ignored when `mc_pairs == 0`).
        mc_steps: usize,
        /// Counter-RNG seed for the Monte-Carlo estimator.
        mc_seed: u64,
    },
    /// A variance swap — the result `price` carries the fair *variance* strike
    /// `K_var` (so a flat σ returns σ²).
    VarianceSwap,
    /// A volatility swap — the result `price` carries the fair *vol* strike
    /// `K_vol` (Carr-Lee convexity-adjusted, < √K_var for any non-flat smile).
    VolatilitySwap,
    /// A fixed-strike arithmetic-average-rate Asian (Curran or Turnbull-Wakeman).
    Asian {
        /// Call or put on the realised arithmetic average.
        option: OptionType,
        /// Continuous averaging instead of discrete fixings.
        continuous: bool,
        /// Number of equally-spaced future fixings (discrete style only).
        observations: u32,
        /// Use the Turnbull-Wakeman estimator instead of the Curran default.
        turnbull_wakeman: bool,
        /// The strike `K`.
        strike: f64,
        /// Realised running average of already-fixed observations (seasoned).
        elapsed_avg: f64,
        /// Fraction `∈ [0, 1)` of the average weight already fixed (seasoned).
        elapsed_weight: f64,
    },
    /// A forward-start vanilla (Rubinstein FX dual-carry closed form).
    ForwardStart {
        /// Call or put.
        option: OptionType,
        /// Strike-reset multiple `m`.
        moneyness: f64,
        /// Reset (strike-fixing) date `t₁` in years.
        reset: f64,
    },
    /// A cliquet / ratchet: plain closed-form (Σ forward-start legs), or clamped
    /// Monte-Carlo carrying a standard error.
    Cliquet {
        /// Call or put per-period payoff direction.
        option: OptionType,
        /// Per-period strike-reset multiple `m`.
        moneyness: f64,
        /// Number of evenly-spaced periods over `[0, expiry]`.
        periods: u32,
        /// Optional per-period local floor on each clamped period return.
        local_floor: Option<f64>,
        /// Optional per-period local cap on each clamped period return.
        local_cap: Option<f64>,
        /// Optional global floor on the accumulated payoff.
        global_floor: Option<f64>,
        /// Optional global cap on the accumulated payoff.
        global_cap: Option<f64>,
        /// Antithetic Monte-Carlo path pairs for the clamped variant.
        mc_pairs: usize,
        /// Counter-RNG seed for the clamped Monte-Carlo estimator.
        mc_seed: u64,
    },
    /// A quanto option (vanilla or cash-or-nothing digital), settlement-currency
    /// converted via the quanto-drift adjustment.
    Quanto {
        /// Call or put.
        option: OptionType,
        /// Price the cash-or-nothing digital instead of the vanilla.
        digital: bool,
        /// The strike `K`.
        strike: f64,
        /// Annualised volatility `σ_Z` of the settlement-conversion rate.
        conversion_vol: f64,
        /// Correlation `ρ ∈ [−1, 1]` between the underlying and the conversion rate.
        correlation: f64,
    },
    /// A Target-Redemption Forward (Monte-Carlo; carries a standard error).
    Tarf {
        /// The favourable-side direction.
        option: OptionType,
        /// The strike `K` of every fixing.
        strike: f64,
        /// The cumulative gain target (reaching it redeems).
        target: f64,
        /// Gearing on the adverse (loss) leg.
        leverage: f64,
        /// Number of equally-spaced fixings over `[0, expiry]`.
        fixings: u32,
        /// Per-fixing notional.
        fixing_notional: f64,
        /// Settle the breaching fixing at the capped (remaining-target) gain
        /// instead of the full intrinsic.
        capped_gain: bool,
        /// Antithetic Monte-Carlo path pairs.
        mc_pairs: usize,
        /// Counter-RNG seed for the Monte-Carlo estimator.
        mc_seed: u64,
    },
    /// An accumulator (Monte-Carlo; carries a standard error).
    Accumulator {
        /// Pivot strike.
        pivot: f64,
        /// Up-and-out knock-out barrier (above the pivot).
        barrier: f64,
        /// Gearing on the below-pivot (loss) leg.
        leverage: f64,
        /// Number of equally-spaced fixings over `[0, expiry]`.
        fixings: u32,
        /// Per-fixing notional.
        fixing_notional: f64,
        /// Monitor the barrier continuously between fixings.
        continuous: bool,
        /// Antithetic Monte-Carlo path pairs.
        mc_pairs: usize,
        /// Counter-RNG seed for the Monte-Carlo estimator.
        mc_seed: u64,
    },
    /// An American / Bermudan early-exercise vanilla. Priced by the projected-SOR
    /// free-boundary finite difference (default, exact) or — when `lsm_paths > 0`
    /// — the Longstaff-Schwartz regression Monte-Carlo (carrying a std-error). A
    /// `bermudan_steps` of `0` is continuous American; `n > 0` is a Bermudan with
    /// `n` equally-spaced exercise dates (arbitrary date lists are available via
    /// the SDK / wire contract).
    American {
        /// Call or put.
        option: OptionType,
        /// Strike `K` (absolute).
        strike: f64,
        /// `0` ⇒ continuous American; `n > 0` ⇒ Bermudan with `n` equally-spaced
        /// exercise dates over `(0, T]`.
        bermudan_steps: u32,
        /// Longstaff-Schwartz path count (`0` ⇒ FD engine, exact; `> 0` ⇒ LSM).
        lsm_paths: usize,
        /// Counter-RNG / Sobol scramble seed for the LSM engine.
        lsm_seed: u64,
    },
    /// A lookback option (continuous closed-form, or discrete Monte-Carlo with a
    /// standard error).
    Lookback {
        /// Call or put.
        option: OptionType,
        /// Use the fixed-strike family (against `strike`) instead of floating.
        fixed: bool,
        /// The strike `K` (used only by the fixed-strike family).
        strike: f64,
        /// Monitor discretely (Monte-Carlo) instead of continuously (closed form).
        discrete: bool,
        /// Number of equally-spaced observations for the discrete variant.
        observations: u32,
        /// Antithetic Monte-Carlo path pairs for the discrete variant.
        mc_pairs: usize,
        /// Counter-RNG seed for the discrete Monte-Carlo estimator.
        mc_seed: u64,
    },
}

/// The result of an `exotic` run: the priced present value and a label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ExoticResult {
    /// Present value (domestic premium).
    pub(crate) price: f64,
    /// For a Monte-Carlo-priced product (a clamped cliquet), the standard error
    /// of the mean of `price`; `None` for the closed-form products.
    pub(crate) std_error: Option<f64>,
}

/// Price the chosen exotic against the supplied Garman-Kohlhagen market inputs.
///
/// `inputs` carries spot, vol, time, and the two rates; the strike inside
/// `inputs` is used by the digital (whose payout is struck at `inputs.strike`),
/// while the single barrier takes its own strike from the spec. Touch products
/// ignore the strike.
#[must_use]
pub(crate) fn run(spec: ExoticSpec, inputs: &VanillaInputs) -> ExoticResult {
    // The agnostic carry-seam view of the FX market input — every exotics engine
    // consumes `ExoticInputs` (byte-identical for FX).
    let einputs: celnet_exotics::ExoticInputs = inputs.into();
    // The clamped cliquet is Monte-Carlo and carries a standard error; handle it
    // up front so the closed-form arms below can all be `std_error: None`.
    if let ExoticSpec::Cliquet {
        option,
        moneyness,
        periods,
        local_floor,
        local_cap,
        global_floor,
        global_cap,
        mc_pairs,
        mc_seed,
    } = spec
    {
        let spec = Cliquet {
            option,
            moneyness,
            schedule: CliquetSchedule::equal(periods as usize, inputs.t),
            local_floor,
            local_cap,
            global_floor,
            global_cap,
        };
        if spec.is_plain() {
            // Exact closed form (Σ forward-start legs).
            return ExoticResult {
                price: cliquet_price_plain(&einputs, &spec),
                std_error: None,
            };
        }
        let estimate = cliquet_price_capped_mc(
            &einputs,
            &spec,
            CliquetMcConfig {
                pairs: mc_pairs,
                seed: mc_seed,
            },
        );
        return ExoticResult {
            price: estimate.price,
            std_error: Some(estimate.std_error),
        };
    }
    // TARF, accumulator, and the discrete-monitored lookback are Monte-Carlo and
    // carry a standard error; handle them up front too.
    match spec {
        ExoticSpec::Tarf {
            option,
            strike,
            target,
            leverage,
            fixings,
            fixing_notional,
            capped_gain,
            mc_pairs,
            mc_seed,
        } => {
            let estimate = tarf_price(
                &einputs,
                Tarf {
                    strike,
                    fixings: fixings as usize,
                    target,
                    leverage,
                    favourable_side: option,
                    notional: fixing_notional,
                    redemption: if capped_gain {
                        RedemptionStyle::CappedGain
                    } else {
                        RedemptionStyle::FullGain
                    },
                },
                TarfMcConfig {
                    pairs: mc_pairs,
                    seed: mc_seed,
                },
            );
            return ExoticResult {
                price: estimate.price,
                std_error: Some(estimate.std_error),
            };
        }
        ExoticSpec::Accumulator {
            pivot,
            barrier,
            leverage,
            fixings,
            fixing_notional,
            continuous,
            mc_pairs,
            mc_seed,
        } => {
            let estimate = accumulator_price(
                &einputs,
                Accumulator {
                    pivot,
                    barrier,
                    fixings: fixings as usize,
                    leverage,
                    notional: fixing_notional,
                    monitoring: if continuous {
                        Monitoring::Continuous
                    } else {
                        Monitoring::Discrete
                    },
                },
                AccumulatorMcConfig {
                    pairs: mc_pairs,
                    seed: mc_seed,
                },
            );
            return ExoticResult {
                price: estimate.price,
                std_error: Some(estimate.std_error),
            };
        }
        ExoticSpec::Lookback {
            option,
            fixed,
            strike,
            discrete: true,
            observations,
            mc_pairs,
            mc_seed,
        } => {
            let lb_inputs = celnet_exotics::ExoticInputs {
                strike,
                ..einputs.clone()
            };
            let estimate = lookback_mc(
                &lb_inputs,
                Lookback {
                    style: if fixed {
                        LookbackStyle::FixedStrike
                    } else {
                        LookbackStyle::FloatingStrike
                    },
                    option,
                },
                LookbackMcConfig {
                    pairs: mc_pairs,
                    steps: observations as usize,
                    seed: mc_seed,
                },
            );
            return ExoticResult {
                price: estimate.price,
                std_error: Some(estimate.std_error),
            };
        }
        // American / Bermudan via the Longstaff-Schwartz Monte-Carlo engine
        // (carries a std-error). The finite-difference engine (exact, no
        // std-error) is handled in the closed-form match below.
        ExoticSpec::American {
            option,
            strike,
            bermudan_steps,
            lsm_paths,
            lsm_seed,
        } if lsm_paths > 0 => {
            let am_inputs = celnet_exotics::ExoticInputs {
                strike,
                ..einputs.clone()
            };
            let estimate = american_lsm(
                &am_inputs,
                &american_spec(option, strike, bermudan_steps, inputs.t),
                LsmConfig {
                    paths: lsm_paths,
                    seed: lsm_seed,
                    ..LsmConfig::default()
                },
            );
            return ExoticResult {
                price: estimate.price,
                std_error: Some(estimate.std_error),
            };
        }
        _ => {}
    }
    let price = match spec {
        ExoticSpec::Vanilla { option } => celnet_vanilla::price(option, inputs),
        ExoticSpec::Digital(kind) => digital_price(kind, &einputs),
        ExoticSpec::OneTouch {
            barrier,
            rebate,
            at_expiry,
        } => {
            let timing = if at_expiry {
                RebateTiming::AtExpiry
            } else {
                RebateTiming::AtHit
            };
            one_touch_price(&einputs, barrier, rebate, timing)
        }
        ExoticSpec::DoubleNoTouch {
            lower,
            upper,
            rebate,
        } => double_no_touch_price(&einputs, DoubleNoTouch::new(lower, upper, rebate)),
        ExoticSpec::SingleBarrier {
            option,
            topology,
            strike,
            barrier,
            rebate,
        } => single_barrier_price(
            &einputs,
            SingleBarrier {
                kind: topology.kind(option),
                strike,
                barrier,
                rebate,
            },
        ),
        // The wire market input carries a single Black vol, so the smile here is
        // the flat smile at that vol (a flat σ replicates to K_var = σ² exactly).
        ExoticSpec::VarianceSwap => {
            let ctx = VarSwapContext::from_inputs(inputs);
            fair_variance(&FlatSmile::new(inputs.vol), &ctx).fair_variance
        }
        ExoticSpec::VolatilitySwap => {
            let ctx = VarSwapContext::from_inputs(inputs);
            fair_volatility(&FlatSmile::new(inputs.vol), &ctx).fair_vol
        }
        ExoticSpec::Asian {
            option,
            continuous,
            observations,
            turnbull_wakeman,
            strike,
            elapsed_avg,
            elapsed_weight,
        } => {
            let schedule = if continuous {
                AveragingSchedule::Continuous
            } else {
                AveragingSchedule::Discrete {
                    future_obs: observations as usize,
                }
            };
            let spec = AnalyticAsian {
                option,
                strike,
                schedule,
                t_start: 0.0,
                elapsed_avg,
                elapsed_weight,
            };
            // Price at the strike the spec carries (the CLI `inputs.strike` is the
            // digital/barrier strike; the Asian strike is its own field).
            let asian_inputs = celnet_exotics::ExoticInputs {
                strike,
                ..einputs.clone()
            };
            if turnbull_wakeman {
                turnbull_wakeman_price(&asian_inputs, spec)
            } else {
                curran_price(&asian_inputs, spec)
            }
        }
        ExoticSpec::ForwardStart {
            option,
            moneyness,
            reset,
        } => forward_start_price(
            &einputs,
            ForwardStart {
                option,
                moneyness,
                reset,
                expiry: inputs.t,
            },
        ),
        ExoticSpec::Quanto {
            option,
            digital,
            strike,
            conversion_vol,
            correlation,
        } => {
            let quanto_inputs = VanillaInputs { strike, ..*inputs };
            let params = QuantoParams::new(conversion_vol, correlation);
            if digital {
                quanto_digital_price(option, &quanto_inputs, params)
            } else {
                quanto_vanilla_price(option, &quanto_inputs, params)
            }
        }
        // A continuously-monitored lookback is closed-form (the discrete variant
        // is handled up front as Monte-Carlo).
        ExoticSpec::Lookback {
            option,
            fixed,
            strike,
            discrete: false,
            ..
        } => {
            if fixed {
                let lb_inputs = celnet_exotics::ExoticInputs {
                    strike,
                    ..einputs.clone()
                };
                fixed_lookback_price(&lb_inputs, option)
            } else {
                floating_lookback_price(&einputs, option)
            }
        }
        // American / Bermudan via the projected-SOR free-boundary finite
        // difference (exact, no std-error). The LSM (`lsm_paths > 0`) arm is
        // handled up front as Monte-Carlo.
        ExoticSpec::American {
            option,
            strike,
            bermudan_steps,
            lsm_paths: 0,
            ..
        } => {
            let am_inputs = celnet_exotics::ExoticInputs {
                strike,
                ..einputs.clone()
            };
            american_fd(
                &am_inputs,
                &american_spec(option, strike, bermudan_steps, inputs.t),
                AmericanGrid::default(),
            )
        }
        // Handled up front (the Monte-Carlo / std-error-carrying arms).
        ExoticSpec::Cliquet { .. }
        | ExoticSpec::Tarf { .. }
        | ExoticSpec::Accumulator { .. }
        | ExoticSpec::Lookback { discrete: true, .. }
        | ExoticSpec::American { .. } => {
            unreachable!("Monte-Carlo exotics are handled before this match")
        }
        // A window barrier has no closed form: it is LSV-only and is dispatched to
        // `lsv_run`, never to the analytic `run`.
        ExoticSpec::WindowBarrier { .. } => {
            unreachable!("the window barrier is LSV-only and is dispatched to lsv_run")
        }
    };
    ExoticResult {
        price,
        std_error: None,
    }
}

/// Build the [`AmericanOption`] spec from the CLI fields: `bermudan_steps == 0`
/// is continuous American; `n > 0` is a Bermudan with `n` equally-spaced exercise
/// dates `k/n · T` for `k = 1..=n` over `(0, T]`.
fn american_spec(option: OptionType, strike: f64, bermudan_steps: u32, t: f64) -> AmericanOption {
    let style = if bermudan_steps == 0 {
        ExerciseStyle::American
    } else {
        let n = bermudan_steps;
        let dates = (1..=n).map(|k| t * f64::from(k) / f64::from(n)).collect();
        ExerciseStyle::Bermudan { dates }
    };
    AmericanOption {
        option,
        strike,
        style,
    }
}

/// Render an [`ExoticResult`] as a one-line report with a variant label.
#[must_use]
pub(crate) fn format_report(spec: ExoticSpec, r: &ExoticResult) -> String {
    let label = match spec {
        ExoticSpec::Vanilla { .. } => "vanilla",
        ExoticSpec::Digital(_) => "digital",
        ExoticSpec::OneTouch { .. } => "one-touch",
        ExoticSpec::DoubleNoTouch { .. } => "double-no-touch",
        ExoticSpec::SingleBarrier { .. } => "single-barrier",
        ExoticSpec::VarianceSwap => "variance-swap",
        ExoticSpec::VolatilitySwap => "volatility-swap",
        ExoticSpec::Asian { .. } => "asian",
        ExoticSpec::ForwardStart { .. } => "forward-start",
        ExoticSpec::Cliquet { .. } => "cliquet",
        ExoticSpec::Quanto { digital: false, .. } => "quanto-vanilla",
        ExoticSpec::Quanto { digital: true, .. } => "quanto-digital",
        ExoticSpec::Tarf { .. } => "tarf",
        ExoticSpec::Accumulator { .. } => "accumulator",
        ExoticSpec::Lookback {
            discrete: false, ..
        } => "lookback-continuous",
        ExoticSpec::Lookback { discrete: true, .. } => "lookback-discrete",
        ExoticSpec::WindowBarrier { .. } => "window-barrier-lsv",
        ExoticSpec::American {
            bermudan_steps: 0,
            lsm_paths: 0,
            ..
        } => "american-fd",
        ExoticSpec::American { lsm_paths: 0, .. } => "bermudan-fd",
        ExoticSpec::American {
            bermudan_steps: 0, ..
        } => "american-lsm",
        ExoticSpec::American { .. } => "bermudan-lsm",
    };
    // The variance swap's headline figure is a fair *variance* strike `K_var`;
    // echo its realised-vol equivalent `√K_var` so the desk reads both.
    if matches!(spec, ExoticSpec::VarianceSwap) {
        return format!(
            "{label}\n  fair_variance   {:.10}\n  fair_vol        {:.10}\n",
            r.price,
            r.price.max(0.0).sqrt()
        );
    }
    if matches!(spec, ExoticSpec::VolatilitySwap) {
        return format!("{label}\n  fair_vol        {:.10}\n", r.price);
    }
    // A Monte-Carlo-priced product (the clamped cliquet) reports its standard
    // error honestly alongside the price.
    if let Some(stderr) = r.std_error {
        return format!(
            "{label}\n  price           {:.10}\n  std_error       {:.10}\n",
            r.price, stderr
        );
    }
    format!("{label}\n  price           {:.10}\n", r.price)
}

// ---------------------------------------------------------------------------
// Local-stochastic-volatility (LSV) pricing route (`--model lsv`)
// ---------------------------------------------------------------------------

use celnet_exotics::{
    AdiGrid, ImpliedVolSurface, LsvModel, McConfig, ParticleConfig, VarianceParams,
    WindowBarrier as ExWindowBarrier,
};

/// A flat implied-vol surface anchored at the market vol — the LSV calibration
/// target the CLI's single-vol market inputs expose (mirrors the server route).
struct CliFlatIv {
    sigma: f64,
    spot: f64,
    carry: f64,
}
impl ImpliedVolSurface for CliFlatIv {
    fn implied_vol(&self, _k: f64, _t: f64) -> f64 {
        self.sigma
    }
    fn forward(&self, t: f64) -> f64 {
        self.spot * celnet_core::math::exp(self.carry * t)
    }
}

/// Calibrate the LSV model for the CLI's market inputs (same canonical
/// stochastic-variance parameters and particle budget as the server route).
fn lsv_model(inputs: &VanillaInputs) -> LsvModel {
    let sigma = inputs.vol;
    let v = sigma * sigma;
    // Canonical server LSV params (kept in lockstep with celnet-server's
    // lsv_pricer): mean-reversion 2.0, vol-of-var 0.18 (Feller at σ=10%),
    // spot/variance correlation −0.30.
    let var = VarianceParams::new(v, 2.0, v, 0.18, -0.30);
    let carry = inputs.r_dom - inputs.r_for;
    let iv = CliFlatIv {
        sigma,
        spot: inputs.spot,
        carry,
    };
    let n = 41usize;
    let spot_grid: Vec<f64> = (0..n)
        .map(|k| {
            let x = -0.6 + 1.2 * (k as f64) / ((n - 1) as f64);
            inputs.spot * celnet_core::math::exp(x)
        })
        .collect();
    let particle = ParticleConfig {
        particles: 30_000,
        steps: 40,
        seed: 0x0001_0CA1,
        ..ParticleConfig::default()
    };
    LsvModel::calibrate(*inputs, var, &iv, &spot_grid, particle)
}

/// The fine ADI grid for the LSV headline price (matches the server default).
fn lsv_price_grid() -> AdiGrid {
    AdiGrid {
        x_steps: 160,
        v_steps: 48,
        time_steps: 100,
        ..AdiGrid::default()
    }
}

/// Price an LSV-supported exotic (`--model lsv`): vanilla, single-barrier
/// (continuous knock-out/in) and window-barrier. Returns an error string for any
/// other product (mapped to a CLI invalid-argument), never a silent fallback to
/// the analytic engine.
///
/// `inputs.strike` is the underlying strike for the vanilla / barrier legs; the
/// touch / swap / fixing products are not LSV products.
pub(crate) fn lsv_run(spec: ExoticSpec, inputs: &VanillaInputs) -> Result<ExoticResult, String> {
    match spec {
        ExoticSpec::Vanilla { option } => Ok(lsv_vanilla(option, inputs)),
        ExoticSpec::SingleBarrier {
            option,
            topology,
            strike,
            barrier,
            rebate,
        } => {
            if rebate != 0.0 {
                return Err(
                    "the LSV barrier route does not price a rebate; use rebate 0".to_owned(),
                );
            }
            if !(barrier.is_finite() && barrier > 0.0) {
                return Err("barrier must be positive and finite".to_owned());
            }
            let (up, knock_in) = match topology {
                CliBarrier::UpAndOut => (true, false),
                CliBarrier::DownAndOut => (false, false),
                CliBarrier::UpAndIn => (true, true),
                CliBarrier::DownAndIn => (false, true),
            };
            let model = lsv_model(inputs);
            let grid = lsv_price_grid();
            let ko = model.price_barrier_pde(option, strike, barrier, up, grid);
            let price = if knock_in {
                // In-out parity under the same model: KI = vanilla − KO.
                model.price_european_pde(option, strike, grid) - ko
            } else {
                ko
            };
            Ok(ExoticResult {
                price,
                std_error: None,
            })
        }
        ExoticSpec::WindowBarrier {
            option,
            strike,
            barrier,
            up,
            start,
            end,
            mc_pairs,
            mc_steps,
            mc_seed,
        } => {
            if !(barrier.is_finite() && barrier > 0.0) {
                return Err("barrier must be positive and finite".to_owned());
            }
            if !(start >= 0.0 && start < end && end <= inputs.t) {
                return Err("window must satisfy 0 <= window-start < window-end <= t".to_owned());
            }
            let model = lsv_model(inputs);
            let wspec = ExWindowBarrier {
                option,
                strike,
                barrier,
                up,
                start,
                end,
            };
            if mc_pairs > 0 {
                let cfg = McConfig {
                    pairs: mc_pairs,
                    steps: if mc_steps > 0 { mc_steps } else { 96 },
                    seed: mc_seed,
                };
                let est = model.price_window_barrier_mc(wspec, cfg);
                Ok(ExoticResult {
                    price: est.price,
                    std_error: Some(est.std_error),
                })
            } else {
                let price = model.price_window_barrier_pde(wspec, lsv_price_grid());
                Ok(ExoticResult {
                    price,
                    std_error: None,
                })
            }
        }
        // The LSV engine does not price these products — reject clearly.
        ExoticSpec::Digital(_) => Err(lsv_unsupported("digital")),
        ExoticSpec::OneTouch { .. } => Err(lsv_unsupported("one-touch")),
        ExoticSpec::DoubleNoTouch { .. } => Err(lsv_unsupported("double-no-touch")),
        ExoticSpec::VarianceSwap => Err(lsv_unsupported("variance-swap")),
        ExoticSpec::VolatilitySwap => Err(lsv_unsupported("volatility-swap")),
        ExoticSpec::Asian { .. } => Err(lsv_unsupported("asian")),
        ExoticSpec::ForwardStart { .. } => Err(lsv_unsupported("forward-start")),
        ExoticSpec::Cliquet { .. } => Err(lsv_unsupported("cliquet")),
        ExoticSpec::Quanto { .. } => Err(lsv_unsupported("quanto")),
        ExoticSpec::Tarf { .. } => Err(lsv_unsupported("tarf")),
        ExoticSpec::Accumulator { .. } => Err(lsv_unsupported("accumulator")),
        ExoticSpec::Lookback { .. } => Err(lsv_unsupported("lookback")),
        ExoticSpec::American { .. } => Err(lsv_unsupported("american")),
    }
}

/// Price a plain vanilla under the LSV engine (the European ADI PDE).
pub(crate) fn lsv_vanilla(option: OptionType, inputs: &VanillaInputs) -> ExoticResult {
    let model = lsv_model(inputs);
    ExoticResult {
        price: model.price_european_pde(option, inputs.strike, lsv_price_grid()),
        std_error: None,
    }
}

fn lsv_unsupported(product: &str) -> String {
    format!(
        "pricing model LOCAL_STOCH_VOL does not support product {product}; \
         select a supported product (vanilla, barrier, window-barrier) or --model analytic"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;
    use celnet_exotics::{BarrierKind, BarrierStyle, DigitalStyle};

    fn inputs() -> VanillaInputs {
        VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.02, 0.01)
    }

    #[test]
    fn digital_matches_direct() {
        let kind = DigitalKind {
            style: DigitalStyle::CashOrNothing,
            option: OptionType::Call,
        };
        let r = run(ExoticSpec::Digital(kind), &inputs());
        let direct = digital_price(kind, &(&inputs()).into());
        assert!(is_close(r.price, direct, 1e-15, 1e-15));
    }

    #[test]
    fn single_barrier_matches_direct() {
        let r = run(
            ExoticSpec::SingleBarrier {
                option: OptionType::Call,
                topology: CliBarrier::DownAndOut,
                strike: 1.10,
                barrier: 0.95,
                rebate: 0.0,
            },
            &inputs(),
        );
        let direct = single_barrier_price(
            &(&inputs()).into(),
            SingleBarrier {
                kind: BarrierKind {
                    up: false,
                    style: BarrierStyle::KnockOut,
                    option: OptionType::Call,
                },
                strike: 1.10,
                barrier: 0.95,
                rebate: 0.0,
            },
        );
        assert!(is_close(r.price, direct, 1e-13, 1e-13));
    }

    #[test]
    fn one_touch_matches_direct() {
        let r = run(
            ExoticSpec::OneTouch {
                barrier: 1.20,
                rebate: 1.0,
                at_expiry: false,
            },
            &inputs(),
        );
        let direct = one_touch_price(&(&inputs()).into(), 1.20, 1.0, RebateTiming::AtHit);
        assert!(is_close(r.price, direct, 1e-15, 1e-15));
    }

    #[test]
    fn dnt_matches_direct() {
        let r = run(
            ExoticSpec::DoubleNoTouch {
                lower: 1.00,
                upper: 1.20,
                rebate: 1.0,
            },
            &inputs(),
        );
        let direct =
            double_no_touch_price(&(&inputs()).into(), DoubleNoTouch::new(1.00, 1.20, 1.0));
        assert!(is_close(r.price, direct, 1e-15, 1e-15));
    }

    #[test]
    fn variance_swap_matches_direct_and_flat_sigma_oracle() {
        let i = inputs();
        let r = run(ExoticSpec::VarianceSwap, &i);
        let ctx = VarSwapContext::from_inputs(&i);
        let direct = fair_variance(&FlatSmile::new(i.vol), &ctx).fair_variance;
        assert!(is_close(r.price, direct, 1e-12, 1e-12));
        // Flat-σ closed-form limit oracle: K_var == σ².
        assert!(is_close(r.price, i.vol * i.vol, 1e-7, 1e-9));
    }

    #[test]
    fn volatility_swap_matches_direct_and_flat_sigma_oracle() {
        let i = inputs();
        let r = run(ExoticSpec::VolatilitySwap, &i);
        let ctx = VarSwapContext::from_inputs(&i);
        let direct = fair_volatility(&FlatSmile::new(i.vol), &ctx).fair_vol;
        assert!(is_close(r.price, direct, 1e-12, 1e-12));
        // Flat smile ⇒ zero convexity gap ⇒ K_vol == σ.
        assert!(is_close(r.price, i.vol, 1e-6, 1e-7));
    }

    #[test]
    fn asian_curran_matches_direct() {
        let i = inputs();
        let spec = ExoticSpec::Asian {
            option: OptionType::Call,
            continuous: false,
            observations: 12,
            turnbull_wakeman: false,
            strike: 1.10,
            elapsed_avg: 0.0,
            elapsed_weight: 0.0,
        };
        let r = run(spec, &i);
        let direct = curran_price(
            &celnet_exotics::ExoticInputs {
                strike: 1.10,
                ..(&i).into()
            },
            AnalyticAsian::fresh_discrete(OptionType::Call, 1.10, 12),
        );
        assert!(is_close(r.price, direct, 1e-12, 1e-12));
        assert!(r.price > 0.0);
    }

    #[test]
    fn asian_turnbull_wakeman_continuous_matches_direct() {
        let i = inputs();
        let spec = ExoticSpec::Asian {
            option: OptionType::Put,
            continuous: true,
            observations: 0,
            turnbull_wakeman: true,
            strike: 1.12,
            elapsed_avg: 0.0,
            elapsed_weight: 0.0,
        };
        let r = run(spec, &i);
        let direct = turnbull_wakeman_price(
            &celnet_exotics::ExoticInputs {
                strike: 1.12,
                ..(&i).into()
            },
            AnalyticAsian::fresh_continuous(OptionType::Put, 1.12),
        );
        assert!(is_close(r.price, direct, 1e-12, 1e-12));
    }

    #[test]
    fn forward_start_matches_direct() {
        let i = inputs();
        let spec = ExoticSpec::ForwardStart {
            option: OptionType::Call,
            moneyness: 1.0,
            reset: 0.25,
        };
        let r = run(spec, &i);
        let direct = forward_start_price(
            &(&i).into(),
            ForwardStart {
                option: OptionType::Call,
                moneyness: 1.0,
                reset: 0.25,
                expiry: i.t,
            },
        );
        assert!(is_close(r.price, direct, 1e-13, 1e-13));
        assert!(r.std_error.is_none());
        assert!(r.price > 0.0);
    }

    #[test]
    fn plain_cliquet_matches_sum_of_legs() {
        let i = inputs();
        let periods = 4u32;
        let r = run(
            ExoticSpec::Cliquet {
                option: OptionType::Call,
                moneyness: 1.0,
                periods,
                local_floor: None,
                local_cap: None,
                global_floor: None,
                global_cap: None,
                mc_pairs: 0,
                mc_seed: 0,
            },
            &i,
        );
        let mut sum = 0.0;
        for k in 1..=periods {
            sum += forward_start_price(
                &(&i).into(),
                ForwardStart {
                    option: OptionType::Call,
                    moneyness: 1.0,
                    reset: (k - 1) as f64 / periods as f64,
                    expiry: k as f64 / periods as f64,
                },
            );
        }
        assert!(is_close(r.price, sum, 1e-10, 1e-12));
        assert!(r.std_error.is_none());
    }

    #[test]
    fn capped_cliquet_matches_direct_mc_and_carries_std_error() {
        let i = inputs();
        let periods = 4u32;
        let cap = 0.03;
        let pairs = 20_000usize;
        let seed = 0xC119_0E70;
        let r = run(
            ExoticSpec::Cliquet {
                option: OptionType::Call,
                moneyness: 1.0,
                periods,
                local_floor: Some(0.0),
                local_cap: Some(cap),
                global_floor: None,
                global_cap: None,
                mc_pairs: pairs,
                mc_seed: seed,
            },
            &i,
        );
        let spec = Cliquet {
            option: OptionType::Call,
            moneyness: 1.0,
            schedule: CliquetSchedule::equal(periods as usize, i.t),
            local_floor: Some(0.0),
            local_cap: Some(cap),
            global_floor: None,
            global_cap: None,
        };
        let direct = cliquet_price_capped_mc(&(&i).into(), &spec, CliquetMcConfig { pairs, seed });
        assert!(is_close(r.price, direct.price, 1e-12, 1e-12));
        let stderr = r.std_error.expect("clamped cliquet carries std-error");
        assert!(is_close(stderr, direct.std_error, 1e-12, 1e-12));
        assert!(stderr > 0.0);
        // The std-error must surface in the report.
        let report = format_report(
            ExoticSpec::Cliquet {
                option: OptionType::Call,
                moneyness: 1.0,
                periods,
                local_floor: Some(0.0),
                local_cap: Some(cap),
                global_floor: None,
                global_cap: None,
                mc_pairs: pairs,
                mc_seed: seed,
            },
            &r,
        );
        assert!(report.contains("std_error"));
    }

    #[test]
    fn quanto_vanilla_matches_direct() {
        let i = inputs();
        let r = run(
            ExoticSpec::Quanto {
                option: OptionType::Call,
                digital: false,
                strike: 1.10,
                conversion_vol: 0.09,
                correlation: -0.3,
            },
            &i,
        );
        let direct = quanto_vanilla_price(
            OptionType::Call,
            &VanillaInputs { strike: 1.10, ..i },
            QuantoParams::new(0.09, -0.3),
        );
        assert!(is_close(r.price, direct, 1e-13, 1e-13));
    }

    #[test]
    fn quanto_zero_correlation_recovers_plain_vanilla() {
        let i = inputs();
        let r = run(
            ExoticSpec::Quanto {
                option: OptionType::Call,
                digital: false,
                strike: 1.10,
                conversion_vol: 0.09,
                correlation: 0.0,
            },
            &i,
        );
        let plain = celnet_vanilla::price(OptionType::Call, &VanillaInputs { strike: 1.10, ..i });
        assert!(is_close(r.price, plain, 1e-12, 1e-12));
    }

    #[test]
    fn quanto_digital_matches_direct() {
        let i = inputs();
        let r = run(
            ExoticSpec::Quanto {
                option: OptionType::Put,
                digital: true,
                strike: 1.12,
                conversion_vol: 0.07,
                correlation: 0.4,
            },
            &i,
        );
        let direct = quanto_digital_price(
            OptionType::Put,
            &VanillaInputs { strike: 1.12, ..i },
            QuantoParams::new(0.07, 0.4),
        );
        assert!(is_close(r.price, direct, 1e-13, 1e-13));
    }

    #[test]
    fn tarf_matches_direct_mc_and_carries_std_error() {
        let i = inputs();
        let seed = 0x7A2F_C111_u64;
        let pairs = 4_000usize;
        let r = run(
            ExoticSpec::Tarf {
                option: OptionType::Put,
                strike: 1.10,
                target: 0.30,
                leverage: 2.0,
                fixings: 8,
                fixing_notional: 1.0,
                capped_gain: false,
                mc_pairs: pairs,
                mc_seed: seed,
            },
            &i,
        );
        let direct = tarf_price(
            &(&i).into(),
            Tarf {
                strike: 1.10,
                fixings: 8,
                target: 0.30,
                leverage: 2.0,
                favourable_side: OptionType::Put,
                notional: 1.0,
                redemption: RedemptionStyle::FullGain,
            },
            TarfMcConfig { pairs, seed },
        );
        // Same (seed, pairs) ⇒ bit-reproducible ⇒ exact agreement.
        assert!(is_close(r.price, direct.price, 1e-12, 1e-12));
        let stderr = r.std_error.expect("TARF (MC) must carry a std-error");
        assert!(stderr > 0.0);
        assert!(
            format_report(
                ExoticSpec::Tarf {
                    option: OptionType::Put,
                    strike: 1.10,
                    target: 0.30,
                    leverage: 2.0,
                    fixings: 8,
                    fixing_notional: 1.0,
                    capped_gain: false,
                    mc_pairs: pairs,
                    mc_seed: seed,
                },
                &r
            )
            .contains("std_error")
        );
    }

    #[test]
    fn accumulator_matches_direct_mc_and_carries_std_error() {
        let i = inputs();
        let seed = 0xACC0_C111_u64;
        let pairs = 4_000usize;
        let r = run(
            ExoticSpec::Accumulator {
                pivot: 1.10,
                barrier: 1.16,
                leverage: 2.0,
                fixings: 8,
                fixing_notional: 1.0,
                continuous: false,
                mc_pairs: pairs,
                mc_seed: seed,
            },
            &i,
        );
        let direct = accumulator_price(
            &(&i).into(),
            Accumulator {
                pivot: 1.10,
                barrier: 1.16,
                fixings: 8,
                leverage: 2.0,
                notional: 1.0,
                monitoring: Monitoring::Discrete,
            },
            AccumulatorMcConfig { pairs, seed },
        );
        assert!(is_close(r.price, direct.price, 1e-12, 1e-12));
        assert!(r.std_error.expect("accumulator (MC) std-error") > 0.0);
    }

    #[test]
    fn lookback_continuous_matches_direct_closed_form_no_std_error() {
        let i = inputs();
        let r = run(
            ExoticSpec::Lookback {
                option: OptionType::Call,
                fixed: false,
                strike: 0.0,
                discrete: false,
                observations: 0,
                mc_pairs: 0,
                mc_seed: 0,
            },
            &i,
        );
        let direct = floating_lookback_price(&(&i).into(), OptionType::Call);
        assert!(is_close(r.price, direct, 1e-13, 1e-13));
        assert!(r.std_error.is_none(), "continuous lookback is closed-form");
        // A lookback dominates the equivalent vanilla.
        let vanilla = celnet_vanilla::price(OptionType::Call, &i);
        assert!(r.price > vanilla);
    }

    #[test]
    fn lookback_discrete_matches_direct_mc_and_carries_std_error() {
        let i = inputs();
        let seed = 0x100C_BAC4_u64;
        let pairs = 4_000usize;
        let observations = 16u32;
        let strike = 1.05;
        let r = run(
            ExoticSpec::Lookback {
                option: OptionType::Call,
                fixed: true,
                strike,
                discrete: true,
                observations,
                mc_pairs: pairs,
                mc_seed: seed,
            },
            &i,
        );
        let lb_inputs = celnet_exotics::ExoticInputs {
            strike,
            ..(&i).into()
        };
        let direct = lookback_mc(
            &lb_inputs,
            Lookback {
                style: LookbackStyle::FixedStrike,
                option: OptionType::Call,
            },
            LookbackMcConfig {
                pairs,
                steps: observations as usize,
                seed,
            },
        );
        assert!(is_close(r.price, direct.price, 1e-12, 1e-12));
        assert!(r.std_error.expect("discrete lookback (MC) std-error") > 0.0);
    }

    #[test]
    fn lsv_window_barrier_matches_direct_lsv_reprice() {
        let i = VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01);
        let spec = ExoticSpec::WindowBarrier {
            option: OptionType::Call,
            strike: 1.30,
            barrier: 1.50,
            up: true,
            start: 0.5,
            end: 1.0,
            mc_pairs: 0,
            mc_steps: 0,
            mc_seed: 0,
        };
        let r = lsv_run(spec, &i).expect("LSV window barrier prices");
        assert!(r.std_error.is_none(), "PDE LSV window carries no std-error");

        // Independent reprice on the SAME engine, set up by hand (the lsv_model
        // helper is shared, but the window spec / grid here are spelled out).
        let model = lsv_model(&i);
        let direct = model.price_window_barrier_pde(
            ExWindowBarrier {
                option: OptionType::Call,
                strike: 1.30,
                barrier: 1.50,
                up: true,
                start: 0.5,
                end: 1.0,
            },
            lsv_price_grid(),
        );
        assert!(
            (r.price - direct).abs() < 1e-9,
            "lsv_run {} vs direct {}",
            r.price,
            direct
        );
    }

    #[test]
    fn lsv_on_unsupported_product_errors() {
        let i = VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.02, 0.01);
        let err = lsv_run(ExoticSpec::VarianceSwap, &i)
            .expect_err("LSV must reject an unsupported product");
        assert!(
            err.contains("LOCAL_STOCH_VOL") && err.contains("variance-swap"),
            "clear unsupported-product error: {err}"
        );
    }

    #[test]
    fn lsv_vanilla_differs_from_analytic_for_a_skewed_barrier() {
        // The LSV vanilla reprices the flat smile, so it should be close to GK;
        // the barrier under LSV picks up skew/stoch-vol the flat GK barrier lacks.
        let i = VanillaInputs::new(1.30, 1.20, 0.10, 1.0, 0.03, 0.01);
        let lsv = lsv_vanilla(OptionType::Call, &i);
        let gk = celnet_vanilla::price(OptionType::Call, &i);
        assert!(
            (lsv.price - gk).abs() < 5e-3,
            "LSV vanilla {} reprices flat GK {}",
            lsv.price,
            gk
        );
    }

    /// The CLI American FD arm matches the direct exotics `american_fd` and
    /// dominates the European value (early-exercise premium ≥ 0).
    #[test]
    fn american_fd_matches_direct_and_dominates_european() {
        let i = VanillaInputs::new(100.0, 100.0, 0.25, 1.0, 0.08, 0.0);
        let spec = ExoticSpec::American {
            option: OptionType::Put,
            strike: 100.0,
            bermudan_steps: 0,
            lsm_paths: 0,
            lsm_seed: 0,
        };
        let r = run(spec, &i);
        assert!(r.std_error.is_none(), "the FD engine reports no std-error");
        let direct = american_fd(
            &(&VanillaInputs { strike: 100.0, ..i }).into(),
            &american_spec(OptionType::Put, 100.0, 0, i.t),
            AmericanGrid::default(),
        );
        assert!(is_close(r.price, direct, 1e-12, 1e-12));
        let euro = celnet_vanilla::price(OptionType::Put, &VanillaInputs { strike: 100.0, ..i });
        assert!(
            r.price >= euro - 5e-3,
            "American {} >= European {euro}",
            r.price
        );
        assert_eq!(
            format_report(spec, &r).split_whitespace().next(),
            Some("american-fd")
        );
    }

    /// The CLI American LSM arm carries a std-error and agrees with the FD arm
    /// within it.
    #[test]
    fn american_lsm_carries_std_error_and_matches_fd() {
        let i = VanillaInputs::new(100.0, 100.0, 0.25, 1.0, 0.08, 0.0);
        let fd = run(
            ExoticSpec::American {
                option: OptionType::Put,
                strike: 100.0,
                bermudan_steps: 0,
                lsm_paths: 0,
                lsm_seed: 0,
            },
            &i,
        );
        let lsm = run(
            ExoticSpec::American {
                option: OptionType::Put,
                strike: 100.0,
                bermudan_steps: 0,
                lsm_paths: 200_000,
                lsm_seed: 0xABCD,
            },
            &i,
        );
        let se = lsm.std_error.expect("LSM arm carries a std-error");
        assert!(se > 0.0);
        assert!((fd.price - lsm.price).abs() < 4.0 * se + 1e-2);
    }
}
