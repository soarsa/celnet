# Celnet — Post-Completion Honest Gap Audit

> Date: 2026-06-07. Audited at HEAD `fd15bb2` (12-wave COMPLETION-PROGRAM + full Raft
> increment landed; `just check` 1231/1231 green). Five gap-audit lenses (analytics,
> deferrals, rigor, architecture, experience) returned and were independently
> spot-verified against the tree (ripgrep, `ls crates`, proto/doc reads) — the
> code-graph MCP was treated as
> possibly offline and not relied upon.

## Headline

**Celnet is materially complete.** No P0 gaps and no P1 *functional* gaps remain in-repo:
the documented analytics baseline (`docs/ANALYTICS-SPEC.md`) is fully built, parity-gated
against independent oracles, and reachable from all five clients; the architecture is a
clean 34-crate one-way acyclic graph on one unversioned contract; the verification estate
(cross-OS×MSRV matrix, lavapipe GPU lane, 100%-mutation-kill on the pricing core, 5 frozen
QuantLib golden tables, ~24 parity rows, three perf gates) is exceptionally strong.

The genuine remaining in-repo tail is **short and falls into two buckets**: (1) **doc-truth
drift** — several load-bearing design docs (`ARCHITECTURE.md`, `INTERFACES.md`,
`SCALE-OUT.md`, `SOTA-2026.md`, `GPU-AT-SCALE-PLAN.md`, research-digest) still claim
capabilities are unbuilt that shipped and are parity-gated, which directly violates
guardrail #10; this is the single highest-leverage cleanliness fix and is mostly P1-by-honesty
/ S-effort; and (2) **two genuinely-new FX-relevant analytics dimensions** (American/Bermudan
early exercise; correlated multi-asset basket / best-of-worst-of) that are absent
platform-wide and would materially raise the product bar, plus a set of P1/P2 rigor-widening
and onboarding-polish items (extend the mutation/fuzz/coverage gold standard beyond
celnet-vanilla; runnable SDK examples). Everything else surfaced by the lenses is either
already done (class B) or deploy/live-gated by the honest boundary (class C).

`materially_complete = true` (no P0/P1 *functional* gaps; the P1 items are honesty/rigor/
onboarding, not missing platform capability). The American-exercise and basket items are
real bar-raisers worth a small follow-on, not a second program.

---

## Ranked backlog (genuine in-repo items only)

Dependency-ordered. P1-DOC items are cheap and should land first (they unblock honest
review and are worktree-parallel). The two analytics items are independent L-effort
follow-ons. The rigor items widen the proven-fault surface.

### 1. PC-DOC-ARCH — Reconcile `docs/ARCHITECTURE.md` to the 34-crate built system (P1, S)
- **Why**: stated architecture source-of-truth, ~37 commits / 12 waves stale; three
  load-bearing false claims verified: `ARCHITECTURE.md:88` "19 crates" (actual **34**),
  `:441/:444` plugin/Wasm host `[deferred]` (built — wasmi WS-G), `:291/:587` Sobol+Brownian-
  bridge "designed, not yet implemented" (built — celnet-qmc + GPU W8). Guardrail #10.
- **Oracle/gate**: `ls crates | wc -l` == claimed count; every "built" claim cites an existing
  crate path; remaining `deferred`/`not yet implemented` mentions map only to class-C items.
- **Client parity**: infrastructure — no client surface.
- **Parallel**: disjoint (docs only).

### 2. PC-DOC-IFACE — Update `docs/INTERFACES.md` frozen-interface registry for W1–W6 wire additions (P1, S)
- **Why**: the stated "current contracts" registry stops at Phase-2 (`INTERFACES.md:62`); the
  entire api-first catalogue program additively extended celnet-proto (VarianceSwap/VolSwap/
  Asian/ForwardStart/Cliquet/Tarf/Accumulator/Quanto/Lookback oneof arms; BookingModel
  selector; AsianMethod/QuantoPayoff/TarfRedemption/AccumulatorMonitoring/LookbackStyle enums;
  SMILE_MODEL_EXTENDED_SURFACE=4) — none registered. The coordination seam is wrong about the
  contract it governs.
- **Oracle/gate**: grep celnet.proto for each oneof variant/enum/field → matching registry
  entry; cross-check `docs/CLIENT-PARITY-MATRIX.md`. No proto symbol present-but-unregistered.
- **Client parity**: infrastructure — registry doc.
- **Parallel**: disjoint (docs only).

