# ADR-0015 — Replicated state & elastic fleet

- **Status:** Proposed / Accepted as a **design direction** (2026-07-01). **NOT yet
  implemented.** Records the intended activation of the built-but-dormant Raft consensus
  and the move to an elastic fleet; the single-node `celnet-journal` `DurableBook` +
  per-shard `arc-swap` snapshot path remains authoritative until `celnet-server` takes a
  `celnet-replog` dependency (**it has none today**). This is program phase **P3** of the
  target architecture (`docs/ARCHITECTURE-TARGET.md` §5) and the concrete build-out of
  `docs/SCALE-OUT.md` §4 / §8 / §2 / §12.
- **Date:** 2026-07-01
- **Extends:** ADR-0011 (Celer-estate ingress — the `DeploymentMode` / fleet boot seams).
  **Constrained by:** ADR-0016 (hot-core embargoes + latency SLO gate) — the
  `Arc<*Curve>`/`Arc<*Surface>`-in-`MarketState` embargo is the hard boundary this ADR's
  surface-distribution feed must honour. Honours CLAUDE.md guardrails #6 (scale is a
  requirement), #7 (OSS/free only), #9 (one unversioned contract), #10 (zero legacy),
  #11 (scale-out aware, zero-cost observability).

## 1. Context (grounded in the code)

Celnet's scale-out substrate is built to a high standard but the **durability /
replication tier is a disconnected island** — fully implemented, validated over real
sockets, and wired to **nothing**. The audit (`docs/ARCHITECTURE-TARGET.md` §0, D3) is
explicit: this is the platform's #1 scaling bottleneck.

**What is built and live:**
- **Governing principle holds in code.** "Distribute for capacity, not for speed"
  (`docs/SCALE-OUT.md` §1): the latency-critical work stays on one fat NUMA-pinned
  thread-per-core node; the router is **provably never on the per-tick price path** —
  `per_tick_price_path_never_crosses_the_router` (`crates/celnet-server/src/services/forward.rs:126`)
  and `cross_shard_forward_is_the_only_router_path` (`forward.rs:142`). The hot core
  `PricingCore::drain` (`crates/celnet-engine/src/core.rs:189`) reads a node-local
  `arc-swap` snapshot + a cache-line-isolated seqlock (`rt.rs:385`), with `MarketState`
  holding **flat `f64`** `r_dom`/`r_for` (`rt.rs:75`) — zero pointer-deref on the hot path.
- **Configurable fleet, static membership.** `FleetTopology`
  (`crates/celnet-risk-fleet/src/lib.rs:534`) = `InProcess | Distributed{endpoints:Vec<String>}`,
  bound at boot by `Edge::start_on_with_topology` (`crates/celnet-server/src/lib.rs:346`)
  reading `CELNET_FLEET_MODE`/`CELNET_FLEET_BACKENDS`. Membership is a **fixed
  `Vec<String>` at boot** — no gossip, no versioned partition map, no dynamic node
  add/remove without restart (`SCALE-OUT.md` §2 asks for "versioned and gossiped").
- **Routing.** HRW rendezvous `rendezvous_weight` (`crates/celnet-router/src/hash.rs:55`),
  argmax over healthy replicas, consulted **only** at connection setup / cross-shard
  forward.
- **In-process fan-out.** `BroadcastRing<T>` SPMC seqlock ring
  (`crates/celnet-fanout/src/ring.rs:424`), zero-alloc publish, conflation with the exact
  `received + skipped == produced` accounting; one producer per underlying keyed by
  `underlying_seed` (`crates/celnet-server/src/services/pricefanout.rs:192`,
  FX byte-identical to `pair_seed:121`). This is the correct in-process counterparty
  fan-out and stays as-is.

