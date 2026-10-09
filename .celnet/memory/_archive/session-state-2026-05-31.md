---
name: session-state-2026-05-31
description: "RESUME HANDOVER — Celnet MASTER-EVOLUTION program (exceed SynOption, multi-asset, no-legacy): W0 foundation COMPLETE & pushed (HEAD e3adc3d); next = W1 multi-asset CORE. Single backlog = docs/WORLD-CLASS-BACKLOG.md; plan = docs/MASTER-EVOLUTION-PROGRAM.md + GUI-EXPERIENCE-DESIGN.md. Carries the SOTA/no-workarounds mandate + the 3 hard gate-verification lessons."
metadata:
  node_type: memory
  type: project
  originSessionId: 3ce7b2e7-2dd4-4f51-a992-c905de0e2df4
---

## ⟳ RESUME AFTER CLEAR (read this first)

### ▶▶▶ CURRENT PROGRAM (2026-06-08) — MASTER-EVOLUTION: exceed SynOption, multi-asset, no-legacy
The leadership/completion/post-audit programs below are all DONE. We are now executing a NEW, larger
program to evolve Celnet from FX-options into a **cross-asset derivatives platform** that exceeds
SynOption's asset-class coverage — fully integrated, no legacy, SOTA, with redesigned APIs/Excel/GUI/SDK/
CLI, fully verified end-to-end, with a convergence loop to 2 dry rounds. User mandate: **"do not stop
until absolutely everything is fully implemented and fully verified end-to-end"**, push at milestones.
- **THE PLAN:** `docs/MASTER-EVOLUTION-PROGRAM.md` (8 waves W0→W7 + 7-lens convergence loop §6).
- **THE SINGLE LIVE BACKLOG (status truth):** `docs/WORLD-CLASS-BACKLOG.md` (OPEN/IN-PROGRESS/DONE per item
  + convergence ledger). Keep statuses current as work lands.
- **GUI sub-program:** `docs/GUI-EXPERIENCE-DESIGN.md` (GW0→GW7).
- **▶ STATUS (2026-06-08): W0 + W1 COMPLETE & pushed (HEAD `73bd899`).** **W1 multi-asset CORE LANDED**
  (`ba0fc03` core + `6143409` GUI merge): the three FX-only Layer-0 seams generalized in place to
  `Underlying`/`Carry`/`CarryModel`/`RateSensitivities` (ADR-0008 carry-producing-market architecture), FX
  byte-identical (`just check` "All gates passed.", 1343 tests; equity-dividend plugin vs independent GBS
  oracle 1e-12; server price-path has the no-silent-fallback carry guard). GW0/GW1 GUI foundation merged
  (accessible DataGrid + breadcrumb scope nav; 7 redundant pair affordances deleted). **ALL fan-out lanes
  now OPEN** in `docs/PARALLEL-SESSIONS.md` (the service-mesh board for parallel sessions): W2 linear/breadth,
  W3 crypto, W4 structured/RFQ, W5 cross-asset risk + equity/commodity, GW2+. Per-wave plans on main
  (`docs/W{2,3,4,5}-*-PLAN.md`, `GW-FOUNDATION-PLAN.md`, `DOWNSTREAM-EXECUTION-MAP.md`). Recommended first
  crate-disjoint parallel set: **W2-A-LINEAR ∥ W3-CRYPTO ∥ W5-B-LEAVES** (3 serialization points: one open
  proto edit at a time; 5-client surfacing serializes per wave). Also done: the **capabilities document**
  (`docs/celnet-capabilities.html`) rebuilt comprehensive + multi-asset + SOTA + audience-targeted
  (Traders/Quants/Tech/Leadership), 30 visuals embedded (`just embed-capabilities` → self-contained
  standalone), 0 hedge language — awaiting operator review then PDF (`deliverable/capabilities-pdf`).
  Follow-up (not W1): `celnet-surface` FX→neutral split (only when a non-FX surface leaf lands).
  **The ledger is now `docs/IMPLEMENTATION-LEDGER.md`** (GUIDE.md keeps only the newest entry inline).
