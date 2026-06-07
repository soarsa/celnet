//! The `exotic` subcommand: price a digital, one-touch, double-no-touch, or
//! single-barrier option from command-line inputs.
//!
//! Every price is the corresponding closed form in `celnet-exotics` (the
//! Reiner-Rubinstein single barrier, the cash-or-nothing digital, and the
//! reflection-principle touch / double-no-touch formulae). This module only maps
//! the chosen variant and inputs onto those functions and formats the result.

use celnet_core::FlatSmile;
use celnet_exotics::{
    AnalyticAsian, AveragingSchedule, Cliquet, CliquetMcConfig, CliquetSchedule, DigitalKind,
    DoubleNoTouch, ForwardStart, QuantoParams, RebateTiming, SingleBarrier, VarSwapContext,
    cliquet_price_capped_mc, cliquet_price_plain, curran_price, digital_price,
    double_no_touch_price, fair_variance, fair_volatility, forward_start_price, one_touch_price,
    quanto_digital_price, quanto_vanilla_price, single_barrier_price, turnbull_wakeman_price,
};
use celnet_types::{OptionType, VanillaInputs};

use crate::args::CliBarrier;

/// Which exotic to price and its variant-specific parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ExoticSpec {
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
                price: cliquet_price_plain(inputs, &spec),
                std_error: None,
            };
        }
        let estimate = cliquet_price_capped_mc(
            inputs,
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
    let price = match spec {
        ExoticSpec::Digital(kind) => digital_price(kind, inputs),
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
            one_touch_price(inputs, barrier, rebate, timing)
        }
        ExoticSpec::DoubleNoTouch {
            lower,
            upper,
            rebate,
        } => double_no_touch_price(inputs, DoubleNoTouch::new(lower, upper, rebate)),
        ExoticSpec::SingleBarrier {
            option,
            topology,
            strike,
            barrier,
            rebate,
        } => single_barrier_price(
            inputs,
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
            let asian_inputs = VanillaInputs { strike, ..*inputs };
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
            inputs,
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
        // Handled up front (it is the only Monte-Carlo / std-error-carrying arm).
        ExoticSpec::Cliquet { .. } => unreachable!("cliquet is handled before this match"),
    };
    ExoticResult {
        price,
        std_error: None,
    }
}

/// Render an [`ExoticResult`] as a one-line report with a variant label.
#[must_use]
pub(crate) fn format_report(spec: ExoticSpec, r: &ExoticResult) -> String {
    let label = match spec {
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
        let direct = digital_price(kind, &inputs());
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
            &inputs(),
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
        let direct = one_touch_price(&inputs(), 1.20, 1.0, RebateTiming::AtHit);
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
        let direct = double_no_touch_price(&inputs(), DoubleNoTouch::new(1.00, 1.20, 1.0));
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
            &VanillaInputs { strike: 1.10, ..i },
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
            &VanillaInputs { strike: 1.12, ..i },
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
            &i,
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
                &i,
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
        let direct = cliquet_price_capped_mc(&i, &spec, CliquetMcConfig { pairs, seed });
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
}
