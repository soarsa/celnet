# ADR-0015 — Replicated state & elastic fleet

- **Status:** Proposed / Accepted as a **design direction** (2026-07-01). **NOT yet
  implemented.** Records the intended activation of the built-but-dormant Raft consensus as
  a **configurable consistency tier** — **wired everywhere, forced nowhere** (opt-in
  `Strong` quorum per book/desk/tenant; ultra-low-latency `Local` the default) — and the
  move to an elastic fleet. This **supersedes any "route every state write through the Raft
  log" framing**: only books that opt into `Strong` are quorum-committed, and the
  regenerable market-data plane is *never* gated on the choice. The single-node
  `celnet-journal` `DurableBook` + per-shard `arc-swap` snapshot path remains authoritative
  until `celnet-server` takes a `celnet-replog` dependency (**it has none today**). This is
  program phase **P3** of the target architecture (`docs/ARCHITECTURE-TARGET.md` §5) and the
  concrete build-out of `docs/SCALE-OUT.md` §4 / §8 / §2 / §12.
- **Date:** 2026-07-01
- **Extends:** ADR-0011 (CelNet-estate ingress — the `DeploymentMode` / fleet boot seams).
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
  (`rt.rs:53`) holding **flat `f64`** `r_dom`/`r_for` (`:57`/`:59`) — zero pointer-deref on
  the hot path.
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

**The operator directive resolves this configurably (§2).** Consistency is not a single
platform-wide switch but a **per-granularity choice**: a book / desk / tenant that needs
linearizable, replicated, zero-data-loss state opts into a **`Strong`** (Raft-quorum) tier
via `RaftNode::propose` (`crates/celnet-replog/src/election.rs:670`); everything else stays
on the **`Local`** ultra-low-latency default (single-node `celnet-journal` `sync_data` fsync
+ a light non-quorum peer broadcast). Raft is therefore **wired everywhere but forced
nowhere**. This supersedes the earlier "route every state write through the Raft log"
framing and — because the regenerable market-data plane is *never* gated on the choice
(§4.3) — elegantly resolves the surface-freshness-vs-quorum-latency tension the audit
raised.

## 2. Decision

Activate the dormant consensus as a **configurable consistency tier — wired everywhere,
forced nowhere** — and make the fleet elastic, in four moves, **without ever putting
consensus, replication, or a cross-node hop on the per-tick price / streaming / market-data
path (§4.3, the hard invariant).**

### 2.1 Raft as a configurable consistency tier at the state-write sinks — `Strong` opt-in, `Local` default

`celnet-server` takes a `celnet-replog` dependency; `Edge::start_on_with_topology`
(`crates/celnet-server/src/lib.rs:346`) grows a consensus stage that boots `RaftNode::boot`
(`crates/celnet-replog/src/election.rs:433`) for a shard's replica group (a leader +
hot-standby followers per HRW partition slice). The authoritative **state-write sinks** —
position booking `PositionStore::book_from_attribution`
(`crates/celnet-server/src/services/risk/store.rs:450`) and rates booking
`RatesPositionStore::book` (`crates/celnet-server/src/services/rates_book.rs:64`) — are each
given a **per-granularity consistency level**, resolved from config (§2.1.1) at the moment
of commit:

- **`Strong` (opt-in — Raft quorum).** The write becomes a `BookUpdate`
  (`crates/celnet-replog/src/state.rs:24`) proposed via `RaftNode::propose`
  (`crates/celnet-replog/src/election.rs:670`): **linearizable, quorum-replicated, survives
  node failure with zero data loss**. The committed write is **ms-scale but durable +
  quorum-replicated** — on the async edge / booking tier, **never on the per-tick price
  path** (§4.3). Opt-in for the books / tenants / desks that require it (regulatory books,
  prime-brokerage state, anything where a lost tail is unacceptable).