**What is built and DORMANT (the gap):**
- **Full Raft consensus, zero production callers.** `celnet-replog` implements the
  complete algorithm — leader election with randomized timeouts + Pre-Vote + the §5.4.1
  up-to-date voting rule, AppendEntries with §5.3 log-matching + durable conflicting-tail
  truncation, §5.4.2 quorum commitment, **deterministic `f64::to_bits` apply**,
  crash-recovery, and §7 log compaction / snapshot / InstallSnapshot — proven over ≥3
  logical nodes on real loopback TCP (`celnet-parity/tests/{raft_election,raft_compaction,raft_snapshot}.rs`).
  Its entry point `RaftNode::boot` (`crates/celnet-replog/src/election.rs:433`, and
  `boot_on:453`) has **zero production callers**, and — verified against
  `celnet-server/Cargo.toml`, which depends on `celnet-engine` (`:31`) and `celnet-limits`
  (`:39`) but **not** `celnet-replog` — **`celnet-server` has no `celnet-replog` dependency
  at all**. The whole replog stack is therefore a disconnected island: `PersistStore`
  (`crates/celnet-replog/src/persist.rs:50`, atomic write→fsync→rename crash recovery) is
  **not referenced anywhere in `celnet-server/src`**, and `celnet_replog::BookState`
  (`crates/celnet-replog/src/state.rs:136`, with `apply:148` / `encode:205`) has **zero
  production callers** — `celnet_replog::BookState::apply` is invoked only by the
  replog-internal `election.rs` (`applied_state` at `election.rs:641`) and by
  `celnet-parity` / `celnet-replog` tests. It is **designed to become the Raft applied
  state**, but today is a dormant type wired to nothing in the server.
- **The live book is a DIFFERENT type.** What `celnet-server` actually runs is
  `celnet_engine::BookState` (`crates/celnet-engine/src/rt.rs:533`), reached via
  `risk/store.rs:10` + `PositionStore` and made durable by `celnet-engine::DurableBook`
  (`journal.rs:171`) over **`celnet-journal`** — with **no connection to
  `celnet_replog::BookState`**. The two `BookState` types are **disjoint**: the live one
  (`celnet_engine`) and the dormant replicated one (`celnet_replog`) share no code path
  today. This is the crux the activation must reconcile (§2.1), not a seam to be wired.
- **Durability is single-node.** `celnet-journal` is CRC-framed append + `sync_data` with
  crash-safe compaction (`lib.rs:437`); `celnet-engine::DurableBook` (`journal.rs:171`) is
  a **single-node** durable book, **not Raft-replicated**.
- **Consequence (the three open bottlenecks, `ARCHITECTURE-TARGET.md` D3 ranking):**
  (1) no multi-node replicated state machine ⇒ **no hot-standby failover** — a backend
  shard crash makes its pairs `Status::unavailable` until restart (the fleet SPOF);
  (2) static membership ⇒ **no elastic node add/remove** (contradicts `SCALE-OUT.md` §2);
  (3) surface/curve distribution has **no cross-fleet coherence channel at all** — each
  shard derives an independent surface view locally in-process via `celnet-integration`'s
  aggregator (`divergence_report` at `divergence.rs:208` is per-node, not cross-fleet), so
  cross-shard staleness is unbounded and unmeasured. Note these surfaces are **regenerable
  derived data** (every shard is always able to re-derive locally), so the fix is a
  **coherence broadcast, not consensus** (§2.1) — `SCALE-OUT.md` §4 names the gap.
- **HFT counterparty fan-out** to remote sessions is unicast over the stock OS TCP stack;
  `SO_REUSEPORT` accept-sharding, `io_uring`, and the Jasper proxy-multicast tree are
  **designed only** (`SCALE-OUT.md` §5, §12).

The latency arithmetic that makes the principle non-negotiable (`SCALE-OUT.md` §1): a
cross-node hop on the hot path costs ~20–50 µs (kernel stack) = **10–25× the p50 ≤ 2 µs
vanilla budget** (`docs/ARCHITECTURE.md` §1.2, `:61`); a Raft quorum is **ms-scale — off
the hot path entirely**. So consensus may carry only must-order durable events, never a
single price.

