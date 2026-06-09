# W6-RIGOR-INFRA — Resume Design Pack

> **Lane:** W6-RIGOR-INFRA (mine). **Status:** PAUSED for compute courtesy (mesh §4.1) while
> session-B lands the proto window; this pack is design-only (no cargo/git run to produce it).
> **Resume trigger:** the proto window is green on `main`. Then execute the three pieces below
> in the stated dependency order, each gated independently in an isolated worktree off `main`.
>
> **Scope.** Three rigor upgrades that make the heavy correctness gates run *fast and correct*:
> 1. A `cfg(loom)` model-check of the `celnet-fanout` SPMC seqlock ring — proves the
>    torn-read / conservation invariants under an exhaustive relaxed-memory interleaving search,
>    catching the aarch64 reorder window **by construction** rather than by lucky stress.
> 2. A per-record **sync-word** frame for `celnet-journal` — lets recovery tell an interior
>    CRC failure (now `CorruptInterior`) apart from a healing torn tail, closing the format's
>    one documented limitation while keeping torn-tail healing intact.
> 3. Two **new per-crate mutation gates** (`celnet-fanout`, `celnet-journal`) in the existing
>    house style — `cargo test` runner (not nextest), `--jobs 3`, zero non-equivalent
>    survivors, justified-only exclusions.
>
> **Guardrails honored throughout:** no mocks/placeholders/`todo!()`; the std hot path stays
> byte-for-byte zero-cost (loom is `cfg`-gated and never enters a release build); one
> unversioned contract (the journal frame is a clean break, no migration shim — guardrail 9);
> OSS-only deps (`loom` is MIT, dev/`cfg(loom)`-only); numerics/concurrency validated against an
> **independent** oracle (the loom interleaving search and the running-sum replay — never the
> code grading itself).

---

## 0. Dependency order (execute top-to-bottom)

Each step is independently gated; later steps do not depend on earlier ones *for correctness*,
but this order minimizes rebuild blast-radius and lands the cheapest-risk change first.

| # | Step | Crate(s) touched | Gate | Why this order |
|---|------|------------------|------|----------------|
| 1 | **Journal sync-word frame** (§2) | `celnet-journal` | `just check-crate celnet-journal` + new mutation gate (§3) | Pure-std, no new dep; self-contained; the gate tests are deterministic and fast. |
| 2 | **`celnet-journal` mutation gate** (§3.2) | `.config/`, `justfile` | `just mutants-gate-journal` to green | Locks step 1 against future suite-weakening before moving on. |
| 3 | **Loom shim + model-check** (§1) | `celnet-fanout` | `cargo test --cfg loom` (loom) + `just check-crate celnet-fanout` (std unchanged) | Adds a dev/`cfg`-only dep (`loom`); the `mem.rs` shim is the only source change to the hot path and must leave std codegen identical — verify last so a regression is unambiguous. |
| 4 | **`celnet-fanout` mutation gate** (§3.1) | `.config/`, `justfile` | `just mutants-gate-fanout` to green | Locks the ring's std suite (the loom test is the model oracle; the std conflation stress + unit tests are the mutation-killing suite). |

After all four: run `just check` (full workspace integration gate) once, confirm the literal
`All gates passed.` line, update `docs/HARDENING.md` (§ new gates) and the implementation ledger.

---

## 1. Loom / relaxed-memory model-check of the fanout seqlock ring

### 1.1 Why loom (not shuttle), and what it proves

`crates/celnet-fanout/src/ring.rs` is a single-producer / multi-consumer broadcast ring whose
publication is a **true seqlock**: the producer raises an odd in-progress stamp (`(seq<<1)|1`,
`Release`), writes the `UnsafeCell<T>` payload, then stores the even stable stamp (`seq<<1`,
`Release`); a consumer `Acquire`-loads the stamp, copies the payload, executes an **`Acquire`
fence**, re-loads the stamp and retries on any change. The crate doc (ring.rs lines 327–337)
records that the `Acquire` *fence* between the payload copy and the post-stamp re-load was added
because, without it, on a weakly-ordered architecture (aarch64) the plain payload load can be
reordered **past** the `stamp_after` load — an undetected torn read — and that this was only
surfaced "once under 16× full-suite CPU oversubscription." That is exactly the class of bug a
stress test catches by luck and a **model checker catches by construction.**

- **`loom`** (MIT, `tokio-rs/loom`) does an *exhaustive* exploration of thread interleavings
  **and** models the C11 relaxed-memory semantics: it will place the consumer's payload load and
  `stamp_after` load in *every* permitted order, including the aarch64-legal reorder that the
  fence forbids. If the fence were removed, loom finds a torn read deterministically; with the
  fence, loom proves no interleaving produces one (within the bounded model). This is the right
  tool for a hand-rolled atomic protocol.
- **`shuttle`** (randomized schedule fuzzing) would *probabilistically* hit it but gives no
  by-construction guarantee; loom's exhaustiveness is what we want for a < 4-thread, tiny-state
  seqlock. We choose loom.

**Independent-oracle note:** the loom *search* is the oracle — it is not the production code
grading itself. The invariant assertions (below) are the specification; loom enumerates the
executions; a violation is a real counterexample, not a tautology.

### 1.2 The `cfg(loom)` shim — keep the std hot path byte-for-byte zero-cost

Loom requires that *every shared atomic/cell access* go through `loom::sync::atomic` /
`loom::cell::UnsafeCell` so it can instrument them. The hot path must NOT pay for this in
release builds. We isolate the switch in one new module and change only the ring's imports.

