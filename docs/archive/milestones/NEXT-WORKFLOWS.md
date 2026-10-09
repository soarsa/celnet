# NEXT-WORKFLOWS — operator runbook for a fresh (post-/clear) session

> Read this, then launch the next two-disjoint-track Workflow from §4. Append-only intent;
> keep in sync with the GUIDE.md ledger after every commit.

> **STATUS-RECONCILED (post-2026-06-07): the §4 backlog below is now BUILT — this section is a
> historical snapshot.** After the §1 snapshot, the full 12-wave `docs/COMPLETION-PROGRAM.md`
> and the 13-item `docs/POST-COMPLETION-AUDIT.md` landed (HEAD advanced well past `9d26c0d`;
> `just check` 1306/1306). Every §4 deepening spec has since shipped & is parity-gated: **(i)**
> full Raft (election/Pre-Vote/truncation/compaction/InstallSnapshot — `celnet-replog`); **(ii)**
> GPU G3 path kernel + G6 pathwise/LR Greeks + QMC-on-GPU KAT (`celnet-gpu`); **(iii)**
> `celnet-fanout` SPMC ring wired under the edge (`celnet-server/services/pricefanout.rs`);
> **(iv)** eSSVI hardening, `celnet-xva` (CVA/DVA/FVA on synthetic netting), Sobol higher-dim
> (`qmc_highdim.rs`). The **§2 recipe and §3 lessons remain current and correct** — use them for
> any new workflow. Treat §1/§4 as the record of what *was* next, not what *is* next; for live
> status see the `GUIDE.md` ledger, `docs/POST-COMPLETION-AUDIT.md`, and
> `docs/CAPABILITIES-REVITALISATION-PLAN.md`.

## 1. STATUS

The Celnet leadership program is **materially COMPLETE**. All FIVE in-repo waves (1–5) are
done & pushed. **HEAD = `9d26c0d` on `main` (== origin/main); 1044/1044 tests; full `just
check` green ("All gates passed.").** Wave 6 is **deploy/live-estate-only and is NEVER
built or claimed in-repo** — the honest boundary: cross-host wire p99 / kernel-bypass NIC,
CUDA/NVIDIA absolute throughput + ≤50ms exotic, the §11 absolute wire-latency SLOs, and the
entire live JVM CelNet estate (sidecar/FX_OPTION/inferred hops/tenant overlays) are
designed+seamed+ADR'd here and proven only at deploy/live-staging. Everything below is
**deepening increments only** (not new waves) — launch only if the user asks.

## 2. THE RECIPE (worked 6× — copy precisely)

Launch a **`Workflow` with two phases: implement → adversarial-verify**, running **TWO
DISJOINT crates per run** (one track each, no shared file).

- **Each new parity/correctness test is a NEW `crates/celnet-parity/tests/<name>.rs`** —
  auto-discovered by cargo, so tracks make **zero shared-file edits** (no race on a mod list
  or a lib.rs). Existing rows: `essvi.rs`, `var_vol_swap.rs`, `asian.rs`, `heston.rs`,
  `forward_start.rs`, `qmc.rs`, `frtb.rs`, `pair_universe.rs`, `exotics.rs`, `surface.rs`, …
- **A NEW product crate auto-joins the workspace via `members = ["crates/*"]`** — **NO root
  `Cargo.toml` edit.** `celnet-parity` references a leaf crate **BY PATH** (exactly like
  `celnet-golden = { path = "../celnet-golden" }`, and as already done for `celnet-heston`,
  `celnet-qmc`). **EITHER** parity references the new crate by path inside the track, **OR**
  the **orchestrator pre-adds the dev-dep** to `crates/celnet-parity/Cargo.toml` before
  launch so the two tracks don't race on that one file.
- **Each track's gate MUST include**, verbatim:
  `cargo clippy -p celnet-parity --test <name> -- -D warnings`
  (clippy the **parity TEST target**, not only the product crate — see Lesson b).
