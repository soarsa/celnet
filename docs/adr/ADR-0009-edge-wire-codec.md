# ADR-0009 — Network price-edge codec: a fixed-offset, zero-copy, flyweight binary encoding (`celnet-wire-edge` / `EdgeFrame`)

- **Status:** **PROPOSED** (2026-06-08).
- **Scope:** the **network price edge** — the high-volume streamed-price datapath from
  `celnet-server` to latency-sensitive (HFT / market-making) counterparties. Does **not**
  touch the pinned zero-alloc hot core, the GPU/PDE/MC math, or the control-plane RPC surface.
- **Honours:** ADR-0007 (one unversioned wire contract — no `schema_version`), ADR-0008
  (single clean contract evolved in place), and CLAUDE.md guardrails #2 (no placeholders),
  #6 (scale/perf are requirements), #7 (OSS-only), #8 (vendor-neutral naming), #9 (no
  versioned APIs), #11 (zero-cost observability, scale-out aware).
- **Provenance (prose only, never in identifiers):** the fixed-offset flyweight technique is
  *Simple Binary Encoding (SBE)*, standardised by the **FIX Trading Community** and proven in
  matching-engine and market-data fast paths. Per guardrail #8 we **build our own** codec and
  name every artifact for its purpose under the `celnet-` namespace; "SBE" and "FIX" appear in
  documentation and code comments to cite the method, **never** in a crate, module, type,
  trait, or function name.

---

## 1. Context — the one remaining encoding gap

The SOTA messaging/encoding review identified four encoding surfaces in Celnet and judged
three of them already correct:

1. **In-process hot path (core → edge).** Carries **no codec at all**: prices fan out as
   `Copy`/POD snapshots over the `rtrb` SPSC core→edge ring and the `celnet-fanout` SPMC
   broadcast ring (per-slot seqlock, zero-alloc publish; ARCHITECTURE §3.2, SCALE-OUT §5).
   There are no bytes to encode in-process — a reader dereferences a struct. **Correct as-is;
   this ADR does not touch it.**
2. **In-host IPC / upgrade state handoff.** `rkyv` zero-copy archived bytes (ARCHITECTURE
   §8.3). **Correct as-is.**
3. **Control plane (RPC).** `celnet-proto` over `tonic`/`prost` (gRPC) with a JSON-over-WS
   mirror (`celnet-server/src/ws/codec.rs`). Subscribe / unsubscribe / request-quote /
   accept-quote / execute / lifecycle frames are **low-frequency, heterogeneous, evolving,
   and ergonomics-sensitive** — exactly where protobuf's self-describing tag-length-value
   model, codegen, and tooling earn their keep. **Correct as-is.**
4. **Network price edge (streaming quotes to counterparties).** This is the **gap.** The
   high-volume per-tick price line is today serialized with the **same** `prost` machinery as
   the control plane (and JSON on the WS mirror). For a latency-sensitive counterparty
   consuming a sustained price stream, protobuf's per-field varint decoding, length-prefix
   scanning, and (for repeated/optional fields) allocation are avoidable per-tick cost on the
   tightest, most repetitive, most fixed-shape payload in the system.

### The shape of the hot edge payload

The streamed price line is small, fixed-shape, and emitted at the §1.2 throughput target of
**≥ 1M price updates/s/core**. In the current contract it is an `Update` carrying a
monotonic **sequence**, a `TwoWayPrice` (`bid: double`, `offer: double`), and one or two
`TradableToken`s (`token: uint64`, `side`, `premium: double`, `valid_until_nanos`) — the
click-to-trade tokens that let a counterparty execute *that exact streamed price* without an
RFQ round-trip. Every field is a fixed-width scalar; the message has no recursion and a
bounded, statically-known token count. **This is the canonical "flyweight" case**: a
fixed-layout record read in place by offset, with no parse step between the wire and the
consumer's struct.

### Latency budget this serves (ARCHITECTURE §1.2 / SCALE-OUT §5, §11)