## 2. Decision

Activate the dormant consensus and make the fleet elastic, in four moves, **without ever
putting consensus, replication, or a cross-node hop on the per-tick price path.**

### 2.1 Wire `celnet-replog` into the server lifecycle — a leader-append log for must-order
state, a lighter broadcast for regenerable surfaces

`celnet-server` takes a `celnet-replog` dependency; `Edge::start_on_with_topology`
(`crates/celnet-server/src/lib.rs:346`) grows a consensus stage that calls
`RaftNode::boot` (`crates/celnet-replog/src/election.rs:433`) for a shard's replica group
(a leader + hot-standby followers per HRW partition slice). Two write streams are activated
and handled by **different** tiers, split on whether they are must-order durable state or
regenerable derived data:

- **Book writes (must-order, quorum-durable)** — `PositionStore` / click-trade booking
  becomes a Raft log entry. This is **not a wiring exercise**: the live book is
  `celnet_engine::BookState` (`crates/celnet-engine/src/rt.rs:533`, via `risk/store.rs:10`
  + `PositionStore`), whereas the replicated applied state machine is the **disjoint**
  `celnet_replog::BookState` (`crates/celnet-replog/src/state.rs:136`, `apply:148`;
  `applied_state` at `election.rs:641`). Activation therefore requires **reconciling /
  migrating the two state types** — either (a) making `celnet_engine::BookState` the Raft
  applied state (teach the engine book to be driven by committed log entries), or (b)
  unifying on `celnet_replog::BookState` and re-pointing `risk/store.rs` at it. Both are
  real capability work, gated by the `to_bits` apply oracle to prove the migrated book
  replays bit-identically. The single-node `DurableBook`
  (`celnet-engine/src/journal.rs:171`, over `celnet-journal`) then becomes the leader's
  local journal *under* the replicated log rather than the top-level durability tier.
- **Surface / curve epochs (decoupled from consensus — recommended)** — surfaces and curves
  are **regenerable derived data**: every shard already derives its own arb-free surface +
  curves locally today and is therefore **always available** even with no peer
  (`divergence_report` at `divergence.rs:208` is per-node). They are **not must-order durable
  state** like a trade, so they are deliberately kept **off** the quorum-committed log and
  ride a **lighter epoch-broadcast** instead — a best-effort, monotonic
  `SurfaceEpoch{epoch, curves, smile-params}` push, **not** quorum-gated, applied on each
  shard to its node-local `arc-swap` snapshot (a flat pre-interpolated slice, never
  `Arc<Curve>` in `MarketState` — the ADR-0016 embargo, §4.3). This keeps market-data
  freshness **decoupled from quorum liveness** — a slow or stalled quorum never delays a
  surface publish — and lets the broadcast meet a **µs-scale** publish→snapshot budget that a
  **ms-scale** quorum commit physically cannot (§4.2). Every quote/tick stays tagged with its
  **surface epoch** (already the deterministic-replay key, `SCALE-OUT.md` §4) so a price is
  attributable to an exact broadcast version. **Operator choice** (one shared log vs.
  decoupled broadcast) is spelled out in §2.2.

This **closes the hot-standby-failover gap and narrows the cross-shard surface-distribution
gap**: failover is bounded by ~2× election timeout with bit-identical replay
(`SCALE-OUT.md` §11 SLO), and every shard in a replica group converges on the same surface
epoch via the decoupled broadcast. Contrary to an earlier framing, this is **not "wiring,
not new capability"**: the `RaftNode` / `Journal` / `PersistStore` machinery exists, but the
live book (`celnet_engine::BookState` + `PositionStore`) is **disjoint** from the dormant
replicated `celnet_replog::BookState`, so activation requires **reconciling / migrating the
two state types** — real capability work, honestly scoped in §4.2.

### 2.2 The Raft log **is** the versioned membership changelog + the authoritative book log
— surface distribution is decoupled (operator choice)