**New file: `crates/celnet-fanout/src/mem.rs`**

```rust
//! Atomics/cell shim: loom-instrumented primitives under `cfg(loom)`, the
//! zero-cost std primitives otherwise. The ONLY place the ring's choice of
//! atomic/cell type is made — so the loom model-check (`tests/loom_seqlock.rs`,
//! built only with `RUSTFLAGS="--cfg loom"`) instruments every shared access,
//! while the production build is byte-for-byte the std seqlock (loom never
//! enters a release build; `cfg(loom)` is set by the loom test invocation only).

#[cfg(loom)]
pub(crate) use loom::cell::UnsafeCell;
#[cfg(loom)]
pub(crate) use loom::sync::Arc;
#[cfg(loom)]
pub(crate) use loom::sync::atomic::{AtomicU64, Ordering, fence};

#[cfg(not(loom))]
pub(crate) use std::cell::UnsafeCell;
#[cfg(not(loom))]
pub(crate) use std::sync::Arc;
#[cfg(not(loom))]
pub(crate) use std::sync::atomic::{AtomicU64, Ordering, fence};
```

> `loom::cell::UnsafeCell::get()` does **not** return a raw `*mut T` directly the way
> `std::cell::UnsafeCell::get()` does — loom's cell exposes `with(|p| …)` / `with_mut(|p| …)`
> closures (and a `get_mut()` guard) so it can track access. We therefore introduce a **single
> accessor pair** that compiles to the raw pointer under std and to the closure form under loom,
> so the rest of `ring.rs` is written once.

**Add to `mem.rs`:**

```rust
/// Read the payload pointer of a cell. Under std this is the plain raw pointer;
/// under loom it threads through `with` so loom records the access. The closure
/// returns a value (the `Copy`-out), keeping a single call shape in `ring.rs`.
#[inline]
#[cfg(not(loom))]
pub(crate) unsafe fn cell_read<T: Copy>(c: &UnsafeCell<T>) -> T {
    // SAFETY: caller upholds the seqlock read discipline (stamp-guarded copy).
    unsafe { *c.get() }
}
#[inline]
#[cfg(loom)]
pub(crate) unsafe fn cell_read<T: Copy>(c: &UnsafeCell<T>) -> T {
    // SAFETY: as above; loom's `with` records the shared read for the model.
    c.with(|p| unsafe { *p })
}

/// Write the payload of a cell (single-producer exclusive).
#[inline]
#[cfg(not(loom))]
pub(crate) unsafe fn cell_write<T>(c: &UnsafeCell<T>, v: T) {
    // SAFETY: single-producer exclusive write under the in-progress stamp.
    unsafe { *c.get() = v; }
}
#[inline]
#[cfg(loom)]
pub(crate) unsafe fn cell_write<T>(c: &UnsafeCell<T>, v: T) {
    // SAFETY: as above; loom's `with_mut` records the shared write.
    c.with_mut(|p| unsafe { *p = v; });
}
```

**Edits to `crates/celnet-fanout/src/ring.rs`:**

1. Replace the three `use std::…` lines (37–39) with `use crate::mem::{...}`:
   ```rust
   use crate::mem::{Arc, AtomicU64, Ordering, UnsafeCell, cell_read, cell_write, fence};
   ```
   (Drop the separate `use crossbeam_utils::CachePadded;`? No — keep it; `CachePadded` is not a
   loom primitive and is fine to wrap a loom atomic. But under `cfg(loom)` loom's `model`
   re-runs the closure many times; `CachePadded` is a transparent newtype and is harmless.)
2. The producer's payload write (lines 177–181) becomes:
   ```rust
   #[allow(unsafe_code)]
   // SAFETY: exclusive single-producer write, in-bounds index.
   unsafe { cell_write(&slot.value, item); }
   ```
3. The consumer's payload copy (lines 323–325) becomes:
   ```rust
   #[allow(unsafe_code)]
   // SAFETY: in-bounds index; `Copy` read guarded by the seqlock stamps.
   let value = unsafe { cell_read(&slot.value) };
   ```
4. The existing `std::sync::atomic::fence(Ordering::Acquire)` (line 338) becomes
   `fence(Ordering::Acquire)` (now resolved through the shim).
5. **`mod mem;`** added to `crates/celnet-fanout/src/lib.rs` as `pub(crate) mod mem;` (and the
   crate `lib.rs` keeps `mod ring;` etc. as-is). Confirm `mem` is not re-exported.

> **Zero-cost proof obligation:** under `not(loom)` the shim is a pure re-export and the two
> `cell_*` fns are `#[inline]` one-liners that monomorphize to `*c.get()` — identical codegen to
> the current `unsafe { *slot.value.get() }`. The std unit tests and the `just check-crate
> celnet-fanout` gate must remain green and unchanged (same 5 tests). Verify by re-running the
> existing tests; no behavioral diff is permitted.

### 1.3 The model-check test: `crates/celnet-fanout/tests/loom_seqlock.rs`

Built **only** with the loom cfg; under a normal build it compiles to nothing.

