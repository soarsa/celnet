//! Celnet mission-critical observability — structured logging/tracing, a metrics
//! facade, and HdrHistogram p50/p99/p99.9/p99.99 latency — designed so the
//! **pinned, zero-alloc hot core never logs, locks, or allocates**.
//!
//! # The hot-path-safe split
//!
//! Observability in a low-latency pricing engine cannot be allowed to perturb
//! the thing it measures. Celnet therefore splits observability across a
//! **producer/consumer boundary**:
//!
//! * **On the pinned hot core** the engine calls exactly one object — a
//!   [`HotProbe`] — which publishes a single `Copy` POD [`HotSample`] into a
//!   bounded wait-free SPSC ring ([`channel`]). That publish is **lock-free,
//!   allocation-free, format-free, and wall-clock-free**: it only does a ring
//!   push plus a `Relaxed` atomic store into a [`crossbeam_utils::CachePadded`]
//!   counter. If the ring is full the sample is *dropped* (telemetry is lossy by
//!   design) rather than blocking the core. The `tests/zero_alloc.rs` perf-guard
//!   proves the publish performs **zero heap allocations**.
//!
//! * **On a non-critical core** a [`TelemetryDrain`] pops the POD samples and
//!   does all the expensive work the hot path is forbidden from doing: feeding
//!   nanosecond latencies into coordinated-omission-corrected HdrHistograms
//!   ([`latency`]), incrementing the [`metrics_facade`] counters/gauges/
//!   histograms, and emitting structured JSON logs / audit records ([`logging`]).
//!
//! The audit log for the quote/trade lifecycle ([`logging::AuditRecord`]) flows
//! on a *separate, lossless* path — the [`audit`] channel ([`AuditSink`] /
//! [`AuditDrain`]). Unlike the lossy telemetry ring it is backed by an unbounded
//! queue, assigns a global gap-free sequence to every record, and **never drops**;
//! records are constructed on the non-critical async edge, so its backpressure
//! never reaches the pinned hot core. The `tests/audit_lossless.rs` guard proves
//! it drops nothing under heavy concurrent load.
//!
//! This crate deliberately has **no dependency on `celnet-engine` or
//! `celnet-server`**: it provides the primitives, and those crates wire them in.

// This crate's *library* code contains zero `unsafe`. The crate restates the
// workspace lint policy with `unsafe_code = "deny"` (see Cargo.toml) only so the
// allocation-counting perf-guard in `tests/zero_alloc.rs` can install a custom
// `#[global_allocator]` (irreducibly `unsafe`) behind an audited `#[allow]`.
#![deny(unsafe_code)]

pub mod audit;
pub mod channel;
pub mod latency;
pub mod logging;
pub mod metrics_facade;
pub mod record;

pub use audit::{AuditClosed, AuditDrain, AuditSink, audit_channel};
pub use channel::{HotProbe, TelemetryDrain, telemetry_channel};
pub use latency::{LatencyByKind, LatencyRecorder, LatencySnapshot, REPORTED_PERCENTILES};
pub use logging::{
    AuditRecord, AuditStage, LogClass, LogConfig, SubscriberInstallError, build_json_subscriber,
    init_json_subscriber,
};
pub use record::{ErrorClass, HotSample, OpKind, TickRate};