- **`Local` (DEFAULT — ultra-low-latency).** The write is made **single-node durable** by
  `celnet-journal` append + `sync_data` fsync (`crates/celnet-journal/src/lib.rs:437` /
  `:454`, via `celnet-engine::DurableBook` `journal.rs:171`) plus a **light, non-quorum
  epoch-broadcast** so hot-standby peers converge best-effort — **µs-scale**. This is the
  fast path: no quorum round-trip, no cross-node hop on commit. The tradeoff is explicit — a
  `Local` book's un-replicated tail can be lost only on that node's own crash (recoverable
  from its local journal), which is exactly why durability-critical books opt into `Strong`.

**The `Strong` tier is real capability work, not wiring.** The live book is
`celnet_engine::BookState` (`crates/celnet-engine/src/rt.rs:533`, via `risk/store.rs:10` +
`PositionStore`), whereas the replicated applied state machine is the **disjoint**
`celnet_replog::BookState` (`crates/celnet-replog/src/state.rs:136`, `apply:148`;
`applied_state` at `election.rs:641`). Activating `Strong` therefore requires **reconciling
/ migrating the two state types** — either (a) making `celnet_engine::BookState` the Raft
applied state (teach the engine book to be driven by committed log entries), or (b) unifying
on `celnet_replog::BookState` and re-pointing `risk/store.rs` at it. Both are gated by the
`to_bits` apply oracle to prove the migrated book replays bit-identically. For a `Strong`
book the single-node `DurableBook` (`celnet-engine/src/journal.rs:171`, over `celnet-journal`)
becomes the leader's local journal *under* the replicated log; for a `Local` book it stays
the top-level durability tier, unchanged. **This wires the dormant `celnet-replog` (already
built + validated over ≥3 real-loopback nodes, `election.rs:433`) as the engine of the
`Strong` tier — no new consensus code.**

**Market-data plane stays fast for every book, regardless of consistency choice.** Surfaces
and curves are **regenerable derived data**: every shard already derives its own arb-free
surface + curves locally and is therefore **always available** even with no peer
(`divergence_report` at `crates/celnet-integration/src/divergence.rs:208` is per-node). They
are **not must-order durable state**, so they are **never** on the quorum log — they ride a
**light, monotonic `SurfaceEpoch{epoch, curves, smile-params}` broadcast** applied on each
shard to its node-local `arc-swap` snapshot (a flat pre-interpolated slice, never
`Arc<Curve>` in `MarketState` — the ADR-0016 embargo, §4.3). This holds **even for a `Strong`
book**: only that book's *state* commit takes the quorum path; its pricing, greeks, streaming,
and surface distribution stay µs. Every quote/tick is tagged with its **surface epoch** (the
deterministic-replay key, `SCALE-OUT.md` §4) so a price is attributable to an exact broadcast
version.

This **closes the hot-standby-failover gap** for `Strong` books (failover bounded by ~2×
election timeout with bit-identical replay, `SCALE-OUT.md` §11 SLO) and **narrows the
cross-shard surface-distribution gap** for every book (each shard in a replica group
converges on the same surface epoch via the decoupled broadcast) — while `Local` books keep
today's ultra-low-latency single-node behaviour byte-for-byte.

#### 2.1.1 Granularity & the config seam — per book → desk → tenant → default

Consistency is selected at **optimal, intuitive granularity: per book, per desk, per
tenant.** One book can run `Strong` while the book beside it runs `Local`. The knob is a
typed `consistency: strong | local` key in ADR-0013's declarative `PlatformConfig`
(`celnet.toml`, ADR-0013 §6) and/or the per-book / per-desk config, resolved by a concrete
**most-specific-wins cascade**: `book → desk → tenant → platform-default`, with the platform
default = **`Local`** (ultra-low-latency). The resolved level is read **once, off the hot
path**, at the booking sink (§2.1) — never re-evaluated per tick, so the config lookup itself
never touches the price path. Because the default is `Local`, a fleet that configures nothing
keeps today's single-node ultra-low-latency behaviour byte-for-byte; `Strong` is purely
additive, opt-in state.

### 2.2 The Raft log carries the must-order state — membership (always) + `Strong`-book writes; surfaces are always decoupled

