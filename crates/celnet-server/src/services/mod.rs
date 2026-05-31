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
//!
//! Every RPC enters the readiness gate (bumping the in-flight drain counter) and
//! refuses new work with `UNAVAILABLE` while the edge is starting or draining.

pub mod pricing;
pub mod quote;
pub mod risk;
pub mod stream;
pub mod surface;

mod attribution;
pub(crate) mod forward;
mod pin;
mod stream_rx;