ARCHITECTURE §1.2 sets the per-shard price budget at **p50 ≤ 2 µs / p99 ≤ 10 µs / p99.9 ≤
25 µs** and **≥ 1M price updates/s/core sustained**; SCALE-OUT §11 carries these as fleet
regression guards and adds the many-counterparty fan-out tail SLO (bounded delivery window
across 100/1000 subscribers, target < quote-validity window). The pricing compute already
fits this; the analysis is explicit that "our published NFRs (2–25 µs pricing) are about
compute, not wire" (SOTA review §2.2). **Precisely because the compute is already inside
budget, the serialize/parse step on the streamed edge is the next-largest controllable
contributor to tail latency and per-core throughput on that path** — and it is the one that a
fixed-offset zero-copy frame removes outright (decode becomes a typed view over the received
bytes; no per-field branching, no allocation, no varint loop). Tail latency is the product
(§1.1): eliminating a per-tick parse step removes a jitter source on the exact path that
faces HFT counterparties.

---

## 2. Decision (PROPOSED)

Introduce a **dedicated network price-edge codec** as a small, focused crate
**`celnet-wire-edge`**, exposing a fixed-layout binary frame type **`EdgeFrame`** (and the
matching reader/writer flyweights, e.g. `EdgeQuoteView` / `EdgeQuoteWriter` — final names set
at build time, all purpose-named, none vendor- or method-named). It is used **only** for the
high-volume streamed-price subset of the contract on the network edge to counterparties that
negotiate the binary transport; everything else continues to use `celnet-proto`.

### 2.1 Fixed-layout, zero-copy, flyweight design

- **Fixed offsets, no tags.** Each field sits at a compile-time-known byte offset. There are
  **no field tags and no length prefixes per field** (the SBE flyweight property): the reader
  computes a field's address as `base + OFFSET` and reads it, with no scan. A block-length
  header (a single fixed prefix) carries the frame's total fixed-block size so a forward-only
  reader can advance frame-to-frame without parsing contents.
- **Natural alignment, no padding-dependent reads.** Field offsets are laid out so every
  scalar is naturally aligned (8-byte `double`/`u64` on 8-byte boundaries, etc.), enabling a
  direct typed read of the received buffer. To stay safe and portable we read through a
  checked zero-copy view (no unaligned-pointer UB, no reliance on host struct padding); the
  layout is chosen so the aligned-read fast path is the common path. The frame layout is the
  contract — **not** any Rust struct's in-memory `repr`.
- **Fixed endianness (little-endian) on the wire.** One canonical byte order, chosen once
  (LE — matches the `aarch64`/`x86_64` deployment targets so the common case is a no-op), and
  always applied regardless of host. This makes the encoding **deterministic across
  OS/arch/rebuild** in the same spirit as ARCHITECTURE §8.1's bit-identity discipline: a given
  price produces exactly one byte sequence, gated by a `to_bits` round-trip test.
- **Zero allocation on encode and decode.** Encode writes scalars into a caller-provided,
  pre-sized buffer (sized from the static block length); decode is a borrow over received
  bytes (`&[u8] → EdgeQuoteView<'_>`) with no owned allocation. This preserves the
  zero-alloc property the rest of the edge already holds (proven elsewhere by counting-
  allocator tests; the same gate applies here).
- **Bounded, fixed token count.** The click-to-trade tokens are encoded as a fixed
  two-slot group (sell-side / buy-side, mirroring the `TwoWayPrice` two-token design) with an
  explicit "present" discriminant per slot — a fixed-size repeating block, not a
  variable-length, length-prefixed list. This keeps the whole frame fixed-size and
  offset-addressable.

### 2.2 The UNVERSIONED single-contract constraint (no schema versioning)

This is the load-bearing design constraint and the reason we can use a flyweight at all
without the usual SBE ceremony:

