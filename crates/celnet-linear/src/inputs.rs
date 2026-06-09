//! The linear-book pricing input and the buy/sell side discriminator.
//!
//! [`LinearInputs`] is the discounted-cashflow analogue of the option leaf's
//! [`celnet_core::CarryInputs`]: it reads the **same** carry seam (an
//! [`Underlying`] identity plus a [`Carry`] forward/discount producer) but
//! carries the linear-product terms — a contract rate (the agreed forward
//! strike), a positive notional, a [`Side`], a near settlement time and an
//! optional far settlement time (used by the swap's far leg).
//!
//! Because the forward and discount are taken **only** through
//! [`Carry::forward_factor`] / [`Carry::discount_df`] (never by matching on the
//! [`Carry`] variant), every pricer in this crate is asset-class-agnostic per
//! ADR-0008: an NDF or forward on a (future) metal or digital-asset underlying
//! reuses the identical engine unchanged.

use celnet_types::{Carry, Underlying};

/// Buy (long the base / receive the underlying) or sell (short / pay) side of a
/// linear contract.
///
/// `celnet-types` carries no buy/sell discriminator (the FX *option* leaf prices
/// a payoff regardless of book direction), so the linear book defines its own:
/// the side enters the PV purely as a `±1` multiplier via [`Side::sign`]. A buy
/// is long the forward (`+1`); a sell is short (`−1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Buy: long the base currency forward (receive base, pay quote at maturity).
    Buy,
    /// Sell: short the base currency forward (pay base, receive quote).
    Sell,
}

impl Side {
    /// The signed multiplier the side contributes to a PV: `+1` for [`Side::Buy`],
    /// `−1` for [`Side::Sell`].
    ///
    /// A linear PV is `side · notional · df · (F − K)`; the sign is the **only**
    /// way the side enters, so a buy and a sell of identical terms net to exactly
    /// zero (gated in `forward.rs`).
    #[must_use]
    pub const fn sign(self) -> f64 {
        match self {
            Side::Buy => 1.0,
            Side::Sell => -1.0,
        }
    }

    /// The opposite side: [`Side::Buy`] ↔ [`Side::Sell`].
    ///
    /// Used to form a swap's far leg, which trades in the opposite direction to
    /// the near leg by market convention.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

/// Why a [`LinearInputs`] could not be constructed.
///
/// Construction validates the economic preconditions a linear product requires;
/// an invalid input is rejected with a typed error rather than silently producing
/// a nonsensical PV (no silent mis-pricing, per ADR-0008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinearInputError {
    /// The notional was not strictly positive (a non-positive notional has no
    /// economic meaning; direction is carried by [`Side`], not by the sign of the
    /// notional).
    NonPositiveNotional,
    /// A settlement time was negative (a forward settles at or after the trade
    /// horizon; `t = 0` is the degenerate spot-settling case and is permitted).
    NegativeSettleTime,
}

impl core::fmt::Display for LinearInputError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LinearInputError::NonPositiveNotional => {
                f.write_str("linear notional must be strictly positive")
            }
            LinearInputError::NegativeSettleTime => {
                f.write_str("linear settlement time must be non-negative")
            }
        }
    }
}

impl core::error::Error for LinearInputError {}

/// The market state + contract terms of a linear (discounted-cashflow) FX
/// product, reading the W1 carry seam.
///
/// The market half — `spot`, the [`Underlying`] identity and the [`Carry`]
/// forward/discount producer — is the same vocabulary the option leaf consumes;
/// the contract half adds the agreed forward strike (`contract_rate`), the
/// `notional`, the [`Side`], the near settlement time `near_settle_t` and an
/// optional `far_settle_t` for the far leg of a swap.
///
/// The forward and discount are taken **only** through the carry accessors
/// ([`LinearInputs::forward`] / [`LinearInputs::discount_df`]), never by matching
/// on the [`Carry`] variant — this is the asset-class-agnostic discipline
/// (ADR-0008 §the no-workaround test).
///
/// `LinearInputs` is `Clone` but not `Copy`: the [`Underlying`] identity now
/// carries string-bearing cross-asset arms (equity / commodity / digital-asset
/// symbols), which cannot be `Copy`. The pricing functions take `&LinearInputs`,
/// so the hot path never copies the inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearInputs {
    /// Spot price of the underlying (quote per 1 unit of base, for FX).
    pub spot: f64,
    /// The agreed forward (contract) rate `K` — the strike of the linear payoff.
    pub contract_rate: f64,
    /// Notional, in units of the base currency. Strictly positive (validated);
    /// direction is carried by [`Side`].
    pub notional: f64,
    /// Buy or sell side.
    pub side: Side,
    /// Near settlement time in years (the (single) settlement of a forward/NDF,
    /// or the near leg of a swap).
    pub near_settle_t: f64,
    /// Optional far settlement time in years — the far leg of a swap. `None` for
    /// an outright forward or an NDF.
    pub far_settle_t: Option<f64>,
    /// The underlying asset identity (FX pair today; metal/crypto by their wave).
    pub underlying: Underlying,
    /// The cost-of-carry model behind the forward and discounting.
    pub carry: Carry,
}

/// The contract terms of a linear product, separated from the market state so a
/// constructor stays within a small argument budget.
///
/// Direction ([`Side`]), the agreed forward strike (`contract_rate`) and the
/// `notional` are the contract half; spot/[`Underlying`]/[`Carry`] are the market
/// half passed alongside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearTerms {
    /// The agreed forward (contract) rate `K`.
    pub contract_rate: f64,
    /// Notional in base-currency units (strictly positive, validated).
    pub notional: f64,
    /// Buy or sell side.
    pub side: Side,
}