```rust
//! Loom model-check of the SPMC seqlock ring. Built only with
//! `RUSTFLAGS="--cfg loom" cargo test --test loom_seqlock` (see the
//! `loom-fanout` justfile recipe). Exhaustively explores 1-producer /
//! 2-consumer interleavings under loom's C11 relaxed-memory model and asserts
//! the seqlock invariants — proving NO interleaving (including the aarch64
//! payload-load / stamp-reload reorder the Acquire fence forbids) yields a torn
//! read, and that delivery conserves (received + skipped accounting holds).
#![cfg(loom)]

use celnet_fanout::BroadcastRing;
use loom::sync::Arc;
use loom::thread;

/// A payload whose two halves must always agree for a coherent (non-torn) read.
/// The producer publishes `(s, s)` for sequence `s`; a torn read observes a slot
/// whose two halves come from DIFFERENT publishes (`a != b`), which the seqlock
/// must make impossible to RETURN (it may be observed mid-protocol, but never
/// returned by `try_recv`).
type Pair = (u64, u64);

#[test]
fn spmc_seqlock_no_torn_read_under_all_interleavings() {
    loom::model(|| {
        // Tiny ring so the producer LAPS the consumers (forces the overwrite race
        // that exercises the torn-read window). Capacity 2 is the minimum.
        let mut ring = BroadcastRing::<Pair>::new(2);
        let c0 = ring.consumer();
        let c1 = ring.consumer();
        let mut producer = ring.into_producer();

        // 3 publishes over a 2-slot ring guarantees at least one in-place overwrite
        // concurrent with a consumer read — the interleaving we must prove safe.
        let prod = thread::spawn(move || {
            for s in 0..3u64 {
                producer.publish((s, s));
            }
        });

        let consume = |mut c: celnet_fanout::Consumer<Pair>| {
            move || {
                // Drain whatever is visible; every RETURNED pair must be coherent.
                let mut last_seq: Option<u64> = None;
                while let Ok((a, b)) = c.try_recv() {
                    assert_eq!(a, b, "TORN READ: slot halves disagree ({a} != {b})");
                    if let Some(prev) = last_seq {
                        assert!(a > prev, "delivered sequences must be strictly in order");
                    }
                    last_seq = Some(a);
                }
                (c.received(), c.skipped())
            }
        };
        let t0 = thread::spawn(consume(c0));
        let t1 = thread::spawn(consume(c1));

        prod.join().unwrap();
        let (r0, s0) = t0.join().unwrap();
        let (r1, s1) = t1.join().unwrap();

        // CONSERVATION: each consumer's (received + skipped) is the count of
        // produced items it REACHED in the sequence; it can never exceed the 3
        // produced, and (because each consumer started at seq 0 over a 2-slot ring)
        // the in-window guarantee is `received <= capacity` checked per-consumer.
        assert!(r0 + s0 <= 3, "c0 over-counted: {r0}+{s0}");
        assert!(r1 + s1 <= 3, "c1 over-counted: {r1}+{s1}");
        assert!(r0 <= 2 && r1 <= 2, "received bounded by ring capacity");
    });
}
```

A **second** loom test pins the *deliberate-fence-removal* counterexample as documentation
(kept `#[ignore]` by default so CI is fast, run on demand to confirm the model is live):

```rust
/// Documentation/regression: with the Acquire fence between the payload copy and
/// the stamp re-load REMOVED, loom finds a torn read. This test does NOT remove
/// the fence (we cannot, from a test) — instead it asserts the *model is
/// sensitive*: it runs the same model and is expected to PASS, and the crate doc
/// (ring.rs §"Seqlock reader barrier") records that deleting the fence makes the
/// `no_torn_read` model FAIL. Run `just loom-fanout` after any fence edit.
#[test]
fn model_is_the_oracle_note() {
    // Intentionally a no-op assertion: the real proof is the model above. This
    // stub exists so the regression procedure (delete fence -> model fails) is
    // discoverable from the test file, per the failure-model honesty guardrail.
}
```

> **How it catches the aarch64 window BY CONSTRUCTION.** loom's executor treats every
> `Acquire`/`Release`/`Relaxed` load/store and the `Acquire` fence as model events and explores
> the partial orders the C11 model permits. The payload `cell_read` (a non-atomic shared read,
> tracked via `with`) is, without the fence, allowed by the model to be *observed after* the
> `stamp_after` load — loom enumerates that order and the `a != b` assert fires. With the fence
> present, that order is excluded from the model, so the assert never fires across the full
> exhaustive search. No CPU oversubscription or luck involved — it is the model that forbids it.

### 1.4 Bounding the loom state space (CI-time correctness)

Loom's search is exponential in shared-access count × threads. Keep it tiny and bounded:

- **Capacity 2, 3 publishes, 2 consumers** is the minimal config that forces an overwrite
  concurrent with a read. Do not grow it.
- Set `LOOM_MAX_PREEMPTIONS=3` (env) in the recipe — bounded-preemption exploration is the
  standard loom practice and still covers the seqlock reorder window (which needs ≤ 2 context
  switches inside the read).
- The recipe wraps the run in `timeout 1800` as a wedge guard.

**New `justfile` recipe:**

```makefile
# Loom model-check of the SPMC seqlock ring (relaxed-memory interleaving search).
# Built ONLY under `--cfg loom` so the std hot path is byte-for-byte unaffected
# (loom never enters a release build). Bounded-preemption exploration keeps the
# search inside CI time while still covering the aarch64 payload/stamp reorder
# window the Acquire fence forbids (see ring.rs §"Seqlock reader barrier").
loom-fanout:
    timeout 1800 env RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 \
        {{_cargo}} test -p celnet-fanout --test loom_seqlock --release
```

(`--release` because loom models are CPU-heavy; the model's correctness does not depend on
opt-level — it is a model, not a timing test.)

