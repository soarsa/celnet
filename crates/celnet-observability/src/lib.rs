//! Celnet mission-critical observability — structured logging/tracing, metrics,
//! and HdrHistogram p50/p99/p99.9 latency — designed so the **pinned, zero-alloc
//! hot core never logs, locks, or allocates**: telemetry is offloaded over a
//! bounded SPSC queue to a non-critical core, latency is recorded with
//! coordinated-omission awareness, and a perf-guard proves zero hot-path
//! allocation (work-stream WS-T/observability). Skeleton — implementation lands
//! in this lane.
#![forbid(unsafe_code)]
