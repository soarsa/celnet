//! The `RiskService.AggregateRatesRisk` edge: server-side **fixed-income rates
//! portfolio risk**, the linear-rates analogue of the options
//! [`AggregateRisk`](super::risk) rollup.
//!
//! A client submits a book of [`RatesPosition`](celnet_proto::RatesPosition)s plus
//! the calibrating [`CurveSet`](celnet_proto::CurveSet); the server prices each
//! position (reusing the `PricingService.PriceRates` engine), builds one additive
//! risk fact per position keyed on its `(entity, ccy, book)` cell, shards the facts
//! across the firm HRW partition map and fans them in to the per-currency net
//! PV / PV01 / DV01 + key-rate-DV01 ladder. The client never prices or sums itself.
//!
//! # Composition
//!
//! * [`convert`] — wire ↔ domain mapping: a priced [`RatesPosition`] →
//!   [`RatesRiskFact`](celnet_risk_fleet::RatesRiskFact), and a
//!   [`RatesFirmRollup`](celnet_risk_fleet::RatesFirmRollup) → the wire response.
//! * [`aggregate`] — the pure *price → scope → shard → rollup* engine.
//!
//! The handler itself lives on [`RiskEdge`](super::risk::RiskEdge) (the type that
//! implements `RiskService`), where it enforces the **same** readiness gate and
//! `resolve_caller` + `authorize_caller(RequiredAuthority::ReadAny)` entitlement
//! guard every other `RiskService` RPC applies — deny-by-default under
//! [`AccessMode::Enforce`](celnet_entitlements::AccessMode), the client asserting an
//! explicit grant-all principal otherwise.

pub mod aggregate;
pub mod convert;

pub use aggregate::aggregate_rates_risk;