The one committed, totally-ordered, bit-identically-applied log is the **single source of
truth** for the must-order, durable entry kinds:

- `Membership{C_old, C_new}` — the **versioned partition map** as a committed log entry.
  **Always** on the log, independent of any book's consistency level: fleet membership is a
  correctness-critical, must-order fact for every node.
- `BookWrite{…}` — position / trade-lifecycle mutations **for books configured `Strong`**
  (§2.1); ordering + durability are correctness requirements, so these are quorum-committed.
  `Local` books do **not** touch the quorum log at all — their durability is the single-node
  journal + light peer broadcast.

Because a membership entry is applied by **all** nodes in log order, the HRW partition map
(`crates/celnet-router/src/hash.rs:55`) is driven off the commit watermark. This **replaces
the "versioned and gossiped" map of `SCALE-OUT.md` §2 with a Raft-log-derived map** — for
membership there is **no separate gossip layer** to reconcile against the log (which would
introduce a second source of truth and its own version-vector divergence). Routers converge
by tailing the committed membership entries; the log commit index **is** the map version.

**Surface/curve epochs are never on this log — this is an invariant (§4.3), not an operator
choice.** Because they are regenerable derived data (every shard can always re-derive locally,
`divergence.rs:208`), there is **no durable truth to diverge from**, so the "two sources of
truth" objection that keeps membership on the log does **not** apply. Folding them into the
quorum log would put a ms-scale commit on the market-data publish path and **violate the
ultra-low-latency-always invariant** (§4.3) — so it is rejected (§5), not offered as an
option. The **only** consistency knob exposed to operators is the per-book / desk / tenant
`Strong | Local` level of §2.1.1, which governs **state** durability, **never** the
market-data plane.

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
- **Tunable / leveled consistency** (Cassandra & DynamoDB per-request consistency levels;
  Azure Cosmos DB's five levels; Spanner / Raft linearizability at the strong end) — the
  reference for the `Strong | Local` tier (§2.1): a system that lets each write choose its
  consistency/latency point rather than imposing one platform-wide. The **novel application**
  is a **per-book consistency SLA in an ultra-low-latency options-trading state layer**,
  where the strong tier is opt-in per book/desk/tenant and the regenerable market-data plane
  is *always* on the fast, non-quorum path regardless of any book's level — an ordering these
  general-purpose stores do not make (they gate reads/writes uniformly, not "state strong,
  derived-market-data always fast").
- **io_uring / kernel-bypass** (Linköping diva2:1789103, DPDK-vs-io_uring-vs-stack) — the
  OSS-first HFT datapath (§2.4).
- **Jasper** (arXiv:2402.09527) — scalable fair multicast for cloud exchanges; the
  many-counterparty proxy-tree tier, deferred and measured-bottleneck-gated (§2.4).
- **FPGA edge** (5–85 ns) — out of scope under the OSS/free mandate; a future hardware edge.

## 4. Consequences

### 4.1 What this closes
- The **#1 D3 bottleneck (replog dormancy)**: hot-standby failover bounded by ~2× election
  timeout with bit-identical replay; the fleet SPOF (shard crash ⇒ pairs `unavailable`) is
  eliminated for any **`Strong`**-configured book in a ≥3-node replica group. `Local` books
  trade that zero-data-loss failover guarantee for µs commits — the operator's explicit
  per-book choice (§2.1.1), not a platform-wide compromise.
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
- The **booking** write path gains an off-hot-path commit whose latency depends on the
  book's consistency level: **`Strong`** = leader-append + quorum-commit (ms-scale,
  acceptable for durable lifecycle events — `SCALE-OUT.md` §8); **`Local`** (default) =
  single-node journal fsync + light peer broadcast (µs-scale, no quorum). The
  **surface-publish** path gains only the lighter epoch-broadcast (µs-scale, no quorum, §2.1)
  for **every** book. None of these is **ever on the per-tick price / streaming path**, which
  still reads only the node-local snapshot.
- `DurableBook` (`journal.rs:171`) is demoted from the top-level durability tier to the
  leader's local journal beneath the replicated log; `celnet-integration`'s in-process
  surface feed becomes a *producer of proposals to the leader*, not the authoritative
  per-node source.
- New `SCALE-OUT.md` §11 SLO gates required before "built", with the **split budget** the
  configurable tier implies:
  - **`Strong` book commit** (quorum-durable `BookWrite` via `RaftNode::propose`) —
    **ms-scale**, off the hot path;
  - **`Local` book commit** (single-node fsync + light peer broadcast) **and ALL
    market-data / surface distribution** — **µs-scale**, no quorum;
  - **surface publish→snapshot lag** (the lighter epoch-broadcast) — p99 ≤ 150 µs, which is
    **only achievable because the surface feed is never on the ms-scale quorum log** (§2.2).
    Routing it through the log would put a quorum commit on the publish path and blow this
    budget by an order of magnitude — which is exactly why surface decoupling is an invariant
    (§4.3), not an operator option;
  - **failover time** (`Strong` books) — kill primary under load, assert standby takeover +
    `to_bits`-identical replay via the f64 oracle;
  - the **in-shard price regression guard** — p50 ≤ 2 µs / p99 ≤ 10 µs **must not regress**
    when `celnet-replog` is linked into the server, **and must be identical whether the book
    under load is configured `Strong` or `Local`** — the ultra-low-latency-always invariant
    (§4.3): pricing is never gated on the consistency choice.

### 4.3 Invariants (non-negotiable)
1. **Ultra-low-latency ALWAYS — the consistency choice never gates the hot plane.** Pricing,
   greeks, surface, and streaming are derived / regenerable and stay **µs regardless of any
   book's `Strong` / `Local` level**. The flat `MarketState`
   (`crates/celnet-engine/src/rt.rs:53`, flat `f64` `r_dom` / `r_for` at `:57` / `:59`, **no
   `Arc<*Curve>`** — the ADR-0016 embargo) + the light surface epoch-broadcast are the only
   things on the price / stream path. Even a `Strong` book's ms-scale quorum commit happens on
   the **booking / state** path, off the pinned pricing thread. **Gate:** the §4.2 SLO harness
   must show pricing p50 / p99 **unchanged whether the book under load is `Strong` or
   `Local`**.
2. **No consensus / replication on the hot pricing thread.** `PricingCore::drain`
   (`core.rs:189`) touches no Raft state; the router is provably off the per-tick path
   (`forward.rs:126`); leader-append + commit happen on the async edge / MD-consumer tier.
3. **Market-data plane is never on the quorum log.** Regenerable surfaces / curves ride the
   light, monotonic `SurfaceEpoch` broadcast for **every** book (§2.1) — never quorum-gated —
   so freshness is decoupled from quorum liveness. The feed lands in the **surface-rebuild
   tier** and publishes a node-local `arc-swap` snapshot: **no `Arc<*Curve>` / `Arc<*Surface>`
   into `MarketState`** (the ADR-0016 embargo). Streaming still pins a pre-interpolated flat
   `CalibratedSmile::Parametric`.
4. **Deterministic `to_bits` apply across `Strong` replicas** — every node applies committed
   entries in identical order to bit-identical state (the existing `celnet-replog` property;
   the f64 CPU oracle reconciles replay).
5. **Single-writer discipline for must-order state** — the leader-append log is the only
   writer of authoritative **membership** (always) and **`Strong`-book** state; followers
   apply in log order only. `Local`-book writes (single-node journal + light peer broadcast)
   and surface/curve epochs are **not** quorum-ordered — surfaces are monotonic by epoch
   rather than quorum-ordered.
6. **In-process fan-out unchanged** — the `BroadcastRing` `received + skipped == produced`
   conflation accounting (`ring.rs:424`) is untouched; consensus is **not** the fan-out.
7. **One unversioned contract** (ADR-0007) — no `celnet.proto` change; the log-entry /
   membership / consistency-level / SBE-transport evolution is internal to the fleet,
   invisible on the wire.

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
  log).** Rejected outright — **not** an operator option: a quorum commit is ms-scale, so it
  **cannot** meet the surface publish→snapshot p99 ≤ 150 µs budget (§4.2) and would couple
  market-data freshness to quorum liveness even though each shard can always re-derive its
  surface locally (`divergence.rs:208`). This directly violates the ultra-low-latency-always
  invariant (§4.3), so surfaces are decoupled for **every** book regardless of its `Strong` /
  `Local` level.
