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
//!   pre-sized at construction). It owns the busy-poll lifecycle:
//!   [`core::PricingCore::run`] drives the loop until a `stop` flag is set, with
//!   a documented, race-free shutdown that performs a final lossless drain and
//!   then returns (so a joining caller is guaranteed termination);
//!   [`core::PricingCore::drain`] is the one-shot bounded poll the loop is built
//!   from. Thread pinning for the core is provided by
//!   [`rt::pin_current_thread_to_core`].
//! * [`handoff`] — the **blue-green state handoff** (§5): a single, current,
//!   *un-versioned* (ADR-0007) byte serialization of the engine's live book /
//!   convention state, so a freshly-started process can restore the running
//!   book and reprice identically.
//! * [`journal`] — **durable crash recovery** (§5): wires the control-plane
//!   booking / accepted-market-state path to a `fsync`'d [`celnet_journal`] log,
//!   reusing the [`handoff`] byte codec (no format fork). [`DurableBook`] appends
//!   each booked line / accepted mark durably; [`recover`] replays the log **at
//!   startup** to rebuild a [`BookState`] / [`MarketState`] that reprices
//!   bit-identically. This is strictly off the hot path — the price() loop never
//!   touches the journal (asserted in `tests/zero_alloc.rs`).
//!
//! # Determinism
//!
//! Every transcendental routes through `celnet_core::math`; every float compare
//! routes through `celnet_core::is_close` / `assert_close!`. The hot path
//! **acquires no memory** — it never `alloc`/`realloc`s, never locks (readers
//! are wait-free via a cached [`rt::StateReader`]; the single writer publishes
//! via `arc-swap` / seqlock), and never blocks. (Reclaiming a *superseded*
//! published [`MarketState`] when its `Arc` refcount reaches zero on a market
//! tick is a `dealloc`, never an allocation; `tests/zero_alloc.rs` proves the
//! acquiring count stays zero even under a concurrent publisher.)

// NOTE: this crate intentionally does **not** `#![forbid(unsafe_code)]` — the
// seqlock (`rt::seqlock`) and the allocation-counting test guard need a tightly
// audited `unsafe` surface. `unsafe_code` is `deny`-by-default (see Cargo.toml)
// and opted into only with an explicit `#[allow(unsafe_code)]` + `// SAFETY:`.

pub mod core;
pub mod handoff;
pub mod journal;
pub mod rt;
pub mod testing;

pub use core::{PriceRequest, PriceResponse, PricingCore};
pub use handoff::{HandoffError, restore_state, serialize_state};
pub use journal::{DurableBook, RecoveryError, recover};
pub use rt::{
    BookState, MarketState, PaddedCounter, PriceSnapshot, RequestRing, ResponseRing, Seqlock,
    StateHandle, StateReader, now_ticks, pin_current_thread_to_core, tick_hz,
};