### 3. PC-DOC-STALE — Kill the "not implemented" claims for Sobol'/Brownian-bridge/GPU-Greeks/GPU-batch/GPU-throughput (P1, S)
- **Why**: `docs/SOTA-2026.md`, `docs/GPU-AT-SCALE-PLAN.md` §1 "Current limitations", and
  `docs/assets/.../research-digest.json` still assert "Sobol/Brownian-bridge designed but NOT
  implemented / grep finds nothing", "No QMC", "No Greeks on GPU", "No GPU throughput
  benchmark" — all falsified by celnet-qmc (sobol.rs/bridge.rs), GPU W8 (path/greeks kernels),
  Wave-5 batch.wgsl, and celnet-bench gpu_load/gpu_gate. Also reconcile `SCALE-OUT.md:51/52/78/88`:
  Raft §7 snapshot/compaction is **Built** (W7 InstallSnapshot), not "next increment" — leave
  only §6 dynamic membership as the genuine next consensus increment.
- **Oracle/gate**: `rg "Sobol.*not impl|No QMC|No Greeks on GPU|No throughput.*GPU" docs/`
  returns zero; docs-anchor/doc-sync lint green; full `just check` "All gates passed."
- **Client parity**: infrastructure.
- **Parallel**: disjoint (docs only) — can merge PC-DOC-ARCH/IFACE/STALE into one docs lane.

### 4. PC-OBS-DEPS — Drop celnet-observability's unused `celnet-types` / `celnet-core` deps (P2, S)
- **Why**: cargo-machete + src verification (no `celnet_core`/`celnet_types` reference under
  the crate) confirm two genuinely-unused workspace deps; removing them tightens the layer graph.
- **Oracle/gate**: `cargo machete` clean for the crate; `cargo build -p celnet-observability`
  green (compiler proves they were unused); full `just check` green.
- **Client parity**: infrastructure.
- **Parallel**: disjoint single crate.

### 5. PC-SDK-EXAMPLES — Runnable SDK quickstart examples under `crates/celnet-client/examples/` (P1, S)
- **Why**: guardrail #11 mandates a trader-grounded SDK; the canonical SOTA onboarding
  affordance — a `cargo run -p celnet-client --example …` one-liner — is **absent** (confirmed:
  no examples/ dir; only rustdoc + integration tests). Highest-leverage intuitivity gap.
- **Scope**: 2–3 small examples vs a live `demo_edge`: quote_and_trade, stream_blotter,
  price_exotic (via the vocab builders). Reference from lib.rs docs + parity matrix.
- **Oracle/gate**: a CI/integration lane boots demo_edge, runs each example, asserts a
  non-empty priced result == server price; `cargo build --examples -p celnet-client` clean
  under clippy -D warnings.
- **Client parity**: SDK onboarding surface.
- **Parallel**: disjoint single crate (depends on demo_edge, already built).

### 6. PC-MUT-WIDEN — Extend the mutation kill-rate gate beyond celnet-vanilla (P1, L)
- **Why**: the sharpest test-quality signal (per HARDENING.md §2) is enforced ONLY on
  celnet-vanilla (verified: single `.config/mutants.toml`, `mutants-gate-vanilla` recipe).
  Surface calibration, exotics PDE/MC/particle, risk-cube non-additive VaR/ES, and celnet-xva
  carry safety-critical numerics with NO kill-rate floor — a future edit can silently weaken
  those suites and stay green.
- **Oracle/gate**: per-crate `.config/mutants-<crate>.toml` (audited equivalence set) + CI job;
  `cargo mutants -p <crate>` exits non-zero on any non-equivalent survivor; baselines in
  HARDENING.md §2. Start with celnet-surface (lowest coverage today).
- **Client parity**: infrastructure.
- **Parallel**: per-crate configs are disjoint; can be split across worktree lanes.

### 7. PC-FUZZ-DECODE — Fuzz the untrusted-input byte parsers (replog LogEntry/Snapshot, journal torn-tail, proto convert) (P1, M)
- **Why**: fuzzing is a single target (`fuzz/fuzz_targets/vanilla_inputs.rs` only). The genuinely
  untrusted surfaces are the hand-rolled byte parsers: celnet-replog decode (length/CRC framing,
  socket+disk bytes), celnet-journal torn-tail recovery, celnet-proto convert.rs — good unit
  coverage but no adversarial-byte proof of no-panic / total-decode / bounded-alloc.
- **Oracle/gate**: new fuzz targets under the existing standalone fuzz crate; contract = no
  panic, Ok-or-typed-Err, bounded allocation; nightly CI fuzz job time-boxed; seed from existing
  decode unit-test vectors.
