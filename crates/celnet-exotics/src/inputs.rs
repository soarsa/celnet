//! The agnostic market-state input every exotic engine prices against.
//!
//! [`ExoticInputs`] is the carry-seam replacement for the FX-only
//! [`celnet_types::VanillaInputs`]: it holds the same `(spot, strike, vol, t)`
//! plus a [`Carry`] producer and an [`Underlying`] identity. Forward and discount
//! factors are formed **only** through `carry` — no engine reads `underlying` in
//! pricing math (it is identity / routing). This is the same shape as
//! [`celnet_core::CarryInputs`], owned crate-locally so the pervasive per-leg
//! struct-update churn (`ExoticInputs { strike: k, ..i }`) and the
//! exotics-specific `forward`/`carry_df` helpers do not pollute the frozen core
//! seam.
//!
//! # FX byte-identity
//!
//! For an FX [`Carry::FxRates`] carry, every accessor reproduces the FX two-rate
//! arithmetic **bit-for-bit**:
//! * `carry_rate()` = `r_dom − r_for` (same two flops, same order),
//! * `discount_rate()` = `r_dom` (verbatim),
//! * `discount_df()` = `e^{−r_dom·t}` (identical `libm::exp` call),
//! * `carry_df()` = `e^{−r_for·t}` reading the **stored** `r_for` via
//!   [`Carry::yield_rate`] (never `r_dom − b`, which would not round-trip),
//! * `forward()` = `spot · e^{(r_dom−r_for)·t}` (identical call).
//!
//! The conversion `From<VanillaInputs>` builds `Carry::FxRates { r_dom, r_for }`
//! from the stored rates verbatim, so a migrated engine called through this path
//! produces the byte-identical FX price the golden grid was generated from.

use celnet_core::CarryInputs;
use celnet_core::gbsm_carry_price;
use celnet_core::math::exp;
use celnet_types::{Carry, OptionType, Underlying, VanillaInputs};

/// The agnostic market state every exotic engine prices against.
///
/// See the module docs for the FX byte-identity contract.
#[derive(Debug, Clone, PartialEq)]
pub struct ExoticInputs {
    /// Spot price of the underlying (quote per 1 unit of base, for FX).
    pub spot: f64,
    /// Strike (quote per 1 unit of base, for FX).
    pub strike: f64,
    /// Annualized volatility (absolute, e.g. `0.10` = 10 vol).
    pub vol: f64,
    /// Time to expiry in years (vol-time).
    pub t: f64,
    /// The underlying asset (identity / routing — never read in pricing math).
    pub underlying: Underlying,
    /// The cost-of-carry model behind the forward and discounting.
    pub carry: Carry,
}

impl ExoticInputs {
    /// Construct an agnostic exotic pricing input.
    #[must_use]
    #[inline]
    pub const fn new(
        spot: f64,
        strike: f64,
        vol: f64,
        t: f64,
        underlying: Underlying,
        carry: Carry,
    ) -> Self {
        Self {
            spot,
            strike,
            vol,
            t,
            underlying,
            carry,
        }
    }

    /// Net cost-of-carry `b` in `F = S·e^{b·t}`. For FX this is `r_dom − r_for`.
    #[must_use]
    #[inline]
    pub fn carry_rate(&self) -> f64 {
        self.carry.carry_rate()
    }

    /// Discount (numeraire) rate `r` in `e^{−r·t}`. For FX this is `r_dom`.
    #[must_use]
    #[inline]
    pub fn discount_rate(&self) -> f64 {
        self.carry.discount_rate()
    }

    /// Yield/foreign rate `q` (`b = r − q`). For FX this is the stored `r_for`,
    /// read verbatim — see [`Carry::yield_rate`].
    #[must_use]
    #[inline]
    pub fn yield_rate(&self) -> f64 {
        self.carry.yield_rate()
    }

    /// Numeraire discount factor `e^{−r·t}` over this input's `t`.
    #[must_use]
    #[inline]
    pub fn discount_df(&self) -> f64 {
        self.carry.discount_df(self.t)
    }

