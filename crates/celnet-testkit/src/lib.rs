//! Celnet shared test harness — financial-invariant assertions, proptest
//! input strategies, and fixture loaders reused across the pricing crates
//! (work-stream WS-T).
//!
//! # Why a dedicated harness
//!
//! Every pricing crate (`celnet-vanilla`, and downstream `celnet-surface` /
//! `celnet-exotics` / `celnet-engine`) must satisfy the *same* model-independent
//! no-arbitrage and consistency laws: put-call parity, monotonicity of value in
//! spot/vol/maturity, convexity of value in strike (butterfly `≥ 0`), the
//! intrinsic-value lower bounds, and agreement of analytic Greeks with finite
//! differences. Re-deriving and re-asserting these in each crate breeds drift
//! and subtle, weakened checks. This crate centralises them once, as an
//! ergonomic dev-dependency, so every consumer validates against *identical*,
//! reference-grounded laws.
//!
//! It also centralises the **valid-market-input generators** so property tests
//! everywhere draw economically-meaningful [`celnet_types::VanillaInputs`]
//! (positive spot/strike, sane vol and rate ranges) instead of each crate
//! re-rolling its own ranges, and provides small **fixture loaders** for the
//! canonical reference market states used in regression tests.
//!
//! # Modules
//!
//! - [`invariants`] — financial-invariant assertion helpers. Each panics (via
//!   [`celnet_core::assert_close`]/`assert!`) with a descriptive message when an
//!   invariant is violated, so they read naturally inside `#[test]` bodies.
//! - [`strategy`] — `proptest` `Strategy` generators for valid FX market
//!   inputs (spot, strike, vol, maturity, the two rates) and composed
//!   [`celnet_types::VanillaInputs`].
//! - [`fixtures`] — named, deterministic reference market states (the textbook
//!   Black-Scholes benchmark and representative G10 / high-vol / inverted-carry
//!   regimes) for regression and golden tests.
//!
//! # Determinism
//!
//! All floating-point comparison routes through [`celnet_core::is_close`] /
//! [`celnet_core::assert_close`] (combined relative + absolute tolerance); the
//! harness never compares with `==` and never asserts on `NaN`. Pricing math is
//! reused from `celnet-vanilla`; this crate adds no model of its own.

#![forbid(unsafe_code)]

pub mod fixtures;
pub mod invariants;
pub mod strategy;

pub use fixtures::{REFERENCE_MARKETS, ReferenceMarket, reference_market, reference_markets};
pub use invariants::{
    GreekKind, assert_call_intrinsic_lower_bound, assert_decreasing_in_strike,
    assert_greek_matches_fd, assert_increasing_in_maturity, assert_increasing_in_spot,
    assert_increasing_in_vol, assert_price_within_bounds, assert_put_call_parity,
    assert_put_intrinsic_lower_bound, assert_strike_convexity, central_difference,
};
pub use strategy::{
    arb_inputs, arb_maturity, arb_rate, arb_spot, arb_strike, arb_vol, arb_vol_quote,
};
