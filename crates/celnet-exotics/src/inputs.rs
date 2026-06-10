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
use celnet_core::math::{exp, ln, norm_cdf, sqrt};
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

/// Generalized closed-form vanilla priced **through the carry seam** — the
/// Black-Scholes-Merton-family vanilla in its `(r, q)` discount/yield form,
/// where `r = carry.discount_rate()` and `q = carry.yield_rate()`:
///
/// ```text
/// d1 = [ln(S/K) + (r − q + ½σ²)·t] / (σ√t),   d2 = d1 − σ√t
/// Call = S·e^{−q·t}·Φ(d1) − K·e^{−r·t}·Φ(d2)
/// Put  = K·e^{−r·t}·Φ(−d2) − S·e^{−q·t}·Φ(−d1)
/// ```
///
/// This is the agnostic vanilla every synthetic-recast site in this crate prices
/// against (geometric-Asian control variates, forward-start unit-spot legs): the
/// recasts build a [`Carry::CostOfCarry`] `{ r, b }` whose effective vol/carry
/// encode the structure, and this closed form completes the leg for **any** asset
/// class.
///
/// # FX byte-identity (the load-bearing property)
///
/// The formula is written in the `(r, q)` form — **not** the `b = carry_rate()`
/// form — deliberately, so that *both* historical FX paths reproduce bit-for-bit:
///
/// * a genuine [`Carry::FxRates`] reads `r = r_dom`, `q = r_for` verbatim, making
///   every expression the identical IEEE-754 op sequence of the FX two-rate
///   vanilla (`celnet_vanilla::price`);
/// * a synthetic [`Carry::CostOfCarry`] `{ r, b }` yields `q = r − b` — the exact
///   float expression the legacy recast used when it materialized
///   `r_for = r_dom − eff_b` into a synthetic FX input. Writing the drift as
///   `b` directly would change the rounding (`r − (r − b)` does not round-trip
///   to `b`), silently breaking the golden grid.
///
/// Gated to_bits against `celnet_vanilla::price` in this module's tests.
#[must_use]
pub(crate) fn carry_vanilla_price(opt: OptionType, i: &ExoticInputs) -> f64 {
    carry_vanilla_price_at(opt, i.spot, i.strike, i.vol, i.t, &i.carry)
}

/// [`carry_vanilla_price`] at an explicit `(spot, strike, vol, t)` market state —
/// the same `(r, q)` closed form for callers (the variance-swap replication
/// strip) that price a whole strike continuum against one [`Carry`] without
/// materializing per-strike [`ExoticInputs`]. Identical IEEE-754 op sequence.
#[must_use]
pub(crate) fn carry_vanilla_price_at(
    opt: OptionType,
    spot: f64,
    strike: f64,
    vol: f64,
    t: f64,
    carry: &Carry,
) -> f64 {
    let r = carry.discount_rate();
    let q = carry.yield_rate();
    let sqt = sqrt(t);
    let vsqt = vol * sqt;
    let d1 = (ln(spot / strike) + (r - q + 0.5 * vol * vol) * t) / vsqt;
    let d2 = d1 - vsqt;
    let s_disc = spot * exp(-q * t);
    let k_disc = strike * exp(-r * t);
    match opt {
        OptionType::Call => s_disc * norm_cdf(d1) - k_disc * norm_cdf(d2),
        OptionType::Put => k_disc * norm_cdf(-d2) - s_disc * norm_cdf(-d1),
    }
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

    /// The carry-seam vanilla reproduces the FX two-rate vanilla BIT-FOR-BIT for a
    /// genuine FX carry, and a synthetic `CostOfCarry { r, b }` reproduces the
    /// legacy synthetic-FX recast (`r_for = r − b`) BIT-FOR-BIT — the two
    /// byte-identity contracts the doc comment claims.
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
                // Genuine FX carry: identical to the FX two-rate vanilla.
                assert_eq!(
                    carry_vanilla_price(opt, &e).to_bits(),
                    celnet_vanilla::price(opt, &v).to_bits(),
                );
                // Synthetic cost-of-carry: identical to the legacy recast that
                // materialized r_for = r − b into a synthetic FX input.
                let (r, b) = (r_dom, 0.0173);
                let synthetic = ExoticInputs {
                    carry: Carry::CostOfCarry { r, b },
                    ..e.clone()
                };
                let legacy = VanillaInputs::new(spot, strike, vol, t, r, r - b);
                assert_eq!(
                    carry_vanilla_price(opt, &synthetic).to_bits(),
                    celnet_vanilla::price(opt, &legacy).to_bits(),
                );
            }
        }
    }
}