    /// Numeraire discount factor `e^{−r·τ}` over an arbitrary horizon `τ`.
    #[must_use]
    #[inline]
    pub fn discount_df_at(&self, tau: f64) -> f64 {
        self.carry.discount_df(tau)
    }

    /// Yield/foreign discount factor `e^{−q·t}` over this input's `t`. For FX this
    /// is `e^{−r_for·t}`, reading the stored `r_for` (no `r_dom − b` recompute).
    #[must_use]
    #[inline]
    pub fn carry_df(&self) -> f64 {
        exp(-self.carry.yield_rate() * self.t)
    }

    /// Yield/foreign discount factor `e^{−q·τ}` over an arbitrary horizon `τ`.
    #[must_use]
    #[inline]
    pub fn carry_df_at(&self, tau: f64) -> f64 {
        exp(-self.carry.yield_rate() * tau)
    }

    /// Outright forward `F = spot · e^{b·t}`. For FX byte-identical to
    /// [`VanillaInputs::forward`].
    #[must_use]
    #[inline]
    pub fn forward(&self) -> f64 {
        self.spot * self.carry.forward_factor(self.t)
    }

    /// Lower this agnostic input to the FX [`VanillaInputs`] for the
    /// Garman-Kohlhagen vanilla engine, **substituting** `strike`.
    ///
    /// Routes through [`celnet_core::fx_vanilla_inputs`], which produces the FX
    /// `VanillaInputs` **byte-identically** (`r_dom`/`r_for` read verbatim from
    /// [`Carry::FxRates`]) and typed-rejects a non-FX carry/underlying — so an
    /// equity/commodity exotic that internally needs an FX vanilla leg fails
    /// loudly rather than being silently mis-priced under FX arithmetic.
    ///
    /// # Errors
    ///
    /// Returns [`CarryPriceError`] when the carry/underlying is not FX.
    #[inline]
    pub fn as_fx_vanilla(
        &self,
        strike: f64,
    ) -> Result<VanillaInputs, celnet_core::CarryPriceError> {
        let mut lowered = CarryInputs::from(self);
        lowered.strike = strike;
        celnet_core::fx_vanilla_inputs(&lowered)
    }
}

/// Generalized closed-form vanilla priced **through the carry seam**, delegating to
/// the one unified generalized-BSM forward-space kernel
/// ([`celnet_core::gbsm_carry_price`]) with `b = carry.carry_rate()`,
/// `r = carry.discount_rate()`.
///
/// This is the agnostic vanilla every synthetic-recast site in this crate prices
/// against (geometric-Asian control variates, forward-start unit-spot legs, the
/// variance-swap replication strip): the recasts build a [`Carry`] whose effective
/// vol/carry encode the structure, and this closed form completes the leg for **any**
/// asset class.
///
/// # FX byte-identity (the load-bearing property, ADR-0012)
///
/// This routes through the *same* kernel as the FX (Garman-Kohlhagen) leaf
/// (`celnet_vanilla::price`). For a genuine [`Carry::FxRates`], both sides assemble
/// `b = r_dom − r_for` (bit-identically: `carry_rate()` and `r_dom − r_for` are the
/// same subtraction), so the carry-seam vanilla is **byte-for-byte identical** to the
/// FX two-rate vanilla — the ADR-0008 invariant, restored on the unified kernel.
/// A synthetic [`Carry::CostOfCarry`] `{ r, b }` passes `b` directly, whereas the
/// legacy FX recast materialized `r_for = r − b` and the FX leaf reconstructs
/// `r − (r − b)` (not bit-equal to `b`); those synthetic paths therefore agree to
/// 1e-12, not bit-for-bit (gated in this module's tests, ADR-0012 §6).
#[must_use]
pub(crate) fn carry_vanilla_price(opt: OptionType, i: &ExoticInputs) -> f64 {
    carry_vanilla_price_at(opt, i.spot, i.strike, i.vol, i.t, &i.carry)
}

