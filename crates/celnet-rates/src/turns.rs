//! Turn-of-year and central-bank-meeting forward jumps — a localized forward overlay on a curve.
//!
//! Funding markets price a sharp, short-lived dislocation in the instantaneous forward across a
//! year-end (the "turn") and across scheduled central-bank meeting dates: a near-vertical spike
//! confined to a one-/two-day window, on top of the smooth term structure (`FI-CURVES-SPEC.md`
//! §4.1). This module bakes such jumps into a discount curve **without changing the hot
//! interpolation path**: it re-samples a base log-linear curve at the original pillars plus each
//! jump's window boundaries, multiplying every discount factor by the jump's discounting impact,
//! then rebuilds an ordinary [`Curve`] through [`Curve::from_log_linear_dfs`].
//!
//! ## Construction
//!
//! A [`TurnJump`] adds a constant continuously-compounded instantaneous forward `size` over
//! `[start, end]`. Its effect on the discount factor at time `t` is a multiplicative
//! `exp(−size · overlap(t))`, where `overlap(t) = max(0, min(t, end) − start)` is the time `t` has
//! accrued inside the window. Jumps compose additively in the exponent. Because the boundaries
//! `start` and `end` are inserted as pillars, the rebuilt log-linear curve carries:
//!
//! - discount factors **before** the window unchanged (`overlap = 0`),
//! - the instantaneous forward **inside** the window raised by exactly `size`,
//! - discount factors **after** the window scaled by exactly `exp(−size · (end − start))`.
//!
//! A negative `size` models an inverted turn (a forward dip). The overlay is exact and
//! allocation-light: it touches only curve *construction*, never the zero-alloc query path.
//!
//! Method/paper provenance lives in prose only — never in identifiers (GUIDE.md §8).

use crate::curve::{Curve, CurveError};
use celnet_types::{Df, Time};

/// Coincidence tolerance for merging a jump boundary with an existing pillar time.
const MERGE_TOL: f64 = 1e-9;

/// A localized forward jump: a constant extra instantaneous forward `size` over `[start, end]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnJump {
    /// Window start (curve time, years from the origin).
    pub start: Time,
    /// Window end (curve time, years from the origin).
    pub end: Time,
    /// Continuously-compounded instantaneous-forward size added over the window (may be negative).
    pub size: f64,
}

impl TurnJump {
    /// The time `t` has accrued inside this jump's window: `max(0, min(t, end) − start)`.
    fn overlap(&self, t: f64) -> f64 {
        (t.min(self.end.0) - self.start.0).max(0.0)
    }
}

/// Failure modes of applying turn jumps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TurnError {
    /// A jump window is not strictly increasing and non-negative (`0 <= start < end`).
    InvalidWindow,
    /// A jump window falls outside the base curve's `[0, max_time]` range.
    OutsideCurve,
    /// The rebuilt curve was rejected.
    Curve(CurveError),
}

impl core::fmt::Display for TurnError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidWindow => f.write_str("a turn window must satisfy 0 <= start < end"),
            Self::OutsideCurve => f.write_str("a turn window falls outside the base curve range"),
            Self::Curve(e) => write!(f, "turn-adjusted curve rejected: {e}"),
        }
    }
}

impl core::error::Error for TurnError {}

/// Build a curve from base log-linear discount-factor pillars with `turns` overlaid.
///
/// The base pillars define a log-linear-on-log-DF curve (as [`Curve::from_log_linear_dfs`]); each
/// [`TurnJump`] is then baked in exactly by re-sampling at the original pillars plus the jump
/// boundaries and applying the multiplicative discounting impact. With no turns the result is the
/// base curve, sampled at its own pillars (identical).
///
/// # Errors
///
/// Returns [`TurnError::InvalidWindow`] for a degenerate window, [`TurnError::OutsideCurve`] when a
/// window leaves the base range, or [`TurnError::Curve`] if the base or rebuilt curve is rejected.
pub fn with_turns(base_pillars: &[(Time, Df)], turns: &[TurnJump]) -> Result<Curve, TurnError> {
    let base = Curve::from_log_linear_dfs(base_pillars).map_err(TurnError::Curve)?;
    let max_t = base.max_time().0;

    for turn in turns {
        if !(turn.start.0 >= 0.0 && turn.end.0 > turn.start.0) {
            return Err(TurnError::InvalidWindow);
        }
        if turn.end.0 > max_t + MERGE_TOL {
            return Err(TurnError::OutsideCurve);
        }
    }

    // Merge the original pillar times with every jump boundary, sorted and de-duplicated.
    let mut times: Vec<f64> = base_pillars.iter().map(|(t, _)| t.0).collect();
    for turn in turns {
        times.push(turn.start.0);
        times.push(turn.end.0);
    }
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() <= MERGE_TOL);

    // Re-sample the base curve and apply each jump's multiplicative discounting impact.
    let pillars: Vec<(Time, Df)> = times
        .into_iter()
        .map(|t| {
            let base_df = base.discount_factor(Time(t)).0;
            let exponent: f64 = turns.iter().map(|turn| -turn.size * turn.overlap(t)).sum();
            (Time(t), Df(base_df * exponent.exp()))
        })
        .collect();

    Curve::from_log_linear_dfs(&pillars).map_err(TurnError::Curve)
}

