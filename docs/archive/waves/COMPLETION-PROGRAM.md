# COMPLETION-PROGRAM — Celnet to fully-complete, SOTA, polished

> **STATUS — ALL 12 WAVES LANDED (record).** This program is complete: W1–W12 below all
> shipped and are parity-gated (HEAD on `main`; `just check` 1306/1306). Only W9 is marked
> `✅ DONE` inline in the §3 table; the rest landed subsequently and are confirmed built by
> `docs/POST-COMPLETION-AUDIT.md` (Class B "already done"). The document is retained as the
> program record of intent and verification strategy; treat the wave table as executed. For
> the post-completion gap tail (honesty/rigor/onboarding bar-raisers, none P0/P1-functional)
> see `docs/POST-COMPLETION-AUDIT.md`.

> Operator-ready, dependency-ordered program that takes Celnet from "five leadership
> waves + Full-Raft (election/truncation/compaction) complete" to a **fully-complete,
> SOTA, intuitive, polished** state: integrated architecture, full functionality,
> **api-first client parity** (GUI + Excel + SDK + CLI), all in-repo targets met, the
> **honest boundary** respected verbatim.
>
> Drive this wave-by-wave with the proven **`Workflow` (implement → adversarial-verify),
> TWO DISJOINT tracks per wave**. Read `docs/NEXT-WORKFLOWS.md` §2 (the recipe) and §3
> (the three hard lessons) before launching each wave. Keep this doc and the CLAUDE.md
> ledger in lockstep after every commit.

---

## 0. Ground truth (verified this session)

