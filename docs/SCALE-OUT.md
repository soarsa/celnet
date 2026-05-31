# Celnet — Distributed / Horizontal Scale-Out Architecture

> Design doc for scaling Celnet to investment-banking-sized portfolios and
> high-performance counterparty fan-out without violating the §1.2 latency budgets.
> Governing rule: **scale *around* the hot path, never *through* it.** This is a **design
> document, not a record of what is built.** §0 states precisely which primitives exist in
> the tree today vs. which are designed-but-unbuilt. Components marked *build now* are
> sequenced (planned next); components marked *defer* are gated on a measured bottleneck.
> Crate names match the real 19-crate tree. Validated against `docs/ARCHITECTURE.md` §1.2
> NFRs and current (2026) low-latency distributed-pricing literature (sources, end).

---

## 0. Built today vs. designed (no overclaim)

This doc describes the **target** distributed topology. The **single-shard substrate** that
the fleet layer composes already exists and is validated; the **HRW partition map + stateless
router primitives** (`celnet-router`) and the **cross-shard risk-aggregation algebra**
(`celnet-risk-fleet`: partition by `(legal-entity, ccy-pair)` → shard-local roll-up →
cross-shard additive merge + firm-level re-gather of non-additive measures, reconciled
**fan-out == single-node**) are now **built and validated in-process**. What remains
**designed, not yet built** is the **physical cross-node fleet plumbing**: the inter-DC
transport that ships partial aggregates between machines, the Raft/Aeron-style replicated
event log, and process-level hot-standby/failover/replay. No code performs *cross-node*
routing, consensus, or live partition assignment over a network — `celnet-risk-fleet`'s
shards are in-process logical shards (a local `Cube` standing in for a separate
`celnet-engine` node) that exercise the reduction algebra exactly, with no sockets/RPC
faking a live cluster. The honest split:

| Mechanism | Status in tree | Where |
|---|---|---|
| Node-local **whole-`MarketState` snapshot** via `arc-swap` (surface+curves+conventions) | **Built** | `celnet-engine` `rt::StateHandle`/`StateReader` (load path avoids `arc-swap`'s allocating guard) |
| Node-local **top-of-book** publication via single-writer **seqlock** (small `Copy` snapshot) | **Built** | `celnet-engine` `rt::Seqlock`, `core::PricingCore` |
| Core→edge **SPSC** ring (hot path stays SPSC) | **Built** (`rtrb`) | `celnet-engine`, `celnet-server` `core_link` |
| Edge async fan-in/fan-out + per-session stream | **Built** (tokio `mpsc` + bounded broadcast depth 256) | `celnet-server` `services/stream`, `core_link` |
| Blue-green **live-state handoff** over a hand-rolled little-endian codec | **Built** (not `rkyv`) | `celnet-engine` `handoff` |
| Multi-source MD aggregation + divergence detection | **Built** | `celnet-integration` |
| HRW/rendezvous **partition map** + stateless **router tier** primitives | **Built** | `celnet-router` (`map`/`key`/`hash`/`replica`/`backpressure`) |
| **Cross-shard risk-aggregation algebra** (partition → shard-local roll-up → additive merge + firm re-gather; reconciled fan-out == single-node) | **Built** (in-process logical shards) | `celnet-risk-fleet` |
| Physical **cross-node transport** (ship partial aggregates between machines) | **Designed only** | — (build-now, §12) |
| Thin **Raft/Aeron-style replicated log** + hot-standby + replay | **Designed only** | — (build-now, §12) |
| In-proc **LMAX-disruptor SPMC** fan-out ring | **Designed only** (today: tokio broadcast) | — (build-now, §12) |
| `SO_REUSEPORT` sharded accept + eBPF steering; `io_uring`/XDP tier | **Designed only** (readiness probe references the handoff intent) | — (build-now, §12) |
| DPDK/RDMA multicast tier | **Deferred** (ADR-gated on measured bottleneck) | — (§12) |

Everything below the §0 line is the **design**; treat "shard", "router", and "log" as the
target topology, not deployed components.

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

**Concrete capacity model (worked):** the partition unit is the **(ccy-pair → optional book/
tenant sub-key)** pair; the routing key is `hrw(node_i, partition_key)` → argmax node. Sizing
is driven by the §1.2 per-core budgets:

- *Liquid universe:* ~28 G10 pairs × O(10–20) standard tenors. Surface rebuild is
  per-pair-all-tenors at p99 ≤ 150 µs, so a single core comfortably owns several **cold/warm**
  pairs (rebuilds are event-driven, not continuous).
- *Hot pairs:* EURUSD/USDJPY/GBPUSD can each saturate a core on quote throughput
  (≥ 1M updates/s/core ceiling). These get a **dedicated shard**, and if a single pair still
  exceeds one core it is **sub-sharded by book/tenant** (the secondary key), with risk-netted
  books pinned together.
- *Tenant fan-out:* a large IB tenant's book set is spread for load but cross-pair-netted
  books are co-located on one shard where netting matters; everything else load-balances.
- *Scale-out shape:* capacity is added by **adding shards** (more nodes), each owning a
  disjoint HRW slice; adding/removing a node reshuffles only ~`1/N` of partitions (HRW
  property), and the partition map is versioned + gossiped so routers converge without a
  global token-ring recompute. This is **horizontal, not vertical** — a single shard never
  has to hold the whole IB portfolio.

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

- Each pricer reads a lock-free **`arc-swap`** snapshot of the whole live `MarketState`
  (surface + curves + conventions); the market-data consumer thread `publish()`es a new
  state atomically. This is **built today** in `celnet-engine` `rt::StateHandle`: the hot
  read path deliberately avoids `arc-swap`'s cheap-but-allocating load guard so it stays
  alloc-free even under concurrent publishes. Readers never block → near-zero staleness,
  zero cross-node read on the hot path, cache-coherent within the node. The small `Copy`
  top-of-book is published separately through a **single-writer seqlock** (`rt::Seqlock`),
  also built.
- Every quote/tick is tagged with the **surface epoch** it was priced against, so a price is
  always attributable to an exact surface version (deterministic replay).
- This reuses Celnet's existing `arc-swap` hot model-swap mechanism (the same primitive that
  publishes a new model set during a hot upgrade).