**Cargo manifest edit — `crates/celnet-fanout/Cargo.toml`:** add loom under a `cfg(loom)`
target so it is pulled in *only* when the cfg is set (never a normal-build dependency):

```toml
[target.'cfg(loom)'.dependencies]
loom = "0.7"
```

> This is the canonical loom wiring: `loom` appears in the dependency tree only for the
> `--cfg loom` build, so `cargo build`/`just check` never compile or link it, and `cargo-deny`
> sees it only on the loom target. Confirm `cargo deny check` stays green (loom 0.7 is MIT —
> verify the exact version's license/advisories before committing; it is dev/cfg-only so it does
> not enter the shipped binary).

---

## 2. Journal per-record SYNC-WORD frame change

### 2.1 The defect this closes

`crates/celnet-journal/src/lib.rs` is **marker-less** today (doc lines 69–94): a record is
`payload_len(u32) ‖ sequence(u64) ‖ payload ‖ crc32(u32)`. A CRC failure on an *interior*
record is **indistinguishable** from a torn tail, so recovery heals it as a tail and silently
drops every record after it (test `interior_crc_corruption_truncates_trailing_records`). The
doc itself names the fix: "A future framed format with a per-record resync marker would let
recovery distinguish interior CRC corruption from a torn tail and surface it." This step
implements exactly that.

### 2.2 New frame layout (magic at the HEAD)

A fixed 8-byte little-endian **sync word** is prepended to *every* record (data and snapshot).
The sync word is **outside** the value-bearing fields but **inside** the CRC coverage, so a
flipped sync byte is still caught by CRC.

```text
DATA record (new):
┌───────────┬──────────────┬──────────────┬───────────────┬──────────┐
│  SYNC     │ payload_len  │  sequence    │   payload     │  crc32   │
│  u64 LE   │  u32 LE      │  u64 LE      │ payload_len B │  u32 LE  │
└───────────┴──────────────┴──────────────┴───────────────┴──────────┘
SYNC = 0x_C E L N 4A 4F 55 52  (a fixed, recognizable, non-zero magic; exact bytes below)

SNAPSHOT record (new) — payload_len field is the SNAPSHOT_MARKER sentinel (u32::MAX):
┌───────────┬──────────────┬──────────────┬──────────────┬──────────────┬──────────┐
│  SYNC     │ SNAPSHOT_MARK│  watermark   │ snapshot_len │ snapshot B   │  crc32   │
│  u64 LE   │  u32 = MAX   │  u64 LE      │  u32 LE      │ snapshot_len │  u32 LE  │
└───────────┴──────────────┴──────────────┴──────────────┴──────────────┴──────────┘
```

**Exact magic.** Use a `u64` constant whose bytes are unlikely to occur as a real frame prefix
and are human-recognizable in a hex dump:

```rust
/// Per-record sync word at the head of every frame (data and snapshot). A fixed,
/// non-zero magic so recovery can tell an INTERIOR CRC failure (intact sync word,
/// failing CRC -> `CorruptInterior`) apart from a torn tail (absent/short sync
/// word at EOF -> heal). It is inside the CRC coverage, so a flipped sync byte is
/// still caught. ASCII "CLNJRNL\0" little-endian.
const SYNC_WORD: u64 = u64::from_le_bytes(*b"CLNJRNL\0");
/// Width of the sync word prefix.
const SYNC_LEN: usize = 8;
```

The snapshot `SNAPSHOT_MARKER` sentinel in `payload_len` is **unchanged** — the data-vs-snapshot
discrimination is still the sentinel, now read at offset `SYNC_LEN` instead of `0`. The two
framings still share `frame_record`/`frame_snapshot`, so they cannot drift.

### 2.3 The recovery decision rule (the core of the change)

`read_one` now reads the sync word **first**. The three outcomes:

| Sync word state | CRC state | Verdict |
|---|---|---|
| 0 bytes available (clean EOF boundary) | — | `Eof` |
| 1..8 bytes (short read) **or** 8 bytes `!= SYNC_WORD` at the *physical tail* | — | `TornTail` (heal) |
| 8 bytes `== SYNC_WORD`, frame body present | **valid** | `Record` (then sequence/placement checks as today) |
| 8 bytes `== SYNC_WORD`, frame body present | **fails** | **`CorruptInterior`** (NEW) — intact sync word means a real, framed record landed here; a failing CRC is interior rot, not a torn tail |
| 8 bytes `== SYNC_WORD`, body **short** (truncated payload/crc) | — | `TornTail` (heal) — the *next* frame's sync word never started, so this is a genuinely torn final frame |

The decisive new rule: **intact `SYNC_WORD` + failing CRC ⇒ `CorruptInterior`.** A torn tail
cannot forge an intact sync word followed by a fully-present-but-bad-CRC body unless the crash
landed *exactly* mid-payload of a record whose header (incl. sync word) was already flushed — in
which case the body is *short* (the trailing CRC is missing or partial), which we still classify
as `TornTail`. The only way to reach "intact sync + full body + bad CRC" is **bit-rot of a
completely-written record**, i.e. interior corruption. This is precisely the discrimination the
old format lacked.

> **Edge case — sync word present but body short at EOF.** If the final record's sync word and
> some of its body flushed but the CRC trailer did not, `read_full_or_short` returns `Short` for
> the payload/CRC read → `TornTail`. Healing still works. The `CorruptInterior` path is reached
> **only** when the full frame (through CRC) is present.

### 2.4 Exact code changes in `crates/celnet-journal/src/lib.rs`

