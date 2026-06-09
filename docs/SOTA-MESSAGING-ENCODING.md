# SOTA assessment — in-process fan-out, durable-log encoding, wire codec (2026)

State-of-the-art review and competitive benchmark of Celnet's messaging / durability /
encoding layers against the best available low-latency designs, grounded in the actual
code under `crates/celnet-fanout`, `crates/celnet-replog`, `crates/celnet-journal`, and
`crates/celnet-router`. Provenance (Aeron, SBE, Chronicle, Disruptor, Raft) appears in
this doc and in code comments **only** — never in product identifiers (CLAUDE.md rule 8).

> Honesty note: every "we" claim below is anchored to a named file/line in this repo.
> Every external claim is anchored to a cited URL. Where the comparison is to a JVM
> system (Aeron/Chronicle), latency numbers are theirs, on their hardware; we do not
> restate them as ours.

---

## 0. Executive summary — three one-line verdicts

| Area | Verdict |
|---|---|
| **(a) In-process fan-out** (`celnet-fanout`) | **At SOTA for the *single-process* SPMC-broadcast-with-conflation design** — the seqlock-with-in-progress-bit + per-consumer cursor + counted-skip conflation is exactly the Disruptor-multi-consumer / Aeron-broadcast pattern, correctly built (the aarch64 Acquire-fence fix is the subtle part most hand-rolls miss). **One genuine gap vs. SOTA: it is `Arc`-shared single-process, not shared-memory cross-process**, and it has **no `loom`/`shuttle` model-checking gate** on the unsafe core. |
| **(b) Durable-log encoding** (`celnet-journal` + `celnet-replog`) | **Functionally complete and crash-correct, but *below* encoding-SOTA.** The journal (CRC32 + torn-tail truncation + atomic compaction) is solid and honest about its one real limitation (marker-less ⇒ interior bit-rot indistinguishable from a torn tail). The replog hand-rolled `Message::decode` over untrusted bytes is correct and bounds-checked, but it is a **bespoke codec with a hand-written allocating decoder** where SOTA (SBE/Chronicle) is a generated, fixed-offset, **zero-copy flyweight**. |
| **(c) Wire codec** (`celnet-proto` = prost/tonic; `celnet-replog::wire` = hand-rolled) | **Pragmatic, not SOTA on the hot path.** prost/protobuf is the correct choice for the *control-plane* gRPC contract (tooling, ecosystem, the WS mirror). It is **not** SOTA for the latency-critical price-tick path, where a fixed-layout zero-copy codec (SBE-style) beats protobuf by a documented >10× on encode throughput and, more importantly, on *predictable* latency. We currently fan out `Copy` structs in-process (no codec on the hot path at all) — so this gap only bites at the **network edge**, which is deploy-gated. |

**Bottom line:** fan-out is SOTA-shaped and correct; the durable log is correct but uses a
hand-rolled codec where a generated zero-copy one is the SOTA; the wire is fine for control
and a gap only at the network price-tick edge. Nothing here is *wrong*; three things are
*not yet best-in-class*. They are enumerated and prioritized in §6.

---

## 1. In-process fan-out — `celnet-fanout`

### 1.1 What we built (grounded)

`crates/celnet-fanout/src/ring.rs`: a single-producer / multi-consumer broadcast ring.
Power-of-two slots; each `Slot<T>` carries an `AtomicU64 stamp` + `UnsafeCell<T>`. The
publish protocol is a **true seqlock with a writer-in-progress low bit**:
`stamp = (seq<<1)|1` (Release) → write payload → `stamp = seq<<1` (Release) → bump `head`
(Release) (`ring.rs:171-189`). Readers do `stamp_before` (Acquire) → copy payload →
**`fence(Acquire)`** → `stamp_after` (Acquire), retry on mismatch (`ring.rs:294-345`).
Overflow policy is **bounded + conflation with counted skips**: a lapped consumer
fast-forwards to `head - capacity` and counts the gap into `skipped`, invariant
`received + skipped == produced` (`ring.rs:266-352`, `lib.rs:19-35`). Zero-alloc hot path
(storage allocated once in `Inner::new`, proven by a counting-allocator test per
`lib.rs:42-43`). `crossbeam_utils::CachePadded` on `head` to kill false sharing
(`ring.rs:104`).

### 1.2 What the SOTA is

