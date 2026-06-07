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

This doc describes the **target** distributed topology. Most of it is now **built and
validated**; what remains is cross-DC *hardening* and the durability tier. The split:

- **Built — in-process algebra:** the HRW partition map + stateless router primitives
  (`celnet-router`) and the cross-shard risk-aggregation algebra (`celnet-risk-fleet`:
  partition by `(legal-entity, ccy-pair)` → shard-local roll-up → cross-shard additive
  merge + firm-level re-gather of non-additive measures, reconciled **fan-out ==
  single-node**).
- **Built — configurable distributed serving (real gRPC, multi-process):** a single
  **`FleetTopology`** knob (`celnet-risk-fleet`), bound at deploy time (`CELNET_FLEET_MODE`
  / `CELNET_FLEET_BACKENDS`), selects **`InProcess` (the default — one process, byte-identical
  to single-node)** or **`Distributed { endpoints }`**. In Distributed mode the
  `celnet-server` edge is a **client of the same `RiskService` it serves** and federates
  across **N backend `celnet-server` processes** over real gRPC: it **federates** RiskService
  (additive summed in wire space — linear, exact, cheap; non-additive re-gathered over the
  union of constituents and re-derived once) and **forwards** owned-pair Pricing/Quote/Surface
  (and relays Stream) by health-aware HRW `route` to the owning backend. An unreachable
  partition slice ⇒ `Status::unavailable` (never a silently-smaller firm number). Proven by an
  integration test (multiple gRPC backends on ephemeral ports) **and a runnable OS-process
  harness** (`cargo run -p celnet-server --example scale_harness`) that spawns real backend +
  edge processes, drives every API capability through the edge, and holds the firm aggregate +
  prices **invariant across node scale-up/scale-down** (3→4→2) — the stateless edge scales by
  being repointed at the new fleet. No `celnet.proto` change (single current contract).
- **Built (the durability/replication tier — `celnet-replog`):** **full Raft consensus** over real
  loopback sockets. A cluster of `RaftNode`s **elects a leader** (randomized election timeouts +
  RequestVote with the §5.4.1 up-to-date voting rule + **Pre-Vote** so a flaky node cannot disrupt a
  healthy leader; persistent `current_term`/`voted_for`/commit-watermark); the leader replicates via
  **AppendEntries with the §5.3 log-matching property** and **durable conflicting-tail truncation**
  (an atomic write-fresh→fsync→rename journal rewrite — the journal is the source of truth, never a
  memory-only mask); an entry **commits** under the §5.4.2 rule (a majority's `match_index ≥ N` AND
  `log[N].term == current_term`); every node applies committed entries in order to bit-identical
  (`f64::to_bits`) state. Proven by ≥3 logical nodes on **ephemeral 127.0.0.1 TCP ports** (genuine OS
  sockets, not a shared-memory fake): kill-leader → survivors auto-elect a higher-term leader & keep
  progressing with no committed loss; partitioned minority cannot win (election safety); a divergent
  uncommitted tail is truncated/overwritten byte-identically; lost-quorum makes no false progress;
  crash-recovery to exact committed state. **The documented next increment** (not half-built) is Raft
  §6 **dynamic membership change** and §7 **log compaction / snapshot install** — fixed membership is
  complete and correct today.
- **Designed only / deferred (the cross-DC tier, §12):** the inter-**datacenter** transport
  *hardening* (TLS, the `io_uring`/DPDK datapath, the §11 latency **SLOs** — the built federation +
  replog prove *correctness + routing + churn + failover + quorum/replay arithmetic* over loopback,
  **not** the absolute cross-host wire-latency budgets, which need a tuned LAN + bench gates and
  stay deploy-gated). The in-process SPMC fan-out ring (`celnet-fanout`) is now **built AND wired
  under the async edge** (Wave 9): the RFS edge fans per-pair price ticks out through ONE producer
  per pair → N session consumers, replacing the old per-subscription spot tickers. These are
  drop-ins behind the now-built seams — no engine or contract change.

The honest split, mechanism by mechanism:

