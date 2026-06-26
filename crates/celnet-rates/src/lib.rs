//! # celnet-rates — interest-rate term-structure engine
//!
//! The foundation crate of the Celnet fixed-income family (`docs/fixed-income/FI-ARCHITECTURE.md`
//! D1). It owns the **immutable, cheaply-cloned discount/forward [`Curve`] snapshot** and its
//! interpolation, on top of which the later slices build USD-SOFR bootstrapping, the linear-rates
//! products (FRA / OIS / IRS / basis / futures), and the PV / PV01 / DV01 / key-rate analytics.
//!
//! ## Scope of this slice
//!
//! - A discount-factor-space [`Curve`] carrying its pillars and the **log-linear-on-log-DF**
//!   interpolation scheme — the shipping default per the curves spec (`FI-CURVES-SPEC.md` §4, Q10),
//!   which yields piecewise-flat (continuous-compounding) instantaneous forwards and is exact,
//!   arbitrage-free in discount-factor space, and allocation-free on the hot path.
//! - Interconvertible accessors: discount factor, continuously-compounded zero rate, instantaneous
//!   forward, and forward rates over an interval (continuous and simple compounding).
//!
//! ## Deliberately not in this slice (no stubs — added by later slices)
//!
//! - Monotone-convex-on-forwards interpolation (`FI-CURVES-SPEC.md` §4, the "smooth view").
//! - The calendar/day-count date→time layer and the sequential-bootstrap calibration (§5).
//! - Turn-of-year / central-bank-meeting forward jumps (§4.1).
//!
//! ## Design discipline
//!
//! The numeric core does **no IO** and never allocates on a query. A [`Curve`] is backed by an
//! [`std::sync::Arc`] slice of nodes, so cloning a snapshot to fan scenarios out in parallel is a
//! single reference-count bump (`FI-CURVES-SPEC.md` §3). Method/paper provenance lives in prose and
//! doc-comments only — never in identifiers (CLAUDE.md §8).

#![forbid(unsafe_code)]

pub mod bootstrap;
pub mod curve;
pub mod fra;
pub mod ois;
pub mod risk;
pub mod schedule;
pub mod solver;

pub use bootstrap::{BootstrapError, OisQuote, bootstrap_ois};
pub use curve::{Curve, CurveError};
pub use fra::{Fra, FraError, FraRisk, fra_par_rate, fra_pv, fra_pv01, fra_risk};
pub use ois::{FixedPeriod, OisSchedule, ScheduleError, ois_annuity, ois_par_rate, ois_pv};
pub use risk::{OisRisk, ois_risk, pv01};
pub use schedule::{us_settlement_calendar, usd_sofr_ois_schedule};
pub use solver::{SolverError, brent_root};
