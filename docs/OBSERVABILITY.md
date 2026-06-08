# Celnet — Observability (zero hot-path cost)

> Design doc for `celnet-observability` and the observability seam between the pinned
> hot core (`celnet-engine`) and the async edge (`celnet-server`). The governing rule:
> **the hot core never logs, locks, or allocates** — all formatting, aggregation and
> export happen off the critical path. This document describes what is *implemented*
> today and explicitly marks what is *deferred*; it is kept in sync with the crate
> manifests (zero-legacy, CLAUDE.md rule 10).

---

## 1. Governing principle — measure without perturbing

Synchronous logging, lock-based metrics, and inline string formatting on a busy-poll
pricing core are stealth tail-latency killers: a single `format!` or mutex acquisition on
the path injects jitter that dwarfs a 2 µs price. So observability is split in two:

1. **On the hot core (`celnet-engine`):** the pinned, zero-alloc loop only bumps
   `Relaxed` atomic counters (each `CachePadded` to avoid false sharing) and pushes one
   plain-old-data `PriceSample` record into a bounded wait-free SPSC ring (`rtrb`). On a
   full ring the telemetry sample is **dropped** (telemetry is lossy by design — see §6).
   No formatting, no allocation, no syscall, no lock. This is proven, not asserted (§5).
2. **Off the path (`celnet-observability` drain + `celnet-server` edge):** a drain thread
   pinned to a non-critical core consumes the ring, folds samples into HdrHistograms,
   updates `metrics` facade counters/gauges, and emits `tracing` spans/events. All cost
   lives here, where jitter does not touch a quoted price.

Deferred formatting is dramatically cheaper on the path than inline formatted logging: the
core pays one atomic store + one ring push (~ns), the drain pays the formatting cost
amortized across many samples.

---

## 2. Stack (implemented vs deferred)

The **implemented** stack (verified against `crates/celnet-observability/Cargo.toml` and
`crates/celnet-bench`):

| Concern | Crate / mechanism | Where it runs |
|---|---|---|
| Structured events & spans | `tracing` + `tracing-subscriber` (`time` feature) | edge + drain only — **never** the hot core |
| Latency distributions | `hdrhistogram` (p50/p99/p99.9, coordinated-omission aware) | drain thread |
| Counters / gauges / histograms facade | `metrics` 0.24 | drain + edge |
| Wall-clock micro-benchmark / latency proof | `divan` (`celnet-bench`) | CI / offline |
| Jitter-free instruction-count regression gate | `iai-callgrind` (`celnet-bench` `benches/iai_instructions.rs`) | CI Linux/Valgrind lane (`iai-instructions`) |
| False-sharing-free hot counters | atomics padded to a cache line | hot core |
| Hot→drain transport | bounded wait-free SPSC ring (`rtrb`) | core→drain |

The `iai-callgrind` instruction-count gate (deferred when this doc was first
written, now **built** — `celnet-bench/Cargo.toml` dep + `benches/iai_instructions.rs`,
runner `iai-callgrind-runner` on the CI `iai-instructions` Linux/Valgrind lane)
complements `divan`: `divan` reports wall-clock medians (jitter-prone), while
Callgrind counts retired instructions deterministically, so a code-level
regression (an extra branch, a lost inlining, an accidental allocation on the hot
path) is caught the moment it lands rather than hiding under measurement noise.

**Deferred (not yet a dependency — do not claim as built):**

- **OTLP export** (`opentelemetry` / `opentelemetry-otlp`). OTel Rust *metrics* are stable
  and *tracing-export* is still pre-release; an OTLP bridge from the `metrics`/`tracing`
  layer is on the roadmap but not wired today.
- **USDT probes** (`usdt`) — free when disabled, but not yet attached.

These are listed so no consumer over-reads the observability posture; promote them to the
implemented table only when they appear in the manifest with passing tests.

---

## 3. Latency histograms & coordinated-omission correction

