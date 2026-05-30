# Celnet — Distributed / Horizontal Scale-Out Architecture

> Design doc for scaling Celnet to investment-banking-sized portfolios and
> high-performance counterparty fan-out without violating the §1.2 latency budgets.
> Governing rule: **scale *around* the hot path, never *through* it.** This is a design
> document; components marked *build now* are sequenced into the engine/integration crates,
> components marked *defer* are gated on a measured bottleneck. Crate names match the real
> 19-crate tree.

---

## 1. Governing principle — distribute for capacity, not for speed

The latency-critical work (surface rebuild, pricing, Greeks, quoting) stays on a **fat,
NUMA-pinned, thread-per-core single node**. Distribution exists only for **capacity,
fault-tolerance, and fan-out** — never to make a single price faster.

The single-thread / single-node ceiling is extremely high. LMAX's business-logic processor
sustains ~6M events/s on **one** thread by keeping caches warm and avoiding
main-memory/cross-core traffic; Celnet already targets ≥1M price updates/s/core and p50 ≤
2 µs vanilla on a pinned core. Crossing a node boundary on the hot path is **disqualifying**:

| Hop | Added latency | vs. p50 ≤ 2 µs budget |
|---|---|---|
| Kernel network stack | ~20–50 µs | 10–25× over budget |
| DPDK userspace floor | ~7 µs | ~3.5× over budget |
| Raft quorum (consensus) | ms-scale | off the path entirely |

So any design that reads the surface remotely, or routes a single price through the cluster,
blows the SLA. Distribution belongs *around* shards and via *node-local* snapshots.

---

## 2. Partitioning — shard per pinned node, by currency-pair

- A **shard** = one `celnet-engine` process, pinned thread-per-core on a NUMA-local node,
  owning a **disjoint** set of partitions. Pricing, surface rebuild, Greeks and quoting for
  an owned partition happen entirely in-node with zero cross-node hop on the hot path.
- **Primary partition key = currency-pair.** All tenors of a pair stay **co-resident** on
  one shard because surface rebuild is per-pair-all-tenors (§1.2 targets p99 ≤ 150 µs) and
  must not cross nodes.
- **Sub-shard hot pairs by book/tenant** where a single pair (EURUSD, USDJPY, GBPUSD) exceeds
  one core. Co-locate a tenant's correlated cross-pair books on the same shard where risk
  netting matters; otherwise spread tenants for load balance.
- **Assignment via rendezvous (HRW / highest-random-weight) hashing**, preferred over classic
  consistent hashing: no token ring to precompute, better load balance, minimal reshuffle on
  shard add/remove. The partition map is **versioned and gossiped**; known-hot pairs support
  a manual pin/override.

**Tension (documented, not hidden):** partitioning by pair maximizes pricing locality but can
split a tenant's cross-pair risk across shards. Joint portfolio Greeks / IPV run **off the
hot path** as a separate fan-in, not by co-locating everything.

---

## 3. Stateless router tier

A **stateless shard router/edge tier** maps each request/subscription to its owning shard via
the same HRW function and forwards over a fast intra-DC transport. Routers hold **no** pricing
state — only the versioned partition map — so they are the one component that scales out "for
free." Optional read-only reflectors fan out indicative (non-tradable) prices without loading
the authoritative shard.

---

## 4. Surface / curve state distribution

Shared state (the arbitrage-free surface + curves) is distributed as an **event-sourced,
replicated log**, fanned out to a **node-local** surface cache per shard — never read remotely
per price:

- Each pricer reads a lock-free **`arc-swap`** snapshot of the latest surface; the
  market-data consumer thread swaps in new surfaces atomically. Readers never block →
  near-zero staleness, zero cross-node read on the hot path, cache-coherent within the node.
- Every quote/tick is tagged with the **surface epoch** it was priced against, so a price is
  always attributable to an exact surface version (deterministic replay).
- This reuses Celnet's existing `arc-swap` hot model-swap mechanism (the same primitive that
  publishes a new model set during a hot upgrade).