- **Fleet distribution of the log feeding these node-local snapshots is the designed-only
  part** (§0): today the snapshot is fed in-process by `celnet-integration`'s aggregator,
  not by a cross-node replicated log.

**Staleness vs throughput:** aggressive conflation lowers load but can serve a quote against a
slightly stale surface during a fast move. Mitigation: bound and measure publish-to-snapshot
lag, tighten conflation on volatile pairs, and **never** conflate the authoritative arb-free
update used for booking.

---

## 5. Market-data fan-out & counterparty streaming

- **In-process SPMC fan-out per shard** (LMAX-Disruptor pattern, *designed*): one producer
  (the pricing core) publishes every event; many consumer threads (one per counterparty
  session group) read the ring with batch consumption and no per-consumer queue contention.
  Mature Rust implementations exist (`disruptor`-style SPMC, broadcast rings). Note: a simple
  `crossbeam` SPSC/MPSC can beat the disruptor for trivial cases — the disruptor's win is
  multi-consumer fan-out with batch publish, which is exactly the quoting use case.
  **What is built today** (§0): the hot path is the `rtrb` **SPSC** core→edge ring, and the
  async edge fans out to per-session subscribers with a tokio `mpsc` + a bounded **broadcast
  depth 256**. The disruptor SPMC ring is the planned upgrade when measured fan-out fan-degree
  per shard makes the broadcast the bottleneck — it does **not** exist in the tree yet.
- **Network edge:** default to `SO_REUSEPORT` sharded accept + eBPF steering (the intended
  blue-green graceful-handoff mechanism; the readiness probe in `celnet-server` is wired for
  it, the socket-level handoff itself is *designed*) with conflation. Adopt `io_uring`/XDP as
  the next performance tier — current (2026) measurements put `io_uring` as a "hybrid bypass"
  (headers still through the kernel TCP stack) that is closing on, but not yet matching, full
  DPDK bypass, while being far easier to operate. Reserve full **DPDK** kernel-bypass for a
  dedicated ultra-low-latency multicast tier **only after** a single shard's edge fan-out is
  measured as the bottleneck — recorded as an ADR with the `io_uring` fallback kept
  first-class (mirroring the wgpu-vs-CUDA precedent).
- **Fan-out to *many* counterparties (cloud/many-receiver tier, designed):** for 100s–1000s of
  streaming counterparties, point-to-point unicast from one shard does not scale linearly and
  blows tail fairness. The target is a **proxy multicast tree** (Jasper, arXiv:2402.09527):
  fan-out `F=10`, depth `D = ⌈log₁₀ N⌉`, optional **VM hedging** (each proxy takes the
  first-arriving of redundant parent/sibling copies, `H=2`) to cut spatial latency variance,
  and **clock-synced fair delivery** (Huygens-style deadlines) so all counterparties see a
  quote within a bounded window — preventing a structural last-look advantage. Reported scale:
  median ~129 µs to 100 receivers, ~238 µs to 1000 receivers, 84–93% perfect-fairness
  probability, on a DPDK/eBPF datapath. This is the **defer/ADR tier**, gated on a measured
  single-shard fan-out limit; the OSS-first datapath (`io_uring`/eBPF) stays the fallback.
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