| Mechanism | Status in tree | Where |
|---|---|---|
| Node-local **whole-`MarketState` snapshot** via `arc-swap` (surface+curves+conventions) | **Built** | `celnet-engine` `rt::StateHandle`/`StateReader` (load path avoids `arc-swap`'s allocating guard) |
| Node-local **top-of-book** publication via single-writer **seqlock** (small `Copy` snapshot) | **Built** | `celnet-engine` `rt::Seqlock`, `core::PricingCore` |
| Core→edge **SPSC** ring (hot path stays SPSC) | **Built** (`rtrb`) | `celnet-engine`, `celnet-server` `core_link` |
| Edge async fan-in/fan-out + per-session stream | **Built** (per-session tokio `mpsc` for control/lifecycle; per-pair price fan-out over the `celnet-fanout` SPMC ring — 1 producer/pair → N consumers, Wave 9) | `celnet-server` `services/stream`, `services/pricefanout`, `core_link` |
| Blue-green **live-state handoff** over a hand-rolled little-endian codec | **Built** (not `rkyv`) | `celnet-engine` `handoff` |
| Multi-source MD aggregation + divergence detection | **Built** | `celnet-integration` |
| HRW/rendezvous **partition map** + stateless **router tier** primitives | **Built** | `celnet-router` (`map`/`key`/`hash`/`replica`/`backpressure`) |
| **Cross-shard risk-aggregation algebra** (partition → shard-local roll-up → additive merge + firm re-gather; reconciled fan-out == single-node) | **Built** | `celnet-risk-fleet` |
| **Configurable fleet topology** (`InProcess` default / `Distributed{endpoints}`), deploy-time bound | **Built** | `celnet-risk-fleet` `FleetTopology`; `celnet-server` `Edge::start_on_with_topology` (`CELNET_FLEET_MODE`/`CELNET_FLEET_BACKENDS`) |
| **Cross-node serving over real gRPC** — edge federates RiskService + forwards owned-pair Pricing/Quote/Surface across N backend processes; reconciled fan-out == single-node; `unavailable` on unreachable slice | **Built** (localhost multi-process; cross-DC hardening + latency SLOs deferred) | `celnet-server` `services/forward.rs`, `services/risk/federate.rs`; OS-process proof `examples/scale_harness.rs` |
| Cross-**datacenter** transport **hardening** (TLS, `io_uring`/DPDK datapath, §11 latency SLOs) | **Designed only** | — (§12) |
| **Full Raft consensus** — leader election (randomized timeouts + RequestVote §5.4.1 + Pre-Vote; persistent term/vote/commit-watermark), AppendEntries with §5.3 log-matching + **durable conflicting-tail truncation**, §5.4.2 commitment, deterministic `to_bits` apply | **Built** (logical nodes over real loopback sockets; Raft §6 membership change + §7 snapshot/compaction = documented next increment; absolute cross-host wire SLO stays deploy-gated) | `celnet-replog` (`election`/`log`/`persist`/`entry`/`state`/`wire`); proofs `tests/replication.rs` + parity row `celnet-parity/tests/raft_election.rs` |
| In-proc **SPMC broadcast** fan-out ring (lock-free, per-slot two-phase seqlock, conflation with exact skip-accounting; no-loss/total-order at 100/1000 consumers) | **Built AND wired** (crate + edge integration, Wave 9: the RFS edge drives per-pair price ticks through 1 producer/pair → N session consumers, replacing the per-subscription spot tickers; in-process loopback throughput is an upper-bound/relative signal — absolute network fan-out stays deploy-gated) | `celnet-fanout` (`ring`); edge wiring `celnet-server` `services/pricefanout.rs` + `services/stream.rs`; proofs `celnet-fanout/tests/broadcast.rs`, `celnet-server/tests/fanout_edge.rs` |
| `SO_REUSEPORT` sharded accept + eBPF steering; `io_uring`/XDP tier | **Designed only** (readiness probe references the handoff intent) | — (build-now, §12) |
| DPDK/RDMA multicast tier | **Deferred** (ADR-gated on measured bottleneck) | — (§12) |

The **partition map, the cross-shard aggregation algebra, the configurable topology, the
cross-node gRPC serving/federation, and now full Raft consensus
(`celnet-replog`: leader election + log-matching + durable conflicting-tail truncation +
quorum commit + deterministic `to_bits` replay + crash-recovery) are built** (see the rows above).
What is still **design** below the §0 line is the **cross-DC hardening tier** (the kernel-bypass
datapath, the §11 latency SLOs) and — atop the built consensus — Raft §6 **membership change** +
§7 **snapshot/compaction**. Treat the §11 absolute cross-host latency numbers as target, not
deployed; the replog's correctness (election safety, byte-identical committed log,
`to_bits`-identical replay, quorum safety, conflicting-tail truncation, bounded failover) is
**built and validated over real loopback sockets**, an upper bound on compute and a lower bound on
real cross-host wire latency — the absolute inter-host SLO stays deploy-gated. Treat "shard /
router / federation / replicated-log" as built (validated multi-process/multi-node on localhost
over real gRPC and real TCP).

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
  **What is built today** (§0): the hot path is the `rtrb` **SPSC** core→edge ring; the async
  edge keeps per-session control/lifecycle frames (snapshot / modify / resync / executed /
  reject / heartbeat / stream-end / market-series) on a bounded tokio `mpsc`, and fans the
  **high-volume per-pair price ticks** out over the `celnet-fanout` SPMC broadcast ring — **ONE
  producer per pair → N session consumers** (Wave 9, `celnet-server` `services/pricefanout.rs`).
  The per-pair deterministic market evolution is computed once on a dedicated off-runtime
  producer thread and broadcast; each session drains an independent ring consumer with
  non-blocking `try_recv` in its `select!` loop and formats its own `Update` (its sequence,
  click-to-trade tokens, snapshot/delta) from the shared tick. This replaced the old
  per-subscription spot tickers (*O(subscribers)* generators for an *O(pairs)* quantity).

