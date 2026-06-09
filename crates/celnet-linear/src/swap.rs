//! FX swap — a near leg plus a far leg, each an outright forward, traded in
//! opposite directions by market convention.
//!
//! An FX swap exchanges one currency for another on a near date and reverses the
//! exchange on a far date. Priced as two outright forwards: the near leg at the
//! input's [`crate::LinearInputs::side`] settling at `near_settle_t`, and the far
//! leg at the **opposite** side settling at `far_settle_t`. Its PV is the sum of
//! the two leg PVs — there is no swap-specific math, so the engine reuses the
//! forward leg pricer unchanged (asset-class-agnostic per ADR-0008).

use crate::forward::pv_at;
use crate::inputs::LinearInputs;

/// Why a swap could not be priced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapError {
    /// The input carried no far settlement time — a swap needs both legs.
    MissingFarLeg,
}

impl core::fmt::Display for SwapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SwapError::MissingFarLeg => {
                f.write_str("an FX swap requires a far settlement time (far_settle_t)")
            }
        }
    }
}

impl core::error::Error for SwapError {}

/// The two outright-forward leg rates of an FX swap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwapLegRates {
    /// The near-leg outright forward `spot · forward_factor(near_settle_t)`.
    pub near_forward: f64,
    /// The far-leg outright forward `spot · forward_factor(far_settle_t)`.
    pub far_forward: f64,
}

/// Present value of an FX swap: near leg (input side) + far leg (opposite side).
///
/// `PV = pv_at(near, side, t_near) + pv_at(far, opposite(side), t_far)`. The near
/// leg uses the input's contract rate and side at `near_settle_t`; the far leg
/// uses the same contract rate at the **opposite** side and `far_settle_t`.
///
/// # Errors
/// Returns [`SwapError::MissingFarLeg`] if the input has no `far_settle_t`.
pub fn pv(inputs: &LinearInputs) -> Result<f64, SwapError> {
    let far_t = inputs.far_settle_t.ok_or(SwapError::MissingFarLeg)?;
    // Near leg: input side at the near settlement time.
    let near_pv = pv_at(inputs, inputs.near_settle_t);
    // Far leg: opposite side at the far settlement time. Same market state &
    // contract rate; only the side and settlement time change.
    let far_leg = LinearInputs {
        side: inputs.side.opposite(),
        ..inputs.clone()
    };
    let far_pv = pv_at(&far_leg, far_t);
    Ok(near_pv + far_pv)
}

/// The two leg forward rates of the swap (near and far outrights).
///
/// The **swap points** — the forward-points spread between the two legs — are
/// `far_forward − near_forward` (see [`swap_points`]).
///
/// # Errors
/// Returns [`SwapError::MissingFarLeg`] if the input has no `far_settle_t`.
pub fn leg_rates(inputs: &LinearInputs) -> Result<SwapLegRates, SwapError> {
    let far_t = inputs.far_settle_t.ok_or(SwapError::MissingFarLeg)?;
    Ok(SwapLegRates {
        near_forward: inputs.forward(inputs.near_settle_t),
        far_forward: inputs.forward(far_t),
    })
}

