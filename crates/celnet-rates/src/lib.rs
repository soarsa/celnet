//! # celnet-rates — interest-rate term-structure engine
//!
//! The foundation crate of the Celnet fixed-income family (`docs/fixed-income/FI-ARCHITECTURE.md`
//! D1). It owns the **immutable, cheaply-cloned discount/forward [`Curve`] snapshot** and its
//! interpolation, on top of which the later slices build USD-SOFR bootstrapping, the linear-rates
//! products (FRA / OIS / IRS / basis / futures), and the PV / PV01 / DV01 / key-rate analytics.
//!
//! ## Scope of this slice
//!
//! - A discount-factor-space [`Curve`] carrying its pillars and two selectable interpolation schemes
//!   (`FI-CURVES-SPEC.md` §4): the shipping-default **log-linear-on-log-DF** (Q10, piecewise-flat
//!   instantaneous forwards) and the **monotone-convex-on-forwards** smooth view (continuous,
//!   monotonicity-preserving forwards). Both are exact at the pillars, arbitrage-free in
//!   discount-factor space, and allocation-free on the hot path.
//! - Interconvertible accessors: discount factor, continuously-compounded zero rate, instantaneous
//!   forward, and forward rates over an interval (continuous and simple compounding).
//!
//! ## Design discipline
//!
//! The numeric core does **no IO** and never allocates on a query. A [`Curve`] is backed by an
//! [`std::sync::Arc`] slice of nodes, so cloning a snapshot to fan scenarios out in parallel is a
//! single reference-count bump (`FI-CURVES-SPEC.md` §3). Method/paper provenance lives in prose and
//! doc-comments only — never in identifiers (CLAUDE.md §8).

#![forbid(unsafe_code)]

pub mod bond;
pub mod bootstrap;
pub mod curve;
pub mod daycount;
pub mod deposit;
pub mod fra;
pub mod futures;
pub mod futures_strip;
pub mod ois;
pub mod risk;
pub mod schedule;
pub mod solver;
pub mod turns;
pub mod vanilla_swap;

pub use bond::{
    BondError, CashBond, asset_swap_spread, bond_pv, fixed_coupon_bond, g_spread, price_at_yield,
    yield_to_maturity, z_spread,
};
pub use bootstrap::{
    BootstrapError, CalibrationInstrument, OisQuote, VanillaIrsQuote, bootstrap_curve,
    bootstrap_ois,
};
pub use curve::{Curve, CurveError, Interpolation};
pub use daycount::AccrualBasis;
pub use deposit::{Deposit, DepositError, deposit_discount_factor, deposit_par_rate};
pub use fra::{Fra, FraError, FraRisk, fra_par_rate, fra_pv, fra_pv01, fra_risk};
pub use futures::{
    Deliverable, FutureError, StirFuture, cheapest_to_deliver, conversion_factor,
    convexity_adjustment, gross_basis, implied_repo_rate, stir_forward_rate, stir_futures_price,
    stir_futures_rate,
};
pub use futures_strip::{
    FuturesStripError, StirFuturesQuote, bootstrap_futures_strip, implied_forward_rate,
};
pub use ois::{FixedPeriod, OisSchedule, ScheduleError, ois_annuity, ois_par_rate, ois_pv};
pub use risk::{OisRisk, ois_risk, pv01};
pub use schedule::{
    us_settlement_calendar, usd_ois_schedule_for_months, usd_ois_schedule_to_maturity,
    usd_ois_schedule_with_basis, usd_sofr_ois_schedule,
};
pub use solver::{SolverError, brent_root};
pub use turns::{TurnError, TurnJump, turn_discount_factor, with_turns};
pub use vanilla_swap::{
    LegPeriod, PaymentFrequency, SwapError, SwapLeg, SwapRisk, VanillaSwap, fixed_annuity,
    float_leg_value, swap_leg_schedule, swap_par_rate, swap_pv, swap_pv01, swap_risk,
};
