//! # `celnet-fanout` — lock-free SPMC broadcast ring
//!
//! A **single-producer / multi-consumer (SPMC) broadcast ring** for streaming
//! prices (and any `Copy` payload) from one pricing core to *many* independent
//! consumers — 100s to 1000s of counterparty-session readers per shard. This is
//! the in-process fan-out substrate referenced by `docs/SCALE-OUT.md` §5 and the
//! §11 fleet SLOs, made real and measured.
//!
//! ## Why a broadcast ring (not a work-stealing queue)
//!
//! Quote fan-out is **broadcast**: every subscriber must observe *the same*
//! published sequence — not a partition of it. Under CelNet's lock-free
//! multi-consumer architecture: one producer
//! writes a monotonic sequence into a power-of-two ring; each consumer holds its
//! **own** read cursor and re-reads the same slots. Consumers never contend with
//! one another (no shared read cursor), and the producer never waits on a
//! consumer on the hot path.
//!
//! ## Overflow policy — bounded + conflation with counted skips (chosen)
//!
//! We deliberately choose **(i) bounded ring with conflation/drop**, not
//! back-pressure. This is the FX-streaming-correct choice (`docs/SCALE-OUT.md`
//! §6 — *conflate, never buffer*): a slow counterparty must **never** be able to
//! stall the pricing core. When a consumer falls more than `capacity` items
//! behind, the producer has already overwritten the slots it had not yet read.
//! On its next read the consumer **fast-forwards to the oldest still-live item**,
//! counts the gap into a per-consumer `skipped` metric, and continues. The
//! invariant the gates assert is exact accounting:
//!
//! > `received + skipped == produced` (observed by that consumer), and every
//! > delivered item is an un-torn, in-order, never-duplicated published value.
//!
//! A slow consumer therefore always converges on the **latest** price with a
//! precisely counted number of conflated intermediates — the right semantics for
//! a market-data tape, where a stale quote is worse than a skipped one.
//!
//! ## Hot path is zero-allocation
//!
//! The ring's backing storage is allocated **once** at construction. `publish`
//! and `try_recv` perform only atomic loads/stores and a `Copy` of the payload
//! into/out of a pre-existing slot — no `alloc`/`realloc`, no locks, no syscalls.
//! Proven directly by `tests/zero_alloc.rs` with a counting global allocator
//! (mirrors `celnet-engine`'s zero-alloc proof).
//!
//! ## Honest measurement boundary
//!
//! The throughput number the gates print and assert is an **in-process,
//! single-host loopback measurement** on the test machine — an *upper* bound on
//! achievable fan-out compute throughput and a useful *relative* regression
//! signal. It is **not** a cross-host wire claim: absolute network fan-out
//! latency/throughput to remote counterparties is provable only on a tuned LAN /
//! the deployed datapath and stays **deploy-gated** (see `docs/SCALE-OUT.md` §5,
//! §11). This crate proves the *ring arithmetic, ordering, conflation accounting,
//! and zero-alloc publish* — nothing about NVIDIA or a live cross-DC fabric.

pub(crate) mod mem;
pub mod multi_lane;
pub mod padded_ring;
mod ring;

pub use multi_lane::{MultiLaneBroadcastRing, MultiLaneConsumer, MultiLaneConsumerFactory, MultiLaneProducer};
pub use padded_ring::{CachePaddedBroadcastRing, CachePaddedConsumer, CachePaddedProducer};
pub use ring::{BroadcastRing, Consumer, Producer, RecvError};