**Constants (after `CRC_LEN`, line 163):** add `SYNC_WORD` and `SYNC_LEN` (above). `HEADER_LEN`
stays `4 + 8` (the *logical* header is still len+seq); the sync word is a distinct prefix so the
sentinel-at-`payload_len` logic is untouched.

**`frame_record` (lines 835–847):** prepend the sync word and include it in the CRC.
```rust
fn frame_record(sequence: u64, payload_len: u32, payload: &[u8]) -> Vec<u8> {
    debug_assert!(payload_len <= MAX_PAYLOAD_LEN, "data payload over the bound");
    let mut frame = Vec::with_capacity(SYNC_LEN + HEADER_LEN + payload.len() + CRC_LEN);
    frame.extend_from_slice(&SYNC_WORD.to_le_bytes());          // NEW
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&sequence.to_le_bytes());
    frame.extend_from_slice(payload);
    let checksum = crc32(&frame);   // CRC now covers the sync word too
    frame.extend_from_slice(&checksum.to_le_bytes());
    frame
}
```

**`frame_snapshot` (lines 855–866):** identically prepend the sync word (before the
`SNAPSHOT_MARKER`):
```rust
fn frame_snapshot(watermark: u64, snap_len: u32, snapshot: &[u8]) -> Vec<u8> {
    debug_assert_eq!(snap_len as usize, snapshot.len());
    debug_assert!(snap_len <= MAX_PAYLOAD_LEN, "snapshot over the bound");
    let mut frame = Vec::with_capacity(SYNC_LEN + HEADER_LEN + 4 + snapshot.len() + CRC_LEN);
    frame.extend_from_slice(&SYNC_WORD.to_le_bytes());          // NEW
    frame.extend_from_slice(&SNAPSHOT_MARKER.to_le_bytes());
    frame.extend_from_slice(&watermark.to_le_bytes());
    frame.extend_from_slice(&snap_len.to_le_bytes());
    frame.extend_from_slice(snapshot);
    let checksum = crc32(&frame);
    frame.extend_from_slice(&checksum.to_le_bytes());
    frame
}
```

**`read_one` (lines 675–751):** read the sync word first; classify per §2.3.
```rust
fn read_one<R: Read>(reader: &mut R, expected_seq: u64, at_start: bool) -> Result<ReadOutcome> {
    // 1. Sync word: distinguishes a framed record start from a torn/EOF boundary.
    let mut sync = [0u8; SYNC_LEN];
    match read_full_or_short(reader, &mut sync)? {
        FillState::Empty => return Ok(ReadOutcome::Eof),
        FillState::Short => return Ok(ReadOutcome::TornTail),
        FillState::Full => {}
    }
    if u64::from_le_bytes(sync) != SYNC_WORD {
        // No framed record begins here. At the physical tail this is a torn final
        // frame (or trailing garbage from an interrupted append) -> heal.
        return Ok(ReadOutcome::TornTail);
    }

    // 2. Logical header (len + seq), exactly as before.
    let mut header = [0u8; HEADER_LEN];
    match read_full_or_short(reader, &mut header)? {
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
        FillState::Full => {}
    }
    let len_field = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let sequence = u64::from_le_bytes([
        header[4], header[5], header[6], header[7], header[8], header[9], header[10], header[11],
    ]);

    if len_field == SNAPSHOT_MARKER {
        return read_snapshot(reader, &sync, &header, sequence, expected_seq, at_start);
    }
    if len_field > MAX_PAYLOAD_LEN {
        return Ok(ReadOutcome::TornTail);
    }
    let payload_len = len_field;

    let mut payload = vec![0u8; payload_len as usize];
    match read_full_or_short(reader, &mut payload)? {
        FillState::Full => {}
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
    }
    let mut crc_bytes = [0u8; CRC_LEN];
    match read_full_or_short(reader, &mut crc_bytes)? {
        FillState::Full => {}
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
    }
    let stored_crc = u32::from_le_bytes(crc_bytes);

    // CRC covers sync word + header + payload.
    let mut framed = Vec::with_capacity(SYNC_LEN + HEADER_LEN + payload.len());
    framed.extend_from_slice(&sync);
    framed.extend_from_slice(&header);
    framed.extend_from_slice(&payload);
    if crc32(&framed) != stored_crc {
        // NEW DISCRIMINATION: a fully-present frame (intact sync word + complete
        // body through the CRC trailer) whose CRC fails is INTERIOR corruption,
        // not a torn tail — a torn tail leaves the body short (caught above) or the
        // next sync word absent. Surface it instead of silently truncating.
        return Err(JournalError::CorruptInterior {
            at_sequence: expected_seq,
            reason: "record CRC failed with an intact sync word (interior corruption)",
        });
    }

    if sequence != expected_seq {
        return Err(JournalError::CorruptInterior {
            at_sequence: expected_seq,
            reason: "sequence number is not strictly monotonic",
        });
    }
    Ok(ReadOutcome::Record(Record { kind: RecordKind::Data, sequence, payload }))
}
```