- **LMAX Disruptor** — the canonical single-writer ring with per-consumer sequences and
  batch consumption ([Thompson et al., 2011](https://lmax-exchange.github.io/disruptor/disruptor.html)).
  Our design *is* the multi-consumer Disruptor pattern.
- **Aeron** (real-logic, Apache-2.0) — the production realization. Two relevant structures:
  the `BroadcastTransmitter`/`BroadcastReceiver` (driver→clients broadcast buffer) and the
  `ManyToOneRingBuffer` (clients→driver), both **flyweights over a shared-memory buffer**,
  using CAS on metadata, term-buffer rotation, NAK-based loss recovery and flow control for
  the *network* publications
  ([Aeron log-buffers docs](https://aeron.io/docs/aeron/log-buffers-images/),
  [theaeronfiles: log buffers](https://theaeronfiles.com/aeron-transport/log-buffers/),
  [cnc.dat](https://theaeronfiles.com/aeron-transport/cnc-dat/),
  [InfoQ: Aeron](https://www.infoq.com/presentations/aeron/)). Aeron's broadcast buffer is
  the closest commercial-grade analog to our ring.
- **Chronicle Queue** (OpenHFT) — append-only memory-mapped IPC, ~0.25 µs RTT 100 B
  same-box, persisted-and-replayable
  ([Chronicle vs Aeron, sanj.dev 2026](https://sanj.dev/post/chronicle-vs-aeron/),
  [OpenHFT/Chronicle-Queue](https://github.com/OpenHFT/Chronicle-Queue)).
- **`bcast`** (HaveFunTrading, Rust) — an SPMC broadcast ring **over shared memory**, the
  closest open-source Rust analog to `celnet-fanout`
  ([github.com/HaveFunTrading/bcast](https://github.com/HaveFunTrading/bcast)).
- **Academic** — wait-free MPMC/SPMC ring buffers
  ([Krizhanovsky / ACM SIGAPP ACR 2015](https://dl.acm.org/doi/10.1145/2835260.2835264));
  the conflated proxy-multicast-tree distribution we already cite for the *network* tier is
  [Jasper, arXiv:2402.09527](https://arxiv.org/abs/2402.09527).

### 1.3 Comparison

| Axis | celnet-fanout | Aeron broadcast buffer | Chronicle Queue | bcast (Rust) |
|---|---|---|---|---|
| Topology | SPMC broadcast | SPMC broadcast (driver→clients) | SP append, MC tail | SPMC broadcast |
| Process scope | **single process (`Arc`)** | **cross-process (mmap shm)** | cross-process (mmap) | **cross-process (mmap)** |
| Zero-copy read | `Copy`-out of slot | flyweight, no copy | flyweight, no copy | flyweight |
| Zero-alloc hot path | **yes** (proven) | yes | yes | yes |
| Loss policy | conflate + counted skips | broadcast overwrite (lapped reader detects) | none (durable) | conflate/overwrite |
| Back-pressure | **never** (core can't stall) | network publs flow-control + NAK | n/a | never |
| Correctness gate | stress tests, 1000 consumers | mature, battle-tested | mature | tests |
| Formal/model-check | **none (gap)** | — | — | — |
| Persisted | no (volatile) | optional (Archive) | **yes** | no |
| License | ours | Apache-2.0 | dual (Enterprise paid) | MIT |

### 1.4 Verdict & gaps

**Match-or-exceed:** the *protocol* is correct and idiomatic. The in-progress-bit seqlock
+ the **Acquire fence at `ring.rs:338`** is the part that most hand-rolled rings get wrong
on weakly-ordered ISAs (the comment documents it was surfaced under 16× CPU
oversubscription — exactly the failure a naive two-load seqlock exhibits on aarch64). Many
"Disruptor clones" never close that window; we did. The counted-skip conflation invariant
is cleaner than a bare overwrite-and-pray.

**Genuine gaps vs. SOTA:**
1. **Single-process only.** Aeron/Chronicle/bcast all live in **mmap'd shared memory** so
   the producer and consumers are *separate OS processes* — that is what lets a market-data
   driver fan out to independently-deployed, independently-crashing session processes. Our
   ring is `Arc<Inner<T>>` (`ring.rs:94-105`) → one address space. For our current topology
   (one pricing core, N session *threads* in the same `celnet-server`) this is correct and
   faster (no shm mapping). It becomes a gap the moment we want session isolation / a
   crashing consumer not to share the producer's address space.
2. **No model-checking gate** on the unsafe seqlock. We have stress tests; SOTA for
   *this exact class* of code is exhaustive interleaving search
   ([loom](https://docs.rs/loom), [shuttle](https://github.com/awslabs/shuttle)) and a
   `miri` UB pass ([Miri](https://github.com/rust-lang/miri)). The fence bug we found by
   luck under oversubscription is precisely what loom finds by construction.
3. **No batch consume.** The Disruptor's headline win is *batched* consumption (read up to
   `head` in one pass). `try_recv` returns one item per call (`ring.rs:266`); a
   `try_recv_batch`/drain-to-slice would amortize the head load + cursor write.

---

## 2. Durable-log encoding — `celnet-journal` + `celnet-replog`

### 2.1 What we built (grounded)

- **`celnet-journal`** (`src/lib.rs`): fsync'd append-only flat file, per-record
  `payload_len:u32 || sequence:u64 || payload || crc32:u32` (`lib.rs:23-36`); CRC32 IEEE
  (table-driven, dependency-free, `crc32.rs`); torn-tail detection + truncate on `open`;
  atomic compaction (temp → fsync → rename → dir-fsync, `lib.rs:117-138`); snapshot record
  distinguished by a `payload_len == u32::MAX` sentinel keeping the data layout
  byte-identical (`lib.rs:38-56`). Honestly documents the one real limitation: marker-less
  ⇒ an **interior** CRC failure is indistinguishable from a torn tail (`lib.rs:80-94`).
- **`celnet-replog`**: full Raft (`election.rs`, `log.rs`, `compaction.rs`), with
  hand-rolled byte codecs fed untrusted network bytes — `Message::encode/decode`
  (`wire.rs:158-317`) and `LogEntry::encode/decode` (`entry.rs:60-118`), the latter with
  its own wire-level CRC over `(term,index,payload)` (`entry.rs:74-118`). The decoder uses
  a bounds-checked `Cursor` (`wire.rs:319-348`, every `take` is `checked_add` + `get`), caps
  frames at 64 MiB (`wire.rs:50`), and pre-allocates `entries` with `n.min(1024)` to resist
  a hostile count (`wire.rs:258`). Truncation/fuzz-style tests assert no panic on every
  prefix (`wire.rs:568-607`).

### 2.2 What the SOTA is

- **Chronicle Queue** — memory-mapped append-only durable log, sub-µs same-box, replay from
  any point; *encoding* is **Chronicle-Wire** (a self-describing-or-binary flyweight)
  ([Chronicle durability blog](https://chronicle.software/insights/blogs/comparing-approaches-to-durability-in-low-latency-messaging-queues),
  [Chronicle vs Aeron vs Kafka, sanj.dev](https://sanj.dev/post/chronicle-vs-aeron-vs-others/)).
- **Aeron Archive / Cluster** — the log is recorded by the Archive; Aeron Cluster sequences
  client requests into a single Raft-replicated log applied to replicated state machines,
  quorum `(n/2)+1`
  ([aeron-cluster README](https://github.com/aeron-io/aeron/blob/master/aeron-cluster/README.md),
  [Aeron Raft](https://aeron.io/product-features/raft-consensus-aeron-cluster/)). This is
  *exactly what `celnet-replog` is* — a Raft-replicated log applied to a deterministic
  `BookState`. Our consensus design is at SOTA parity (we have election + log-matching +
  conflicting-tail truncation + §7 snapshot + InstallSnapshot catch-up, per `lib.rs:54-114`).
- **Encoding SOTA**: a generated **fixed-offset zero-copy** codec (SBE / Chronicle-Wire) —
  forward-only access, hardware-prefetch-friendly, no intermediate copies
  ([SBE reference](https://github.com/aeron-io/simple-binary-encoding),
  [Aeron: Discover SBE](https://aeron.io/other/sbe-simple-binary-encoding/)).

### 2.3 Comparison (durable-log axes)

| Axis | celnet-journal/replog | Chronicle Queue | Aeron Archive/Cluster |
|---|---|---|---|
| Durability | fsync per append, atomic rename compaction | mmap + msync | recorded + replicated |
| Integrity | CRC32 per record **and** per wire entry | CRC optional | CRC + quorum |
| Interior bit-rot detect | **no (marker-less, documented gap)** | resync via index | resync via term |
| Consensus | **full Raft** (election/log-match/snapshot) | Enterprise | **full Raft** |
| Codec | **hand-rolled allocating decoder** | Chronicle-Wire flyweight | SBE flyweight |
| Untrusted-byte safety | bounds-checked, frame-capped, fuzz-tested | — | — |
| License | ours | dual (paid Enterprise) | Apache-2.0 |

### 2.4 Verdict & gaps

**Match:** the *consensus + durability* story is SOTA-parity with Aeron Cluster for a
fixed-membership cluster, and more honest about its failure model than most (the
interior-bit-rot limitation is stated, not hidden — `lib.rs:80-94`). The decoder's untrusted
-byte hardening (bounds-checked cursor, frame cap, `n.min(1024)` pre-alloc, no-panic fuzz
tests) is genuinely good defensive engineering.

**Gaps vs. SOTA:**
1. **Marker-less framing** ⇒ a single rotted interior byte silently drops everything after
   it (`lib.rs:80-94`, already documented as a non-goal). SOTA logs have a per-record
   **resync/sync-word** so recovery distinguishes interior corruption from a torn tail and
   surfaces it. This is the **one durability gap worth closing** for IB-grade audit.
2. **Hand-rolled allocating decoder.** `Message::decode`/`LogEntry::decode` allocate a
   `Vec` per entry/payload (`wire.rs:262,305`, `entry.rs:116`). SOTA is a **zero-copy
   flyweight** returning `&[u8]` views into the frame. For replog this is *replication-path*,
   not hot-pricing-path, so the payoff is modest — but the codec is also bespoke where a
   generated one would be safer and faster.
3. **CRC32 is not collision-hard.** Fine for accidental corruption (its stated scope,
   `crc32.rs:8-9`); for an IB audit trail, a keyed integrity (the doc already defers the
   "MAC/identity story" to `celnet-server`) is the eventual SOTA bar.

---

## 3. Wire codec — `celnet-proto` (prost/tonic) + `celnet-replog::wire`

### 3.1 What we built

- Control-plane / client contract: **prost + tonic** generated from `.proto`
  (`celnet-proto/Cargo.toml:15-16`, `lib.rs`), one unversioned current contract, gRPC bidi
  stream + a WebSocket mirror serializing the **same** messages (`docs/SCALE-OUT.md` §5).
- Replication: the hand-rolled `wire.rs` framing above.
- **Hot price-tick fan-out carries no codec at all** — it broadcasts `Copy` structs
  in-process through `celnet-fanout`; encoding happens only when a session *formats its own
  `Update`* at the edge (`docs/SCALE-OUT.md` §5).

### 3.2 SOTA & where it beats protobuf

- **SBE** (FIX Trading Community / real-logic, Apache-2.0): fixed-layout, forward-only,
  zero-copy **flyweight** access; **>10× protobuf** on encode throughput for densely-
  populated messages with *predictable* (low-variance) latency; explicitly preferred over
  FlatBuffers/Cap'n Proto for hot reads because forward-only access is prefetch-friendly
  (no data-dependent pointer loads)
  ([SBE reference impl](https://github.com/aeron-io/simple-binary-encoding),
  [real-logic.github.io/simple-binary-encoding](https://real-logic.github.io/simple-binary-encoding/),
  [SBE FIX standard](https://www.fixtrading.org/standards/sbe/)).
- **FlatBuffers / Cap'n Proto**: zero-copy random access, but relative-pointer / vtable
  indirection ⇒ data-dependent loads; for tiny hot payloads, format choice is often
  dominated by TLS/alloc/handler cost
  ([API design 2026](https://techbytes.app/posts/api-design-2026-protobuf-vs-flatbuffers-vs-capn-proto/),
  [buffer-benchmarks](https://github.com/kcchu/buffer-benchmarks)).
- **rkyv** (Rust): zero-copy archived access, faster than prost/capnp/flatbuffers in the
  author's bench ([rkyv-is-faster-than](https://david.kolo.ski/blog/rkyv-is-faster-than/)) —
  notable because `docs/SCALE-OUT.md` §7 already calls rkyv a *possible future* handoff
  optimization (we currently use a little-endian handoff codec, not rkyv).

### 3.3 Comparison (codec axes)

| Axis | prost/protobuf (ours, control) | replog hand-rolled (ours) | SBE | FlatBuffers | Cap'n Proto | rkyv |
|---|---|---|---|---|---|---|
| Encode throughput | baseline | ~baseline | **>10×** | high | high | **very high** |
| Latency predictability | varies (varint) | fixed LE | **fixed-offset, low-var** | good | good | good |
| Zero-copy decode | no | no (allocates) | **yes (flyweight)** | yes | yes | **yes** |
| Access pattern | parse-then-use | parse-then-use | forward-only (prefetch-friendly) | random (ptr) | random (ptr) | direct |
| Versioning | field tags (we're **unversioned**) | tag byte | **fixed schema** (fits unversioned) | optional | optional | schema |
| Tooling/ecosystem | **excellent** | none (bespoke) | good | good | moderate | Rust-only |
| License | BSD | ours | Apache-2.0 | Apache-2.0 | MIT | MIT/Apache |

### 3.4 Verdict & gaps

**Match where it matters today:** the hot price path carries *no codec* (in-process `Copy`),
so protobuf's cost is irrelevant there — a genuinely SOTA decision (the fastest serialization
is none). prost/tonic is the *right* control-plane choice for tooling and the WS mirror.

**Gap, deploy-gated:** the moment price ticks cross the **network** to a counterparty, a
fixed-offset zero-copy codec (SBE-style) is the SOTA and protobuf is not. Because that edge
is itself deploy-gated (`docs/SCALE-OUT.md` §5/§11), this is a *future* gap, not a current
defect. Our **unversioned** contract is actually a *good fit* for SBE-style fixed layout:
SBE's main cost (schema versioning machinery) is something we explicitly don't need
(CLAUDE.md rule 9).

---

## 4. Broader 2026 SOTA sweep — ranked relevance

| Topic | Relevance to us | Verdict |
|---|---|---|
| **Disruptor multi-consumer / sequencer** | core of `celnet-fanout` | **already adopted** |
| **Aeron broadcast buffer (shm, cross-process)** | session isolation | **port-the-idea** (shm variant) — see §6 |
| **loom / shuttle / miri** | the unsafe seqlock | **adopt-as-dev-dep** — highest-value, lowest-effort §6 |
| **SBE fixed-offset zero-copy** | network price edge | **port-the-idea**, deploy-gated |
| **Chronicle Wire flyweight** | durable-log codec | port-the-idea (modest payoff) |
| **Aeron Cluster Raft** | `celnet-replog` | **already at parity** |
| **io_uring ZC-RX / XDP / DPDK** | network edge | already the documented tiering (`SCALE-OUT.md` §5); io_uring ~4.2 µs, XDP ~2.1 µs, DPDK ~850 ns median ([prod metrics](https://anshadameenza.com/blog/technology/2025-01-15-kernel-bypass-networking-dpdk-spdk-io_uring), [arXiv:2502.09281](https://arxiv.org/pdf/2502.09281)) — **deploy-gated, correctly deferred** |
| **QUIC / Aeron-Cluster cross-host** | fleet fan-out | proxy-multicast-tree (Jasper) already designed (`SCALE-OUT.md` §5) |
| **seqlock on weak ISAs (aarch64 fence)** | `ring.rs:338` | **already fixed & documented** |
| **FlatBuffers / Cap'n Proto / rkyv** | codec alternatives | considered; SBE-style wins for forward-only hot reads, rkyv noted for handoff |

---

## 5. Where we already match or exceed SOTA (with evidence)

1. **Seqlock correctness on aarch64.** The Acquire fence between payload copy and post-check
   (`ring.rs:326-345`) closes the weak-memory reorder window that a naive two-load seqlock
   leaves open — and it's *documented* as having been caught under 16× oversubscription.
   This is better than most open-source Disruptor clones.
2. **Conflation accounting.** `received + skipped == produced` with strict-increasing,
   never-duplicated delivery, gated at 1000 consumers (`lib.rs:27-35`, `ring.rs:474-500`) —
   more rigorous than a bare overwrite ring.
3. **Zero-alloc, no-back-pressure-to-core invariant** — the FX-streaming-correct policy
   (`lib.rs:19-35`, `docs/SCALE-OUT.md` §6), matching the conflate-never-buffer SOTA.
4. **Raft completeness** — election + log-matching + conflicting-tail truncation + §7
   snapshot + InstallSnapshot catch-up + bit-identical replay oracle (`replog/src/lib.rs`),
   at parity with Aeron Cluster for fixed membership.
5. **Untrusted-byte decoder hardening** — bounds-checked cursor, 64 MiB frame cap,
   `n.min(1024)` pre-alloc, no-panic-on-any-prefix fuzz tests (`wire.rs`).
6. **Honest failure model** — the journal *states* its marker-less interior-corruption
   limitation rather than hiding it (`journal/src/lib.rs:80-94`).

---

## 6. Prioritized recommendations

Each: (i) gap, (ii) concrete change (file-level), (iii) effort vs payoff, (iv) disposition.
All respect the hard rules: OSS/permissive only; provenance in comments/docs but **never**
in identifiers; one unversioned contract; zero-alloc/lock-free hot core preserved.

### P0 — Model-check the unsafe seqlock (loom + shuttle + miri gate)
- **Gap:** the `celnet-fanout` seqlock is hand-written `unsafe`; correctness rests on stress
  tests that found the aarch64 fence bug *by luck*. SOTA for this code class is exhaustive
  interleaving search.
- **Change:** add `loom` as a dev-dependency; behind `#[cfg(loom)]`, swap
  `std::sync::atomic`/`UnsafeCell` for `loom::sync::atomic`/`loom::cell::UnsafeCell` in
  `ring.rs`; add `crates/celnet-fanout/tests/loom_seqlock.rs` driving 1-producer/2-consumer
  interleavings asserting no-torn-read + conflation invariant. Add a `cargo miri test` job
  for UB. Use `shuttle` for larger randomized runs the exhaustive loom search can't reach.
- **Effort:** S–M (a day). **Payoff:** very high — turns "we got lucky" into "proven".
- **Disposition:** **adopt-as-dev-dep** (loom/shuttle/miri are MIT/Apache).

### P1 — Resync marker in the journal framing (close the interior-corruption gap)
- **Gap:** marker-less framing makes an interior CRC failure indistinguishable from a torn
  tail → silent loss of acknowledged records (`journal/src/lib.rs:80-94`).
- **Change:** prepend a fixed 32-bit **sync word** to each record frame in `celnet-journal`;
  on recovery, if a CRC fails but the *next* sync word is found at the expected offset,
  surface `JournalError::CorruptInterior` instead of truncating. Keeps the snapshot-sentinel
  trick. One unversioned format (we have no readers to break — rule 9).
- **Effort:** M. **Payoff:** high for IB-grade audit (no silent acknowledged-record loss).
- **Disposition:** **port-the-idea** (Chronicle/Aeron use sync words).

### P2 — Batch consume on the fan-out ring
- **Gap:** `try_recv` is one-item-per-call (`ring.rs:266`); the Disruptor's batch-drain win
  is unrealized.
- **Change:** add `Consumer::try_recv_batch(&mut self, out: &mut [T]) -> usize` (or a
  drain-into-slice) that loads `head` once and copies the in-window run, amortizing the
  cursor write. Zero-alloc (caller-provided slice).
- **Effort:** S. **Payoff:** medium (throughput on bursty fan-out).
- **Disposition:** **port-the-idea** (Disruptor batch consumption).

### P3 — Shared-memory cross-process broadcast variant (session isolation)
- **Gap:** `celnet-fanout` is single-process (`Arc`); Aeron/Chronicle/bcast are cross-process
  shm, enabling crash-isolated session processes.
- **Change:** a *new* sibling module/crate (do not disturb the in-process ring) backing
  `Inner<T>` with a `memmap2`-mapped buffer + the identical seqlock protocol; only the
  storage backing changes, the algorithm is shared. Gate on a 2-process loopback test.
- **Effort:** L. **Payoff:** medium, conditional — only if/when we want process isolation
  per session group. **Defer** until the single-shard edge is measured as the limiter.
- **Disposition:** **port-the-idea**, deploy-gated.

### P4 — SBE-style fixed-offset zero-copy codec for the network price-tick edge
- **Gap:** prost/protobuf is not SOTA for the latency-critical network tick path.
- **Change:** when the network price edge lands, define a fixed-layout, forward-only,
  zero-copy flyweight codec (our own, provenance-cited) for the `PriceTick` message *only*;
  keep prost/tonic for the control plane. Our unversioned contract removes SBE's main cost
  (version machinery). Record as an ADR with prost kept first-class for control.
- **Effort:** M–L. **Payoff:** high *at the edge*, but deploy-gated (no current hot-path
  codec). **Defer** to the network-edge milestone.
- **Disposition:** **port-the-idea**, deploy-gated.

### Already covered (no action)
- io_uring/XDP/DPDK tiering, proxy-multicast-tree cross-host fan-out, conflate-never-buffer,
  Raft consensus, the aarch64 fence — all already designed/built/documented.

---

## 7. What is explicitly a capability GAP to be "SOTA in every way"

1. **No model-checking of the unsafe lock-free core** (P0) — the single most important gap;
   IB-grade unsafe concurrency should be loom/miri-gated, not stress-tested-and-hoped.
2. **Marker-less journal framing** (P1) — silent interior-corruption loss is below audit-grade.
3. **No cross-process shm fan-out** (P3) — limits session crash-isolation; conditional gap.
4. **No zero-copy network codec for ticks** (P4) — deploy-gated; protobuf is not SOTA at the
   network price edge.

Everything else (the seqlock protocol, conflation accounting, zero-alloc invariant, full
Raft, decoder hardening, the kernel-bypass tiering plan) is **at or above** the 2026 SOTA bar
for its scope, with the evidence cited in §5.

---

## Sources

- Aeron: [log buffers docs](https://aeron.io/docs/aeron/log-buffers-images/) · [theaeronfiles log buffers](https://theaeronfiles.com/aeron-transport/log-buffers/) · [cnc.dat](https://theaeronfiles.com/aeron-transport/cnc-dat/) · [InfoQ presentation](https://www.infoq.com/presentations/aeron/) · [aeron repo](https://github.com/aeron-io/aeron)
- Aeron Cluster / Raft: [aeron-cluster README](https://github.com/aeron-io/aeron/blob/master/aeron-cluster/README.md) · [Raft in Aeron Cluster](https://aeron.io/product-features/raft-consensus-aeron-cluster/)
- SBE: [reference impl](https://github.com/aeron-io/simple-binary-encoding) · [docs](https://real-logic.github.io/simple-binary-encoding/) · [FIX SBE standard](https://www.fixtrading.org/standards/sbe/) · [Discover SBE](https://aeron.io/other/sbe-simple-binary-encoding/)
- Chronicle: [vs Aeron, sanj.dev](https://sanj.dev/post/chronicle-vs-aeron/) · [vs Aeron vs Kafka](https://sanj.dev/post/chronicle-vs-aeron-vs-others/) · [durability blog](https://chronicle.software/insights/blogs/comparing-approaches-to-durability-in-low-latency-messaging-queues) · [OpenHFT/Chronicle-Queue](https://github.com/OpenHFT/Chronicle-Queue)
- Disruptor: [LMAX Disruptor](https://lmax-exchange.github.io/disruptor/disruptor.html)
- Rust SPMC: [HaveFunTrading/bcast](https://github.com/HaveFunTrading/bcast)
- Codecs: [API design 2026](https://techbytes.app/posts/api-design-2026-protobuf-vs-flatbuffers-vs-capn-proto/) · [buffer-benchmarks](https://github.com/kcchu/buffer-benchmarks) · [rkyv-is-faster-than](https://david.kolo.ski/blog/rkyv-is-faster-than/)
- Verification: [loom](https://docs.rs/loom) · [shuttle](https://github.com/awslabs/shuttle) · [Miri](https://github.com/rust-lang/miri) · [Kani comparison](https://model-checking.github.io/kani/tool-comparison.html)
- Kernel bypass: [DPDK/io_uring/XDP metrics](https://anshadameenza.com/blog/technology/2025-01-15-kernel-bypass-networking-dpdk-spdk-io_uring) · [io_uring ZC-RX kernel docs](https://docs.kernel.org/networking/iou-zcrx.html) · [Fast userspace networking, arXiv:2502.09281](https://arxiv.org/pdf/2502.09281)
- Academic: [wait-free MPMC ring, ACM SIGAPP](https://dl.acm.org/doi/10.1145/2835260.2835264) · [Jasper proxy-multicast, arXiv:2402.09527](https://arxiv.org/abs/2402.09527)
