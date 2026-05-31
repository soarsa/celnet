<sub>**[Celnet Capabilities](../CELNET-CAPABILITIES.md)** › System Architecture</sub>

# 3. System Architecture

Celnet's architecture starts from a single design conviction: **adapt to the desk, never the other way around.** The same engine runs self-contained on a quant's laptop, beside a regional risk hub, or wired straight into the Celer trade lifecycle as the firm's FX-options pricing system-of-record — and it moves between those shapes through deliberate, reversible adapter swaps on a small set of seam traits, not a rewrite. Every market-data source, price sink, and order/execution path is a pluggable adapter, so a venue feed, a distributor, or a counterparty channel changes without touching the pricing core.

![Celnet system architecture and adaptability](../assets/celnet-capabilities/fig-01-system-architecture-adaptability.png)
*Figure 3.1 — One engine, three deployment shapes. Pricing, risk, and surface logic sit behind seam traits; market-data, price-sink, and order/execution adapters swap underneath them, so the same binary serves Standalone, Hybrid, and Celer-Integrated desks without forking the core.*

### 3.1 Two tiers: a hot core, an async edge

Celnet is built as two cleanly separated tiers joined by a wait-free seam.

The **async edge** is where the outside world meets Celnet: gRPC, a byte-identical WebSocket JSON mirror, and a real FIX engine all terminate here, alongside vendor-feed normalization and the egress governor. The edge is asynchronous, elastic, and free to do everything a networked service must — accept connections, fan out streams, coalesce updates, speak many protocols at once.

The **hot core** is the opposite discipline. Pricing runs on pinned, zero-allocation cores that never log, never lock, and never allocate on the hot path. All the work that would jitter a tail latency — memory management, telemetry, I/O — is kept off the core entirely. The result is in-core pricing fast enough that **network framing, not the mathematics, is the only meaningful latency**: the wire is the floor, and the maths sits comfortably beneath it.

The two tiers are bridged by **wait-free single-producer/single-consumer rings**. Requests cross from edge to core, and results cross back, without locks and without blocking either side — the edge stays responsive under load while the core runs uninterrupted.

| Tier | Responsibility | Discipline |
|------|----------------|------------|
| Async edge | Protocol termination (gRPC / WebSocket mirror / FIX), streaming, conflation, feed normalization, telemetry offload | Asynchronous, elastic, protocol-rich |
| SPSC rings | Edge↔core handoff of requests and results | Wait-free, lock-free, non-blocking |
| Hot core | Vanilla, Greek, surface, and exotic pricing | Pinned, zero-allocation, log/lock/alloc-free |

### 3.2 Lock-free state publication

The core prices against shared market state without ever stalling on a lock. State is **published**, not contended: the live market-state snapshot is swapped atomically so readers always observe a complete, consistent view; top-of-book is maintained through a single-writer seqlock that lets readers stream prices without blocking the writer; and hot counters are cache-padded to keep cores from contending on shared cache lines. Reads dominate, writes are rare and clean, and no pricing thread waits on another.

![Engine concurrency and state publication](../assets/celnet-capabilities/fig-13-engine-concurrency.png)
*Figure 3.2 — The read-mostly publication model: edge producers hand work across wait-free SPSC rings into pinned hot cores, which price against an atomically-swapped market-state snapshot and a single-writer seqlock top-of-book, while cache-padded counters and a bounded telemetry ring keep the hot path free of locks and allocation.*

### 3.3 A disciplined multi-crate workspace

Celnet is a large multi-crate Rust workspace with strictly **one-way dependencies**. Foundational vocabulary and the wire contract sit at the base; conventions, vanilla pricing, the surface engine, exotics, the GPU backend, the hot engine, the plugin host, integration, and the edge build upward on top of them, never sideways into each other. The crate boundaries are the architecture: a change rebuilds and re-tests only its own subtree, dependencies always point in one direction, and the design stays legible as it grows. An extensive automated test suite and regression-gated benchmarks ride along every boundary, so structure and behaviour are verified together rather than asserted.

### 3.4 Zero-downtime upgrades and durable recovery

Because the platform is mission-critical, it is designed to be upgraded and to survive a crash without losing a tick.

**Blue-green hot upgrade.** A new engine generation is brought up alongside the running one and the live state is handed off between them, so a version change happens without a pricing outage and without a mixed-version window — one clean contract, deployed uniformly.

**Durable journal.** Every state-changing event is written to a checksummed, append-only journal. On restart, Celnet replays the journal to reconstruct book and market state exactly, with clean crash recovery: a torn final write left by an abrupt kill heals to the last good record, while genuine interior corruption is surfaced rather than silently masked. Recovery is therefore both deterministic replay and a standalone write-ahead log — and it stays strictly off the hot path, so durability never taxes pricing latency.

### 3.5 Built to scale out

The same two-tier model is the unit of horizontal scale. A node-local hot substrate gives each instance its lock-free, zero-allocation pricing core; an intra-fleet partition map then shards the instrument universe across nodes by highest-random-weight assignment, so adding capacity rebalances minimally and predictably. The async edge fans streaming prices out to many counterparties at once, with the egress governor conflating and pacing each consumer independently. From a single laptop to a sharded fleet, the architecture is the same — only how many of it you run changes.

---
<sub>[← Capability Map](02-capability-map.md)  ·  **[Contents](../CELNET-CAPABILITIES.md)**  ·  [Quant & Pricing Methodology Coverage →](04-quant-coverage.md)</sub>