- **(archived) W0 (verification & hygiene foundation) COMPLETE & pushed** — HEAD `e3adc3d` (full `just
  check` "All gates passed."; celnet-parity 127/127; GUI vitest 335/335; Excel real-edge e2e 81/81).
  W0 delivered: central workspace-dep registry + `workspace-deps` lint (`7c40aaf`); a golden-vector corpus
  (84 vectors / all 18 product families, independent oracles, NO circular oracle) + 5-client conformance
  (SDK/CLI/GUI in-process + a NEW Excel REAL-EDGE e2e suite `excel/e2e/` that boots demo_edge over a real
  `ws` socket — closes the FakeSocket gap); `docs/VERIFICATION-CONTRACT.md`; a `verification-coverage` lint
  (proto arms ⇄ golden vector ⇄ celnet-parity row, now 18/18, wired into `just check`); two NEW parity rows
  `celnet-parity/tests/{american,strategy}.rs`. Corpus build surfaced+fixed 2 real defects (one-touch
  at-hit; lookback bridge extremum). The arm⇄parity⇄vector lint now blocks any new product arm without
  full verification — USE IT as the W1+ contract.
- **W1 IN PROGRESS — plan `docs/W1-CORE-PLAN.md` (6 staged green increments S1–S6, FX byte-identical).**
  - **S1 DONE & pushed (`3572b9f`):** additive multi-asset vocabulary in `celnet-types` — `Underlying`
    (Fx arm), `Carry` (`FxRates`{r_dom,r_for} exact + `CostOfCarry`{r,b} generalized; FX `forward_factor`/
    `discount_df` proven `to_bits`-identical to `VanillaInputs::forward`/`df_dom`), `RateSensitivities`
    (Fx{rho_dom,rho_for} + Carry{discount_rho,carry_rho}). Purely additive ⇒ full `just check` green.
  - **▶ NEXT: S2** — migrate `celnet-vanilla` (GK leaf) onto `Carry`, arithmetic UNTOUCHED, FX golden CSV +
    `celnet-parity/greeks.rs` byte-identical. Then S3 (exotics/surface/engine/risk consumers), S4 (proto +
    convert, FX round-trip to_bits + W0 conformance corpus green), S5 (plugin-api/WIT/wasmi ABI + a NEW
    equity-dividend `CostOfCarry` model gate vs QuantLib), S6 (delete legacy r_dom/r_for/rho_* per #10 +
    reconcile docs + re-index). NB: the VanillaInputs field replacement (S2/S6) is the atomic 80-crate
    blast — do it as a focused effort; consider an additive `.carry()` accessor first to migrate readers
    incrementally before flipping the internal representation. The GOVERNING PRINCIPLE: never touch the GK
    arithmetic — generalize the TYPE system so FX byte-identity holds by construction.
- **(ARCHIVED design note) W1 — Multi-asset CORE wave (P0, single COORDINATED wave, highest risk).** Generalize the three
  FX-only Layer-0 seams IN PLACE (one unversioned contract, FX byte-identical, delete legacy):
  (1) `celnet-types` `VanillaInputs`/`Greeks` → `Underlying` enum + `Carry` (generalized-BSM `(r,b)`, FX =
  `b=r_dom−r_for`) + carry-tagged `Sensitivities`; (2) a thin `celnet-pricing-core` trait layer; (3)
  `celnet-proto` `Instrument`/`MarketContext`/`CcyPair` → `Underlying`+`CarryModel`+`RateSensitivities` +
  product×underlying validity matrix; (4) `celnet-plugin-api::PricingModel` generalized + WIT/wasmi ABI.
  Gate = FX golden/parity/conformance/4-plugin-gates ALL byte-identical (the no-regression gate) + re-index
  graph + reconcile INTERFACES/ARCHITECTURE. Per §2 sequencing this is ONE wave (NOT disjoint lanes) since
  all four touch the frozen seams; THEN W2+ leaves fan out to disjoint lanes. See MASTER-EVOLUTION §2/§3.

### 📍 HANDOVER ARTIFACTS (prepared 2026-06-06 for a clean post-/clear resume)
- **Repo ledger (status source of truth):** `/Users/adrian/code/celnet/GUIDE.md` — top entry =
  "Wave 5 DONE → ALL IN-REPO WAVES (1–5) COMPLETE". Auto-loaded each session.
- **Wiki (narrative knowledge layer):** `~/wiki/celnet/index.md` (graph project
  `Users-adrian-code-celnet`; registered in `~/wiki/index.md` root registry under Standalone
  products). Loaded by the wiki-session-start hook when cwd is this repo. Per-cluster entity/concept
  pages under `~/wiki/celnet/{entities,concepts}/`. Keep it current via `/wiki-ingest` after commits.
