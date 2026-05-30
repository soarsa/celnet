//! Path-dependent payoff specifications shared by the PDE and Monte-Carlo
//! engines.
//!
//! A payoff here is a *terminal* functional of a discretely-observed spot path
//! (plus, for barriers, a continuous-monitoring flag the engines resolve with
//! their respective bias corrections). Keeping the payoff types engine-agnostic
//! lets the same specification be priced three ways — closed form (where one
//! exists), PDE, and Monte-Carlo — and cross-validated on the overlap.

use celnet_types::OptionType;

/// A single-barrier, discretely- or continuously-monitored knock specification
/// used to drive the numerical engines.
///
/// This mirrors the analytic [`crate::SingleBarrier`] vocabulary but is the form
/// the path engines consume: it carries the monitoring schedule implicitly
/// through the engine's grid/step count rather than an explicit observation set,
/// matching the continuously-monitored closed form when the grid is refined.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiscreteBarrier {
    /// Call or put underlying.
    pub option: OptionType,
    /// Strike `K`.
    pub strike: f64,
    /// Barrier level `H`.
    pub barrier: f64,
    /// `true` if the barrier sits above spot (an *up* barrier), else *down*.
    pub up: bool,
    /// Knock-in (`true`) activates the option on a touch; knock-out (`false`)
    /// extinguishes it.
    pub knock_in: bool,
}

impl DiscreteBarrier {
    /// Apply the terminal payoff given the realised terminal spot and whether
    /// the barrier was touched over the monitored path.
    ///
    /// A knock-out pays the vanilla payoff only if **never** touched; a knock-in
    /// pays it only if touched **at least once**.
    #[must_use]
    pub fn terminal(&self, terminal_spot: f64, touched: bool) -> f64 {
        let alive = if self.knock_in { touched } else { !touched };
        if alive {
            vanilla_intrinsic(self.option, terminal_spot, self.strike)
        } else {
            0.0
        }
    }

    /// Has the path (currently at `spot`) breached the barrier?
    #[must_use]
    pub fn is_breach(&self, spot: f64) -> bool {
        if self.up {
            spot >= self.barrier
        } else {
            spot <= self.barrier
        }
    }
}

/// A fixed-strike arithmetic-average-rate (Asian) option on a discrete set of
/// equally-weighted observation dates.
///
/// The payoff is `(φ·(Ā − K))⁺` with `Ā` the arithmetic mean of the observed
/// spots. There is no exact closed form, which is precisely why the Monte-Carlo
/// engine pairs it with its *geometric*-average twin (which **does** have a
/// closed form) as a control variate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArithmeticAsian {
    /// Call or put.
    pub option: OptionType,
    /// Strike `K`.
    pub strike: f64,
    /// Number of equally-spaced averaging observations (excludes `t=0`).
    pub observations: usize,
}

impl ArithmeticAsian {
    /// Terminal payoff from the arithmetic average of the observed spots.
    #[must_use]
    pub fn terminal(&self, arithmetic_average: f64) -> f64 {
        vanilla_intrinsic(self.option, arithmetic_average, self.strike)
    }
}

/// Vanilla call/put intrinsic `(φ·(x − K))⁺`.
#[inline]
#[must_use]
pub fn vanilla_intrinsic(option: OptionType, x: f64, strike: f64) -> f64 {
    (option.sign() * (x - strike)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    #[test]
    fn vanilla_intrinsic_signs() {
        assert_close!(vanilla_intrinsic(OptionType::Call, 110.0, 100.0), 10.0);
        assert_close!(vanilla_intrinsic(OptionType::Call, 90.0, 100.0), 0.0);
        assert_close!(vanilla_intrinsic(OptionType::Put, 90.0, 100.0), 10.0);
        assert_close!(vanilla_intrinsic(OptionType::Put, 110.0, 100.0), 0.0);
    }

    #[test]
    fn knock_out_alive_only_if_untouched() {
        let b = DiscreteBarrier {
            option: OptionType::Call,
            strike: 100.0,
            barrier: 120.0,
            up: true,
            knock_in: false,
        };
        assert_close!(b.terminal(115.0, false), 15.0);
        assert_close!(b.terminal(115.0, true), 0.0);
    }

    #[test]
    fn knock_in_alive_only_if_touched() {
        let b = DiscreteBarrier {
            option: OptionType::Call,
            strike: 100.0,
            barrier: 120.0,
            up: true,
            knock_in: true,
        };
        assert_close!(b.terminal(115.0, true), 15.0);
        assert_close!(b.terminal(115.0, false), 0.0);
    }

    #[test]
    fn breach_direction() {
        let up = DiscreteBarrier {
            option: OptionType::Call,
            strike: 100.0,
            barrier: 120.0,
            up: true,
            knock_in: false,
        };
        assert!(up.is_breach(121.0));
        assert!(!up.is_breach(119.0));
        let down = DiscreteBarrier {
            up: false,
            barrier: 80.0,
            ..up
        };
        assert!(down.is_breach(79.0));
        assert!(!down.is_breach(81.0));
    }
}