Celnet's per-shard zero-downtime upgrade (`SO_REUSEPORT` socket handoff *(designed)* + a
**built** little-endian live-state handoff codec in `celnet-engine` `handoff` — **not `rkyv`**;
zero-copy `rkyv` is a possible future optimization, not the current mechanism — plus the
built `arc-swap` model swap and a SHADOW-style pre-warm) is preserved per shard.
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

Beyond the per-shard §1.2 NFRs, the fleet adds measured (HdrHistogram) acceptance criteria.
Each row is a **benchmark to write before declaring the fleet layer built** — none are
asserted, all are measured against committed baselines (mirroring §1.2's discipline):

| SLO | Acceptance target | How to prove (benchmark) |
|---|---|---|
| In-shard price (regression guard) | p50 ≤ 2 µs / p99 ≤ 10 µs (= §1.2, unchanged by fleet) | `celnet-bench` divan + `iai-callgrind` instruction-count gate; must not regress when fleet code is linked in |
| Cross-shard routing overhead | router add p99 ≤ 25 µs (≤ ~10× an in-shard price; **never** on the critical price path — routing is connection-setup/subscription, not per-tick) | bench the router hop in isolation; assert the per-tick path never crosses it (architectural test, not just latency) |
| Surface publish→local-snapshot lag | p99 ≤ 150 µs (= surface-rebuild budget; staleness ≤ one rebuild) | timestamp at log append vs. `StateHandle::publish` visible to a reader; HdrHistogram in `celnet-engine` test harness |
| Sustained per-shard throughput | ≥ 1M price updates/s/core (= §1.2) at the above tail under fan-out load | load-gen N subscribers, measure producer steady-state under conflation |
| Many-counterparty fan-out tail | bounded delivery window across all subscribers (target: < quote-validity window) at 100/1000 subs | Jasper-style tree bench; record per-receiver delivery-time spread + fairness probability |
| Conflation correctness for slow subs | a slow subscriber **never** back-pressures the core (last-value wins; no unbounded queue) | inject a stalled consumer; assert core throughput + p99 unchanged and memory bounded |
| Failover time | ≤ ~2× election timeout, zero in-flight order loss | kill a primary under load; measure standby takeover + assert replay reproduces bit-identical prices via the f64 oracle |
| Fleet rolling upgrade | zero dropped connections / zero in-flight quote loss, one shard at a time, single uniform contract | drain+cutover one shard under live streams; assert no connection reset, no quote gap |

These rows should be appended to `docs/ARCHITECTURE.md` §1.2 **when** the fleet layer is
built — not before, to avoid documenting unmeasured targets as guarantees.

---

## 12. Build now vs defer

- **Build now** (planned next — *none of these exist in the tree yet*, see §0; the only piece
  already built is the per-shard node-local `arc-swap`/seqlock snapshot substrate): HRW
  partitioner + versioned partition map; stateless shard router; the **replicated MD log**
  feeding the (already-built) node-local snapshot; in-proc SPMC fan-out with conflation;
  hot-standby shard + deterministic replay from the log.
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

*Sources — internal: `docs/_research/api-obs-scale.json` (scaling topic);
`docs/ARCHITECTURE.md` §1.2/§5; `docs/OBSERVABILITY.md`; `docs/API-CLIENTS.md`; and a
read-only audit of the tree (`celnet-engine` `rt`/`core`/`handoff`, `celnet-server`
`core_link`/`services/stream`, `celnet-integration`) to fix overclaims — §0 now separates
built vs. designed, the §7 handoff is corrected from `rkyv` to the built little-endian codec,
and the §5 fan-out notes the built tokio-broadcast reality vs. the designed disruptor SPMC.
External (2026): rendezvous/HRW sharding for shard-key→node assignment (chaotic.land,
*Data Sharding Algorithms*); DPDK vs io_uring vs Linux-stack packet-processing comparison
(Linköping Univ., diva2:1789103) and io_uring "hybrid bypass" trajectory; Jasper — scalable
fair multicast for cloud financial exchanges (arXiv:2402.09527) for the many-counterparty
proxy-tree fan-out tier (F=10, D=⌈log₁₀N⌉, VM hedging, Huygens clock-sync fairness,
129–238 µs to 100–1000 receivers). The N/N-1 wire-compat phrasing was previously replaced
with the single-current-contract blue-green model per CLAUDE.md rule 9.*