- **A single platform-wide consistency switch (force all books `Strong`, or all `Local`).**
  Rejected: forcing `Strong` everywhere puts a ms-scale quorum commit on every booking and
  spends replication on books that do not need it; forcing `Local` everywhere denies
  zero-data-loss to the books (regulatory / prime-brokerage) that require it. The operator
  directive is a **per book / desk / tenant** choice at optimal, intuitive granularity
  (§2.1.1), defaulting to `Local`, so each book pays exactly the consistency cost it needs and
  no more.
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

- *(decision)* — **Raft is a configurable consistency tier, wired everywhere but forced
  nowhere.** The state-write sinks `PositionStore::book_from_attribution`
  (`celnet-server/src/services/risk/store.rs:450`) and `RatesPositionStore::book`
  (`celnet-server/src/services/rates_book.rs:64`) take a per-granularity level: **`Strong`**
  routes the write as a `BookUpdate` (`celnet-replog/src/state.rs:24`) through
  `RaftNode::propose` (`celnet-replog/src/election.rs:670`) — linearizable, quorum-replicated,
  zero-data-loss, ms-scale; **`Local`** (default) uses `celnet-journal` `append` + `sync_data`
  (`celnet-journal/src/lib.rs:437` / `:454`) + a light non-quorum peer broadcast — µs.
  Granularity is per book → desk → tenant → default, selected in ADR-0013's typed
  `PlatformConfig` (`celnet.toml`, ADR-0013 §6) `consistency: strong|local` key. Anchors:
  `RaftNode::propose` (`celnet-replog/src/election.rs:670`), `BookUpdate`
  (`celnet-replog/src/state.rs:24`), `PositionStore::book_from_attribution`
  (`celnet-server/src/services/risk/store.rs:450`), `RatesPositionStore::book`
  (`celnet-server/src/services/rates_book.rs:64`), `celnet-journal` `append` / `sync_data`
  (`celnet-journal/src/lib.rs:437` / `:454`), `FleetTopology`
  (`celnet-risk-fleet/src/lib.rs:534`).
