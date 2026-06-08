//! Funding-carry assembly for digital-asset (crypto) options.
//!
//! A crypto option's net cost of carry is driven by the **funding** the base
//! coin earns (the perpetual-swap funding rate, a staking/lending yield, or a
//! borrow cost). It plugs into the *same* asset-class-agnostic cost-of-carry seam
//! ([`celnet_types::Carry::CostOfCarry`]) every other leaf uses, ADR-0008
//! §Decision-2: funding is **not** a new `Carry` variant — it is just how the net
//! carry `b` is assembled before the forward `F = S·e^{b·t}` and the discount
//! `e^{−r·t}` are formed:
//!
//! ```text
//! b = r − funding                 (net cost of carry)
//! ```
//!
//! where `r` is the (continuously-compounded) USD numeraire discount rate and
//! `funding` is the continuously-compounded coin funding/lease yield. Storage has
//! no meaning for a digital asset; a positive funding (the coin earns yield)
//! *lowers* the forward exactly like a dividend yield lowers an equity forward,
//! and a negative funding (it costs to hold/short) raises it.
//!
//! ## Honest boundary (W3 §9, deploy-bound carve-out)
//!
//! The carry **assembly** `b = r − funding` and its sensitivities are in-repo and
//! gated. The **live perpetual-funding number** itself is exchange/venue data
//! (ENV) — seamed here as the `funding` input, validated at deploy, never a
//! fabricated constant in-repo.

use celnet_types::Carry;

/// Assemble the crypto cost-of-carry `Carry::CostOfCarry { r, b = r − funding }`.
///
/// `r` is the continuously-compounded USD numeraire discount rate; `funding` is the
/// continuously-compounded coin funding/lease yield (positive = the coin earns
/// yield, lowering the forward). This is the *only* place the funding is combined
/// into the net carry, so every downstream quantity reads `b` from the seam and no
/// pricer branches on a funding-vs-dividend-vs-convenience discriminator
/// (ADR-0008 "no-match-carry" rule).
#[must_use]
pub fn funding_carry(r: f64, funding: f64) -> Carry {
    Carry::CostOfCarry { r, b: r - funding }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `b = r − funding`, and the seam reproduces the forward/discount off that `b`.
    #[test]
    fn funding_assembles_net_carry() {
        let r = 0.05;
        let funding = 0.12; // a high coin funding/lease yield
        let carry = funding_carry(r, funding);
        assert_eq!(carry.discount_rate(), r);
        assert_eq!(carry.carry_rate(), r - funding);
        // The forward factor is e^{(r−funding)·t}, the discount e^{−r·t}.
        let t = 0.5;
        assert_eq!(
            carry.forward_factor(t).to_bits(),
            celnet_core::math::exp((r - funding) * t).to_bits()
        );
        assert_eq!(
            carry.discount_df(t).to_bits(),
            celnet_core::math::exp(-r * t).to_bits()
        );
    }

    /// `funding = r ⇒ b = 0`: the forward equals spot (a martingale-forward / Black-76
    /// degenerate), the way a coin whose yield exactly offsets the discount rate has a
    /// flat forward.
    #[test]
    fn funding_equal_to_r_gives_zero_carry() {
        let carry = funding_carry(0.07, 0.07);
        assert_eq!(carry.carry_rate(), 0.0);
        assert_eq!(carry.forward_factor(2.0).to_bits(), 1.0_f64.to_bits());
    }
}