- **No `schema_version`, no template/version field, no `schemaId` negotiation.** ADR-0007 and
  CLAUDE.md #9 establish that Celnet runs exactly **one** current contract with **no
  mixed-version window** (blue-green full cutover, ARCHITECTURE §5; cross-fleet rolling
  cutover, SCALE-OUT §7). The `EdgeFrame` therefore carries **no version field**. SBE's
  message-header `version`/`schemaId` fields and its "never renumber, only append, reserve
  removed slots" evolution rules exist to let an **old reader decode a new writer** — a
  requirement we explicitly do not have. We drop that machinery entirely.
- **Evolution is editing, not versioning.** Changing the streamed-price shape means editing
  the `EdgeFrame` layout **and every peer (server + every SDK/decoder) in the same change**,
  exactly as §8.3 already prescribes for `celnet-proto`. A uniform fleet upgrade deploys one
  layout; no reader ever sees two layouts. This keeps the flyweight maximally lean (a header
  that is just block-length, no version, no template id) while staying consistent with the
  rest of the platform's zero-legacy posture (#10).
- **One generator, two projections.** To prevent the binary frame and the proto contract from
  drifting, the `EdgeFrame` layout is **generated from / checked against `celnet-proto`** as
  its single source of truth (a build-time consistency test asserts the binary frame's field
  set is exactly the streamed-price projection of the proto `Update`). The binary frame is a
  **derived projection** of the one contract for the hot subset — never a second, competing
  schema to keep in sync by hand.

### 2.3 Why NOT protobuf on this path

protobuf is the right tool for the control plane and the wrong tool for the streamed price
edge, for reasons specific to *this* payload:

- **Tag-length-value parsing per field.** prost decode walks each field: read a varint tag,
  switch on field number and wire type, read a varint length or scalar. For a fixed-shape
  record at 1M+/s/core this is per-tick branching and varint work the flyweight removes (offset
  read, no branch).
- **Varint cost on the wrong data.** Our hot fields are `double` premia and large `u64`
  tokens/timestamps — values for which protobuf's varint encoding is *larger and slower* than
  a fixed 8 bytes (protobuf stores `double` as fixed64 anyway, and large `u64` varints are
  near-worst-case). Fixed-width wins on both size and decode time here.
- **Optionality and allocation.** Repeated/optional fields invite per-message allocation on
  the decode side; the flyweight's fixed two-slot token group is allocation-free.
