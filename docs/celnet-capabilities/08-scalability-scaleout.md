<sub>**[Celnet Capabilities](../CELNET-CAPABILITIES.md)** › Scalability & Scale-Out</sub>

# 8. Scalability & Scale-Out

Celnet scales along two independent axes at once: **down** into a single node, where a pinned hot core prices an investment-bank-sized book without ever leaving cache or touching the allocator; and **out** across a fleet, where a partition-map fabric shards the universe and fans live prices to many counterparties. The same binary serves a one-desk standalone deployment and a firm-wide grid — you add nodes, not architecture.

![Scale-out fabric: node-local hot substrate, GPU backend, and horizontal partition-map sharding with fan-out](../assets/celnet-capabilities/fig-08-scaleout.png)
*Three layers compose into one scaling story: the in-core hot substrate on each node, a cross-platform GPU backend for batch-parallel work, and a horizontal scale-out fabric that shards by partition map and fans out to counterparties.*

### 8.1 The node-local hot substrate

Every Celnet node carries a complete pricing substrate. An async edge — gRPC, the byte-identical WebSocket JSON mirror, or FIX — hands work across wait-free single-producer-single-consumer rings into pinned, zero-allocation hot cores. Inside a core there are no locks, no logging, and no allocation on the pricing path: market state is published as an atomically-swapped snapshot, top-of-book rides a single-writer seqlock, and counters are cache-padded to avoid false sharing. Because in-core pricing runs at nanosecond scale, a single node already absorbs an investment-bank-sized portfolio — re-pricing whole books across the full Greek set without the maths becoming the bottleneck. The practical consequence: network framing, not computation, is the latency floor, so scaling the system is about moving *messages* efficiently rather than buying compute headroom for the *maths*.

State handoff is built for continuous operation. Blue-green zero-downtime handoff swaps a freshly-loaded core in for a running one without dropping a price, so capacity can be grown, drained, and hot-upgraded under live flow. A durable, checksummed, append-only journal gives each node clean crash recovery — a restarted node rebuilds its book and market state and rejoins, rather than cold-starting blind.

### 8.2 Cross-platform GPU acceleration

For the embarrassingly-parallel workloads — large strike/tenor batches, Monte-Carlo exotics, scenario grids — Celnet drives a pricing-backend abstraction over the cross-platform GPU stack (Metal, Vulkan, DX12), backed by a high-precision CPU oracle and a CPU SIMD fallback. A counter-based RNG is bit-identical between CPU and GPU, and every GPU result is reconciled against the CPU oracle, so acceleration never costs determinism: the same inputs yield the same numbers whether the desk runs on a workstation GPU, a server accelerator, or pure CPU. This lets a deployment use whatever silicon it has — and stay reproducible across a heterogeneous fleet.

| Scaling layer | What it provides | How a desk uses it |
|---|---|---|
| Node-local hot substrate | Pinned, zero-allocation, lock-free in-core pricing of an IB-sized book at nanosecond scale | One node prices a whole book and streams it; add nodes for capacity, not to relieve the maths |
| GPU backend | Cross-platform (Metal/Vulkan/DX12) batch acceleration with CPU oracle + SIMD fallback, deterministic CPU↔GPU | Offload large batch / Monte-Carlo / scenario work; reproducible on any silicon |
| Horizontal scale-out fabric | Partition-map sharding of the instrument universe + fan-out to many counterparties | Grow the trading universe and the client count by adding nodes; no central choke point |
| Conflating egress governor | Per-client conflation, token-bucket pacing, counted drops | Stream to slow and fast counterparties from the same source without head-of-line stalls |

### 8.3 Horizontal scale-out fabric

When one node is not the whole firm, Celnet fans out. An intra-fleet highest-random-weight (rendezvous) partition map assigns each currency pair / instrument deterministically to a node, so the trading universe shards cleanly across the fleet with minimal reshuffling when nodes join or leave — every node agrees on ownership without a coordinator on the hot path. Pricing and risk for a partition live where that partition lives; clients are routed to the owning node, and capacity grows by adding nodes rather than by widening a central server.

Fan-out to counterparties is governed for both fast and slow consumers. A conflating egress governor collapses superseded updates per subscriber, paces with a token bucket, and counts what it drops — a high-performance counterparty gets every tick while a slower one gets the latest coherent state, neither stalling the other or the source. The multiplex StreamSession carries many subscriptions over one bidirectional channel with per-subscription sequencing and gap-driven resync, so a single connection scales to a desk's full watchlist and recovers cleanly across reconnects.

### 8.4 One scaling model, no rewrite

The crucial property is that these layers compose without changing the contract. The identical unversioned API, the identical hot-core pricing, and bit-identical values hold whether Celnet runs as a single self-contained node on a trader's desk or as a sharded, GPU-accelerated, fanned-out fleet inside the firm's estate. A deployment moves along the scale-out curve by swapping adapters and adding nodes — never by re-implementing the platform — and zero-cost observability (tail-percentile latency histograms, coordinated-omission aware, on a bounded drop-on-full telemetry ring) plus regression-gated benchmarks keep that scaling honest as the fleet grows.

---
<sub>[← Performance & Latency](07-performance-latency.md)  ·  **[Contents](../CELNET-CAPABILITIES.md)**  ·  [API & Wire Contract + API-First Client Parity →](09-api-contract-parity.md)</sub>