/// [`carry_vanilla_price`] at an explicit `(spot, strike, vol, t)` market state —
/// the same kernel call for callers (the variance-swap replication strip) that price
/// a whole strike continuum against one [`Carry`] without materializing per-strike
/// [`ExoticInputs`]. Identical IEEE-754 op sequence.
#[must_use]
pub(crate) fn carry_vanilla_price_at(
    opt: OptionType,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    carry: &Carry,
) -> f64 {
    gbsm_carry_price(
        opt,
        carry.carry_rate(),
        carry.discount_rate(),
        spot,
        strike,
        vol,
        t,
    )
}

impl From<&ExoticInputs> for CarryInputs {
    #[inline]
    fn from(i: &ExoticInputs) -> Self {
        CarryInputs::new(i.spot, i.strike, i.vol, i.t, i.underlying.clone(), i.carry)
    }
}

impl From<CarryInputs> for ExoticInputs {
    #[inline]
    fn from(i: CarryInputs) -> Self {
        Self {
            spot: i.spot,
            strike: i.strike,
            vol: i.vol,
            t: i.t,
            underlying: i.underlying,
            carry: i.carry,
        }
    }
}

impl From<&VanillaInputs> for ExoticInputs {
    /// FX projection: the stored `(r_dom, r_for)` become `Carry::FxRates` verbatim,
    /// so every accessor is byte-identical to the FX two-rate form. The underlying
    /// is the canonical FX identity placeholder; engines never read it in math.
    #[inline]
    fn from(i: &VanillaInputs) -> Self {
        Self {
            spot: i.spot,
            strike: i.strike,
            vol: i.vol,
            t: i.t,
            underlying: Underlying::Fx(
                celnet_types::CcyPair::parse("EURUSD").expect("static FX identity placeholder"),
            ),
            carry: Carry::FxRates {
                r_dom: i.r_dom,
                r_for: i.r_for,
            },
        }
    }
}