- **Five in-repo waves DONE + Full Raft landed.** `celnet-replog` has `election.rs`
  (Role/RaftConfig/RaftNode, randomized election, conflicting-tail truncation) +
  `compaction.rs` (Snapshot/SnapshotStore atomic CRC'd save + prefix discard); parity
  rows `raft_election.rs` + `raft_compaction.rs` exist. **The ONLY honestly-open Raft
  piece is the `InstallSnapshot` wire RPC** (documented at `election.rs:651-688`;
  `safe_compact_index` is the interim discipline "until the InstallSnapshot RPC lands").
- **33 crates, acyclic, ONE unversioned contract** (`celnet-proto`).
- **The dominant gap is api-first client-parity, not missing math.** A large, *built and
  parity-gated* catalogue is unreachable through any client because
  `celnet.proto` `Instrument.product` oneof stops at vanilla/strategy/single_barrier/
  double_barrier/digital/touch (`celnet.proto:533-549`) and the server pricer matches
  only those six:
  - **Built + parity-gated, NOT on the wire:** Asian (`asian.rs`, TW/Curran),
    forward-start/cliquet (`forward_start.rs`), variance/vol swaps (`var_swap.rs`,
    `vol_swap.rs`), TARF (`tarf.rs`), accumulator (`accumulator.rs`), quanto
    (`quanto.rs`), lookback (`lookback.rs`) — parity in `asian.rs`, `forward_start.rs`,
    `var_vol_swap.rs`, `structured.rs`.
  - **Built but NOT parity-gated vs an independent oracle, and not exposed:** the full
    **LSV** engine (`lsv.rs`/`adi.rs`/`particle.rs`/`stochvol.rs`/`leverage.rs`) — only
    in-crate self-tests; no `celnet-parity/tests/lsv.rs`.
  - **On the wire + server but invisible to EVERY client:** eSSVI
    (`SMILE_MODEL_EXTENDED_SURFACE=4`, `celnet.proto:253`) — GUI/Excel enum codecs stop
    at index 3, Excel `parseSmileModel` throws on it, no SDK vocab entry, no GUI chip.
  - **Server prices barriers/digitals/touches but the GUI ticket builds vanilla/strategy
    only** (`TicketWorkspace.tsx`); Excel has no exotic function.
- **§4(iii) premise correction (already folded in):** the live edge fan-out is **tokio
  `mpsc` per-session + per-series conflation throttle** (`services/stream.rs`), **not** a
  tokio `broadcast`. Wiring `celnet-fanout` is a real per-session-mpsc → shared SPMC-ring
  change; the oracle must be the **actual current** observable sequence.
- **Doc drift:** `ANALYTICS-SPEC.md` / `ROADMAP.md` P3 still describe LSV/TARF/
  accumulator/lookback/quanto/Sobol-QMC as deferred; they are built. `SCALE-OUT.md` is
  current.

---

## 1. Architecture rationale & sequencing

The program is ordered for **maximum compounding value and minimum rework**, in three
arcs:

**Arc I — close the api-first parity gap (P0, highest product value).** Every built,
parity-gated product is made reachable from the one contract → server pricer → SDK → CLI
→ Excel → GUI, **in lockstep**, each proven `API==SDK==GUI==Excel`. We extend the single
unversioned oneof **additively** (guardrail #9: no `schema_version`). This is sequenced
**first** because (a) it is the largest unrealized value already paid for, and (b) the
later risk-cube exotic-leaf (Arc III) and GPU reval lanes build on these products being
wire-citizens. Wave order inside Arc I groups closed-form products first (cheap, exact
oracles) before MC/structured products (price+stderr) before LSV (needs its own
independent oracle row first).

**Arc II — infrastructure & numerics depth (P1).** The `NEXT-WORKFLOWS §4` deepening:
replog `InstallSnapshot` (smallest, no new edge — first in this arc), GPU G3/G6 +
QMC-on-GPU KAT (new `celnet-gpu → celnet-qmc` edge), and wiring `celnet-fanout` under the
edge (**highest blast radius — sequenced last in this arc** so it lands on a stable base
and must not regress the §1.2 hot-core gate). These are **infrastructure with no new
client surface** (the contract is intentionally unchanged; verification is invariance /
ratios) — stated honestly so we do not over-scope a GUI/Excel knob that should not exist.

**Arc III — catalogue/numerics breadth, risk completeness, then polish (P2).** XVA (new
leaf crate; the one item that *would* warrant a future client surface), eSSVI calibration
hardening + Sobol higher-dim (pure numerics rows), exotic risk-cube aggregation
(correctness: roll-ups must stop silently excluding exotics once they are tradable),
journal compaction, and a dedicated **polish wave** (GUI Playwright e2e + axe a11y,
observability surfacing, typed provenance, Excel ergonomics, docs/onboarding + the
doc-sync reconcile).

**New dependency edges introduced (all one-way toward leaves; verify acyclic with
`cargo tree -i <crate> --edges normal`):** `celnet-gpu → celnet-qmc` (W7),
`celnet-server → celnet-fanout` (W9), `celnet-xva → {celnet-vanilla, celnet-types,
celnet-qmc}` (W10). No new edge creates a cycle. The `celnet-risk-cube → celnet-gpu` edge
already exists (`scenario_grid.rs`) — W11 reuses it without a new edge.

**Standing invariants (orchestrator review gate every wave):** acyclicity; ONE contract
(no second version); zero-legacy (delete superseded paths — e.g. the old mpsc distributor
in W9); the pinned **zero-alloc/lock-free hot core** stays log/lock/alloc-free and the
`celnet-bench core_load` §1.2 truth-gate (p50≤2µs/p99≤10µs/p99.9≤25µs) + `fleet_slo`
loopback benches stay green; full `just check` shows the **literal** `All gates passed.`
line (Lesson a).

---

## 2. Verification strategy (every wave)

- **Independent oracle, never circular (Lesson c):** QuantLib `celnet-golden` tables,
  closed-form limits, finite-difference cross-checks, single-node deterministic replay,
  or **hand-pinned published constants**. When an oracle and the code could share a
  mis-stated constant, **also pin that constant directly to a hand-computed published
  value** (the FRTB `0.75ρ` circular-oracle near-miss).
- **Gate (Lesson b):** per-crate `just check-crate <crate>` during a track; **each new
  parity row gates `cargo clippy -p celnet-parity --test <name> -- -D warnings`** (the
  parity TEST target, not only the product crate); the orchestrator runs full `just
  check` and **greps the literal `All gates passed.`** (Lesson a), then `cargo fmt
  --all`, re-derives the math, commits, pushes, verifies `HEAD == origin/main`, updates
  the ledger + resume memory.
- **Client-parity (where the contract changes):** prove `API==SDK==GUI==Excel` end-to-end
  against a live `demo_edge` (mirror the prior SABR-vs-VV Excel e2e and the
  `scale_harness` pattern): the same product priced through every client equals the
  `celnet-exotics`/`celnet-golden` reference. Infrastructure waves verify **invariance**
  (same input ⇒ same observable sequence) — explicitly **no** new client knob.
- **New crate** auto-joins via `members=["crates/*"]` (no root `Cargo.toml` edit);
  parity references it **by path**; the orchestrator pre-adds the parity dev-dep before
  launch so tracks don't race on that file.

---

## 3. Wave table

Legend: **CP** = client-parity surface. Each wave is two disjoint tracks unless marked
SINGLE-TRACK.

| Wave | Title | Tracks (disjoint) | Oracle | Gate | Target | CP / e2e | Boundary |
|------|-------|-------------------|--------|------|--------|----------|----------|
| **W1** | Closed-form exotic catalogue on the wire (var/vol swap, Asian, forward-start/cliquet) — part 1 | A: proto oneof variants + server pricer wiring for **VarianceSwap/VolSwap** → existing `var_swap.rs`/`vol_swap.rs` closed forms; B: proto + pricer for **Asian** (TW/Curran) → `asian.rs` | `celnet-exotics` fair strike / price = wire price to ~1e-9 (closed-form) or within reported stderr (Curran); `celnet-golden` limits | clippy parity test targets; `just check` green | wire==server==`celnet-exotics` price; flat-σ `K_var==σ²` limit | SDK `price()` product builders + CLI subcmds + `CELNET.VARSWAP`/`CELNET.ASIAN` + GUI ticket product selector; e2e `API==SDK==GUI==Excel` | in-repo numerics only |
| **W2** | Closed-form exotic catalogue on the wire — part 2 (forward-start/cliquet) + quanto | A: proto + pricer for **ForwardStart/Cliquet** → `forward_start.rs` (Rubinstein + Σ legs); B: proto + pricer for **Quanto** (vanilla+digital closed form) → `quanto.rs` | closed-form limits (t1→0 → GK vanilla ~1e-9; plain cliquet == Σ legs ~1e-10); quanto vs `celnet-golden` | clippy parity test targets; `just check` green | wire==server==`celnet-exotics` | SDK/CLI/Excel/GUI in lockstep; e2e parity | in-repo numerics only |
| **W3** | MC/structured products on the wire (TARF, accumulator, lookback) | A: proto + pricer for **Tarf** (reuse existing `FixingSchedule` message) + **Accumulator** → `tarf.rs`/`accumulator.rs`; B: proto + pricer for **Lookback** (floating/fixed) → `lookback.rs` | existing parity rows 16-19 (`structured.rs`); MC variants return **price+stderr honestly** | clippy parity test targets; `just check` green | wire price within reported stderr of `celnet-exotics`; structural monotonicity | SDK/CLI/Excel/GUI; MC products surface stderr in every client; e2e parity | in-repo synthetic pricing; live fixings deploy-gated |
| **W4** | GUI ticket + Excel reach for already-contracted barriers/digitals/touches | A: GUI `TicketWorkspace` product selector (vanilla&#124;strategy&#124;barrier&#124;digital&#124;touch) + trigger/barrier inputs, pricing via `transport.price`; B: Excel `CELNET.BARRIER`/`DIGITAL`/`TOUCH` + SDK ergonomic builders (`barrier()`, `one_touch()`) | server price == `celnet-golden` (e.g. EURUSD up-and-out) | GUI vitest + Excel codec tests; `just check` green | GUI/Excel/SDK price == server == golden | api-first close: contracted-but-unbuildable products now buildable everywhere | in-repo |
| **W5** | eSSVI client parity (`SMILE_MODEL_EXTENDED_SURFACE=4`) — fix the hard parity break | A: GUI enum codec index 4 + `SurfaceWorkspace` eSSVI model chip; B: Excel enum codec index 4 + accept `"ESSVI"`/`"EXTENDED"` in `parseSmileModel` (error string generated from enum list) + SDK `surface_vocab.rs` variant | server-fitted eSSVI smile (existing `essvi.rs` parity unchanged) | GUI/Excel unit tests; `just check` green | GUI chip + `CELNET.MARKSURFACE(...,"ESSVI")` + SDK `mark_surface_with(EXTENDED)` all round-trip; model=extended in arb note | e2e eSSVI selection from all three clients vs live edge (mirror SABR-vs-VV e2e) | in-repo |
| **W6** | LSV independent-oracle parity row + booking-model selector on the wire | A: `celnet-parity/tests/lsv.rs` — LSV PDE/MC window-barrier + vanilla reprice vs `celnet-golden`/published Guyon-Henry-Labordère fixtures + pure-LV (ξ=0) Dupire limit pinned to hand values; B: booking-model selector field on the exotic-pricing contract + server route + SDK/CLI/Excel/GUI selector | `celnet-golden`/QuantLib LSV + published fixtures; **pin the Dupire-limit constant to hand values** (Lesson c) | clippy `--test lsv`; `just check` green | LSV reprices vanilla surface to documented band; ξ=0 == Dupire to ~1e-9; ADI≈MC within stderr | booking-model selectable from all clients; e2e | in-repo numerics + published fixtures |
| **W7** | replog `InstallSnapshot` RPC — close the last Raft seam | A: `Message::InstallSnapshot{...}` (append-only tag) in `wire.rs` + leader send-when-behind in `election.rs`; B: follower install+reseed handler + `celnet-parity/tests/raft_snapshot.rs` | single-node deterministic replay (`celnet-journal`) as golden committed state | clippy `--test raft_snapshot`; `just check` green | leader compacted-past-follower brings a lagging/restarted follower to **bit-identical** (`f64::to_bits`) BookState; snapshot+tail == full-log replay; election still converges | INFRASTRUCTURE — no client surface (contract unchanged) | localhost multi-process only; cross-DC/partitions/§11 SLOs deploy-gated |
| **W8** | GPU G3 multi-step path kernel + G6 pathwise/LR Greeks + QMC-on-GPU KAT (new edge `celnet-gpu → celnet-qmc`) | A: `path.wgsl`/`path.rs` (multi-step GBM/Heston under **shared Sobol direction numbers**) + `celnet-parity/tests/gpu_path.rs`; B: `greeks.wgsl`/`pathwise.rs` (pathwise δ/vega + LR for discontinuous payoffs) + `gpu_greeks.rs` | `celnet-golden` + f64 CPU-MC; **central FD Greeks** independent check; reuse `derived_batch_bound`/`as_erf_price_bound` | clippy `--test gpu_path` & `--test gpu_greeks`; `just check` green | three-way GPU-MC≈CPU-MC≈golden within derived f32 bound, node-by-node; **CPU↔GPU KAT** (same Sobol draws ⇒ same path within round-off); Philox bit-stable; `is_gpu()==true` asserted | INFRASTRUCTURE — same prices/Greeks, faster; RATIO harness only | **M4 Metal lacks f64 ⇒ correctness + RATIOS only**; NVIDIA absolute throughput / ≤50ms exotic / Workload-A/B absolutes DEFERRED to CUDA deploy-gate. Never claim f64 on Metal |
| **W9 ✅ DONE** | Wire `celnet-fanout` SPMC ring under the async edge (new edge `celnet-server → celnet-fanout`) | A: replace per-session `mpsc` distributor in `services/stream.rs` with a **per-pair `celnet-fanout` ring shared across that pair's subscribers**, producer off-runtime on the engine thread, consumers drain `try_recv` in `select!` (non-blocking), preserve per-series conflation; **DELETE the superseded mpsc path**; B: `celnet-parity/tests/fanout_edge.rs` — boot real edge, multiple subscribers, drive ticks | the **current** mpsc+conflation observable sequence captured from the live edge (NOT the mis-stated "broadcast") | clippy `--test fanout_edge`; `just check` green; **§1.2 core_load + fleet_slo benches unregressed** | per-session no-loss (in order) + conflation parity (`received+skipped==produced`); hot core stays zero-alloc/lock-free | INFRASTRUCTURE — no client knob (StreamSession contract unchanged); negative/invariance test only | in-process/single-edge; cross-host fan-out tail + §11 SLOs deploy-gated |
| **W10** | `celnet-xva` — CVA/DVA/FVA on synthetic netting sets (new leaf crate) | A: new crate `celnet-xva` (one-way → `{celnet-vanilla, celnet-types, celnet-qmc}`, auto-joins) — exposure profile (EPE/ENE) + CVA aggregation; B: `celnet-parity/tests/xva.rs` vs hand-derived closed-form CVA | closed-form CVA on a single-factor exposure; **hazard/survival/LGD constants pinned to hand-computed published values** (Lesson c) | clippy `--test xva`; `just check-crate celnet-xva`; `cargo machete`/deny clean; `just check` green | CVA == closed form to ~1e-9 in exact limits; monotone ↑ in hazard & LGD; CVA==0 at zero default prob | NO contract change this wave (internal crate). **IF** later surfaced: contract field + GUI XVA panel + `CELNET.XVA` + SDK `xva()` in lockstep | synthetic netting sets; NOT live CSAs/collateral/wrong-way risk |
| **W11** | Exotic risk-cube aggregation + eSSVI/Sobol numerics depth | A: grow `celnet-risk-cube` with an exotic-leaf risk measure (Greeks from `celnet-exotics` analytic/AAD/FD), record exotic positions in `store.rs`, gate **fan-out==single-node** including exotic legs (reuse `celnet-risk-fleet` reconcile); B: `essvi_hardening.rs` (arb-free calibration on wide/sparse quotes; density≥0 + calendar-monotone, zero violations across stress grid) **and** `qmc_highdim.rs` (high-dim Asian/basket convergence vs published variance-reduction, constants pinned) | independent re-derivation of exotic Greeks; Breeden-Litzenberger density + calendar monotonicity; published eSSVI (Hendriks-Martini 2019) + QMC convergence figures | clippy `--test essvi_hardening`/`--test qmc_highdim` (+ risk-cube gate); `just check` green | exotic roll-ups no longer silently exclude exotics; eSSVI always no-arb under stress; Sobol high-dim beats plain MC by stated factor | risk-cube exotic legs visible in RiskService roll-ups (ties W1-W6) | in-repo numerics; Sobol direction numbers already MAX_DIM=300 (narrow to convergence-rate row, do NOT re-add direction numbers) |
| **W12** | Polish wave — GUI e2e + a11y, observability surfacing, typed provenance, journal compaction, docs/onboarding | A: GUI Playwright e2e (boot demo_edge: ticket→price, surface mark→version pin, stream→click-to-trade, risk drill) + `@axe-core/playwright` (zero serious/critical) + observability surfacing (server price p99 + conflation-drop count + surface_version/correlation provenance in StatusRibbon) + **typed provenance field** on `Smile` replacing the fragile arb-note regex; B: `celnet-journal` checkpoint+compaction (snapshot watermark, fresh post-watermark log, fsync + atomic rename(2)) + **docs reconcile** (`ANALYTICS-SPEC.md`/`ROADMAP.md` P3 → Built citing crate+row; new `docs/CLIENT-PARITY-MATRIX.md`) | live `demo_edge` numbers == server; a11y rules; replay-from-compacted == replay-from-full (bit-identical) | Playwright+axe CI lane green; clippy parity (journal compaction gate); `just check` green; docs-anchor lint | zero serious a11y violations; server latency/drops shown; crash mid-compaction leaves complete old OR complete new log | observability + typed provenance surfaced identically in GUI/Excel/SDK; capability matrix in lockstep | in-repo loopback metrics; §11 ABSOLUTE wire SLOs never claimed |

---

## 4. Honest boundary (verbatim — NEVER built or claimed in-repo)

The following are **designed + seamed + ADR'd** only, and proven exclusively at
deploy/live-staging. The program keeps them OUT of in-repo build scope:

- **Cross-host wire p99 / kernel-bypass NIC latency.**
- **CUDA/NVIDIA ABSOLUTE throughput + ≤50ms exotic + Workload-A/B absolute numbers.**
  M4 Metal lacks f64 ⇒ in-repo proves **correctness + RATIOS only** (M4/Lavapipe). Never
  claim f64 on Metal.
- **The §11 ABSOLUTE wire-latency SLOs** (in-repo proves the in-core §1.2 truth-gate +
  loopback benches only).
- **The entire live JVM Celer estate lifecycle** (sidecar / FX_OPTION / inferred hops /
  tenant overlays).

Plus the in-wave deferrals: live CSAs/collateral/wrong-way risk (W10 is synthetic netting
sets only); live vendor quote VALUES (W11 eSSVI is synthetic/published only); live
fixings (W3 MC products are synthetic); cross-DC transport / real partitions (W7/W9
localhost multi-process only).

---

## 5. Execution order summary

**Arc I (api-first parity, P0):** W1 → W2 → W3 → W4 → W5 → W6.
**Arc II (infra/numerics depth, P1):** W7 (first — smallest, no new edge) → W8 (new GPU
edge) → W9 (highest blast radius — last). W8 can run in parallel with W5/W6 (disjoint
crates). W7 can run in parallel with any Arc-I wave.
**Arc III (breadth + risk + polish, P2):** W10 (independent leaf, any free lane) → W11 →
W12 (last).

Each wave: two disjoint tracks, implement→adversarial-verify, independent oracle, the
mandatory parity-TEST-target clippy, the literal `All gates passed.` grep, push, ledger +
memory update. SOTA, zero workarounds.