- **The ORCHESTRATOR (you), after the tracks return, ALWAYS:**
  1. runs the full `just check`;
  2. **greps for the literal `All gates passed.` line** — NEVER trusts a background-wrapper
     exit code or a per-track self-report;
  3. runs `cargo fmt --all` (agents' final state is sometimes not fmt-clean);
  4. **INDEPENDENTLY RE-DERIVES the math vs the published source** — not just re-runs the
     gate (see Lesson c);
  5. commits, `git push origin main`, **verifies local==remote** (`git rev-parse HEAD` ==
     `git rev-parse origin/main`);
  6. updates the GUIDE.md ledger (top entry) + the resume memory
     (`session-state-2026-05-31.md`); optionally `/wiki-ingest`.

Build discipline: `source "$HOME/.cargo/env" && …` (or the justfile). Per-crate
`just check-crate <crate>` during a track; full `just check` before the milestone commit.

## 3. THE THREE HARD LESSONS (each cost a near-miss — verbatim)

- **(a) Always verify the literal `All gates passed.` line yourself.** `375b808` was pushed
  on a false background-wrapper "exit 0" while the full gate had FAILED fail-fast. Always
  `grep -q "All gates passed"` on the real `just check` output before committing on green.
- **(b) The per-track gate must clippy the parity TEST target.** W4b/W4c each shipped a
  parity test that compiled but FAILED workspace clippy because the gate only ran clippy on
  the product crate. The gate must include
  `cargo clippy -p celnet-parity --test <newtest> -- -D warnings`.
- **(c) Independently re-derive constants vs the published source.** A longhand "independent"
  oracle can re-derive the **same wrong constant**: W4d's FRTB LOW-correlation scenario was
  missing the `0.75ρ` floor, and it PASSED its own gate because the oracle re-derived the
  identical wrong constant (circular). When an oracle and the code can share a mis-stated
  constant, **ALSO pin the constant directly to hand-computed published values.**

## 4. BACKLOG — ready-to-launch deepening workflow SPECS (prioritized)

Each is a two-disjoint-track Workflow following §2. Format: tracks · oracle · honest boundary.

### (i) `celnet-replog` — FULL RAFT  [highest value: closes the distributed-correctness depth]

Today `celnet-replog` is leader-replicated bit-identical replay (thin, pre-Raft). Deepen to
full consensus.

- **Track A — leader election + conflicting-tail truncation.** In `celnet-replog`:
  randomized-timeout election, term/voting, AppendEntries with the log-matching property;
  on a conflicting entry at the follower, **truncate the divergent tail** and adopt the
  leader's. New parity row `crates/celnet-parity/tests/raft_election.rs`.
- **Track B — log compaction / snapshotting.** In `celnet-replog` (disjoint module/file from
  A — `compaction.rs` vs `election.rs`): snapshot install, log prefix discard, replay from
  snapshot+tail == replay from full log (bit-identical state).
- **Gate:** randomized election + induced partitions (mirror `risk_federation.rs` /
  `forwarding.rs` multi-process boot) **still reach bit-identical committed state** across
  all surviving nodes; the `f64` oracle reconciles the replayed `BookState`/risk to the
  single-node aggregate; `clippy -p celnet-parity --test raft_election -- -D warnings`.
- **Oracle:** single-node deterministic replay (existing `celnet-journal` replay) as the
  golden committed-state; Raft cluster must converge to byte-identical state.
- **Honest boundary:** correctness + election + truncation + compaction proven **in-process /
  localhost multi-process**; cross-DC transport, real network partitions, and the §11 wire
  SLOs stay deploy-gated.

### (ii) GPU G3 multi-step path kernel + G6 pathwise/LR Greeks + QMC-on-GPU KAT

Deepens Wave-5 (`celnet-gpu` has the closed-form batch kernel + f32↔f64 reconcile; W4c
`celnet-qmc` Sobol+Brownian-bridge exists and is GPU-shareable).

- **Track A — G3 multi-step path kernel + QMC-on-GPU KAT.** New `path.wgsl`/`path.rs` in
  `celnet-gpu`: multi-step GBM/Heston path under **shared Sobol direction numbers** from
  `celnet-qmc` so CPU and GPU consume identical draws. Parity row
  `crates/celnet-parity/tests/gpu_path.rs`.
- **Track B — G6 GPU pathwise / likelihood-ratio Greeks.** Disjoint file `greeks.wgsl`/
  `pathwise.rs` in `celnet-gpu`: pathwise delta/vega + LR for discontinuous payoffs. Parity
  row `crates/celnet-parity/tests/gpu_greeks.rs`.
- **Gate:** **three-way GPU-MC ≈ CPU-MC ≈ golden** within the derived f32 bound;
  **f32↔f64 reconcile node-by-node**; bit-stable Philox; CPU↔GPU **KAT** (same Sobol draws ⇒
  same path within round-off); `clippy -p celnet-parity --test gpu_path -- -D warnings` and
  `--test gpu_greeks`.
- **Oracle:** `celnet-golden` (QuantLib tables) + the f64 CPU-MC; finite-difference Greeks
  as an independent check on pathwise/LR.
- **Honest boundary:** M4 Metal lacks f64 — prove **correctness + RATIOS** on M4/Lavapipe
  only; the **NVIDIA absolute throughput headline + ≤50ms exotic + Workload-A/B absolutes
  stay deferred** to the CUDA deploy-gate CI. Never claim f64 on Metal.

### (iii) Wire `celnet-fanout` UNDER the async server edge

`celnet-fanout` (SPMC ring, built in Wave 3) exists but the live `celnet-server` stream path
still uses a bounded tokio broadcast.

- **Track A — edge integration.** In `celnet-server` (`services/stream.rs` path): replace the
  bounded tokio `broadcast` with the `celnet-fanout` SPMC ring as the per-session price
  distributor; keep the conflation semantics.
- **Track B — parity harness.** New `crates/celnet-parity/tests/fanout_edge.rs` (disjoint
  file): boot a real edge, multiple subscribers, drive ticks.
- **Gate:** **per-session no-loss** (every committed publish observed in order) + **conflation
  parity** (latest-value collapse matches the broadcast baseline) **through the real edge**;
  zero-alloc on the hot path unchanged; `clippy -p celnet-parity --test fanout_edge -- -D
  warnings`.
- **Oracle:** the existing tokio-broadcast path as the reference sequence (same input tick
  stream ⇒ same conflated observable sequence per subscriber).
- **Honest boundary:** proven **in-process / single-edge**; cross-host fan-out tail and the
  §11 wire SLOs stay deploy-gated.

### (iv) Catalogue / numerics depth (pick per user ask)

- **eSSVI surface calibration hardening** — Track A: arb-free calibration robustness (wide
  smiles, sparse quotes) in `celnet-surface`; Track B: parity `tests/essvi_hardening.rs`.
  **Oracle:** no-arb constraints (positive density, calendar monotonicity) checked directly
  + published eSSVI fixtures. **Boundary:** synthetic/published quotes only.
- **XVA on synthetic netting sets** — Track A: new crate `celnet-xva` (one-way dep →
  {`celnet-vanilla`,`celnet-types`}, auto-joins via `members=["crates/*"]`); Track B: parity
  `tests/xva.rs` vs an analytic CVA benchmark. **Oracle:** closed-form CVA on a single-factor
  exposure profile (hand-derived). **Boundary:** synthetic netting sets, not live CSAs.
- **Sobol higher-dim** — Track A: extend `celnet-qmc` direction numbers to higher dimensions
  + scrambling; Track B: parity `tests/qmc_highdim.rs`. **Oracle:** known integral (Asian/
  basket) convergence rate vs published variance-reduction figures; pin constants to hand
  values per Lesson c. **Boundary:** in-repo numerics only.

## 5. DO NOT

- Do **not** claim cross-host wire p99 / kernel-bypass NIC latency from this repo.
- Do **not** claim NVIDIA absolute GPU throughput / ≤50ms exotic / Workload-A/B absolutes
  from this repo (M4 Metal lacks f64 — ratios + correctness only).
- Do **not** claim the live JVM CelNet estate lifecycle from this repo (designed+seamed only).
- Do **not** add a second wire-contract version (guardrail #9: one clean current contract).
- Do **not** fake a cluster / shared-memory transport (use real localhost multi-process).
- Do **not** lower a gate, add `#[ignore]`, or `#[allow]`/`as any`/`@ts-ignore`-dodge clippy
  to make a track pass. SOTA, zero workarounds — the verify phase greps the diff for these.
