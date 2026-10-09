# Celnet — Delivery Model (how we build, in lanes)

How Celnet is built to be production-grade *and* fast: many agents progressing safely in
parallel across the whole scope, gate-by-gate, each unit verified before it counts as done.
This is the operational companion to `docs/ROADMAP.md` (what to build) and `GUIDE.md`
(the rules + live ledger).

## 1. Lanes, waves, and gates

- **Lane** = one work-stream owning disjoint crate dir(s) (see the ledger in `GUIDE.md`).
  A lane edits only its own `crates/<name>/` and never the root manifest.
- **Wave** = one orchestration pass that runs several lanes concurrently and then runs a
  **critique milestone** (full integration gate + adversarial review + competitive gap
  analysis). A wave maps to a gate (G0, G1, G2, …).
- **Gate** = the exit criteria in `docs/ROADMAP.md` §2/§3. A gate is *reached* only when the
  full workspace is green (`just check`) and the critique's findings are resolved.

A wave's wall-clock is its **longest lane**, not the sum — so lanes are sized to overlap
(e.g. a long surface pipeline runs alongside a quick bench lane).

## 2. The parallelism rules (safe concurrency)

1. **Lanes parallelize *within* one workflow.** Use a single workflow with a wide
   `parallel([...])` fan-out. Do **not** run two top-level workflows against the live
   working tree at once — they would collide on `target/`, `Cargo.lock`, and the critique's
   workspace-wide `just check`.
2. **Pre-register everything centrally, once, before a wave.** All crate path entries and
   external deps go into the root `[workspace.dependencies]` in the wave's setup commit, so
   lane agents never touch the root manifest (the only shared file).
3. **Ship compiling skeletons before the wave.** Every batch crate exists as a valid empty
   crate (doc-comment `lib.rs`) so the workspace always compiles and a sibling lane mid-edit
   can't break another lane's `-p <crate>` build.
4. **No two concurrent lanes may mutate a crate the other compiles.** Verify the dep graph:
   a lane that *reads* crate X must not run while another lane *mutates* X. Frozen interface
   crates (types/core/…) are read-only during waves.
5. **Incremental gates per iteration** (`just check-crate`, `just check-changed`); the full
   `just check` is the wave's integration gate before the milestone commit.

## 3. Verify-as-you-go (no unverified scope)

Every lane agent must reach `just`-green for its crate (fmt + clippy `-D warnings` +
nextest) before reporting done — never weaken a test to pass. The critique milestone then:

- re-runs the **full integration gate** in the live tree (source of truth),
- runs an **adversarial review** (mocks/placeholders, numerical/financial correctness, rule
  violations) that must *actively try to break* the new code (e.g. confirm arbitrage gates
  fire on a constructed arbitrageable smile),
- runs a **competitive gap analysis** vs the incumbents, feeding the backlog.

Findings are fixed before the milestone is committed. The G1 wave already proved the value:
the adversarial reviewer caught a sign-inverted charm in the SDK example; we fixed it and
added a permanent full-Greek finite-difference gate.

The structural hardening gates that wrap this — the cross-platform CI matrix
(`.github/workflows/ci.yml`), mutation testing, coverage tracking, and the
`fuzz/` adversarial-input harness — and their latest measured kill-rate /
coverage numbers are specified in **`docs/HARDENING.md`** (WS-T).

## 4. Dependency vetting is part of the gate (the wasmtime lesson)

External deps are OSS-only **and** must pass `cargo-deny` advisories + licenses before a lane
that needs them runs. When `wasmtime` (plugin-host) pulled open 2026 RustSec advisories, we
**deferred that lane** rather than ship a vulnerable dependency or suppress the advisory.
Rule: vet a heavy/new dependency's `cargo-deny` status in the wave-setup commit; if it fails,
either pin a patched version or defer the lane and record it in the ledger.

## 5. The orchestrator between waves (never idle, never blocking)

While a wave runs, the orchestrator does only **non-conflicting** work: evolve docs/plans
(this file, ROADMAP, COMPETITIVE-ANALYSIS, the ledger), pre-stage the next wave's plan, and
set up long-pole environment dependencies (e.g. installing QuantLib for the golden-oracle
lane) so the next wave launches instantly and wide. It does **not** add dep-heavy skeletons
to the tree mid-wave (that would burden the running critique's `just check`).

## 6. Knowledge stays live

After any structural change, lodestar re-indexes automatically via its filesystem watcher; run
`mcp__lodestar__detect_changes` to verify scope after structural changes; update the `GUIDE.md`
ledger + work-stream table; record durable decisions as ADRs (`mcp__lodestar__manage_adr`) and
auto-memory; keep `docs/` free of stale references (zero-legacy).