- **Self-description we don't need.** protobuf's tags make a message self-describing so an
  evolving/loosely-coupled reader can skip unknown fields. On the **single-contract** edge
  there are no unknown fields and no loose coupling — we pay for a property (#9 says) we never
  use. (Note also: the bytes-on-the-wire framing is independent of `prost`'s **in-process**
  zero-copy story; this ADR is about the cross-host edge, not in-host IPC, which §8.3 already
  solves with `rkyv`.)

What protobuf keeps: **everything else.** Control-plane RPC, RFQ/quote/execute negotiation,
lifecycle, admin, the JSON-over-WS mirror — heterogeneous, evolving, low-frequency, tooling-
rich. We are **not** replacing `celnet-proto`; we are adding a specialized fast lane for the
one high-volume fixed-shape stream.

### 2.4 Relationship to the existing proto control-plane contract

- **proto remains the source of truth and the only published schema.** `EdgeFrame` is a
  performance projection of the streamed-price subset, generated/checked against
  `celnet-proto::Update` / `TwoWayPrice` / `TradableToken`. There is exactly one logical
  contract (#9); two *encodings* of the same data is not two contracts.
- **Transport negotiation, not version negotiation.** A counterparty session selects the
  binary edge transport at **connection setup** (a control-plane capability on the gRPC/WS
  handshake), the same place SCALE-OUT §5 already does "one contract, two transports" (gRPC
  primary, WS mirror). This is a *transport* choice, not a *schema version* choice — no N/N-1
  contract negotiation is introduced. Counterparties that do not request the binary edge keep
  receiving the proto/JSON stream unchanged.
- **The fan-out stays upstream of the codec.** `celnet-fanout` publishes the `Copy` tick once
  per pair; each session consumer formats *its own* outbound frame (its sequence, its tokens)
  from the shared tick (SCALE-OUT §5, `services/pricefanout.rs`). The `EdgeFrame` writer slots
  in **at that per-session format step**, replacing the per-session `prost`/JSON encode for
  binary-transport sessions only — no change to the producer, the ring, or the conflation
  accounting (`received + skipped == produced` invariant is untouched).

### 2.5 Open-source-only dependency stance

- **Hand-rolled, OSS primitives only.** We **build** the codec (guardrail #8: "we build our
  own, citing provenance"). At most it depends on permissively-licensed (MIT/Apache-2.0)
  byte primitives already in or compatible with the workspace (e.g. `bytes`, and a
  `zerocopy`/`bytemuck`-style checked-cast crate for the safe aligned-view path) —
  `cargo-deny`'s license policy gates this. **No vendor SBE toolkit, no commercial codec, no
  proprietary market-data SDK**, ever (guardrail #7). The `fefix`/FerrumFIX codec *ideas* and
  dictionary model may inform the design (CELNET-FIX-INTEGRATION-PLAN §; self-described
  "wildly unstable, not for production") but are **not** taken as a runtime dependency — same
  posture we already hold for `celnet-fix`.
- **Determinism as a gate, not an assertion.** A `to_bits` round-trip test (encode→decode→
  re-encode is byte-identical, and prices survive bit-for-bit) plus a cross-arch byte-vector
  golden lock the encoding, mirroring §8.1/§8.2. Fuzzing (`Arbitrary` bytes → decoder, Linux-
  nightly job, §8.2) hardens the decode path against adversarial/truncated frames.

---

## 3. Phased adoption plan (with the honest deploy-gated boundary)

Mirrors the platform's "measure before you claim" discipline (§1.2, SCALE-OUT §0/§11/§12):
each phase has a written, gated acceptance criterion, and the **absolute cross-host win is
never claimed in-repo**.

**Phase 0 — codec library + layout (in-repo, fully gatable).** Build `celnet-wire-edge`:
`EdgeFrame` layout, `EdgeQuoteWriter`/`EdgeQuoteView` flyweights, the proto-consistency check
(layout == streamed projection of `celnet-proto::Update`). Gates: round-trip `to_bits`
identity, zero-alloc encode/decode (counting-allocator test), cross-arch golden byte vectors,
decode fuzz corpus committed. **Provable in-repo; no deploy needed.**

**Phase 1 — encode-side wiring behind transport negotiation (in-repo).** Wire the writer into
the per-session format step in `services/pricefanout.rs` for sessions that negotiate the
binary transport at handshake; proto/JSON remains the default. Gates: an `iai-callgrind`
instruction-count comparison (binary encode vs `prost` encode for the `Update` payload, the
deterministic relative-regression signal §1.2 endorses), and an in-process throughput
benchmark (binary vs proto encode under fan-out load) as a **relative** signal. **Honest
boundary:** this proves the *encode-side* compute win in-process — an upper bound and a
relative-regression guard, **not** a wire claim (identical honesty boundary to the
`celnet-fanout` throughput figure, SCALE-OUT §5).

**Phase 2 — typed SDK decoder (in-repo).** Add the zero-copy decoder to `celnet-client`
(and document the layout for non-Rust SDKs) so a counterparty consumes `EdgeFrame`s with no
parse step. Gates: SDK round-trip parity against the proto path (same prices, same tokens,
same sequence semantics) so the binary and proto streams are provably the *same contract*.

**Phase 3 — DEPLOY-GATED: absolute cross-host edge latency/throughput.** The end-to-end win
(tick-to-counterparty wire latency and per-core sustained throughput on the real network
datapath) is **provable only on the deployed datapath** — it depends on the NIC, the kernel
socket path (`SO_REUSEPORT`/eBPF, and later `io_uring`/XDP/DPDK, SCALE-OUT §5), TLS, and the
cross-DC fabric, none of which exist in-repo. Per SCALE-OUT §11/§12, the absolute cross-host
SLO rows (and any "X µs to N receivers" figure) are **deploy-gated and never asserted in the
repository**. This ADR's in-repo deliverable is the codec + its compute gates (Phases 0–2);
the network latency SLO is recorded as a benchmark to run on the deployed datapath, alongside
the kernel-bypass and Jasper-tree tiers it composes with.

---

## 4. Consequences

**Positive.**
- Removes the per-tick serialize/parse step from the highest-frequency, most latency-sensitive
  edge — a direct jitter and throughput improvement on the path facing HFT counterparties,
  consistent with §1.2's tail-latency-is-the-product principle.
- Stays inside the platform's invariants: one unversioned contract (#9), zero-alloc edge,
  OSS-only (#7), vendor-neutral naming (#8), zero-legacy (#10). No new schema, no negotiation
  ceremony, no proto replacement.
- Composes cleanly with the already-built/already-designed scale-out tiers (`celnet-fanout`
  conflation, `SO_REUSEPORT`/eBPF, the deferred kernel-bypass and multicast-tree tiers): the
  codec is orthogonal to transport and slots beneath them.

**Costs / risks.**
- **Two encodings of one contract** must be kept in lockstep. Mitigated by generating/checking
  `EdgeFrame` against `celnet-proto` and a build-time consistency gate — drift fails CI.
- **Fixed layout is rigid by design.** Any contract change to the streamed-price shape edits
  the frame + every peer in one change (the §8.3 editing-not-versioning discipline, now
  spanning two encodings). Accepted: this is the single-contract guardrail working as intended.
- **`unsafe`/alignment surface.** Mitigated by using a checked zero-copy crate (no raw
  unaligned pointer reads), the round-trip/golden/fuzz gates, and keeping the codec a small,
  audited crate.
- **In-repo proof is partial by necessity.** The absolute network win is deploy-gated (Phase
  3). We state this honestly rather than claim an unmeasured wire latency.

**Status:** **PROPOSED.** No code lands under this ADR until it is accepted and a workstream
claims `celnet-wire-edge` on the parallel-session board. On acceptance, this ADR is registered
via `manage_adr`, `docs/INTERFACES.md` records the new edge crate, and `docs/SOTA-2026.md`
gap #4 is marked ADR-tracked.

---

## 5. References

- `docs/ARCHITECTURE.md` §1.2 (NFR latency/throughput budget), §3.2 (lock-free hot path),
  §7 (data flow — the async edge), §8.1 (determinism), §8.3 (single current wire contract:
  proto / rkyv / `celnet-fix`).
- `docs/SCALE-OUT.md` §5 (market-data fan-out & counterparty streaming; `celnet-fanout`;
  one-contract-two-transports; deploy-gated cross-host boundary), §6 (conflate, never buffer),
  §11 (fleet SLOs), §12 (built vs defer).
- `docs/SOTA-2026.md` §2.1–§2.2 (runtime/transport decisions; "NFRs are about compute, not
  wire"), §2.4 (lock-free primitives unchanged), §6 (next-actions mapping).
- ADR-0007 (one unversioned contract), ADR-0008 (single clean contract, evolved in place).
- `celnet-proto` (`proto/celnet.proto`: `Update`, `TwoWayPrice`, `TradableToken`) — the source
  of truth the `EdgeFrame` projects; `celnet-server/src/ws/codec.rs` and
  `services/pricefanout.rs` — the current edge encode/fan-out sites.
- `docs/CELNET-FIX-INTEGRATION-PLAN.md` (the `fefix`/FerrumFIX assessment: codec ideas usable,
  not a runtime dependency) — the same "build our own, cite provenance" posture as `celnet-fix`.
- **Method provenance (prose only):** Simple Binary Encoding (SBE), FIX Trading Community —
  fixed-offset flyweight market-data/order encoding. Cited as method; **not** used in any
  Celnet identifier.

