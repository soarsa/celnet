//! # celnet-rates-risk — fixed-income rate-scenario VaR / Expected-Shortfall
//!
//! The standalone first increment (central-core Phase C2a) of the unified single VaR engine: a
//! scenario **bump-and-revalue** rate-risk engine for *linear* fixed income. Given a base discount
//! curve and a set of rate shocks, it reprices a book of FI positions under every shocked curve and
//! reduces the P&L distribution to Value-at-Risk and Expected Shortfall.
//!
//! ## What it wraps (and never re-implements)
//!
//! - **The discount/forward curve** — [`celnet_rates::Curve`] and its
//!   [`celnet_rates::Curve::from_zero_rates`] builder generate every shocked curve
//!   ([`RatePillars::shocked_curve`]).
//! - **OIS pricing** — [`celnet_rates::ois_pv`] reprices a fixed-vs-OIS swap on the shocked curve
//!   ([`FiPosition::OisSwap`]); the analytic [`celnet_rates::pv01`] / [`celnet_rates::ois_annuity`]
//!   underpin the first-order oracle.
//! - **Cash-bond pricing** — [`celnet_bond::price_from_curve`] reprices a bond off the shocked curve
//!   ([`FiPosition::CashBond`]).
//!
//! Both instrument types value **through a `&Curve`**, so a single shocked curve reprices a
//! heterogeneous FI book with no per-instrument branching in the scenario engine (`curve_shock`).
//!
//! ## Shock model
//!
//! A scenario is an absolute additive shift of the base curve's per-pillar zero rates: a
//! [`RateShock::parallel`] shift, a per-pillar [`RateShock::key_rate`] (curve-hedge) bump, or an
//! arbitrary [`RateShock::new`] historical/prescribed vector. [`standard_bump_scenarios`] builds the
//! prescribed ± parallel + ± per-pillar grid. On the log-linear-on-log-DF curve a per-pillar
//! zero-rate shift has an **exact** closed-form effect on every discount factor (see `curve_shock`),
//! which the reprice-under-shock is oracle-validated against to ≤1e-12.
//!
//! ## Scope (Phases C2a–C2b) and what is deferred
//!
//! - **In scope (C2a):** the scenario generator, FI reprice-under-shock (OIS + cash bond), and the
//!   VaR/ES reduction, all standalone and oracle-validated.
//! - **In scope (C2b):** the FRTB Standardised-Approach **GIRR delta** capital charge ([`girr`]) —
//!   the prescribed MAR21 risk-weight and correlation tables, vertex mapping of the key-rate ladder,
//!   intra-bucket `K_b`, and cross-bucket aggregation with the `S_b` cap/floor, under all three
//!   correlation scenarios; oracle-validated against the published tables and a hand computation.
//! - **In scope (C2c):** the sign-normalized **key-rate axis** ([`ladder`]) — the per-tenor signed
//!   DV01 ladder that reconciles the OIS receive-fixed (signed) vs bond (positive-magnitude) P&L
//!   conventions into the one signed convention before aggregation — and the VaR/ES reduction
//!   ([`rate_var_es`]) delegating to the one platform-wide primitive [`celnet_core::tail_var_es`],
//!   which the risk cube also reduces through. The cube folds these into its non-additive path
//!   ([`celnet_risk_cube::fi`]) so a portfolio's FI rate scenarios ride alongside the options
//!   spot/vol scenarios in one joint tail.
//! - **Deferred:** the GIRR **vega** and **curvature** charges (C2b-extension).
//!
//! ## Determinism
//!
//! No RNG; the only transcendental is the curve's `exp`. For a fixed `(positions, base, shocks,
//! alpha)` every result is bit-reproducible. Method/paper provenance lives in prose only, never in
//! identifiers (CLAUDE.md §8).

#![forbid(unsafe_code)]

pub mod curve_shock;
pub mod error;
pub mod girr;
pub mod ladder;
pub mod position;
pub mod var;

pub use curve_shock::{RatePillars, RateShock, standard_bump_scenarios};
pub use error::RateRiskError;
pub use girr::{
    CorrelationScenario, CurveId, GIRR_CORRELATION_FLOOR, GIRR_CORRELATION_THETA,
    GIRR_CROSS_BUCKET_GAMMA, GIRR_DELTA_RISK_WEIGHTS, GIRR_DIFFERENT_CURVE_FACTOR, GIRR_VERTICES,
    GirrBucketCharge, GirrDeltaCharge, GirrError, GirrSensitivity, LadderPoint, girr_delta_charge,
    girr_delta_charges_all, map_ladder_to_vertices,
};
pub use ladder::{
    KeyRateLadder, KeyRatePoint, key_rate_ladder, normalize_bond_dv01, normalize_ois_dv01,
    signed_parallel_dv01,
};
pub use position::FiPosition;
pub use var::{RateRiskReport, RateVarEs, rate_scenario_var_es, rate_var_es, scenario_pnls};