- **`celnet-fanout` — lock-free SPMC broadcast ring (BUILT, Wave 3).** A single-producer /
  multi-consumer broadcast ring (the LMAX-Disruptor multi-consumer pattern, Thompson et al.
  2011): one producer publishes a monotonic sequence into a power-of-two ring via a true
  per-slot **seqlock** (an in-progress write flag straddling the payload store, so a reader
  copying concurrently always detects an overlapping write — no torn read); each of N
  consumers holds its **own** read cursor and observes every published item in order, with no
  inter-consumer contention and a **zero-allocation, lock-free publish hot path** (storage
  allocated once; proven by a counting-allocator test). **Overflow policy = bounded +
  conflation with counted skips** (the FX-streaming-correct choice, §6): a slow consumer is
  never able to back-pressure the producer — when lapped it fast-forwards to the oldest
  still-live item, counts the gap into a per-consumer `skipped` metric, and converges on the
  latest price, with the exact invariant `received + skipped == produced`. Gated
  (`crates/celnet-fanout/tests/`): broadcast **no-loss + total-order at 100 and 1000
  concurrent consumer threads** (every consumer receives the exact published sequence, in
  order, zero skips, deadline-bounded); **conflation-correctness under overflow** (lapped slow
  consumer sees the latest, strict-increasing, never-duplicated delivery, `received + skipped
  == produced`, skip count > capacity proving conflation actually engaged); and a **measured
  in-process throughput** figure. **Honest measurement boundary:** the throughput number
  (best-of-bursts raw publish ~3×10⁷ items/s on the dev M4; a contended busy-poll 16-reader
  fan-out figure reported alongside) is an **in-process loopback** measurement — an *upper*
  bound on compute throughput and a *relative-regression* signal, **not** a cross-host wire
  claim. Absolute network fan-out latency/throughput to remote counterparties is provable only
  on the deployed datapath and stays **deploy-gated** (see the proxy-multicast-tree tier below
  and §11); this crate proves the ring arithmetic, ordering, conflation accounting, and
  zero-alloc publish — nothing about a live cross-DC fabric or NVIDIA.
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
