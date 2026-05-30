//! The `exotic` subcommand: price a digital, one-touch, double-no-touch, or
//! single-barrier option from command-line inputs.
//!
//! Every price is the corresponding closed form in `celnet-exotics` (the
//! Reiner-Rubinstein single barrier, the cash-or-nothing digital, and the
//! reflection-principle touch / double-no-touch formulae). This module only maps
//! the chosen variant and inputs onto those functions and formats the result.

use celnet_exotics::{
    DigitalKind, DoubleNoTouch, RebateTiming, SingleBarrier, digital_price, double_no_touch_price,
    one_touch_price, single_barrier_price,
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
    };
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
}
