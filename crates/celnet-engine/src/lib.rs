//! Celnet pricing engine — the stateful, non-async, core-pinned hot path and the
//! blue-green upgrade state handoff (work-stream WS-F, `docs/ARCHITECTURE.md` §3,
//! §5).
//!
//! This crate is the *runtime* that turns the closed-form analytics of
//! [`celnet_vanilla`] and the calibrated smile of [`celnet_surface`] into a
//! production low-latency pricing service. It owns three concerns, one per
//! module:
//!
//! * [`rt`] — the **latency runtime** (§3.2): core pinning via
//!   `core_affinity`, wait-free SPSC request/response rings ([`rt::RequestRing`]
//!   / [`rt::ResponseRing`], `rtrb`) between the async *edge* producer and the
//!   busy-poll pricing *core*, lock-free read-mostly publication of the live
//!   market/convention state behind [`rt::StateHandle`] (`arc-swap`), a
//!   single-writer [`rt::Seqlock`] for `Copy` price snapshots, and
//!   [`rt::PaddedCounter`] (`crossbeam-utils` `CachePadded`) on shared hot
//!   atomics to kill false sharing.
//! * [`core`] — the **zero-allocation pricing core** (§3.3): consumes
//!   [`celnet_vanilla`] and a published [`MarketState`] smile, producing price +
//!   the full Greek set with **no heap allocation on the hot path** (all pools
//!   pre-sized at construction).
//! * [`handoff`] — the **blue-green state handoff** (§5): a single, current,
//!   *un-versioned* (ADR-0007) byte serialization of the engine's live book /
//!   convention state, so a freshly-started process can restore the running
//!   book and reprice identically.
//!
//! # Determinism
//!
//! Every transcendental routes through `celnet_core::math`; every float compare
//! routes through `celnet_core::is_close` / `assert_close!`. The hot path never
//! allocates, never locks (readers are wait-free; the single writer publishes via
//! `arc-swap` / seqlock), and never blocks.

// NOTE: this crate intentionally does **not** `#![forbid(unsafe_code)]` — the
// seqlock (`rt::seqlock`) and the allocation-counting test guard need a tightly
// audited `unsafe` surface. `unsafe_code` is `deny`-by-default (see Cargo.toml)
// and opted into only with an explicit `#[allow(unsafe_code)]` + `// SAFETY:`.

pub mod core;
pub mod handoff;
pub mod rt;
pub mod testing;

pub use core::{PriceRequest, PriceResponse, PricingCore};
pub use handoff::{HandoffError, restore_state, serialize_state};
pub use rt::{
    BookState, MarketState, PaddedCounter, PriceSnapshot, RequestRing, ResponseRing, Seqlock,
    StateHandle, pin_current_thread_to_core,
};