- **▶ READY-TO-LAUNCH NEXT WORK:** `/Users/adrian/code/celnet/docs/NEXT-WORKFLOWS.md` — the operator
  runbook with the proven workflow RECIPE, the 3 hard lessons, and ready-to-launch dynamic-workflow SPECS
  for the deepening increments (full Raft in replog; GPU G3/G6 + QMC-on-GPU KAT; fanout-under-edge; etc.).
  **If asked to continue, read that doc and launch the next two-disjoint-track Workflow from it.**
- **Program status:** materially COMPLETE against the leadership bar. Wave 6 is deploy/live-estate-only
  (never built/claimed here). Further work = deepening increments only, and only if the user asks.
- **▶▶ POST-COMPLETION-AUDIT BACKLOG COMPLETE (2026-06-07, HEAD `970fb27`; `just check` 1306/1306).**
  After the 12-wave program, an honest fan-out gap-audit (`docs/POST-COMPLETION-AUDIT.md`) found the
  platform MATERIALLY COMPLETE (no P0/P1 functional gaps) + a 13-item bar-raising tail — all done in 3
  staged rounds (disjoint worktree lanes; American/basket sequential since they share proto/exotics/
  clients). Added: American/Bermudan early-exercise (PSOR FD + LSM, american=24), correlated multi-asset
  basket/best-of/worst-of (Cholesky GBM, basket=25) — both across all 5 clients; mutation-gate widened
  (found+killed 33 real survivors); decoder fuzz + proptests; Heston published golden; surface coverage
  gate; SDK examples; GUI a11y-widen + shortcuts overlay; full doc reconciliation (crate count, Built
  citations, INTERFACES registry). **Bonus correctness fix:** celnet-fanout SPMC seqlock reader needed the
  canonical Acquire fence (rare aarch64 torn-read window the stress test surfaced under 16x contention) —
  fixed at root. **LESSON (recurring this session): the orchestrator's full `just check` re-gate is the
  real gate** — it caught downstream-crate breaks from worktree-only-gated waves (FactMeasure fields),
  merge-marker mistakes, contention flakes (fanout throughput floor + conflation; SDK smoke deadlines), a
  freshly-published dev-only RUSTSEC advisory, and cargo-mutants worktree-leak merge artifacts; NEVER trust
  a per-track self-report or a workflow verdict alone. **Only deploy/live-gated frontier remains (NEVER
  in-repo): NVIDIA absolutes, cross-host wire/§11 SLOs, live JVM estate, Raft §6 membership, plugin
  Tier-1/3.** The in-repo platform is complete, SOTA, api-first, polished.

- **▶▶ COMPLETION-PROGRAM COMPLETE (2026-06-07, HEAD `1983956`; `just check` 1231/1231).** `docs/
  COMPLETION-PROGRAM.md` (12 waves, 3 arcs) is fully DONE + pushed. Planned via a fan-out→architect
  Workflow; executed wave-by-wave as gated implement→adversarial-verify Workflows, with the disjoint
  Arc-II/III waves run on PARALLEL git-worktree lanes (`isolation:'worktree'` → commit-to-branch → I merge +
  re-gate; Arc-I forced-sequential since all waves share the ONE contract). **What landed:** Arc I (W1–W6) =
  the full exotic catalogue on the ONE wire contract reachable from ALL 5 clients (server/SDK/CLI/Excel/GUI):
  var/vol swap, Asian, fwd-start/cliquet, quanto, TARF, accumulator, lookback, barriers/digitals/touches,
  eSSVI client parity, LSV parity row + booking-model selector. Arc II = W7 Raft InstallSnapshot, W8 GPU
  G3/G6 + QMC-on-GPU KAT, W9 celnet-fanout under the async edge (Copy PriceTick ring, §1.2 unregressed).
  Arc III = W10 celnet-xva (CVA/DVA/FVA), W11 exotic risk-cube roll-up + eSSVI-hardening/Sobol-highdim
  numerics, W12 capstone (typed Smile provenance, server observability in StatusRibbon, GUI Playwright e2e +
  axe a11y booting real demo_edge, journal compaction, docs + CLIENT-PARITY-MATRIX). **Hard lessons that
  recurred & held:** verify the literal "All gates passed." line myself (W6 was rejected on an fmt blocker);
  MC honesty on EVERY transport not one (W2 rejected on a WS-quote stderr gap); independent non-circular
  oracle + hand-pinned constants (Lesson c); and a NEW one — **worktree waves that only run per-crate gates
  can break DOWNSTREAM crates they didn't gate (W11-A broke celnet-limits/-entitlements fixtures via a
  shared-type field add); the orchestrator's full-`just check` at merge is what catches it.** Also: never
  `git add -A` a conflicted file (a w8/w10 merge marker slipped into parity Cargo.toml; caught + fixed).
  **Remaining in-repo frontier:** only Raft §6 dynamic membership (documented next increment). Honest
  boundary unchanged (cross-host wire / NVIDIA absolutes / live JVM estate stay deploy/live-gated).