/// The aggregate multiplicative discount-factor impact of `turns` at time `t`:
/// `exp(−Σ size · overlap(t))`. Equal to `1` before all windows and to the full
/// `exp(−Σ size · width)` beyond them.
#[must_use]
pub fn turn_discount_factor(turns: &[TurnJump], t: Time) -> f64 {
    let exponent: f64 = turns
        .iter()
        .map(|turn| -turn.size * turn.overlap(t.0))
        .sum();
    exponent.exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A smooth base curve from continuously-compounded zero rates, expressed as DF pillars.
    fn base_pillars() -> Vec<(Time, Df)> {
        [
            (0.0, 0.040),
            (0.5, 0.041),
            (1.0, 0.042),
            (2.0, 0.043),
            (5.0, 0.044),
        ]
        .into_iter()
        .map(|(t, z)| (Time(t), Df((-z * t).exp())))
        .collect()
    }

    fn year_end_turn() -> TurnJump {
        // A +50bp/yr funding spike confined to a two-day window straddling 1.0y.
        TurnJump {
            start: Time(0.995),
            end: Time(1.0),
            size: 0.50,
        }
    }

    #[test]
    fn no_turns_reproduces_the_base_curve() {
        let pillars = base_pillars();
        let base = Curve::from_log_linear_dfs(&pillars).unwrap();
        let curved = with_turns(&pillars, &[]).unwrap();
        for &t in &[0.25, 0.75, 1.5, 3.0, 4.9] {
            assert!(
                (curved.discount_factor(Time(t)).0 - base.discount_factor(Time(t)).0).abs() < 1e-14,
                "t {t}"
            );
        }
    }

    #[test]
    fn discount_factors_before_the_turn_are_unchanged() {
        let pillars = base_pillars();
        let base = Curve::from_log_linear_dfs(&pillars).unwrap();
        let curved = with_turns(&pillars, &[year_end_turn()]).unwrap();
        for &t in &[0.1, 0.5, 0.9, 0.99] {
            assert!(
                (curved.discount_factor(Time(t)).0 - base.discount_factor(Time(t)).0).abs() < 1e-12,
                "t {t}"
            );
        }
    }

    #[test]
    fn discount_factors_after_the_turn_scale_by_the_jump_factor() {
        let pillars = base_pillars();
        let base = Curve::from_log_linear_dfs(&pillars).unwrap();
        let turn = year_end_turn();
        let curved = with_turns(&pillars, &[turn]).unwrap();
        let scale = (-turn.size * (turn.end.0 - turn.start.0)).exp();
        for &t in &[1.0, 1.5, 2.0, 5.0] {
            let expected = base.discount_factor(Time(t)).0 * scale;
            assert!(
                (curved.discount_factor(Time(t)).0 - expected).abs() < 1e-12,
                "t {t}: {} vs {expected}",
                curved.discount_factor(Time(t)).0
            );
        }
    }

    #[test]
    fn forward_inside_the_window_is_raised_by_the_jump_size() {
        let pillars = base_pillars();
        let base = Curve::from_log_linear_dfs(&pillars).unwrap();
        let turn = year_end_turn();
        let curved = with_turns(&pillars, &[turn]).unwrap();
        let base_fwd = base.forward_rate_continuous(turn.start, turn.end).0;
        let bumped_fwd = curved.forward_rate_continuous(turn.start, turn.end).0;
        assert!(
            (bumped_fwd - base_fwd - turn.size).abs() < 1e-9,
            "base {base_fwd} bumped {bumped_fwd}"
        );
    }

    #[test]
    fn inverted_turn_lowers_the_window_forward() {
        let pillars = base_pillars();
        let base = Curve::from_log_linear_dfs(&pillars).unwrap();
        let dip = TurnJump {
            start: Time(0.995),
            end: Time(1.0),
            size: -0.30,
        };
        let curved = with_turns(&pillars, &[dip]).unwrap();
        let base_fwd = base.forward_rate_continuous(dip.start, dip.end).0;
        let bumped_fwd = curved.forward_rate_continuous(dip.start, dip.end).0;
        assert!(bumped_fwd < base_fwd, "base {base_fwd} bumped {bumped_fwd}");
    }

    #[test]
    fn multiple_turns_compose_additively_after_both_windows() {
        let pillars = base_pillars();
        let base = Curve::from_log_linear_dfs(&pillars).unwrap();
        let turns = [
            TurnJump {
                start: Time(0.995),
                end: Time(1.0),
                size: 0.50,
            },
            TurnJump {
                start: Time(1.995),
                end: Time(2.0),
                size: 0.30,
            },
        ];
        let curved = with_turns(&pillars, &turns).unwrap();
        // Beyond both windows, both jump factors apply.
        let scale = (-(0.50_f64 * 0.005 + 0.30 * 0.005)).exp();
        let expected = base.discount_factor(Time(3.0)).0 * scale;
        assert!((curved.discount_factor(Time(3.0)).0 - expected).abs() < 1e-12);
        assert!((turn_discount_factor(&turns, Time(3.0)) - scale).abs() < 1e-14);
        // Before either window the aggregate impact is unity.
        assert!((turn_discount_factor(&turns, Time(0.5)) - 1.0).abs() < 1e-14);
    }

    #[test]
    fn rejects_degenerate_and_out_of_range_windows() {
        let pillars = base_pillars();
        assert_eq!(
            with_turns(
                &pillars,
                &[TurnJump {
                    start: Time(1.0),
                    end: Time(1.0),
                    size: 0.5,
                }]
            )
            .unwrap_err(),
            TurnError::InvalidWindow
        );
        assert_eq!(
            with_turns(
                &pillars,
                &[TurnJump {
                    start: Time(4.9),
                    end: Time(6.0),
                    size: 0.5,
                }]
            )
            .unwrap_err(),
            TurnError::OutsideCurve
        );
    }
}