/// The swap points: the far outright forward minus the near outright forward.
///
/// Positive when the far forward is above the near (a forward premium over the
/// near leg), which by the carry identity occurs when the net carry `b` is
/// positive (the base currency trades at a forward premium).
///
/// # Errors
/// Returns [`SwapError::MissingFarLeg`] if the input has no `far_settle_t`.
pub fn swap_points(inputs: &LinearInputs) -> Result<f64, SwapError> {
    let r = leg_rates(inputs)?;
    Ok(r.far_forward - r.near_forward)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forward;
    use crate::inputs::Side;
    use celnet_types::{Carry, CcyPair, Underlying};

    fn eurusd() -> Underlying {
        Underlying::Fx(CcyPair::parse("EURUSD").unwrap())
    }

    /// An outright-forward leg (`far_settle_t = None`).
    fn leg(spot: f64, k: f64, n: f64, side: Side, r_dom: f64, r_for: f64, t: f64) -> LinearInputs {
        LinearInputs::outright(
            spot,
            eurusd(),
            Carry::FxRates { r_dom, r_for },
            crate::inputs::LinearTerms::new(k, n, side),
            t,
        )
        .unwrap()
    }

    /// A two-legged swap input: an outright `leg` with a far settlement time set.
    fn swap_inp(near: LinearInputs, far_t: f64) -> LinearInputs {
        near.with_far(far_t).unwrap()
    }

    #[test]
    fn missing_far_leg_is_rejected() {
        let li = leg(1.2345, 1.30, 1e6, Side::Buy, 0.04, 0.01, 0.25);
        assert_eq!(pv(&li), Err(SwapError::MissingFarLeg));
        assert_eq!(swap_points(&li), Err(SwapError::MissingFarLeg));
    }

    /// Structural: near == far date with opposite sides nets to exactly 0
    /// (the two legs are identical forwards at opposite signs).
    #[test]
    fn equal_dates_opposite_legs_net_to_zero() {
        let t = 0.5;
        let li = swap_inp(leg(1.2345, 1.30, 1e6, Side::Buy, 0.04, 0.01, t), t);
        assert_eq!((pv(&li).unwrap() + 0.0).to_bits(), 0.0_f64.to_bits());
    }

    /// INDEPENDENT two-leg sum: the swap PV equals the sum of two separately
    /// constructed outright forwards (near at input side, far at opposite side),
    /// each priced by the standalone `forward::pv` — a different assembly than
    /// the in-swap `pv_at`/struct-update route.
    #[test]
    fn pv_equals_independent_two_leg_sum() {
        let cases = [
            (1.2345, 1.30, 1e6, Side::Buy, 0.04, 0.01, 0.25, 0.75),
            (150.0, 145.0, 2e6, Side::Sell, 0.005, 0.02, 0.5, 1.5),
            (0.65, 0.70, 3e6, Side::Buy, 0.03, 0.01, 0.1, 2.0),
        ];
        for (s, k, n, side, rd, rf, nt, ft) in cases {
            let near = leg(s, k, n, side, rd, rf, nt);
            let swap = swap_inp(near.clone(), ft);
            // Independent legs as standalone outright forwards.
            let far = leg(s, k, n, side.opposite(), rd, rf, ft);
            let independent = forward::pv(&near) + forward::pv(&far);
            assert!(
                (pv(&swap).unwrap() - independent).abs() <= 1e-9 * independent.abs().max(1.0),
                "swap {} vs two-leg {independent}",
                pv(&swap).unwrap()
            );
        }
    }

    /// Swap points carry the sign of the net carry `b = r_dom − r_for`:
    /// positive carry ⇒ far forward above near ⇒ positive swap points.
    #[test]
    fn swap_points_sign_matches_carry() {
        // Positive carry (r_dom > r_for) ⇒ positive points.
        let pos = swap_inp(leg(1.2345, 1.30, 1e6, Side::Buy, 0.05, 0.01, 0.25), 0.75);
        assert!(swap_points(&pos).unwrap() > 0.0);
        // Negative carry (r_dom < r_for) ⇒ negative points.
        let neg = swap_inp(leg(1.2345, 1.30, 1e6, Side::Buy, 0.01, 0.05, 0.25), 0.75);
        assert!(swap_points(&neg).unwrap() < 0.0);
        // Flat carry ⇒ zero points (to_bits: both forwards share spot·e^{0}).
        let flat = swap_inp(leg(1.2345, 1.30, 1e6, Side::Buy, 0.03, 0.03, 0.25), 0.75);
        assert_eq!(
            (swap_points(&flat).unwrap() + 0.0).to_bits(),
            0.0_f64.to_bits()
        );
    }
}