impl From<VanillaInputs> for ExoticInputs {
    #[inline]
    fn from(i: VanillaInputs) -> Self {
        ExoticInputs::from(&i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The FX projection of `ExoticInputs` reproduces the FX two-rate forward and
    /// both discount factors BIT-FOR-BIT — the byte-identity invariant.
    #[test]
    fn fx_projection_byte_identical() {
        for &(spot, strike, vol, t, r_dom, r_for) in &[
            (1.30, 1.25, 0.10, 1.0, 0.03, 0.01),
            (100.0, 95.0, 0.20, 0.25, 0.05, 0.0),
            (0.85, 0.90, 0.12, 2.5, -0.004, 0.031),
        ] {
            let v = VanillaInputs::new(spot, strike, vol, t, r_dom, r_for);
            let e = ExoticInputs::from(&v);
            assert_eq!(e.forward().to_bits(), v.forward().to_bits());
            assert_eq!(e.discount_df().to_bits(), v.df_dom().to_bits());
            assert_eq!(e.carry_df().to_bits(), v.df_for().to_bits());
            assert_eq!(e.carry_rate().to_bits(), (r_dom - r_for).to_bits());
            assert_eq!(e.discount_rate(), r_dom);
        }
    }

    /// Seam-accessor identities re-derived RAW (the horizon-τ accessors and the
    /// carry-seam identities the per-`t` byte-identity test does not reach):
    /// `discount_df_at(τ) = e^{−r_dom·τ}`, `carry_df_at(τ) = e^{−r_for·τ}` at a
    /// horizon `τ ≠ t`, `yield_rate` verbatim, and the `as_fx_vanilla`
    /// round-trip (every field bit-exact, strike substituted) — the ADR-0008
    /// carry-seam identity — plus the typed rejection of a non-FX carry.
    #[test]
    fn seam_accessor_identities_raw() {
        let v = VanillaInputs::new(1.30, 1.25, 0.10, 0.75, 0.03, 0.01);
        let e = ExoticInputs::from(&v);
        let tau = 0.4;
        assert_eq!(e.discount_df_at(tau).to_bits(), exp(-0.03 * tau).to_bits());
        assert_eq!(e.carry_df_at(tau).to_bits(), exp(-0.01 * tau).to_bits());
        assert_eq!(e.yield_rate().to_bits(), 0.01f64.to_bits());
        assert_eq!(e.carry_rate().to_bits(), (0.03f64 - 0.01).to_bits());

        let k = 1.372_905_124_873_311_4;
        let rt = e.as_fx_vanilla(k).expect("FX carry must lower");
        assert_eq!(rt.spot.to_bits(), v.spot.to_bits());
        assert_eq!(rt.strike.to_bits(), k.to_bits());
        assert_eq!(rt.vol.to_bits(), v.vol.to_bits());
        assert_eq!(rt.t.to_bits(), v.t.to_bits());
        assert_eq!(rt.r_dom.to_bits(), v.r_dom.to_bits());
        assert_eq!(rt.r_for.to_bits(), v.r_for.to_bits());

        // A non-FX carry typed-rejects instead of silently mis-pricing.
        let synthetic = ExoticInputs {
            carry: Carry::CostOfCarry { r: 0.03, b: 0.0173 },
            ..e
        };
        assert!(synthetic.as_fx_vanilla(k).is_err());
    }

    /// `carry_vanilla_price_at` (the strike-continuum form) is the SAME IEEE-754
    /// op sequence as `carry_vanilla_price` on materialized inputs — bit-for-bit
    /// across option types and carry kinds.
    #[test]
    fn carry_vanilla_at_matches_materialized_bitwise() {
        let v = VanillaInputs::new(1.30, 1.25, 0.10, 0.75, 0.03, 0.01);
        let e = ExoticInputs::from(&v);
        for carry in [e.carry, Carry::CostOfCarry { r: 0.03, b: 0.0173 }] {
            let i = ExoticInputs { carry, ..e.clone() };
            for opt in [OptionType::Call, OptionType::Put] {
                assert_eq!(
                    carry_vanilla_price_at(opt, i.spot, i.strike, i.vol, i.t, &i.carry).to_bits(),
                    carry_vanilla_price(opt, &i).to_bits()
                );
            }
        }
    }

    /// The carry-seam vanilla reproduces the FX two-rate vanilla BIT-FOR-BIT for a
    /// genuine FX carry (both assemble `b = r_dom − r_for` and route through the one
    /// gBSM kernel — ADR-0012). A synthetic `CostOfCarry { r, b }` passes `b`
    /// directly, whereas the legacy FX recast reconstructs `r − (r − b)` (not
    /// bit-equal to `b`), so that path agrees to 1e-12, not bit-for-bit (the
    /// `funding_maps` class — ADR-0012 §6; residual ~1e-15).
    #[test]
    fn carry_vanilla_byte_identical_to_fx_forms() {
        for &(spot, strike, vol, t, r_dom, r_for) in &[
            (1.30, 1.25, 0.10, 1.0, 0.03, 0.01),
            (100.0, 95.0, 0.20, 0.25, 0.05, 0.0),
            (0.85, 0.90, 0.12, 2.5, -0.004, 0.031),
        ] {
            let v = VanillaInputs::new(spot, strike, vol, t, r_dom, r_for);
            let e = ExoticInputs::from(&v);
            for opt in [OptionType::Call, OptionType::Put] {
                // Genuine FX carry: RESTORED bit-for-bit (both route the kernel with
                // b = r_dom − r_for).
                assert_eq!(
                    carry_vanilla_price(opt, &e).to_bits(),
                    celnet_vanilla::price(opt, &v).to_bits(),
                );
                // Synthetic cost-of-carry: the kernel takes b directly while the
                // legacy recast materialized r_for = r − b (FX leaf reconstructs
                // r − (r − b)); agree to 1e-12 (ADR-0012 §6).
                let (r, b) = (r_dom, 0.0173);
                let synthetic = ExoticInputs {
                    carry: Carry::CostOfCarry { r, b },
                    ..e.clone()
                };
                let legacy = VanillaInputs::new(spot, strike, vol, t, r, r - b);
                assert!(celnet_core::is_close(
                    carry_vanilla_price(opt, &synthetic),
                    celnet_vanilla::price(opt, &legacy),
                    1e-12,
                    1e-12
                ));
            }
        }
    }
}
