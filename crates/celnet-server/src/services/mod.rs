//! The gRPC service implementations over the `celnet-proto` wire contract.
//!
//! Each submodule implements one generated `tonic` service trait, all sharing the
//! [`crate::core_link::CoreLink`] (the async⇄core bridge), the
//! [`crate::readiness::ReadinessGate`] (`/readyz` + drain), the
//! [`crate::spread::SpreadModel`] (two-way markets), the [`crate::pricer`]
//! (instrument → Greeks), and the [`crate::clock::Clock`] (edge timestamping):
//!
//! * [`pricing`] — `PricingService::Price`: one-shot instrument pricing.
//! * [`quote`] — `QuoteService`: the RFQ lifecycle (request → quote → accept →
//!   execution) with client idempotency and a last-look validity window.
//! * [`stream`] — `StreamService::StreamSession`: the multiplexed bidirectional
//!   RFS session (per-subscription snapshot + sequenced deltas + heartbeat +
//!   server-assisted resync, in-place modify, and click-to-trade execution).
//! * [`pin`] — shared `surface_version` pinning: resolve a request's optional
//!   pinned surface version into the marked vol it prices against.
//! * [`surface`] — `SurfaceService`: `GetSmile` / `MarkSurface` / `Scenario`.
//! * [`access`] — the entitlements **trust boundary**: the deny-by-default
//!   authorization decision (+ per-decision security audit) every
//!   entitlement-gated `RiskService` RPC passes before serving.
//!
//! Every RPC enters the readiness gate (bumping the in-flight drain counter) and
//! refuses new work with `UNAVAILABLE` while the edge is starting or draining.

pub mod acceptance;
pub mod access;
pub mod aggregation;
pub mod analytics;
pub mod auto_hedge;
pub mod consensus;
pub mod corpactions;
pub mod desk;
/// End-to-end trade-lifecycle test suite (drives a real incoming order through the whole
/// pipeline: last-look → acceptance → book → route → internalise/hedge → aggregation).
#[cfg(test)]
mod e2e_lifecycle;
pub mod error_status;
pub mod internalise;
pub mod liquidity_feed;
pub mod pricing;
pub mod pricing_control;
pub mod quote;
pub mod rates_book;
pub mod rates_risk;
pub mod risk;
pub mod risk_transfer;
pub mod stream;
pub mod surface;
pub mod telemetry;
pub mod trace;
pub mod transfer_apply;

pub mod deploy;
pub mod sessions;

pub mod auth;
pub mod fix;
pub mod fix_admin;
pub mod fix_monitor;
pub mod fix_registry;
/// Wire ⇄ domain mapping for the instrument reference-data registry (`AuthService`
/// instrument RPCs), kept out of [`auth`] to bound that file's size.
pub mod instrument_wire;

mod attribution;
pub(crate) mod clicktrade;
pub(crate) mod forward;
mod pin;
pub mod pricefanout;
pub(crate) mod stream_rx;