- *(invariant)* — **Ultra-low-latency always: the consistency choice never gates the hot
  pricing / streaming / market-data plane.** Pricing / greeks / surface stay µs whether a book
  is `Strong` or `Local`; even a `Strong` book's quorum commit is off the pinned thread, on
  the booking path. Regenerable surfaces are **never** on the quorum log (an invariant, not an
  operator option). Gate: SLO harness shows pricing p50 / p99 identical for `Strong` vs
  `Local`. Anchors: `MarketState` (`celnet-engine/src/rt.rs:53`, flat `f64` `r_dom` / `r_for`
  `:57` / `:59`), `PricingCore::drain` (`celnet-engine/src/core.rs:189`), `divergence_report`
  (`celnet-integration/src/divergence.rs:208`).
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
  membership changelog (always) + `Strong`-book writes; **surface/curve epochs are decoupled
  onto a lighter, non-quorum epoch-broadcast for every book** (regenerable derived data, µs
  freshness budget, not coupled to quorum liveness — an invariant, never on the quorum log,
  §4.3). No separate gossip layer for membership (single source of truth). Anchors:
  `FleetTopology`, `rendezvous_weight`
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
  (the ADR-0016 embargo). Anchors: `MarketState` (`celnet-engine/src/rt.rs:53`, flat `f64`
  `r_dom` / `r_for` `:57` / `:59`), `rt::Seqlock` (`rt.rs:385`).