The one committed, totally-ordered, bit-identically-applied log is the **single source of
truth** for the two **must-order, durable** entry kinds:

- `BookWrite{…}` — position / trade-lifecycle mutations (§2.1); ordering + durability are
  correctness requirements, so these are quorum-committed;
- `Membership{C_old, C_new}` — the **versioned partition map** as a committed log entry.

Because a membership entry is applied by **all** nodes in log order, the HRW partition map
(`crates/celnet-router/src/hash.rs:55`) is driven off the commit watermark. This **replaces
the "versioned and gossiped" map of `SCALE-OUT.md` §2 with a Raft-log-derived map** — for
membership there is **no separate gossip layer** to reconcile against the log (which would
introduce a second source of truth and its own version-vector divergence). Routers converge
by tailing the committed membership entries; the log commit index **is** the map version.

**Surface/curve epochs are NOT on this log (recommended, §2.1).** Because they are
regenerable derived data (every shard can always re-derive locally, `divergence.rs:208`),
there is **no durable truth to diverge from**, so the "two sources of truth" objection that
keeps membership on the log does **not** apply — a best-effort epoch-broadcast is safe and,
being un-gated by quorum, is the only option that meets the µs freshness budget (§4.2).
**Operator choice:** either **(A — recommended)** route only `BookWrite` / `Membership`
through Raft and distribute `SurfaceEpoch` over the lighter, non-quorum broadcast; or **(B)**
fold `SurfaceEpoch` into the one shared log for a single tail, at the cost of coupling surface
freshness to quorum latency/liveness — which then **cannot** meet the §4.2 surface SLO, so
that SLO must be relaxed to ms if B is chosen.

### 2.3 Raft §6 dynamic membership (joint consensus) — elastic add/remove without restart

Add the one remaining consensus increment — Raft §6 cluster-membership change via **joint
consensus** (the `C_{old,new}` overlapping-majority transition) — so a node joins or leaves
a replica group **without a fleet restart window**. Elastic scale-out then adds capacity by
committing a `Membership` entry (§2.2): a new shard catches up via InstallSnapshot
(already built, §7), the map version advances at the commit, and HRW reshuffles only the
~1/N partitions the new node owns. This promotes the item `SCALE-OUT.md` §12 lists as "the
one documented next increment" from deferred to in-scope for elasticity.

### 2.4 HFT counterparty fan-out tier — gated on a measured single-shard bottleneck

