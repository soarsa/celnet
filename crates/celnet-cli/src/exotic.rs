//! The `exotic` subcommand: price a digital, one-touch, double-no-touch, or
//! single-barrier option from command-line inputs.
//!
//! Every price is the corresponding closed form in `celnet-exotics` (the
//! Reiner-Rubinstein single barrier, the cash-or-nothing digital, and the
//! reflection-principle touch / double-no-touch formulae). This module only maps
//! the chosen variant and inputs onto those functions and formats the result.

use celnet_core::FlatSmile;
use celnet_exotics::{
    AnalyticAsian, AveragingSchedule, DigitalKind, DoubleNoTouch, RebateTiming, SingleBarrier,
    VarSwapContext, curran_price, digital_price, double_no_touch_price, fair_variance,
    fair_volatility, one_touch_price, single_barrier_price, turnbull_wakeman_price,
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
}

/// The result of an `exotic` run: the priced present value and a label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ExoticResult {
    /// Present value (domestic premium).
    pub(crate) price: f64,
}

/// Price the chosen exotic against the supplied Garman-Kohlhagen market inputs.
///
/// `inputs` carries spot, vol, time, and the two rates; the strike inside
/// `inputs` is used by the digital (whose payout is struck at `inputs.strike`),
/// while the single barrier takes its own strike from the spec. Touch products
/// ignore the strike.
#[must_use]
pub(crate) fn run(spec: ExoticSpec, inputs: &VanillaInputs) -> ExoticResult {
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
    };
    ExoticResult { price }
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
}