impl LinearTerms {
    /// Construct linear contract terms.
    #[must_use]
    pub const fn new(contract_rate: f64, notional: f64, side: Side) -> Self {
        Self {
            contract_rate,
            notional,
            side,
        }
    }
}

impl LinearInputs {
    /// Construct and validate a single-settlement linear input (forward / NDF).
    ///
    /// `far_settle_t` is `None`; use [`LinearInputs::with_far`] to add a far leg
    /// for a swap.
    ///
    /// # Errors
    /// Returns [`LinearInputError::NonPositiveNotional`] if `notional <= 0` (or
    /// NaN), and [`LinearInputError::NegativeSettleTime`] if `near_settle_t < 0`.
    pub fn outright(
        spot: f64,
        underlying: Underlying,
        carry: Carry,
        terms: LinearTerms,
        near_settle_t: f64,
    ) -> Result<Self, LinearInputError> {
        Self::validate(terms.notional, near_settle_t, None)?;
        Ok(Self {
            spot,
            contract_rate: terms.contract_rate,
            notional: terms.notional,
            side: terms.side,
            near_settle_t,
            far_settle_t: None,
            underlying,
            carry,
        })
    }

    /// Add a far settlement time to a single-settlement input, forming a swap
    /// input.
    ///
    /// # Errors
    /// Returns [`LinearInputError::NegativeSettleTime`] if `far_settle_t < 0`.
    pub fn with_far(self, far_settle_t: f64) -> Result<Self, LinearInputError> {
        if far_settle_t < 0.0 {
            return Err(LinearInputError::NegativeSettleTime);
        }
        Ok(Self {
            far_settle_t: Some(far_settle_t),
            ..self
        })
    }

    /// Validate the notional and settlement times shared by every constructor.
    fn validate(
        notional: f64,
        near_settle_t: f64,
        far_settle_t: Option<f64>,
    ) -> Result<(), LinearInputError> {
        // Reject a non-positive OR NaN notional (direction is carried by `Side`,
        // not the sign of the notional). Phrased without a negated comparison so
        // NaN — which fails every ordered comparison — is still caught.
        if notional <= 0.0 || notional.is_nan() {
            return Err(LinearInputError::NonPositiveNotional);
        }
        if near_settle_t < 0.0 || far_settle_t.is_some_and(|t| t < 0.0) {
            return Err(LinearInputError::NegativeSettleTime);
        }
        Ok(())
    }

    /// The outright forward `F = spot · forward_factor(t)` to a settlement time
    /// `t`, taken through the [`Carry`] producer (asset-class-agnostic).
    ///
    /// For an FX [`Carry::FxRates`] carry this is byte-identical to
    /// [`celnet_core::CarryInputs::forward`] at the same `t`.
    #[must_use]
    pub fn forward(&self, t: f64) -> f64 {
        self.spot * self.carry.forward_factor(t)
    }

    /// The numeraire discount factor `df = discount_df(t)` to a settlement time
    /// `t`, taken through the [`Carry`] producer (asset-class-agnostic).
    #[must_use]
    pub fn discount_df(&self, t: f64) -> f64 {
        self.carry.discount_df(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::CcyPair;

    fn eurusd() -> Underlying {
        Underlying::Fx(CcyPair::parse("EURUSD").unwrap())
    }

    fn fx() -> Carry {
        Carry::FxRates {
            r_dom: 0.04,
            r_for: 0.01,
        }
    }

    #[test]
    fn side_sign_and_opposite() {
        assert_eq!(Side::Buy.sign(), 1.0);
        assert_eq!(Side::Sell.sign(), -1.0);
        assert_eq!(Side::Buy.opposite(), Side::Sell);
        assert_eq!(Side::Sell.opposite(), Side::Buy);
    }

    #[test]
    fn rejects_non_positive_notional() {
        for n in [0.0, -1.0, -1e9, f64::NAN] {
            assert_eq!(
                LinearInputs::outright(
                    1.1,
                    eurusd(),
                    fx(),
                    LinearTerms::new(1.1, n, Side::Buy),
                    0.5,
                ),
                Err(LinearInputError::NonPositiveNotional)
            );
        }
    }

    #[test]
    fn rejects_negative_settle_time() {
        assert_eq!(
            LinearInputs::outright(
                1.1,
                eurusd(),
                fx(),
                LinearTerms::new(1.1, 1.0, Side::Buy),
                -0.1,
            ),
            Err(LinearInputError::NegativeSettleTime)
        );
        let near = LinearInputs::outright(
            1.1,
            eurusd(),
            fx(),
            LinearTerms::new(1.1, 1.0, Side::Buy),
            0.5,
        )
        .unwrap();
        assert_eq!(
            near.with_far(-0.2),
            Err(LinearInputError::NegativeSettleTime)
        );
    }

    /// The carry accessors reproduce `celnet_core::CarryInputs` bit-for-bit (the
    /// linear book reads the same seam as the option leaf).
    #[test]
    fn forward_and_df_match_core_carry_inputs_byte_for_byte() {
        use celnet_core::CarryInputs;
        let t = 0.75;
        let li = LinearInputs::outright(
            1.2345,
            eurusd(),
            fx(),
            LinearTerms::new(1.30, 1.0, Side::Buy),
            t,
        )
        .unwrap();
        let ci = CarryInputs::new(1.2345, 1.30, 0.10, t, eurusd(), fx());
        assert_eq!(li.forward(t).to_bits(), ci.forward().to_bits());
        assert_eq!(li.discount_df(t).to_bits(), ci.discount_df().to_bits());
    }
}