Keep the in-process `BroadcastRing` SPMC seqlock ring (`celnet-fanout/src/ring.rs:424`) as
the **in-process** fan-out — it is Disruptor-shaped and already correct. For the
**cross-host** many-counterparty tier, build in order, each gated on a **measured**
single-shard unicast limit (never speculatively — CLAUDE.md #6/#7):

1. `SO_REUSEPORT` sharded accept + `io_uring` datapath (the OSS-first kernel-bypass tier;
   `io_uring` is a "hybrid bypass" closing on DPDK while being far easier to operate);
2. **only after** a shard's edge fan-out is measured as the bottleneck, a **Jasper-style
   proxy-multicast tree** (arXiv:2402.09527 — `F=10`, `D=⌈log₁₀N⌉`, VM hedging, Huygens
   clock-synced fair delivery) recorded as its own ADR with the `io_uring` fallback kept
   first-class.

Full DPDK/RDMA and FPGA edge stay **out of scope** (DPDK/RDMA gated on a further measured
limit; FPGA is disqualified under the OSS/free mandate, guardrail #7 — noted only as a
future hardware edge).

## 3. SOTA basis

- **LMAX Disruptor** (Thompson et al. 2011) — the single-thread ≥6M events/s ceiling is
  *why* we keep pricing on one fat node; our `BroadcastRing` SPMC seqlock fan-out is
  Disruptor-shaped (multi-consumer batch publish, per-slot two-phase write flag).
- **Aeron / Aeron-Cluster** (inter-process messaging + a thin Raft-replicated event log
  carrying only must-order durable events) — the reference for §2.1/§2.2: pricing math
  replays off the ordered log **asynchronously**, never through quorum. The cross-node
  transport reference to converge toward is **Aeron** (inter-process) with **SBE** (Simple
  Binary Encoding, zero-copy wire) as the log-entry / RPC serialization model; our current
  `wire::Cursor`/`write_frame` is the hand codec placeholder, SBE/Aeron the SOTA target
  (a transport-only evolution, no contract change — guardrail #9).
- **Raft** (Ongaro & Ousterhout) — the consensus algorithm already implemented in
  `celnet-replog`; §6 joint consensus is the elasticity increment (§2.3).
- **io_uring / kernel-bypass** (Linköping diva2:1789103, DPDK-vs-io_uring-vs-stack) — the
  OSS-first HFT datapath (§2.4).
- **Jasper** (arXiv:2402.09527) — scalable fair multicast for cloud exchanges; the
  many-counterparty proxy-tree tier, deferred and measured-bottleneck-gated (§2.4).
- **FPGA edge** (5–85 ns) — out of scope under the OSS/free mandate; a future hardware edge.

## 4. Consequences

### 4.1 What this closes
- The **#1 D3 bottleneck (replog dormancy)**: hot-standby failover bounded by ~2× election
  timeout with bit-identical replay; the fleet SPOF (shard crash ⇒ pairs `unavailable`) is
  eliminated for any pair in a ≥3-node replica group.
- The **#2 bottleneck (static membership)**: elastic node add/remove without restart via
  §6 joint consensus, HRW reshuffling only ~1/N partitions.
- The **#3 bottleneck (surface distribution)**: a coherent surface epoch across a replica
  group via the lighter epoch-broadcast (§2.1) — **decoupled from the quorum log** so
  freshness is not gated by consensus; cross-shard staleness becomes **bounded and
  measurable** (the publish→local-snapshot lag SLO, `SCALE-OUT.md` §11).

### 4.2 Blast radius / costs
- `celnet-server` gains its **first** `celnet-replog` dependency (`Cargo.toml` has none
  today) and a new leader/follower lifecycle stage inside `Edge::start_on_with_topology`
  (`lib.rs:346`). `InProcess` topology (the default) stays byte-identical to single-node —
  the consensus stage is inert with a 1-node group.
- **State-type reconciliation is the main capability cost (not wiring).** The live book
  `celnet_engine::BookState` (`rt.rs:533`, via `PositionStore`) and the dormant replicated
  `celnet_replog::BookState` (`state.rs:136`) are **disjoint types** today (§1). Activation
  must migrate one onto the other (§2.1) — a real change to how the engine book is driven,
  gated by the `to_bits` apply oracle to prove the migrated book replays bit-identically.
- The **booking** write path gains an off-hot-path leader-append + quorum-commit latency
  (ms-scale, acceptable for durable lifecycle events — `SCALE-OUT.md` §8); the
  **surface-publish** path gains only the lighter epoch-broadcast (µs-scale, no quorum,
  §2.1). Neither is **ever on the per-tick price path**, which still reads only the
  node-local snapshot.
- `DurableBook` (`journal.rs:171`) is demoted from the top-level durability tier to the
  leader's local journal beneath the replicated log; `celnet-integration`'s in-process
  surface feed becomes a *producer of proposals to the leader*, not the authoritative
  per-node source.
- New `SCALE-OUT.md` §11 SLO gates required before "built", with the **split budget** the
  decoupling implies:
  - **book commit** (quorum-durable `BookWrite`) — **ms-scale**, off the hot path;
  - **surface publish→snapshot lag** (the lighter epoch-broadcast) — p99 ≤ 150 µs, which is
    **only achievable because the surface feed is decoupled from the ms-scale quorum**.
    Routing it through the log (§2.2 option B) would put a quorum commit on the publish path
    and blow this budget by an order of magnitude, forcing the SLO to relax to ms;
  - **failover time** — kill primary under load, assert standby takeover + `to_bits`-identical
    replay via the f64 oracle;
  - the **in-shard price regression guard** — p50 ≤ 2 µs / p99 ≤ 10 µs **must not regress**
    when `celnet-replog` is linked into the server.

### 4.3 Invariants (non-negotiable)
1. **No consensus / replication on the hot pricing thread.** `PricingCore::drain`
   (`core.rs:189`) touches no Raft state; the router is provably off the per-tick path
   (`forward.rs:126`); leader-append + commit happen on the async edge / MD-consumer tier.
2. **Deterministic `to_bits` apply across replicas** — every node applies committed entries
   in identical order to bit-identical state (the existing `celnet-replog` property; the
   f64 CPU oracle reconciles replay).
3. **Per-node latency budgets preserved.** `MarketState` keeps **flat `f64`** rates
   (`rt.rs:75`); the `SurfaceEpoch` feed lands in the **surface-rebuild tier** and
   publishes a node-local `arc-swap` snapshot — it introduces **no `Arc<*Curve>` /
   `Arc<*Surface>` into `MarketState`** (the ADR-0016 embargo). Streaming still pins a
   pre-interpolated flat `CalibratedSmile::Parametric`.
4. **Single-writer discipline for must-order state** — the leader-append log is the only
   writer of authoritative **book / membership** state; followers apply in log order only.
   Surface/curve epochs are **not** must-order durable state and ride the decoupled
   epoch-broadcast (§2.1), monotonic by epoch rather than quorum-ordered.
5. **In-process fan-out unchanged** — the `BroadcastRing` `received + skipped == produced`
   conflation accounting (`ring.rs:424`) is untouched; consensus is **not** the fan-out.
6. **One unversioned contract** (ADR-0007) — no `celnet.proto` change; the log-entry /
   membership / SBE-transport evolution is internal to the fleet, invisible on the wire.

## 5. Alternatives rejected

- **Keep Raft a validated-but-dormant island (status quo).** Rejected: leaves the #1 D3
  bottleneck open — no hot-standby failover (the fleet SPOF), no cross-shard surface
  coherence, no cross-shard deterministic replay — and a SOTA capability built and
  proven over real sockets sits unused, exactly the "disconnected islands" anti-pattern the
  target architecture exists to eliminate (`ARCHITECTURE-TARGET.md` §0).
- **A separate gossip layer for the versioned membership map instead of the Raft log.**
  Rejected **for membership**: two sources of truth. The committed Raft log is already a
  total order applied bit-identically by every node, so the membership map rides it for free
  (§2.2); a gossip layer needs its own version-vector reconciliation and can diverge from the
  committed log — precisely the inconsistency consensus is there to prevent. **This does NOT
  extend to surfaces:** surfaces are regenerable derived data with no durable truth to
  diverge from, so they are deliberately kept **off** the quorum log and distributed by a
  lighter epoch-broadcast (§2.1/§2.2).
- **Route regenerable surface/curve epochs through the quorum-committed log (one shared
  log).** Rejected as the default: a quorum commit is ms-scale, so it **cannot** meet the
  surface publish→snapshot p99 ≤ 150 µs budget (§4.2), and it couples market-data freshness to
  quorum liveness even though each shard can always re-derive its surface locally
  (`divergence.rs:208`). Retained only as the explicit operator **option B** (§2.2), which
  requires relaxing the surface SLO to ms.
- **Route single prices / quotes through the cluster (consensus on the per-tick path).**
  Rejected: blows the SLA (`SCALE-OUT.md` §1 — a kernel-stack hop is 10–25× the p50 ≤ 2 µs
  budget; a Raft quorum is ms-scale, off the path entirely). The hot core stays single-node;
  consensus carries only must-order durable lifecycle/MD events (§1 governing principle).
- **Static-membership scale-out via full-cluster restart.** Rejected: contradicts elastic
  scale-out (`SCALE-OUT.md` §2). Raft §6 joint consensus adds/removes nodes with no restart
  window (§2.3).
- **Build the DPDK/RDMA multicast (or FPGA) fan-out tier up front.** Rejected: premature —
  the HFT fan-out tier is gated on a *measured* single-shard bottleneck with the OSS-first
  `io_uring`/`SO_REUSEPORT` datapath first (§2.4); DPDK/RDMA are a later ADR and FPGA is
  out of scope under the OSS/free mandate (guardrail #7).

## 6. Supporting verified claims (lodestar knowledge layer)

To author graph-anchored, lifecycle **draft** on acceptance (promotion to `active` awaits a
Stage-2 review **and** implementation — the flat-carry / `InProcess`-default path is
authoritative until then):

- *(decision)* — Activate `celnet-replog`: wire `RaftNode::boot` into
  `Edge::start_on_with_topology`; route **must-order** `PositionStore` book writes through
  the leader-append log, and **reconcile the two disjoint `BookState` types** — the live
  `celnet_engine::BookState` (`rt.rs:533`, via `risk/store.rs:10` + `PositionStore`) and the
  dormant replicated `celnet_replog::BookState` (`state.rs:136`, zero production callers) —
  onto one Raft applied state (capability work, not wiring). Anchors: `RaftNode::boot`
  (`election.rs:433`), `Edge::start_on_with_topology` (`celnet-server/src/lib.rs:346`),
  `celnet_engine::BookState` (`celnet-engine/src/rt.rs:533`), `celnet_replog::BookState`
  (`celnet-replog/src/state.rs:136`, `apply:148`), `RaftNode::applied_state`
  (`election.rs:641`), `celnet-server/Cargo.toml` (no `celnet-replog` dependency),
  `FleetTopology` (`celnet-risk-fleet/src/lib.rs:534`).
- *(decision)* — The Raft log carries the **must-order** state only — the versioned
  membership changelog + book writes; **surface/curve epochs are decoupled onto a lighter,
  non-quorum epoch-broadcast** (regenerable derived data, µs freshness budget, not coupled to
  quorum liveness — operator option A). No separate gossip layer for membership (single
  source of truth). Anchors: `FleetTopology`, `rendezvous_weight`
  (`celnet-router/src/hash.rs:55`), `divergence_report` (`celnet-integration/src/divergence.rs:208`),
  `celnet_replog::BookState::encode` (`state.rs:205`).
- *(decision)* — Elastic membership via Raft §6 joint consensus (node add/remove without
  restart); HFT fan-out tier (`SO_REUSEPORT`+`io_uring`, then Jasper arXiv:2402.09527)
  gated on a measured single-shard bottleneck. Anchors: `RaftNode::boot`,
  `BroadcastRing` (`celnet-fanout/src/ring.rs:424`),
  `underlying_seed` (`celnet-server/src/services/pricefanout.rs:192`).
- *(invariant)* — No consensus/replication on the hot pricing thread; the router is
  provably off the per-tick price path. Anchors: `PricingCore::drain`
  (`celnet-engine/src/core.rs:189`), `per_tick_price_path_never_crosses_the_router`
  (`celnet-server/src/services/forward.rs:126`),
  `cross_shard_forward_is_the_only_router_path` (`forward.rs:142`).
- *(invariant)* — Deterministic `to_bits` apply across replicas is preserved. Anchors:
  `BookState::apply` (`state.rs:148`),
  `celnet-parity/tests/{raft_election,raft_compaction,raft_snapshot}.rs`.
- *(invariant)* — Per-node latency budgets preserved: `MarketState` stays flat `f64`; the
  surface-distribution feed introduces no `Arc<*Curve>`/`Arc<*Surface>` into `MarketState`
  (the ADR-0016 embargo). Anchors: `MarketState` (`celnet-engine/src/rt.rs:75`),
  `rt::Seqlock` (`rt.rs:385`).