**Staleness vs throughput:** aggressive conflation lowers load but can serve a quote against a
slightly stale surface during a fast move. Mitigation: bound and measure publish-to-snapshot
lag, tighten conflation on volatile pairs, and **never** conflate the authoritative arb-free
update used for booking.

---

## 5. Market-data fan-out & counterparty streaming

- **In-process SPMC fan-out per shard** (LMAX-Disruptor pattern): one producer (the pricing
  core) publishes every event; many consumer threads (one per counterparty session group)
  read the ring with batch consumption and no per-consumer queue contention. This is the
  pattern `celnet-integration` bridges to for the Celer in-proc distributor; mature Rust
  implementations exist (`disruptor`-style SPMC, broadcast rings). Note: a simple
  `crossbeam` SPSC/MPSC can beat the disruptor for trivial cases — the disruptor's win is
  multi-consumer fan-out with batch publish, which is exactly the quoting use case.
- **Network edge:** default to `SO_REUSEPORT` sharded accept + eBPF steering (already used for
  blue-green graceful handoff) with conflation. Adopt `io_uring`/XDP as the next performance
  tier. Reserve full **DPDK** kernel-bypass for a dedicated ultra-low-latency multicast tier
  **only after** a single shard's edge fan-out is measured as the bottleneck — recorded as an
  ADR with the `io_uring` fallback kept first-class (mirroring the wgpu-vs-CUDA precedent).
- **One contract, two transports:** the typed gRPC bidi stream is the primary low-latency
  path; the WebSocket mirror is the firewall-friendly transport. Both serialize the **same**
  single current contract (see `docs/API-CLIENTS.md`).

---

## 6. Backpressure — conflate, never buffer

For slow counterparties the correct primitive is **conflation** (last-value-cache /
coalescing) at the edge, never unbounded buffering. A slow subscriber can **never**
back-pressure the pricing core. As in observability (`docs/OBSERVABILITY.md` §6), the audit
stream is the one exception: it is lossless and back-pressures the edge, never the core.

---

## 7. Blue-green across a fleet

Celnet's per-shard zero-downtime upgrade (`SO_REUSEPORT` socket handoff + `rkyv` zero-copy
live-state transfer + `arc-swap` model swap + SHADOW-style pre-warm) is preserved per shard.
The distributed layer adds a **router-coordinated rolling upgrade**: drain and cut over **one
shard at a time** while hot standbys cover, preserving zero dropped connections and zero
in-flight quote/order loss across the fleet. There is **no mixed-version window** — the wire
is a single current contract, so the fleet converges to one uniform version (CLAUDE.md rule 9;
this replaces the superseded N/N-1 wire-compat model).

---

## 8. Consistency, failover & replay

A **thin** Raft-style replicated event log (Aeron-Cluster pattern) carries only the
must-order, durable events (authoritative market-data updates and the quote/trade lifecycle);
pricing math stays **off** consensus and replays from the log asynchronously. This yields:

- deterministic replicated-state-machine rebuild (bit-identical, reconciled by the f64 CPU
  oracle), seeded so RNG draws are reproducible from log position;
- **hot-standby** shards (preferred over dynamic membership) with failover bounded by ~2×
  election timeout;
- **resync from `last_seq`** for reconnecting consumers without message loss — the same
  resync semantics the RFS stream already exposes.

Routing pricing/quoting *through* Raft would serialize on quorum and destroy tail latency, so
the cluster is kept deliberately thin.

---

## 9. Decision matrix

| Workload | Shape | Rationale |
|---|---|---|
| Single price / quote for an owned pair | Fat pinned node, in-shard | Any cross-node hop blows p50 ≤ 2 µs |
| Surface rebuild (pair, all tenors) | In-shard, node-local arc-swap | Per-pair-all-tenors; must not cross nodes |
| Capacity beyond one node | Add shards (HRW) + stateless routers | Linear capacity, minimal reshuffle |
| HA / failover | Hot-standby shards + thin Raft log | Bounded failover, deterministic replay |
| Counterparty fan-out | In-proc SPMC ring + edge conflation | Slow consumers never touch the core |
| Cross-node ultra-low-latency multicast | **Defer** (DPDK/RDMA, ADR) | Only on a measured bottleneck |