- **Client parity**: infrastructure (robustness of wire/disk paths).
- **Parallel**: disjoint targets; lane-safe.

### 8. PC-AMERICAN — American / Bermudan early-exercise pricer (vanilla + barrier) (P1, L)
- **Why**: physically-settled FX options DO trade American-style; early exercise is the single
  most material option-mechanics capability ENTIRELY ABSENT platform-wide (verified: zero
  matches for american/bermudan/longstaff/psor across celnet-exotics + celnet-vanilla; wire
  Vanilla is European-only). A recognized, non-speculative FX-desk omission, not padding.
- **Scope**: celnet-exotics american.rs — PSOR free-boundary FD on the existing log-spot
  Crank-Nicolson grid for American vanilla + single-barrier; optional Longstaff-Schwartz LSM on
  the existing MC for Bermudan/path-dependent. Additive ExerciseStyle field on Vanilla/SingleBarrier
  proto (no version field); wire server + SDK/CLI/Excel/GUI selector in lockstep.
- **Oracle/gate**: independent oracle = QuantLib American FX (FdBlackScholes / Barone-Adesi-Whaley)
  hand-pinned in celnet-golden + structural invariants (American≥European; ==European to ~1e-9 when
  exercise never optimal; PSOR-PDE == LSM-MC within stderr). `clippy -p celnet-parity --test american
  -D warnings` + full `just check`.
- **Client parity**: full five-surface (additive proto field).
- **Parallel**: celnet-exotics + proto + clients; mostly disjoint, proto edit needs coordination.

