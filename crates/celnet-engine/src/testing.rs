//! Deterministic fixtures shared by the engine's unit and integration tests.
//!
//! This is a small, real (non-mock) support module: it builds a *genuinely
//! calibrated* EURUSD smile via [`celnet_surface::build_smile`] and assembles a
//! complete [`MarketState`], so the tests exercise the same code paths the
//! production engine does. It is compiled into the library (not gated behind
//! `cfg(test)`) so the crate's integration tests — which link the library as an
//! external dependency — can reuse it.

use celnet_conventions::ConventionRecord;
use celnet_surface::{MarketContext, MarketQuotes, VannaVolgaSmile, build_smile};

use crate::rt::{MarketState, PriceSnapshot};

/// Build a calibrated [`VannaVolgaSmile`] for a EURUSD-like slice from broker
/// quotes (ATM / 25Δ risk-reversal / 25Δ butterfly).
///
/// # Panics
///
/// Panics if the broker-strangle calibration fails to converge (it does not for
/// the mild quotes used here).
#[must_use]
pub fn smile(
    spot: f64,
    atm: f64,
    rr: f64,
    bf: f64,
    conv: ConventionRecord,
    t: f64,
) -> VannaVolgaSmile {
    let ctx = MarketContext::new(spot, 0.02, 0.01, t, conv);
    let q = MarketQuotes::three_point(atm, rr, bf);
    build_smile(&ctx, &q).expect("calibration converges for mild EURUSD quotes")
}

/// A complete [`MarketState`] for a EURUSD-like 1Y slice at the given broker
/// quotes.
///
/// # Panics
///
/// Panics if the underlying [`smile`] calibration fails.
#[must_use]
pub fn market_state(spot: f64, atm: f64, rr: f64, bf: f64, conv: ConventionRecord) -> MarketState {
    let t = 1.0;
    MarketState {
        spot,
        r_dom: 0.02,
        r_for: 0.01,
        t,
        conventions: conv,
        smile: smile(spot, atm, rr, bf, conv, t),
    }
}

/// A standard [`MarketState`] at `spot` with fixed mild EURUSD quotes — the
/// convenience used by the concurrency tests where only spot varies.
///
/// # Panics
///
/// Panics if the underlying [`smile`] calibration fails.
#[must_use]
pub fn make_state(spot: f64, conv: ConventionRecord) -> MarketState {
    market_state(spot, 0.105, 0.015, 0.0035, conv)
}

/// A [`PriceSnapshot`] whose fields satisfy a fixed internal relation
/// (`delta_spot == i`, `vega == 2·i`) keyed on `request_id == i`.
///
/// The seqlock consistency test publishes these and asserts the relation holds
/// on every read: a *torn* read (one field from snapshot `i`, another from `j`)
/// would break the relation and fail the test, so the relation is a structural
/// torn-read detector.
#[must_use]
pub fn consistent_pair(i: u64) -> PriceSnapshot {
    PriceSnapshot {
        request_id: i,
        price: i as f64 * 0.5,
        delta_spot: i as f64,
        vega: i as f64 * 2.0,
        vol: 0.1,
    }
}
