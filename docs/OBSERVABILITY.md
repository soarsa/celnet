The document is written to `/Users/adrian/code/celeroption/docs/OBSERVABILITY.md`.

Wrote `/Users/adrian/code/celeroption/docs/OBSERVABILITY.md` — Celnet's zero-hot-path-cost observability design. It covers, all mapped to owning crates:

- **Hot-path rule**: the pinned zero-alloc core (`celnet-engine`, `src/core.rs`/`src/rt.rs`) only bumps `Relaxed` `CachePadded` atomics, reads TSC/`cntvct`, and pushes one POD `PriceSample` to a bounded `rtrb` SPSC ring (drop-on-full for telemetry); all formatting/aggregation/export is offloaded to a drain thread on a non-critical, NUMA-local core.
- **Stack (concrete crates)**: `tracing`/`tracing-subscriber` (edge+drain only), `opentelemetry`+`opentelemetry-otlp` (metrics stable; tracing-export treated as pre-release and SLO-isolated), `hdrhistogram`, `usdt` probes, `crossbeam-utils::CachePadded`, plus `criterion`/`divan`/`iai-callgrind`.
- **HdrHistogram p50/p99/p99.9** with mandatory coordinated-omission correction (`record_correct`), per-`(OpKind, core_id)`, validated against the §1.2 budgets (vanilla p50 ≤ 2 µs / p99 ≤ 10 µs / p99.9 ≤ 25 µs), with both in-core and edge wire-to-wire / RFS quote-to-tick numbers.
- **Health/readiness/liveness**: `/livez` (core progress + drain heartbeat), `/readyz` (the existing `ReadinessGate` blue-green state machine in `src/readiness.rs`), `/healthz` composite, served via HTTP + the gRPC `Readiness` RPC.
- **Audit logging** for the quote/trade lifecycle on a *separate, lossless, backpressured* ring/drain with `fsync` and idempotency-store handoff across blue-green cutover (never drops, unlike telemetry).
- **Structured error taxonomy**: a `#[repr(u16)] ErrorClass` carried as a discriminant in the POD sample (no string/alloc on the path), with deterministic gRPC-status mapping.
- **Proof of zero hot-path cost**: the allocation-counting `GlobalAlloc` guard (`tests/zero_alloc.rs`) run *with telemetry enabled* must stay at zero allocations/syscalls (plus a negative control), and a telemetry-on-vs-off `criterion`/`iai-callgrind` benchmark regression gate against committed baselines.

New crate proposed: **`celnet-observability`** (POD records, rings, drain threads, histograms, OTLP, audit, error taxonomy), keeping `celnet-engine` free of `tracing`/`hdrhistogram`/`opentelemetry` deps and `celnet-server` owning the edge spans, OTLP lifecycle, and health endpoints.

This directly addresses pending task #18.