//! `proptest` strategies that generate **valid** FX market inputs.
//!
//! These are the single source of truth for the economically-meaningful ranges
//! used by property tests across the pricing crates, so a check written in
//! `celnet-vanilla` and one written in `celnet-surface` sample the same input
//! space rather than each crate re-rolling (and silently diverging on) its own
//! bounds. Every generated [`VanillaInputs`] has a strictly positive spot and
//! strike, a strictly positive vol and maturity, and continuously-compounded
//! rates inside a band that spans the realistic FX regime — including the
//! mildly-negative rates seen in EUR/CHF/JPY — so the forward, the two discount
//! factors, and `σ√T` are always finite and well-conditioned.
//!
//! The ranges are deliberately *closed-open / open-open* to exclude the
//! degenerate endpoints (`vol = 0`, `t = 0`, `strike = 0`) that would make
//! `d1`/`d2` ill-defined; callers that want to probe those boundaries should do
//! so with explicit fixtures, not random draws.

use celnet_types::VanillaInputs;
use proptest::prelude::*;

/// Inclusive lower bound on generated spot/strike (quote per unit base).
pub const PRICE_MIN: f64 = 0.25;
/// Inclusive upper bound on generated spot/strike.
pub const PRICE_MAX: f64 = 250.0;
/// Inclusive lower bound on generated annualized volatility (absolute).
pub const VOL_MIN: f64 = 0.02;
/// Inclusive upper bound on generated annualized volatility (absolute).
pub const VOL_MAX: f64 = 0.80;
/// Inclusive lower bound on generated maturity (years).
pub const MATURITY_MIN: f64 = 0.02;
/// Inclusive upper bound on generated maturity (years).
pub const MATURITY_MAX: f64 = 5.0;
/// Inclusive lower bound on a generated continuously-compounded rate.
pub const RATE_MIN: f64 = -0.03;
/// Inclusive upper bound on a generated continuously-compounded rate.
pub const RATE_MAX: f64 = 0.12;

/// A valid spot (or strike) FX level: strictly positive, in a realistic band.
pub fn arb_spot() -> impl Strategy<Value = f64> {
    PRICE_MIN..=PRICE_MAX
}

/// A valid strike: same band as [`arb_spot`] (strikes and spots share units).
pub fn arb_strike() -> impl Strategy<Value = f64> {
    PRICE_MIN..=PRICE_MAX
}

/// A valid annualized volatility (absolute; `0.10` = 10 vol), strictly positive.
pub fn arb_vol() -> impl Strategy<Value = f64> {
    VOL_MIN..=VOL_MAX
}

/// A valid annualized volatility wrapped in the [`celnet_types::Vol`] newtype,
/// for callers that consume the typed vocabulary directly.
pub fn arb_vol_quote() -> impl Strategy<Value = celnet_types::Vol> {
    arb_vol().prop_map(celnet_types::Vol)
}

/// A valid time-to-expiry (years), strictly positive.
pub fn arb_maturity() -> impl Strategy<Value = f64> {
    MATURITY_MIN..=MATURITY_MAX
}

/// A valid continuously-compounded interest rate (may be mildly negative).
pub fn arb_rate() -> impl Strategy<Value = f64> {
    RATE_MIN..=RATE_MAX
}

/// A complete, valid [`VanillaInputs`] drawing each field from its strategy.
///
/// Spot and strike are sampled independently across the band, so the generated
/// options span deep-ITM through deep-OTM in both directions; this is the
/// workhorse generator for parity, monotonicity and Greek property tests.
pub fn arb_inputs() -> impl Strategy<Value = VanillaInputs> {
    (
        arb_spot(),
        arb_strike(),
        arb_vol(),
        arb_maturity(),
        arb_rate(),
        arb_rate(),
    )
        .prop_map(|(spot, strike, vol, t, r_dom, r_for)| {
            VanillaInputs::new(spot, strike, vol, t, r_dom, r_for)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    proptest! {
        /// Every generated input is economically valid: the fields lie in their
        /// declared bands and the derived forward / discount factors are finite
        /// and positive (a precondition for `d1`/`d2` to be well-defined).
        #[test]
        fn generated_inputs_are_valid(i in arb_inputs()) {
            prop_assert!(i.spot >= PRICE_MIN && i.spot <= PRICE_MAX);
            prop_assert!(i.strike >= PRICE_MIN && i.strike <= PRICE_MAX);
            prop_assert!(i.vol >= VOL_MIN && i.vol <= VOL_MAX);
            prop_assert!(i.t >= MATURITY_MIN && i.t <= MATURITY_MAX);
            prop_assert!(i.r_dom >= RATE_MIN && i.r_dom <= RATE_MAX);
            prop_assert!(i.r_for >= RATE_MIN && i.r_for <= RATE_MAX);

            let f = i.forward();
            prop_assert!(f.is_finite() && f > 0.0);
            prop_assert!(i.df_dom().is_finite() && i.df_dom() > 0.0);
            prop_assert!(i.df_for().is_finite() && i.df_for() > 0.0);
        }

        /// The scalar strategies stay strictly inside the non-degenerate region
        /// (no zero vol, no zero maturity, no zero strike).
        #[test]
        fn scalar_strategies_avoid_degenerate_endpoints(
            v in arb_vol(),
            t in arb_maturity(),
            k in arb_strike(),
        ) {
            prop_assert!(v > 0.0);
            prop_assert!(t > 0.0);
            prop_assert!(k > 0.0);
        }

        /// The typed-vol wrapper carries the same valid range.
        #[test]
        fn vol_quote_in_band(q in arb_vol_quote()) {
            prop_assert!(q.0 >= VOL_MIN && q.0 <= VOL_MAX);
        }
    }
}