### 9. PC-BASKET — Multi-asset FX basket / best-of / worst-of (correlated MC) (P2, L)
- **Why**: FX baskets and worst-of/best-of-N-pair notes are genuinely traded; the entire exotics
  MC stack is single-underlying (verified: no basket/best-of/worst-of/cholesky/correlated-path in
  celnet-exotics — correlation exists only as quanto's analytic drift). Adds a structurally new
  catalogue dimension, not a variant.
- **Scope**: extend the MC engine with a Cholesky-correlated multi-asset GBM path generator (reuse
  celnet-qmc Sobol + bridge) + weighted-basket/best-of/worst-of payoffs; Basket/BestOfWorstOf proto
  message (additive) + server pricer + client parity.
- **Oracle/gate**: independent oracle = 2-asset analytic (Stulz 1982 best/worst-of-two / Kirk-style
  basket) hand-pinned vs correlated MC within stderr; correlation-recovery check; structural sandwich
  worst-of ≤ singles ≤ best-of; ρ→1 collapse. `clippy -p celnet-parity --test basket -D warnings`.
- **Client parity**: full five-surface (additive proto message).
- **Parallel**: celnet-exotics + proto + clients; proto edit needs coordination.

### 10. PC-HESTON-GOLD — QuantLib Heston golden table (P2, M)
- **Why**: Heston is anchored only by one literature value (Albrecher 2007 ≈5.785) + an internal
  CM≈COS cross-check (verified: no Heston golden under data/ or celnet-golden). QuantLib ships
  AnalyticHestonEngine (the sanctioned oracle) — a frozen grid upgrades Heston to the same
  independent-oracle bar vanilla/barriers/digitals already meet, catching a shared CF/quadrature error.
- **Oracle/gate**: frozen `data/heston_gk.csv` via QuantLib AnalyticHestonEngine; celnet-golden test
  gates CM/COS to ~1e-6 abs + 1e-5 rel over ≤3y FX domain; fails on drift.
- **Client parity**: infrastructure (verification).
- **Parallel**: disjoint (celnet-golden test + data file).

### 11. PC-SURFACE-COV — Close surface coverage backlog (strangle/stochvol/market_hedge) + add a coverage gate (P2, M)
- **Why**: HARDENING.md §3 explicitly records celnet-surface strangle.rs/stochvol.rs/market_hedge.rs
  at ~78–86% line coverage as "the standing backlog for the surface lane" — load-bearing calibration
  code with real uncovered branches (solver-failure, degenerate-quote). Named-by-docs unfinished work.
- **Oracle/gate**: targeted branch tests, then `cargo llvm-cov nextest -p celnet-surface
  --fail-under-lines <floor> --fail-under-regions <floor>` in CI at a committed floor — same gate as
  celnet-vanilla.
- **Client parity**: infrastructure.
- **Parallel**: disjoint single crate.

### 12. PC-A11Y-WIDEN — Extend axe sweep to Cube + Universe-navigator views (P2, S)
- **Why**: W12's axe gate covers 5 core workspaces but omits CubeWorkspace + UniverseNavigator —
  the two data-dense views (color-encoded heatmap, virtualised listbox) where serious a11y issues
  typically hide. Makes the zero-serious claim hold product-wide.
- **Oracle/gate**: `@axe-core/playwright` expectNoSeriousA11y on both views over the live edge in the
  existing CI a11y lane.
- **Client parity**: GUI.
- **Parallel**: disjoint (gui only).

### 13. PC-SHORTCUTS — In-app keyboard-shortcut cheatsheet / "?" overlay (P2, S)
- **Why**: the product is explicitly keyboard-first (⌘K, ⌘1-5, ⌘P, ⌘↩/Esc) but there is no
  discoverable list of bindings — standard SOTA discoverability affordance for the keyboard-first
  mandate.
- **Oracle/gate**: vitest/Playwright press "?" → asserts each documented binding appears; bindings
  derived from the same source-of-truth map the Shell uses (no drift).
- **Client parity**: GUI.
- **Parallel**: disjoint (gui only).

---

## Class B — already done (do NOT re-propose)

Full exotic catalogue §4 (digitals/touches/DNT/all-8-barriers/window/Asian-geo+arith/var+vol-swap/
forward-start/cliquet/TARF/accumulator/quanto/lookback) on the wire & reachable from all 5 clients;
LSV booking model (Heston+Dupire leverage, particle calibration, ADI HV PDE); standalone Heston
CM+COS; all surface families (VV/SABR/SVI/SSVI/eSSVI, broker→smile, Dupire local vol); full 12-Greek
set incl. both FX rhos; Sobol QMC + Owen scramble + Brownian bridge (~38×/88× measured variance
reduction); GPU G3 path kernel + G6 pathwise/LR Greeks over shared Sobol'; full Raft
(election/Pre-Vote/truncation/compaction/InstallSnapshot); celnet-fanout SPMC ring under the async
edge; celnet-xva (CVA/DVA/FVA, synthetic netting); exotic risk-cube aggregation; RiskService consumed
by all 5 surfaces (client-side loop deleted); eSSVI client parity (index 4); GUI Playwright e2e + axe
(5 core views); typed provenance + StatusRibbon observability; journal compaction; cross-OS×MSRV CI
matrix; lavapipe GPU lane; 100%-mutation-kill on celnet-vanilla; 5 frozen QuantLib golden tables;
three perf gates (bench_gate/core_load/iai-callgrind); ~24 parity rows vs independent oracles;
FRTB circular-oracle defense internalized (xva pins literal published values). ANALYTICS-SPEC.md +
ROADMAP P3 + CLIENT-PARITY-MATRIX reconciled in W12.

## Class C — deploy/live-gated by the honest boundary (list, never build in-repo)

CUDA/NVIDIA ABSOLUTE GPU throughput + ≤50ms exotic + Workload-A/B absolutes (M4 Metal lacks f64 ⇒
in-repo proves correctness + host-local RATIOS only); cross-host wire p99 / kernel-bypass NIC /
§11 ABSOLUTE wire-latency SLOs (in-repo proves the §1.2 in-core truth-gate + loopback fleet_slo only);
live JVM Celer estate lifecycle (sidecar/FX_OPTION/inferred hops/tenant overlays); CUDA-backend
(CubeCL) device-resident f64 parity lane on real NVIDIA; Raft §6 dynamic membership change (correctness
complete for fixed membership; live cross-host reconfig is deploy-gated); cross-owner multiplexing
within one StreamSession + cross-node transport for risk-fleet/risk-cube (in-process algebra proven;
physical transport deploy-gated); plugin Tier-1 stabby signed .so + Tier-3 Landlock/seccomp ring
(designed-only by intent); tuned global allocator (mimalloc/jemalloc, benchmark-gated; hot core is
structurally zero-alloc); live vendor implied-vol VALUES / live FX fixing VALUES / live
CSAs-collateral-wrong-way-risk for xva (synthetic/published in-repo); non-Premium TrendModes +
EventClock real magnitudes + live attribution-identity + composite-surface feed (all live-feed
dependent); any XVA client surface (xva is deliberately internal-only, no contract change).
Forward-start/cliquet forward-smile LSV repricing is in-repo-buildable numerics-depth but borderline
P2 (current GBM closed form is exact and parity-gated) — noted, not proposed as bar-raising.
