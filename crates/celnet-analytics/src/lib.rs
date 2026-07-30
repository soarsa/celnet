//! Celnet **analytics** — the pure, asset-agnostic **client-flow / P&L-
//! attribution** rollup (pillar A of `docs/ANALYTICS-REQUIREMENTS.md` §11).
//!
//! This leaf crate answers the desk's daily client-flow questions —
//! *how much are we making on each client, is our pricing right, and who is
//! just harvesting our prices?* — as a **deterministic fold** over neutral
//! input records. It is **server-, wire-, and market-data-free**: the server
//! layer (a later phase) builds a [`FlowRecord`] per quote/RFQ/execution from
//! its own records + the `PricingProvenance` waterfall + the RFQ panel, then
//! this crate folds `&[FlowRecord]` into [`ClientFlowMetrics`]. Nothing here
//! re-prices or re-derives anything from a market model; every economic
//! quantity is an **input**, so the rollup is a pure function of its inputs and
//! is validated against an independent hand-computed oracle (guardrail 5).
//!
//! # What it computes (§11.1a / §11.3)
//!
//! Grouped per client (with per-counterparty / per-instrument helpers):
//!
//! - **Dollar-per-million ($/mm).** `dpm_gross = Σ margin / (Σ traded notional
//!   / 1e6)`; `dpm_net = (Σ margin − Σ markout − Σ hedge_cost) / traded_mm`.
//!   Zero traded notional ⇒ `None`, never `NaN`/`inf`.
//! - **Spread economics.** `captured_vs_offered` (realised margin ÷ quoted
//!   spread), mean `cover_distance`, and `breakeven_spread` (the offered spread
//!   at which net $/mm hits zero given markout + cost).
//! - **Quote-fishing.** `quote_to_trade_ratio`, `hit_rate`, and a bounded
//!   `[0, 1]` `fishing_score` that rises with high quote-to-trade **and** low
//!   hit-rate **and** ~zero net $/mm (see [`fishing_score`]).
//! - **Volume / P&L.** `traded_notional`, `traded_count`, `quote_count`,
//!   `gross_pnl`, `net_pnl`.
//!
//! # Purity
//!
//! [`metrics_from`] and the [`group_by_client`] / [`group_by_counterparty`] /
//! [`group_by_instrument`] / [`group_by_asset`] helpers are pure functions of
//! their input slice — no clock, no rng, no I/O — hence deterministic and
//! oracle-testable.

mod fishing;
mod grouping;
mod metrics;
mod record;

pub use fishing::{DPM_VALUE_SCALE, fishing_score};
pub use grouping::{group_by_asset, group_by_client, group_by_counterparty, group_by_instrument};
pub use metrics::{ClientFlowMetrics, NOTIONAL_PER_MILLION, metrics_from};
pub use record::{FlowRecord, Side};
