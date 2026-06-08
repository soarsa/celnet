//! Settlement-style routing for digital-asset (crypto) vanillas.
//!
//! Crypto introduces a denomination axis FX never had: a vanilla on the same
//! `(S, K, σ, t, carry)` market state settles either
//!
//! * **Linear** (USD(T)-margined) — premium and payoff in USD(T); the
//!   asset-class-agnostic generalized-BSM path ([`crate::linear`]); or
//! * **InverseCoin** (coin-margined) — premium and payoff in the base coin, with
//!   the `1/S_T` convexity transform ([`crate::inverse`]).
//!
//! [`SettlementStyle`] is the local routing discriminator. It is the *settlement*
//! axis, NOT a cost-of-carry variant: both styles share the identical carry seam
//! and forward/discount; only the payoff denomination differs (ADR-0008 — the
//! inverse is a genuinely new payoff *shape*, the linear is the shared agnostic
//! engine). [`route_price`] dispatches the coin/USD price; a desk picks the unit by
//! the contract spec it is pricing.
//!
//! The wire/`celnet-types` `SettlementStyle` arm and the `Underlying::DigitalAsset`
//! seam are the coordinator's contract change (W3 §2); this leaf-local enum lets the
//! crate gate standalone today and is the obvious lowering target for that wire arm.

use celnet_types::{Carry, OptionType};

/// How a crypto vanilla settles — the premium/payoff denomination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementStyle {
    /// USD(T)-margined: premium and payoff in USD(T). Generalized-BSM.
    Linear,
    /// Coin-margined: premium and payoff in the base coin, with the `1/S_T`
    /// convexity transform (the classic Deribit inverse contract).
    InverseCoin,
}

/// Price a crypto vanilla in its **settlement unit**: USD per USD-notional-1 for
/// [`SettlementStyle::Linear`], coins per USD-notional-1 for
/// [`SettlementStyle::InverseCoin`]. Routes to the linear generalized-BSM engine or
/// the inverse coin closed form — there is no silent fallback between them, because
/// they are different units.
#[must_use]
pub fn route_price(
    style: SettlementStyle,
    opt: OptionType,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    carry: Carry,
) -> f64 {
    match style {
        SettlementStyle::Linear => crate::linear::price(
            opt,
            &crate::linear::LinearInputs::new(spot, strike, vol, t, carry),
        ),
        SettlementStyle::InverseCoin => crate::inverse::price(
            opt,
            &crate::inverse::InverseInputs::new(spot, strike, vol, t, carry),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carry() -> Carry {
        crate::funding::funding_carry(0.05, 0.02)
    }

    /// `route_price` dispatches to the matching engine in the matching unit, and the
    /// two units genuinely differ (the inverse is NOT a rescale of the linear).
    #[test]
    fn routes_to_the_matching_engine() {
        let (s, k, vol, t) = (30_000.0, 31_000.0, 0.65, 0.5);
        let lin = route_price(
            SettlementStyle::Linear,
            OptionType::Call,
            s,
            k,
            vol,
            t,
            carry(),
        );
        let inv = route_price(
            SettlementStyle::InverseCoin,
            OptionType::Call,
            s,
            k,
            vol,
            t,
            carry(),
        );
        // Linear == the linear engine (USD); inverse == the inverse engine (coins).
        assert_eq!(
            lin.to_bits(),
            crate::linear::price(
                OptionType::Call,
                &crate::linear::LinearInputs::new(s, k, vol, t, carry())
            )
            .to_bits()
        );
        assert_eq!(
            inv.to_bits(),
            crate::inverse::price(
                OptionType::Call,
                &crate::inverse::InverseInputs::new(s, k, vol, t, carry())
            )
            .to_bits()
        );
        // Different units: inverse·S ≠ linear (convexity), so the routes are distinct.
        assert!((inv * s - lin).abs() > 1e-6);
    }
}