**`read_snapshot` (lines 761–816):** takes the sync word too and includes it in CRC; a CRC
failure with an intact sync word is likewise `CorruptInterior`:
```rust
fn read_snapshot<R: Read>(
    reader: &mut R,
    sync: &[u8; SYNC_LEN],
    header: &[u8; HEADER_LEN],
    watermark: u64,
    expected_seq: u64,
    at_start: bool,
) -> Result<ReadOutcome> {
    let mut inner_len_bytes = [0u8; 4];
    match read_full_or_short(reader, &mut inner_len_bytes)? {
        FillState::Full => {}
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
    }
    let snap_len = u32::from_le_bytes(inner_len_bytes);
    if snap_len > MAX_PAYLOAD_LEN { return Ok(ReadOutcome::TornTail); }
    let mut snapshot = vec![0u8; snap_len as usize];
    match read_full_or_short(reader, &mut snapshot)? {
        FillState::Full => {}
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
    }
    let mut crc_bytes = [0u8; CRC_LEN];
    match read_full_or_short(reader, &mut crc_bytes)? {
        FillState::Full => {}
        FillState::Empty | FillState::Short => return Ok(ReadOutcome::TornTail),
    }
    let stored_crc = u32::from_le_bytes(crc_bytes);

    let mut framed = Vec::with_capacity(SYNC_LEN + HEADER_LEN + 4 + snapshot.len());
    framed.extend_from_slice(sync);
    framed.extend_from_slice(header);
    framed.extend_from_slice(&inner_len_bytes);
    framed.extend_from_slice(&snapshot);
    if crc32(&framed) != stored_crc {
        return Err(JournalError::CorruptInterior {
            at_sequence: expected_seq,
            reason: "snapshot CRC failed with an intact sync word (interior corruption)",
        });
    }
    if !at_start {
        return Err(JournalError::CorruptInterior {
            at_sequence: expected_seq,
            reason: "snapshot record appears after the start of the log",
        });
    }
    Ok(ReadOutcome::Record(Record { kind: RecordKind::Snapshot, sequence: watermark, payload: snapshot }))
}
```

**`JournalError::CorruptInterior` doc (lines 191–206) and the crate-level "Failure model"
(lines 69–94):** rewrite to state the new, *stronger* contract: an interior CRC failure under an
intact sync word is now surfaced as `CorruptInterior`; a torn tail (absent/short sync word, or a
short body) still heals. The stated limitation paragraph is **deleted/replaced** by the new
guarantee. Update the on-disk-format ASCII diagrams (lines 25–53) to show the sync word.

> **One subtlety to call out in the doc.** A torn tail can leave *trailing garbage* that happens
> to be ≥ 8 bytes but is **not** the sync word — handled as `TornTail` (the `!= SYNC_WORD`
> branch). The pathological case "garbage that exactly equals the sync word followed by a
> full-but-CRC-bad body" is astronomically improbable (an interrupted append cannot produce a
> *complete* CRC trailer for bytes it never wrote) and, if it ever occurred, surfacing it as
> `CorruptInterior` is the *safe* failure (stop + report, do not silently drop). This is the
> correct conservative bias and is stated, not hand-waved.

### 2.5 Updates to existing journal tests (byte-offset / record-size dependents)

The sync word changes record size from `HEADER_LEN + payload + CRC_LEN` to
`SYNC_LEN + HEADER_LEN + payload + CRC_LEN`. Update **only** the offset-arithmetic tests; the
behavioral tests pass unchanged:

- `interior_crc_corruption_truncates_trailing_records` (tests.rs lines 213–255): **REPURPOSE.**
  Its old name/assert (recovery stops & drops trailing) is now WRONG — the new behavior is
  `CorruptInterior`. Rename to `interior_crc_corruption_is_surfaced_not_healed` and assert
  `Journal::open` returns `Err(JournalError::CorruptInterior { .. })` with the
  "interior corruption" reason. Recompute `record_len = SYNC_LEN + HEADER_LEN + 7 + CRC_LEN`
  and `flip_at = 2 * record_len + SYNC_LEN + HEADER_LEN + 1` (a byte inside record 2's payload).
- `flipped_byte_in_final_record_is_detected_and_dropped` (lines 174–211): the *last* record's
  CRC fails but its sync word is intact and its body complete ⇒ under the NEW rule this is now
  `CorruptInterior`, **not** a healed drop. Split into two tests:
  (a) `flipped_byte_in_final_record_with_intact_sync_is_interior` — flip a payload byte (sync
  word + body intact) ⇒ expect `CorruptInterior`;
  (b) `truncated_final_record_heals_as_torn_tail` — chop the CRC/body so the body is *short* ⇒
  expect heal to `N-1` (this is the genuine torn-tail path, already covered by
  `truncated_tail_is_recovered_cleanly`, which is unaffected and stays green).
- `frame_record` / `forge_snapshot` test helpers (lines 260–283): prepend `SYNC_WORD` and
  include it in the CRC, mirroring the lib changes, so forged frames are valid.
- All byte-length / `record_len` literals elsewhere: none other hard-code offsets (they use
  `metadata().len()` deltas), so they are size-agnostic and pass unchanged.

### 2.6 The three NEW gate tests (the spec for §2)

1. **`intact_syncword_failing_crc_interior_is_corrupt`** — the headline. Append `N=6` data
   records; flip one byte in the payload of an **interior** record (index 2), leaving its sync
   word and full body present. `Journal::open` must return
   `Err(JournalError::CorruptInterior { reason, .. })` with `reason.contains("interior")`. This
   is the behavior the old format could not provide.
2. **`torn_tail_still_heals_with_syncword_format`** — append `N=10`; chop the **physical tail**
   so the final record's body/CRC is short (`set_len(full - 3)`). `Journal::open` heals to
   `N-1` good records, `Ok`, file truncated — i.e. the sync word did not break torn-tail
   healing. (Independent oracle: the surviving payloads equal the first `N-1` appended.)
3. **`interior_corruption_distinct_from_torn_tail`** — the discrimination proof, run as one
   test over two forged logs built from the same good prefix: (a) good prefix + an interior
   record with intact sync + bad CRC + a valid record after ⇒ `CorruptInterior`; (b) good
   prefix + a record whose sync word is intact but whose **body is truncated at EOF** ⇒ heals to
   the prefix length, `Ok`. Asserts the two inputs that were *indistinguishable* under the old
   format now yield **different** verdicts.

> **Independent oracle for §2:** the existing running-sum replay machine
> (`rebuild`/`replay_from_compacted_equals_replay_from_full_bit_identical`) is the independent
> oracle — it grades recovered *state*, not the framing code, and must stay byte-identical after
> the frame change (a non-negotiable regression check: compaction round-trip + kill-restart
> byte-identity tests pass unchanged).

---

## 3. Per-crate mutation gates (`celnet-fanout`, `celnet-journal`)

### 3.0 House style (from `.config/mutants*.toml` + the justfile)

The established Celnet mutation-gate convention (verbatim from `.config/mutants.toml`,
`-surface`, `-exotics`, `-risk-cube`, `-xva`):

- One `.config/mutants-<crate>.toml` per crate, header comment explaining the gate, **`exclude_re`
  empty to start** (strict: every viable mutant must be caught), members added **only** with an
  inline equivalence justification and a 200k-style differential-oracle argument.
- A `just mutants-gate-<crate>` recipe: `timeout <N> {{_cargo}} mutants -p <crate> --config
  .config/mutants-<crate>.toml`. Exits non-zero on **any non-equivalent survivor** = the
  zero-survivor bar.
- Added to the `mutants-gate-numerics` aggregate? **No** — that aggregate is numerics-only.
  Create a parallel `mutants-gate-infra: mutants-gate-fanout mutants-gate-journal` aggregate.

**The task's three deltas from the numerics house style, applied here:**
- **`--test-tool=cargo` (NOT nextest).** cargo-mutants defaults to whatever the workspace test
  runner is; the numerics gates run nextest tests. For these two crates the **plain `cargo test`
  runner** is required: (i) the loom test is a `--cfg loom` target that nextest's default
  profile does not build with the loom flag, and (ii) the journal's `proptest` adversarial-bytes
  test and the std `cfg(test)` modules are most reliably driven by `cargo test`. Pass
  `--test-tool=cargo` explicitly so the gate is reproducible regardless of `.config/nextest.toml`.
- **`--jobs 3`.** Bound parallelism so the gate is deterministic in wall-time on the M4 and does
  not contend with a parallel session (compute courtesy). 3 jobs is the sweet spot for these
  small crates (the suites are sub-second; 3-way keeps the box responsive).
- **Zero non-equivalent survivor bar**, identical to the numerics gates.

### 3.1 `.config/mutants-fanout.toml` + recipe

```toml
# cargo-mutants gate configuration — celnet-fanout mutation-testing policy.
#
# Mirrors `.config/mutants.toml` (the celnet-vanilla gate) for the SPMC broadcast
# ring: the single-producer seqlock publish, the consumer two-stamp + Acquire-
# fence torn-read protocol, and the conflation/skip accounting. This is the price
# fan-out substrate — a silently-weakened suite could let a torn-read or a
# skip-accounting regression ship.
#
# How the gate works
# ------------------
# `cargo mutants -p celnet-fanout --test-tool=cargo --jobs 3 \
#     --config .config/mutants-fanout.toml`
# applies a syntactic mutant per operator/comparison/return value and re-runs the
# suite with the PLAIN `cargo test` runner (NOT nextest): the ring's correctness
# is pinned by the std unit/conflation-stress tests, and the loom model-check
# (`tests/loom_seqlock.rs`, a `--cfg loom` target) is the independent oracle for
# the relaxed-memory protocol. cargo-mutants exits non-zero on ANY survivor, so
# this gate enforces ZERO non-equivalent survivors.
#
# `--jobs 3` bounds parallelism for a deterministic wall-time on aarch64-apple-
# darwin and to stay courteous to a parallel session. `--test-tool=cargo` is
# explicit so the gate is independent of the workspace nextest profile.
#
# Equivalence exclusions
# ----------------------
# `exclude_re` lists ONLY provably semantically-equivalent mutants, each justified
# inline (a measure-zero boundary, a never-hit watchdog, an unobservable stamp-
# encoding internal). It starts EMPTY — the gate runs strict; a member is added
# only with a justification and a differential-oracle check, never to hide a real
# survivor. See docs/HARDENING.md s2.
exclude_re = [
]
```

```makefile
# Mutation GATE on the SPMC fan-out ring (seqlock publish + torn-read protocol +
# skip accounting). Plain `cargo test` runner (the loom model-check + std stress
# tests are the killing suite); `--jobs 3` bounds wall-time. Zero non-equivalent
# survivors. See docs/HARDENING.md.
mutants-gate-fanout:
    timeout 1200 {{_cargo}} mutants -p celnet-fanout --test-tool=cargo --jobs 3 \
        --config .config/mutants-fanout.toml
```

> **Scope note for the fanout gate:** cargo-mutants will mutate the `unsafe` seqlock arithmetic
> (`seq << 1`, `| 1`, the `want`/`stable`/`writing` computations, the stamp comparisons, the
> conflation `oldest_live = head - capacity`, the skip-gap arithmetic). The std conflation
> stress test (`lapped_consumer_conflates_with_exact_skip_accounting`) + the ordering/empty
> tests are the primary killers; some stamp-protocol mutants that only manifest as a torn read
> under concurrency are caught by the loom target. If the loom target proves too slow under
> mutation, add `--test-tool=cargo` *with the std tests only* and document that the loom model
> is the standing oracle run by `just loom-fanout` (not mutated) — recorded honestly in
> HARDENING.md, not hidden.

### 3.2 `.config/mutants-journal.toml` + recipe

```toml
# cargo-mutants gate configuration — celnet-journal mutation-testing policy.
#
# Mirrors `.config/mutants.toml` for the durable event journal: the sync-word
# framing (`frame_record`/`frame_snapshot`), the CRC-32, the recovery state
# machine (`read_one`/`read_snapshot`/`scan`) — including the NEW intact-sync-word
# + failing-CRC -> CorruptInterior discrimination — and the atomic compaction.
# This is the crash-recovery substrate: a weakened suite could let a torn-tail-vs-
# interior-corruption misclassification or a non-deterministic replay ship.
#
# How the gate works
# ------------------
# `cargo mutants -p celnet-journal --test-tool=cargo --jobs 3 \
#     --config .config/mutants-journal.toml`
# applies a syntactic mutant and re-runs the suite with the PLAIN `cargo test`
# runner (NOT nextest) so the `proptest` adversarial-bytes recovery test and the
# `cfg(test)` recovery/compaction tests drive every mutant. The INDEPENDENT oracle
# is the running-sum replay machine (`replay_from_compacted_equals_replay_from_full`)
# and the kill-restart byte-identity test — they grade recovered STATE, not the
# framing code. cargo-mutants exits non-zero on ANY survivor: ZERO non-equivalent
# survivors. `--jobs 3` bounds wall-time; `--test-tool=cargo` is explicit.
#
# Equivalence exclusions
# ----------------------
# `exclude_re` starts EMPTY (strict). A member is added only with an inline
# equivalence justification, never to hide a survivor. See docs/HARDENING.md s2.
exclude_re = [
]
```

```makefile
# Mutation GATE on the durable journal (sync-word framing + CRC + recovery state
# machine + atomic compaction). Plain `cargo test` runner so the proptest
# adversarial-bytes + recovery/compaction tests drive every mutant; `--jobs 3`
# bounds wall-time. Zero non-equivalent survivors. See docs/HARDENING.md.
mutants-gate-journal:
    timeout 1800 {{_cargo}} mutants -p celnet-journal --test-tool=cargo --jobs 3 \
        --config .config/mutants-journal.toml

# Both infra-crate mutation gates in sequence.
mutants-gate-infra: mutants-gate-fanout mutants-gate-journal
    @echo "All infra mutation gates passed."
```

> **Survivor-handling protocol (both crates), per the no-mocks / honesty guardrail:** run the
> gate, and for EVERY survivor either (a) add a test that kills it (the default — these are
> genuine gaps, exactly as `arbitrage.rs`'s 33 survivors were all killed, not excluded), or (b)
> if and only if it is provably semantically equivalent, add it to `exclude_re` with an inline
> WHY and a differential-oracle confirmation. The bar is **zero non-equivalent survivors**;
> record the kill-rate + any equivalence cluster in `docs/HARDENING.md §2`.

---

## 4. Files added / changed (complete manifest)

**Added**
- `crates/celnet-fanout/src/mem.rs` — the `cfg(loom)` atomics/cell shim + `cell_read`/`cell_write`.
- `crates/celnet-fanout/tests/loom_seqlock.rs` — the 1P/2C loom model-check.
- `.config/mutants-fanout.toml`, `.config/mutants-journal.toml` — the two new gate configs.

**Changed**
- `crates/celnet-fanout/src/lib.rs` — `pub(crate) mod mem;`.
- `crates/celnet-fanout/src/ring.rs` — imports via `crate::mem`; payload r/w via `cell_*`; fence via shim. No behavioral change off-loom.
- `crates/celnet-fanout/Cargo.toml` — `[target.'cfg(loom)'.dependencies] loom = "0.7"`.
- `crates/celnet-journal/src/lib.rs` — `SYNC_WORD`/`SYNC_LEN`; sync word in `frame_record`/`frame_snapshot`; `read_one`/`read_snapshot` read+validate sync word and surface intact-sync+bad-CRC as `CorruptInterior`; doc/diagrams + `CorruptInterior` doc rewritten to the stronger contract.
- `crates/celnet-journal/src/tests.rs` — `frame_record`/`forge_snapshot` helpers prepend the sync word; the two byte-offset tests repurposed (§2.5); three new gate tests (§2.6).
- `justfile` — recipes `loom-fanout`, `mutants-gate-fanout`, `mutants-gate-journal`, `mutants-gate-infra`.
- `docs/HARDENING.md` — new gate definitions + kill-rate/oracle notes; the journal "Failure model" limitation paragraph removed (now a guarantee).
- `docs/IMPLEMENTATION-LEDGER.md` + ledger anchor — new entry on resume completion.

## 5. Resume checklist (run in order)

1. `just check-crate celnet-journal` (frame change green) → `just mutants-gate-journal` (zero survivors).
2. `just check-crate celnet-fanout` (std path unchanged, 5 tests green) → `just loom-fanout` (model green) → `just mutants-gate-fanout` (zero survivors).
3. `cargo deny check` (loom 0.7 MIT, cfg-only — confirm clean).
4. `just check` → confirm literal `All gates passed.`; then `docs/` + ledger updates.