- **▶ §4(i) FULL-RAFT DEEPENING — COMPLETE & pushed (2026-06-06, HEAD `aabdb8f`).** Ran as TWO SEQUENTIAL
  implement→adversarial-verify workflows (the two tracks are NOT file-disjoint — both live in the single
  `celnet-replog` crate, so per-crate gates can't run concurrently; and snapshotting depends on the
  finalized Raft log). **Track A (`e5f6738`):** thin log → full Raft (election + Pre-Vote + log-matching +
  durable atomic conflicting-tail truncation + §5.4.2 commit), zero-legacy (leader/follower/standby
  deleted), parity row `raft_election.rs`. **Track B (`aabdb8f`):** Raft §7 log compaction/snapshotting
  (durable CRC'd atomic snapshot + base-index offset + on-disk prefix discard + snapshot-aware recovery),
  parity row `raft_compaction.rs` + live-cluster gate_e; the independent oracle caught 2 real bugs (recovery
  double-apply; solo-cluster never self-electing), both fixed forward. **1077/1077 tests**, full `just
  check` green, cargo-deny clean. **Remaining Raft depth (documented as next increments, NOT half-built):**
  InstallSnapshot wire RPC (far-behind follower catch-up over the wire; bridged by `safe_compact_index`) +
  dynamic membership change (§6). **Next backlog (deepening only, launch if the user asks):** §4(ii) GPU
  G3/G6 + QMC-on-GPU KAT, §4(iii) wire `celnet-fanout` under the async server edge — see
  `docs/NEXT-WORKFLOWS.md`.

We are executing the **Celnet leadership program** (`docs/LEADERSHIP-PROGRAM.md`): a 6-wave,
dependency-ordered, gate-defined plan to make Celnet the world's most performant / fully-integrated /
intuitive / clean / SOTA FX-options product. Executed as **gated implement→adversarial-verify dynamic
workflows**, one wave at a time; commit + **push to `origin` (github.com/soarsa/celnet)** after each.

- **HEAD = Wave 5 ledger commit** (clean, synced to origin/main). Trail tail: `416951c` **W4d** ·
  `d3945af` W4d-ledger · `1a57def` **W3** (replog + fanout) · `e9436cd` W3-ledger · `b743fea` **W5** (GPU
  batch kernel + perf harness) · then the W5 ledger/memory commit. GUIDE.md ledger (top entry) is the
  canonical wave status.
- **▶▶ ALL FIVE IN-REPO LEADERSHIP WAVES (1–5) ARE COMPLETE & PUSHED. The program is materially done.**
  W1 truth-gates+integration · W2 fleet-SLO+experience · W3 distributed-correctness (replog+fanout) ·
  W4 functionality catalogue (4a–4d: eSSVI, swaps, Asian, Heston, fwd-start/cliquet, Sobol QMC, FRTB-SA,
  pairs) · **W5** (`b743fea`) GPU ratios: `celnet-gpu` batch closed-form kernel (f32↔f64↔golden reconcile,
  max round-off 3.11e-7, real Metal) + `celnet-bench` gpu_load/gpu_gate harness (RATIOS only, M4 curve
  0.68×→53×). **1044/1044 tests** (started the session at 898). Every wave a gated parity/correctness row
  vs an independent oracle, committed+pushed with local==remote.
- **ONLY Wave 6 REMAINS — and it is NEVER built/claimed in-repo by design** (the honest boundary):
  cross-host wire p99 / kernel-bypass NIC, CUDA deploy-gate absolute throughput / NVIDIA headline / ≤50ms
  exotic, the live JVM CelNet estate lifecycle (sidecar/FX_OPTION/4 inferred hops/tenant overlays), and the
  §11 absolute wire-latency SLOs. In-repo these are closed by the built seams + ADRs + the docs-anchor
  lint; they are proven only at deploy/live-staging. There is no further in-repo workflow to launch for
  the program. If the user wants more: deepen any existing wave (e.g. W3 full Raft election + conflicting-
  tail truncation; W5 G3 multi-step path kernel / G6 GPU pathwise Greeks / QMC-on-GPU KAT; wire celnet-
  fanout under the async edge), or harden/polish — but the program bar is met. W1/W2: in-core
  §1.2 ABSOLUTE latency gate + iai; live FIX acceptor + `CELNET_DEPLOY` edge; fleet §11 SLO benches; CLI
  parity; concurrent fan-out; GUI vitest. **W4a** `0c54958` eSSVI + var/vol swaps · **W4b** `91d8e2c`
  analytic Asian (TW+Curran) + Heston crate `celnet-heston` · **W4c** `428a424` forward-start/cliquet +
  QMC crate `celnet-qmc` (Sobol+bridge, ~37.7×/~88.5× VR, reused by Wave-5 GPU) · **W4d** `416951c`
  FRTB-SA (SbM+RRAO+honest DRC) + pair-universe (19 pairs, internal FixingSource, **wire UNCHANGED**).
  Each catalogue product = a gated `celnet-parity` row vs an INDEPENDENT oracle. Full `just check` green,
  **1009/1009 tests** (started the session at 898).
- **HARD LESSONS (all three cost a near-miss; now baked into the recipe):**
  1. **Always run the real `just check` myself + grep for the literal "All gates passed" line** — never
     trust per-track self-reports or a wrapper exit code.
  2. **Per-track gate spec MUST include `clippy -p celnet-parity --test <newtest> -- -D warnings`** (not
     just clippy on the product crate) — W4b/W4c both shipped a parity test that failed workspace clippy.
  3. **Independently re-derive the MATH against the published source, not just re-run the gate** — W4d had
     a real FRTB bug (LOW corr scenario missing the `0.75ρ` floor) that PASSED its gate because the
     longhand "independent" oracle re-derived the SAME wrong constant (circular). When an oracle and the
     code can share a mis-stated constant, ALSO pin the constant directly to hand-computed published
     values. The workflow verifier rubber-stamped it; my own review caught it.

### THE MANDATE (non-negotiable, from the user)
- **State-of-the-art, ZERO workarounds.** No lowered gates/budgets, no `#[ignore]`, no `#[allow]` to
  dodge clippy, no `as any`/`@ts-ignore`/eslint-disable, no skipped-and-pretended tests, no mocked-as-real,
  no overclaim. Each workflow's **verify phase must grep the diff for these** and reject if any found.
- **Honest boundary (never claim from this repo):** cross-host wire p99, kernel-bypass NIC, NVIDIA GPU
  throughput / ≤50ms exotic, the §11 absolute wire-latency SLOs, and the **entire live JVM CelNet estate**
  (sidecar/FX_OPTION/4 inferred hops/tenant overlays) — designed+seamed here, proven only at deploy.
  Loopback/M4 numbers are labelled as such; the program doc reproduces the full list verbatim.
- **⚠️ HARD-WON RULE: after `just check`, verify the literal "All gates passed" line — NEVER trust the
  background-task wrapper "exit 0".** I pushed `375b808` on a false "exit 0" while the full gate had
  FAILED fail-fast (a latency budget UNIT test measured under parallel-nextest contention — methodology
  bug). Always `grep -q "All gates passed"` before committing on green.
- Build discipline: `source "$HOME/.cargo/env" && ...` (or justfile); per-crate `just check-crate <crate>`
  during a wave, full `just check` before the milestone commit; re-gate + diff-review INDEPENDENTLY
  before each commit; re-run any perf bin (`core_load`, `surface_rebuild`, `bench_gate`) yourself.

### ✅ PROGRAM COMPLETE — Waves 1–5 all done & pushed (HEAD = W5 ledger commit). No further in-repo wave.
The five in-repo leadership waves are finished (see the top ledger entry). Wave 6 is deploy/live-estate-
only and is never built or claimed in this repo (honest boundary). If resuming: there is no queued
workflow — either report the program complete, or pick a DEEPENING increment (W3 full Raft; W5 G3/G6/QMC-
on-GPU; wire celnet-fanout under the async edge) only if the user asks. The recipe + hard lessons below
still apply to any future wave.

### (ARCHIVED) The Wave-5 launch note — kept for the recipe; the wave is DONE:
**Waves 1–4 + Wave 3 are all COMPLETE & pushed.** Wave 5 is the final wave provable in-repo (Wave 6 is
deploy/live-estate-only — never built/claimed here). Wave 5 = **GPU perf at scale, RATIOS only** in the
existing **`celnet-gpu`** crate (wgpu/Metal baseline + CPU-SIMD fallback already there; f32↔f64 reconcile
machinery exists from the earlier `scenario.rs`/`scenario.wgsl` work):
- **GPU perf harness** `gpu_load` / `gpu_batch` / `gpu_gate` runnable **headless on M4 Metal / Lavapipe**
  (CI Linux) — measure the closed-form vanilla batch (Workload A / G2) + a multi-step path kernel (G3) +
  payoff kernels (G4); **reuse the `celnet-qmc` Sobol+Brownian-bridge** (built in W4c) for the QMC path so
  the GPU and CPU share direction numbers (CPU↔GPU KAT).
- **Gates:** three-way **GPU-MC ≈ CPU-MC ≈ golden** within the derived f32 bound; f32↔f64 reconcile
  node-by-node; a **measured GPU/CPU speedup RATIO** on M4 (honest — a ratio, not an absolute throughput);
  bit-stable Philox. **HONEST BOUNDARY (verbatim, never violate): the NVIDIA absolute throughput headline
  + the ≤50ms exotic + Workload-A/B absolute numbers are DEFERRED** — M4 Metal lacks f64, so the absolute
  production baseline comes only from the CUDA deploy-gate CI; in-repo we prove correctness + RATIOS on
  M4/Lavapipe. GPU pathwise/LR Greeks (G6) is a further increment.
- Disjointness: this is largely ONE crate (`celnet-gpu`). Consider a single focused track, OR two disjoint
  tracks: Track A = the closed-form batch kernel + harness (`gpu_load`/`gpu_batch`); Track B = the QMC
  path kernel reusing celnet-qmc (a distinct kernel/file). Watch for the M4-Metal-no-f64 constraint (the
  CPU oracle is f64; GPU is f32 reconciled within a derived bound — never claim f64 on Metal).
After Wave 5: the program's in-repo waves are DONE; Wave 6 stays designed+seamed (ADRs + docs-anchor lint
only). Then the leadership program is materially complete — report status honestly vs the program doc bar.
**THE RECIPE (worked 4×, refined):** `Workflow` implement→adversarial-verify; disjoint crates; each new
parity test is a NEW `celnet-parity/tests/*.rs` (auto-discovered); a NEW crate auto-joins via
`members=["crates/*"]` (NO root Cargo.toml edit) and parity references it BY PATH (or I pre-add the
dev-dep to `parity/Cargo.toml` before launch so tracks don't race on it); **each track's gate MUST
include `clippy -p celnet-parity --test <newtest> -D warnings`**; **I (orchestrator) ALWAYS** run the
full `just check`, verify the literal "All gates passed" line, run `cargo fmt --all` (agents' final state
is sometimes not fmt-clean), **independently re-derive the math vs the published source** (not just
re-run the gate — see lesson #3), then commit + `git push origin main` + verify local==remote, update
ledger+memory. Workflow scripts persist under the session workflows/scripts dir; resume via {scriptPath}.
(If the user redirects to Wave 3 — replicated log + hot-standby + SPMC fan-out, the XL distributed-
correctness item — that's the other valid next; new crate `celnet-replog`, one-way dep → {journal,types},
deterministic bit-identical cross-node replay reconciled by the f64 oracle, mirror `risk_federation.rs`
multi-process boot, thin leader-replicated before full Raft, never fake a cluster.)

### Git / push
`origin` = `https://github.com/soarsa/celnet.git` (private, account `soarsa`, `gh` authed). Push permitted
**only** there (GUIDE.md guardrail #1; settings deny-rules removed 2026-06-05). After each wave: commit +
`git push origin main` + verify `local==remote`.

### Demo/services (optional; may have survived the clear)
`source "$HOME/.cargo/env" && CELNET_GRPC_ADDR=127.0.0.1:50551 CELNET_WS_ADDR=127.0.0.1:8081 cargo run -q
-p celnet-server --example demo_edge` (WS `:8081`, gRPC `:50551`); GUI `cd gui && npm run dev`
(http://localhost:5173). Not needed to run a workflow.

See [[token-and-context-discipline]], [[no-mocks-policy]], [[api-first-client-parity]], [[git-local-only]].