**Recommendation:** stay **single-node per shard** for pricing/quoting; distribute only at the
granularity of shards (capacity + HA) and the market-data/event log (consistency + replay).

---

## 10. Deployment topology

```
            ┌─────────────────────────────────────────────────────────┐
            │ stateless router tier (HRW map; no pricing state)        │
            └───────┬───────────────────┬───────────────────┬─────────┘
                    │                   │                   │
            ┌───────▼──────┐    ┌───────▼──────┐    ┌───────▼──────┐
            │ shard A       │    │ shard B       │    │ shard C       │
            │ EURUSD,EURGBP │    │ USDJPY,...    │    │ GBPUSD,...    │
            │ pinned cores  │    │ pinned cores  │    │ pinned cores  │
            │ node-local    │    │ node-local    │    │ node-local    │
            │ arc-swap surf │    │ arc-swap surf │    │ arc-swap surf │
            └───────┬───────┘    └───────┬───────┘    └───────┬───────┘
                    │  node-local snapshot fed by ↓           │
            ┌───────▼──────────────────────────────────────────▼──────┐
            │ thin Raft/Aeron-style replicated event log               │
            │ (authoritative MD + quote/trade lifecycle; replay)       │
            └──────────────────────────────────────────────────────────┘
                       + hot-standby shards cover failover/upgrade
```

---

## 11. Fleet-level SLOs

Beyond the per-shard §1.2 NFRs, the fleet adds measured (HdrHistogram) targets:

- cross-shard routing overhead budget (router add ≪ in-shard price);
- surface-staleness bound (publish-to-local-snapshot lag);
- failover time bound (~2× election timeout);
- conflation latency for slow subscribers.

These should be appended to `docs/ARCHITECTURE.md` §1.2 when the fleet layer is built.

---

## 12. Build now vs defer

- **Build now:** HRW partitioner + versioned partition map; stateless shard router; per-shard
  node-local surface snapshot (`arc-swap`) fed by a replicated MD log; in-proc SPMC fan-out
  with conflation; hot-standby shard + deterministic replay from the log.
- **Defer (gate on measured single-shard limits):** full DPDK multicast tier; RDMA cross-node
  fabric; dynamic cluster membership; any cross-node distribution of a single option's pricing.

---

## 13. Risks

1. **Cross-node hop on the hot path is fatal** — mitigate with node-local snapshots only.
2. **Hot-pair skew** (EURUSD/USDJPY) overwhelms one shard — sub-shard by book/tenant, allow
   manual partition-map pins, monitor and rebalance.
3. **Surface co-residency vs risk netting** — align jointly-netted books on one shard, run
   portfolio risk off the hot path.
4. **Staleness from conflation** — bound and measure publish-to-snapshot lag, tag quotes with
   surface epoch, never conflate booking-authoritative updates.
5. **Consensus on the hot path** — keep the cluster thin; only durable lifecycle events go
   through Raft.
6. **DPDK/RDMA ops cost** — defer behind `io_uring`/`SO_REUSEPORT`, gate on a measured
   bottleneck, document as an ADR with the OSS fallback first-class.
7. **Fleet upgrade correctness** — versioned partition map + router-coordinated drain +
   standby coverage; the single current contract removes the mixed-version-window hazard.
8. **Determinism across the distributed layer** — drive all pricing inputs from the ordered
   replicated log (single source of truth), seed RNG from log position, reconcile via the f64
   CPU oracle.

---

*Sources: `docs/_research/api-obs-scale.json` (scaling topic); `docs/ARCHITECTURE.md` §1.2/§5;
`docs/OBSERVABILITY.md`; `docs/API-CLIENTS.md`. Crate references corrected to the real tree
(`celnet-engine`/`celnet-integration`; there is no `celnet-distributor`), and the N/N-1
wire-compat phrasing replaced with the single-current-contract blue-green model per CLAUDE.md
rule 9.*