HdrHistogram records p50/p99/p99.9 per logical operation, keyed by `(OpKind, core_id)` so a
slow core or op-class is visible rather than averaged away. Tail measurement uses
**coordinated-omission correction** (`record_correct` with the expected inter-arrival
interval): a stall that delays subsequent samples is back-filled, so a GC-free busy-poll
core that hiccups cannot hide its tail behind the missing samples it would otherwise drop.

Two measurement planes:

- **In-core latency** — the time the pinned loop spends pricing + risk for one request,
  captured from the monotonic cycle counter (TSC / `cntvct` on aarch64) and pushed in the
  POD sample; folded into the histogram by the drain. Compared against the
  `docs/ARCHITECTURE.md` §1.2 budgets (vanilla price + full Greeks p50 ≤ 2 µs / p99 ≤ 10 µs
  / p99.9 ≤ 25 µs).
- **Edge wire-to-wire** — request-arrival to response-flush at the `celnet-server` edge, and
  RFS quote-to-tick, measured on the async side where blocking is acceptable.

The committed `celnet-bench` (`divan`) reference run on the Apple M4 single core is the
offline counterpart of the in-core plane and is the single source of truth for the headline
latency numbers cited elsewhere (see `crates/celnet-bench/benches/README.md`).

---

## 4. Health, readiness, liveness

The async edge exposes three orthogonal signals, composed from engine state rather than
from log scraping:

- **`/livez`** — the core is making progress (counter monotonically advancing) and the
  drain thread heartbeat is fresh; a wedged core or dead drain fails liveness.
- **`/readyz`** — gated by the blue-green readiness state machine: a draining instance
  reports *not ready* so the router stops sending it new connections during cutover, while
  in-flight work finishes (see `docs/ARCHITECTURE.md` §5 and `docs/SCALE-OUT.md` §7).
- **`/healthz`** — composite of the two plus dependency checks.

These are served over HTTP and mirrored on the gRPC `Readiness` RPC so both transports of
the single current contract agree on instance state.

---

## 5. Proof of zero hot-path cost

The zero-cost claim is *tested*, not asserted:

- An allocation-counting global allocator guard (`tests/zero_alloc.rs`) runs the hot loop
  **with telemetry enabled** and asserts **zero** heap allocations and zero syscalls on the
  path, plus a negative control that deliberately allocates so the guard is proven to fire.
- A telemetry-on vs telemetry-off `divan` benchmark bounds the residual cost of the atomic
  counter bump + ring push, gated against the committed `celnet-bench` baseline so a
  regression that adds path cost fails CI.

---

## 6. Telemetry (lossy) vs audit (lossless)

Telemetry and the quote/trade **audit** trail have opposite delivery guarantees and so use
**separate** rings and drains:

- **Telemetry** — drop-on-full. Losing a latency sample is acceptable; back-pressuring the
  pricing core to deliver one is not.
- **Audit** — the quote/RFQ/execution lifecycle is written to a *separate, lossless,
  back-pressured* ring with `fsync` on the drain side and idempotency-store handoff across
  a blue-green cutover, so it **never** drops a booking-relevant event. A slow audit sink
  applies back-pressure to the edge, never to the hot core.

A structured `#[repr(u16)] ErrorClass` discriminant travels inside the POD sample (no
string, no allocation on the path) and maps deterministically to gRPC status at the edge,
so error taxonomy is observable without formatting on the core.

---

## 7. Crate ownership

`celnet-observability` owns the POD records, the rings, the drain threads, the
HdrHistograms, the `metrics` registration and the error taxonomy. This keeps `celnet-engine`
free of `tracing`/`hdrhistogram`/`metrics` dependencies (so the hot core cannot accidentally
take a logging or allocating dependency), while `celnet-server` owns the edge spans and the
health/readiness endpoints.

---

*Sources: `docs/_research/api-obs-scale.json` (observability topic);
`crates/celnet-observability/Cargo.toml`; `crates/celnet-bench/benches/README.md`;
`docs/ARCHITECTURE.md` §1.2/§3.3/§5. Stale opentelemetry/usdt/criterion/iai-callgrind
references from the prior draft have been demoted to the deferred list to match the real
manifest.*
