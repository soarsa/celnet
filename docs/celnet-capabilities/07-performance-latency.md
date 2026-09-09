<sub>[← Prev: Risk Management](06-risk-management.md) · [Index](../CELNET-CAPABILITIES.md) · [Next: Scalability & Scale-Out →](08-scalability-scaleout.md) · [Showcase ↗](../celnet-capabilities.html)</sub>

# 7. Performance & Latency

Celnet is built so that **pricing is never the bottleneck**. The maths runs in nanosecond-scale pinned hot cores; by the time a quote reaches a counterparty, network framing — not computation — is the only meaningful latency. The desk experiences a platform whose response time is governed by the wire, not by the model. And — uniquely among platforms that make this claim — Celnet *proves* it: every latency number below is measured, asserted by a regression gate, and labelled with exactly what it does and does not cover.

![A latency ladder showing the in-core pricing path, the wait-free ring handoff, the async edge, and the network as the dominant remaining cost.](../assets/celnet-capabilities/fig-07-performance-ladder.png)

*Fig 7 ([index](../CELNET-CAPABILITIES.md#figure-index)) — The Celnet latency ladder: a pinned, allocation-free hot core feeds a wait-free ring that bridges to the async edge — the network is the only layer left that matters.*

## Pricing as a non-event

The full FX desk Greek set — price plus 13 Greeks — is computed in a single pass over the pinned hot core, with no locks, no allocations, and no logging on the critical path. Because in-core valuation completes in nanosecond-scale time, batch revaluation across many instruments and tenors stays comfortably ahead of the streaming cadence a desk demands. A price request never waits on the maths — it waits, briefly, on the network that carries it.

## Architecture that keeps the core hot

| Layer | What it does for latency |
|-------|--------------------------|
| Pinned hot cores | Zero-allocation, lock-free, log-free valuation — the critical path does nothing but compute |
| Wait-free SPSC rings | Single-producer/single-consumer handoff bridges the async edge to the cores without contention |
| Read-mostly state publication | Atomically-swapped market-state snapshots and a single-writer seqlock top-of-book let readers price without blocking writers |
| Lock-free SPMC fan-out ring | Each pair's tick is broadcast to all subscribers from one producer with exact, accounted conflation under back-pressure — never a torn read, never an allocation |
| Cache-padded counters | Telemetry and bookkeeping never cause false sharing against the hot path |
| Blue-green state handoff | Hot upgrades swap state with zero downtime, so performance is sustained across deploys |

The result is a clean separation: the async edge — gRPC, the byte-identical WebSocket JSON mirror, and FIX — absorbs the messy realities of connection management, while the cores stay pristine and deterministic.

### Measured, gated — the in-core §1.2 truth-gate

Speed that is only asserted is no speed at all. The headline latency budget in `docs/ARCHITECTURE.md` §1.2 is committed as **absolute per-option percentile ceilings**, and Celnet ships a binary that measures them on every run and **exits non-zero if any percentile is breached** (`crates/celnet-bench/src/bin/core_load.rs`). It times *each individual* `celnet_vanilla::greeks` call (price + the full 13-Greek set — the exact quantity the budget governs) into a coordinated-omission-aware HdrHistogram across 10 million timed samples after a 2-million-sample warmup, with priority elevation and (where the OS permits) core pinning.

| Metric | §1.2 budget | Measured (Apple M4, single core) | Margin |
|---|---|---|---|
| **p50** | ≤ 2 µs | **42 ns** | **~48× inside** |
| **p99** | ≤ 10 µs | **125 ns** | **~80× inside** |
| **p99.9** | ≤ 25 µs | **~1.0–1.4 µs** | **~18–25× inside** |
| throughput | (≥ 1M/s) | **~13.0 M opt/s/core** | **~13×** |

*Source: committed snapshot `crates/celnet-bench/baselines/core_path.json`; reference table in `crates/celnet-bench/benches/README.md`.* These figures are **host-local, single-core** — the intrinsic cost of the pricing core, not a wire latency. (`pinned: false` on Apple Silicon is reported honestly: macOS exposes no per-thread CPU affinity, so pinning is a no-op there and the binary says so rather than faking it; priority elevation still applies, and on Linux CI both apply. The genuine OS-jitter deep tail appears only at p99.99+, *deeper* than the gate, and is reported transparently — never used to weaken the gate.)

A companion **surface-rebuild §1.2 gate** (`surface_rebuild.rs`) measures the per-tick sticky-delta recompute of all 11 standard tenors × 11 strikes for both the Vanna-Volga and SSVI models, asserting the p99 ≤ 150 µs budget — measured ~19.6 µs (VV) / ~7.8 µs (SSVI), i.e. ~7.7× / ~19× inside. The one-off cold calibration (~2.2 ms VV / ~3.4 ms SSVI) is *reported but deliberately not §1.2-gated*, because it runs on a quotes change, not on every spot tick — surfaced, never mis-gated against a budget that does not govern it.

### A second, machine-independent gate: instruction counts

Wall-clock time floats with host load, so a CI gate on it must carry a generous tolerance. Celnet adds a complementary **instruction-count gate** (`crates/celnet-bench/benches/iai_instructions.rs`, via `iai-callgrind`/Callgrind) that measures a **deterministic, machine-independent** quantity — retired instructions, cache accesses and estimated cycles per pricing call — over `price`, `greeks` (the §1.2 quantity) and `batch_greeks` (a 64-strike slice). That makes it the right primitive for catching a *code-level* regression — an extra branch, a lost inlining, an accidental allocation — the moment it lands, with a tight baseline and no flake. Callgrind is Linux/Unix-only, so this runs as a dedicated Linux CI lane; the bench merely *compiles* on the M4 dev host, so it never affects `just check`/nextest there.

### Observability that costs nothing on the hot path

Mission-critical operation demands visibility, and Celnet delivers it without taxing the very latency it measures. Latency is captured into tail-percentile histograms that are **coordinated-omission aware**, so the numbers reflect what counterparties actually experience rather than flattering averages — and those same percentiles are surfaced live to clients on the stream Heartbeat (server-side p50/p99/p99.9 in nanoseconds, plus conflation-drop counts). Telemetry leaves the hot core over a **bounded, drop-on-full ring**: under pressure the platform sheds observability data, never throughput. The pinned core remains log-, lock-, and allocation-free; everything instrumental is offloaded.

### Performance held by regression gates

A change that would slow the hot path is caught before it lands. The `bench_gate` binary runs **four arms** on every PR: the in-core §1.2 absolute gate (arm 1), the surface-rebuild §1.2 absolute gate (arm 1b), the wire-path *relative* regression gate (arm 2, fail on >2× the committed baseline), and the fleet §11 loopback SLO gate (arm 3) — all four must pass. Performance is therefore a property the platform continuously proves, not a one-time claim.

### The cross-platform GPU path: correctness and ratios

For batch revaluation, Celnet drives a `wgpu`/Metal GPU backend whose results are reconciled **node-by-node** against the f64 CPU oracle and the QuantLib golden tables. The `gpu_load` harness sweeps the batch size and reports per-dispatch latency tails plus a **host-local GPU/CPU throughput ratio**: on the M4, dispatch amortization runs from **0.68× at 4 096 paths** (small batches lose to dispatch overhead) up to **~53× at 1 048 576 paths** (the device saturates and the GPU pulls decisively ahead) — `crates/celnet-bench/src/bin/gpu_load.rs`, sweep `[4 096 … 1 048 576]`. The committed CI-sized baseline (`baselines/gpu_batch.json`) records the trimmed sweep up to 262 144 paths at ~16.5×, gated for relative regression by `gpu_gate`. When no GPU adapter is present the backend falls back to the CPU oracle and the run is honestly labelled a completes-within-ceiling check, not a speedup claim.

### The wire-path, kept honest

The in-core figures above are the *compute floor*. The figure a real counterparty observes is the **wire-path round-trip**. The `wire_load` harness spins the real `celnet_server::Edge` in-process on a loopback port, opens concurrent RFS streaming subscriptions as sustained background load, and times `RequestQuote` round-trips through the full tonic/gRPC/HTTP-2 stack + codec + async⇄core SPSC hop + pricing. The wire-path is ~4 orders of magnitude above the in-core floor — dominated by framing, not by the model — which is exactly the point: **pricing is never the bottleneck.** These loopback numbers are an **upper bound on compute+framing** and a **lower bound on real cross-host wire latency** (which adds NIC + switch + propagation). They are never converted into a concrete cross-host headline.

---

### Honest boundary

The in-repo proofs above establish the **in-core §1.2 truth-gate**, the surface-rebuild gate, the instruction-count gate, and **loopback** wire/fleet benches — and nothing beyond that. The following are **deploy-gated** and are never claimed as in-repo-proven:

- **Cross-host wire p99 / kernel-bypass NIC latency** — deploy-gated; in-repo proves the in-core §1.2 truth-gate + loopback benches only.
- **The §11 ABSOLUTE wire-latency SLOs** — deploy-gated (in-repo proves the §1.2 truth-gate + loopback only).
- **CUDA/NVIDIA ABSOLUTE GPU throughput + ≤50ms exotic + Workload-A/B absolute numbers** — deploy-gated. M4 Metal lacks f64 ⇒ in-repo proves **correctness + RATIOS only** (M4/Lavapipe). Never f64 on Metal.

Network framing — not computation — is the only meaningful latency floor, and Celnet states it as a *shape*, never as a fabricated wire number.

**See also:** [§8 Scalability & Scale-Out](08-scalability-scaleout.md) extends the hot-core latency story across the fleet; [§3 System Architecture](03-system-architecture.md) is the two-tier model these gates measure; [§14 Engineering Rigor](14-engineering-rigor.md) holds the regression and instruction-count gates.

---
<sub>[← Prev: Risk Management](06-risk-management.md) · [Index](../CELNET-CAPABILITIES.md) · [Next: Scalability & Scale-Out →](08-scalability-scaleout.md) · [Showcase ↗](../celnet-capabilities.html)</sub>
