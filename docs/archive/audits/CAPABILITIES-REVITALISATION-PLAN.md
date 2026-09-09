# Capabilities Doc-Set Revitalisation Plan

> **Status:** operator-ready. Authored against HEAD `8572574` (34 crates, `just check` 1306/1306,
> 12-wave COMPLETION-PROGRAM + full Raft + 13-item POST-COMPLETION-AUDIT all landed).
> **Scope:** revitalise the whole capabilities doc set — hub `docs/CELNET-CAPABILITIES.md`,
> chapters 01-14, the 13 branded figures, the two competition docs, and a NEW self-contained
> HTML showcase — so the docs reflect the now-complete platform, lead with a sharpened
> competitive thesis, exhaustively detail every API/Excel/CLI surface, and respect the verbatim
> honest boundary in EVERY claim.
> **Governing rule:** every claim grounded in shipped code (cite crate/path/proto-line); nothing
> omitted, nothing overclaimed. The single dominant defect to delete everywhere: the false
> "first-generation exotics catalogue" framing.

---

## 0. Ground truth (verified at HEAD `8572574`)

- **34 crates** (`ls crates | wc -l`). ARCHITECTURE.md's stale "19 crates" is a known drift —
  state **34** wherever a count appears.
- **6 gRPC services** (`crates/celnet-proto/proto/celnet.proto:2447-2504`):
  `PricingService.Price`; `QuoteService.{RequestQuote, AcceptQuote, RejectQuote}`;
  `StreamService.StreamSession`; `RiskService.{ListPositions, AggregateRisk, DrillRisk,
  LimitStatus}`; `SurfaceService.{GetSmile, MarkSurface, Scenario}`. Plus the **byte-identical
  WebSocket JSON mirror** of the same contract (`crates/celnet-server/src/lib.rs:31,177,359`).
- **18-product oneof arms** on the unified `Instrument` (`celnet.proto:1016-1062`, field numbers
  7-25 with 22 unused): vanilla(7), strategy(8), single_barrier(9), double_barrier(10),
  digital(11), touch(12), variance_swap(13), volatility_swap(14), asian_option(15),
  forward_start(16), cliquet(17), quanto(18), tarf(19), accumulator(20), lookback(21),
  window_barrier(23), american(24), basket(25). Each MC product carries `price_std_error`.
- **5 smile models** (`celnet.proto:372-382`): MARKET_HEDGE(0, VV), STOCHASTIC_VOL(1, SABR),
  PARAMETRIC(2, SVI), PARAMETRIC_SURFACE(3, SSVI), **EXTENDED_SURFACE(4, eSSVI)**.
- **PricingModel selector**: DEFAULT (analytic) vs LOCAL_STOCH_VOL (LSV booking model),
  field 22 on Instrument.
- **27 Excel `CELNET.*` functions** (`excel/src/functions/functions.ts`, `@customfunction` count
  = 27 distinct names): PRICE, GREEKS, SURFACE, MARKSURFACE, MARK, RFQ, SUBSCRIBE, SERIES,
  BARRIER, WINDOWBARRIER, DIGITAL, TOUCH, VARSWAP, VOLSWAP, ASIAN, FORWARDSTART, CLIQUET, QUANTO,
  TARF, ACCUMULATOR, LOOKBACK, AMERICAN, BASKET, RISK, POSITIONS, LIMITS, STATUS.
- **celnet-exotics src** (`crates/celnet-exotics/src/`): accumulator, adi, american, asian,
  barrier, digital, forward_start, leverage, lookback, lsv, market_hedge_overlay, mc, multiasset,
  normal, particle, pde, payoff, quanto, rng, stochvol, tarf, touch, var_swap, vol_swap.
- **celnet-parity tests** (independent oracles): asian, basket, broker_smile, conventions,
  determinism, essvi, essvi_hardening, exotic_risk_cube, exotics, forward_start, frtb,
  gpu_greeks, gpu_path, greeks, heston, lsv, pair_universe, qmc, qmc_highdim, raft_compaction,
  raft_election, raft_snapshot, structured, surface, var_vol_swap, xva (~26 parity rows).
- **celnet-xva** (`crates/celnet-xva/src/`): cva, exposure, netting, survival — CVA/DVA/FVA on
  **synthetic netting sets**, **internal-only, NOT on the wire** (grep of celnet.proto for
  cva/xva returns nothing).
- **GUI** (`gui/src/`): SurfaceWorkspace has the **eSSVI chip** (5 model chips); TicketWorkspace
  builds asian/barrier/digital/touch/forwardStart/cliquet/quanto/lookback/tarf/accumulator/
  varianceSwap/volatilitySwap/windowBarrier/**american**/**basket** instruments; **ShortcutsOverlay**
  (`?` cheatsheet) present; Playwright e2e + axe a11y suites (`gui/e2e/`).
- **Plugin host** (`crates/celnet-plugin-host/src/`): only `native.rs` (Tier-0) + `wasm.rs`
  (Tier-2 wasmi). **Tier-1 signed-.so + Tier-3 Landlock/seccomp are DESIGNED-ONLY**
  (POST-COMPLETION-AUDIT class C).
- **Distributed**: celnet-replog (Raft election/Pre-Vote/truncation/compaction + **InstallSnapshot**,
  bit-identical replay), celnet-fanout (SPMC ring **wired under the edge**,
  `crates/celnet-server/src/services/pricefanout.rs` / W9), celnet-risk-fleet (HRW + fan-out==single-node).
- **Golden tables** (`crates/celnet-golden/data/`): vanilla, both digital styles, all 8 barriers,
  touch (touch_gk.csv), **Heston (heston_fo.csv)** — frozen QuantLib. Other products gated vs
  closed-form limits / FD / hand-pinned published constants / honest stderr bands.
- **Performance**: `crates/celnet-bench/src/bin/core_load.rs` §1.2 in-core truth-gate
  p50≤2µs/p99≤10µs/p99.9≤25µs ABSOLUTE; measured M4 ~42ns/125ns/~1µs. GPU dispatch-amortization
  RATIO 0.68×@4k → 53×@1M paths (M4, ratio only). iai-callgrind instruction gate (Linux CI).

---

## 1. The sharpened competitive thesis (the whole set leads with this)

> **Celnet is not a thin challenger closing gaps — it is a functionally complete, evidence-backed
> superset of what a derivatives desk stitches together today, proven by a runnable parity matrix
> against independent oracles, behind one unversioned contract reachable identically from five
> clients.**

Three pillars, each grounded:

1. **Catalogue depth that matches the front-to-back platforms.** Vanilla → the full first-generation
   exotics → structured & path-dependent products (var/vol swaps, Asians, forward-start/cliquet,
   quanto, TARF, accumulator, lookback) → American/Bermudan early exercise → correlated multi-asset
   basket/best-of/worst-of → a particle-calibrated LSV booking model + standalone Heston — all on
   the **one wire**, parity-gated, reachable from **all five clients**. This is the breadth Murex
   MX.3 / Numerix CrossAsset / Fenics kACE charge for, delivered open and microsecond-class.
2. **Edges the deep-catalogue incumbents structurally lack:** an open quant SDK (run private IP
   in-engine, sandboxed/deterministic), one clean unversioned contract with bit-identical values
   across GUI/SDK/CLI/Excel/WS, a pinned zero-alloc nanosecond hot core, server-side hierarchical
   risk (clients never loop-and-sum), and an honest evidence trail (parity matrix + golden tables +
   mutation + fuzz).
3. **Honesty as a differentiator.** Every Celnet figure is labelled (in-core / M4 / loopback);
   every competitor claim is a stated inference; deploy-gated absolutes are never claimed in-repo.
   A reviewer doing diligence finds the proof, not marketing fiction.

The thesis explicitly does **not** assert any deploy-gated absolute (see §6).

---

## 2. Chapter rewrite briefs (01-14 + hub)

> **Parallelism:** chapters live in disjoint files and are independently rewritable. The only
> shared dependency is consistency of the recurring strings (catalogue list, 5 smile models, the
> verbatim boundary lines) — these are FIXED in §1 and §6 of this plan, so parallel authors copy
> them verbatim and need no cross-talk. Figures are a separate disjoint workstream (§3). Recommended
> batching: Batch A (01,02,04,13 — catalogue spine), Batch B (05,06,07,08 — infra/risk),
> Batch C (09,10,11,12,14 — API/clients/rigor), hub last (depends on figure captions). All batches
> are parallel-safe within and across each other.

### Hub — `docs/CELNET-CAPABILITIES.md`
- **Rewrite brief:** Lead with the §1 thesis sentence. Refresh the figure-index captions for
  Fig 3 / Fig 5 / Fig 8 / Fig 12 to match the re-authored figures (delete "first-generation",
  add catalogue tiers to Fig 3, label Tier-1/3 designed in Fig 5, name Raft/SPMC in Fig 8).
  Add the verbatim honest-boundary block (§6) as a footer section. Add an "all 18-products × 5
  surfaces, proven by CLIENT-PARITY-MATRIX.md" line.
- **Must cover:** thesis; 34 crates; figure index synced to re-rendered figures; boundary footer.
- **Parallel-safe:** YES, but author **last** (its captions must match the final figure content).

### Ch 01 — Executive Summary — HIGH PRIORITY
- **Rewrite brief:** Replace both "first-generation exotics catalogue" strings (lines 5, 12) with
  the full coverage ceiling. Add the §1 competitive line. Add eSSVI to the smile list. Add an
  XVA/FRTB line to the risk bullet (with the internal/synthetic caveat for XVA). Keep
  "microsecond-class pricing" (qualitative, honest). Frame Fenics/Bloomberg/Refinitiv/EBS as
  **adapter integration targets**, not live connections.
- **Must cover:** coverage ceiling = vanilla → full first-gen exotics → structured/path-dependent
  (var/vol swap, Asian, fwd-start/cliquet, quanto, TARF, accumulator, lookback) → American/Bermudan
  → correlated basket/best-of/worst-of → LSV booking model + standalone Heston; 5 smile families
  incl. eSSVI; Sobol QMC variance reduction; server-side firm risk + FRTB-SA + XVA(internal);
  one unversioned contract / 5 clients / bit-identical. Cite CLIENT-PARITY-MATRIX.md, celnet.proto
  oneof, celnet-exotics modules. DROP any Greek *count* (keep enumerated list).
- **Parallel-safe:** YES.

### Ch 02 — Capability Map — HIGH PRIORITY (master inventory; propagates everywhere)
- **Rewrite brief:** §2.1 — rename "First-generation exotics" to a full-catalogue row enumerating
  the shipped products; add a "Structured & path-dependent" row, an "Early exercise (American/
  Bermudan, PSOR FD + LSM)" row, a "Correlated multi-asset (basket/best-of/worst-of, Cholesky MC)"
  row, an "LSV booking model + standalone Heston" row. Add eSSVI → 5 smile models everywhere.
  §2.2 GPU — name the multi-step path kernel (path.wgsl/path.rs), pathwise/LR Greeks
  (greeks.wgsl/pathwise.rs), batch closed-form (batch.wgsl), Sobol-QMC-on-GPU KAT, with the
  M4-Metal-f64 RATIO-only boundary. §2.3 risk — add FRTB-SA, XVA (internal/synthetic), exotic-leg
  risk-cube aggregation rows. §2.4 — re-label the "Hardened tiers" row as **designed/deploy-gated**;
  re-cast the SDK as "extend an already-deep catalogue", not "the sole route beyond first-gen".
- **Must cover:** the full inventory above; eSSVI; GPU kernels (ratios/correctness only);
  FRTB-SA/XVA/exotic-leg risk; Tier-1/3 designed flag. Cite proto, celnet-exotics, celnet-gpu,
  celnet-risk-cube/{frtb,exotic}.rs, celnet-xva.
- **Parallel-safe:** YES.

### Ch 03 — System Architecture
- **Rewrite brief:** §3.3 — state **34 one-way-acyclic crates** (cite count). §3.5 — add the
  distributed-correctness substrate: leader-replicated event log + full Raft (election + Pre-Vote,
  conflicting-tail truncation, snapshot compaction, **InstallSnapshot** reseed) proven bit-identical
  (`f64::to_bits`) on localhost multi-process; the lock-free SPMC broadcast ring **under the edge**
  fanning each pair's tick to all subscribers (zero-alloc, exact skip-accounting conflation). §3.4 —
  note journal compaction/checkpoint now built. Add the verbatim cross-DC/partition/Raft-§6
  boundary line.
- **Must cover:** 34 crates; Raft incl. InstallSnapshot (celnet-replog); SPMC ring under edge
  (celnet-server/services/pricefanout.rs); journal compaction; boundary.
- **Parallel-safe:** YES.

### Ch 04 — Quant & Pricing Methodology — HIGHEST-PRIORITY REWRITE
- **Rewrite brief:** Restructure the chapter around the FULL catalogue. Keep §4.1/§4.2 (vanilla +
  Greeks + delta conventions). §4.3 — add eSSVI (5 smile families) + Dupire local vol. Replace §4.4
  ("First-generation exotics catalogue") with a tiered catalogue: (a) first-generation
  (digitals/touches/DNT/all-8 barriers + window) by analytic/CN-Rannacher-PDE/Philox-MC/VV-overlay;
  (b) closed-form structured (var/vol swap = log-contract replication/Carr-Lee; Asian =
  Turnbull-Wakeman/Curran; forward-start/cliquet = Rubinstein; quanto; lookback); (c) MC/path-
  dependent (TARF, accumulator, basket/best-of/worst-of Cholesky MC — carry honest stderr);
  (d) early exercise (American/Bermudan = PSOR free-boundary FD + Longstaff-Schwartz LSM);
  (e) LSV booking engine (Heston backbone + Dupire leverage, particle calibration, ADI PDE) +
  standalone Heston (Carr-Madan + Fang-Oosterlee COS). Add a §4.x on Sobol QMC variance reduction
  (~38×/88× measured). **CRITICAL honesty fix in §4.5:** state the validation method PER product
  class — frozen QuantLib golden tables ONLY for vanilla/digital/barrier/Heston; closed-form limits
  + FD + hand-pinned published constants + honest stderr bands for the rest. Do **not** blanket-claim
  "machine precision vs QuantLib". Rewrite §4.6 — SDK as ADDITIONAL extensibility; remove the false
  "TARFs/quantos/baskets via SDK" claim (they are shipped core). Note fig-03 must be re-rendered.
- **Must cover:** every catalogue tier with engine + parity-row + oracle cited; eSSVI; LSV; Heston;
  QMC; the per-class validation honesty split. Cite celnet-exotics/*, celnet-heston, celnet-qmc,
  celnet-parity/tests/*, celnet-golden/data/*.
- **Parallel-safe:** YES.

### Ch 05 — Extensibility / Plugin Host — OVERCLAIM FIX
- **Rewrite brief:** Keep the SDK contract + Native (Tier-0) + WebAssembly (Tier-2 wasmi) tiers as
  fully shipped (14 tests, 4 WS-G gates). Add a **status column**: "Shipped" for Native/WASM,
  "Designed (deploy-gated)" for Tier-1 signed-.so (stabby) and Tier-3 OS-sandbox (Landlock/seccomp);
  reword §5.2 to "the in-repo host ships the native and WebAssembly tiers; the signed-shared-object
  and OS-sandbox tiers are designed to slot in behind the same frozen contract." Fix §5.1
  "first-generation" → "the full built-in catalogue". Add eSSVI to the SmileModel list. Keep §5.3
  (replay + butterfly self-check). Note fig-05 must be re-rendered with the built/designed split.
- **Must cover:** the two built tiers; the two designed tiers clearly flagged; SDK as extend-deep-
  catalogue; eSSVI. Cite celnet-plugin-host/src/{native,wasm}.rs, PLUGIN-HOST-ALT.md.
- **Parallel-safe:** YES.

### Ch 06 — Risk Management
- **Rewrite brief:** Keep §6.1-§6.3 (accurate). Add §6.x "Regulatory capital — FRTB-SA" (SbM
  within/cross-bucket K_b, three correlation scenarios → max, the 0.75ρ low-corr floor with
  hand-pinned BCBS constants, curvature, RRAO; honest DRC=0 for deliverable FX) citing
  celnet-risk-cube/frtb.rs. Add §6.x "Counterparty valuation adjustments — XVA" (CVA/DVA/FVA over
  EPE/ENE on **synthetic netting sets**, closed-form CVA parity) citing celnet-xva, **with the
  caveat: internal-only, no client surface, synthetic netting sets — live CSAs/collateral/wrong-way
  risk are deploy-gated.** Update §6.2 to state the cube now aggregates **exotic legs**
  (fan-out==single-node, celnet-risk-cube/exotic.rs). Note fig-06 may add FRTB/XVA nodes.
- **Must cover:** FRTB-SA; XVA(internal/synthetic); exotic-leg aggregation. Cite the modules.
- **Parallel-safe:** YES.

### Ch 07 — Performance & Latency
- **Rewrite brief:** Add a "Measured, gated" subsection: the §1.2 in-core truth-gate
  (p50≤2µs/p99≤10µs/p99.9≤25µs **absolute**, measured ~42ns/125ns/~1µs on the M4 dev host,
  24-80× margin), labelled **host-local, single-core**. Add the iai-callgrind instruction-count
  gate (Linux CI), the GPU dispatch-amortization **RATIO** curve (0.68×@4k → 53×@1M, M4, ratio
  only), the loopback fleet_slo benches. **In the same breath, state the verbatim boundary:**
  absolute cross-host wire p99 / kernel-bypass NIC / §11 absolute wire SLOs and NVIDIA absolute
  throughput / ≤50ms exotic / Workload-A/B are deploy-gated; M4 Metal lacks f64 ⇒ GPU is
  correctness + ratios only. Keep "network framing, not computation, is the only meaningful latency
  floor" (honest — never convert to a concrete wire number). fig-07 stays qualitative.
- **Must cover:** the measured §1.2 numbers (labelled); iai gate; GPU ratio (labelled); the boundary.
  Cite celnet-bench/src/bin/core_load.rs, the bench READMEs.
- **Parallel-safe:** YES.

### Ch 08 — Scalability & Scale-Out
- **Rewrite brief:** Add §8.x "Distributed correctness": leader-replicated log + full Raft
  (election/Pre-Vote/truncation/compaction/InstallSnapshot) with bit-identical (to_bits) replay;
  the SPMC broadcast ring under the edge fanning each pair's tick (zero-alloc, exact skip-accounting
  conflation); cross-shard risk fan-out (celnet-risk-fleet, additive merge + non-additive re-gather,
  reconciled fan-out==single-node to 1e-12). Expand §8.2 GPU (path kernel, pathwise/LR Greeks, batch
  closed-form, Sobol-QMC-on-GPU) with the M4-Metal-f64 RATIO-only boundary. Add the verbatim
  boundary block: localhost multi-process proves correctness/quorum/framing only; cross-host wire
  p99, cross-DC transport, real partitions, Raft §6 dynamic membership, physical cross-node risk
  transport are deploy-gated. Note fig-08 must be re-rendered.
- **Must cover:** Raft+InstallSnapshot; SPMC ring under edge; cross-shard risk fan-out; GPU kernels;
  boundary. Cite celnet-replog, celnet-fanout, celnet-risk-fleet, celnet-server/services/pricefanout.rs.
- **Parallel-safe:** YES.

### Ch 09 — API & Wire Contract — HIGH PRIORITY (must be exhaustive)
- **Rewrite brief:** §9.1 — replace the four-family table with a **six-service spec**: one row per
  gRPC service listing each RPC with request/response message + one-line purpose
  (PricingService.Price; QuoteService.{RequestQuote,AcceptQuote,RejectQuote}; StreamService.
  StreamSession; RiskService.{ListPositions,AggregateRisk,DrillRisk,LimitStatus}; SurfaceService.
  {GetSmile,MarkSurface,Scenario}). State 5 smile models incl. eSSVI + the PricingModel directive
  (analytic vs LSV). Add a **complete product table**: every oneof arm with field number,
  description, engine/method, and whether it reports MC std-error. Lead with the std-error honesty
  feature (PriceResponse/Quote `price_std_error`). §9.3 — add the market-series feed multiplexed on
  the same StreamSession (5 observables ATM_VOL/SPOT/RISK_REVERSAL/BUTTERFLY/FORWARD), and the
  Heartbeat observability (conflation_drops + server_price_p50/p99/p999_nanos HdrHistogram +
  surface_version + correlation_id), plus StreamEnd/StreamReject typed reasons. §9.5 — document the
  SDK as a typed method list grouped by service family (~20 methods), the InstrumentSpec builder
  (all 18-products + pricing_model/with_lsv), the StreamSession/series async iterators, and the
  three runnable examples; document the CLI subcommand tree (7 top-level: price/surface/exotic/
  basket/convention/risk/stream; ~14 exotic sub-variants; 4 risk sub-subcommands). §9.6 — cite
  CLIENT-PARITY-MATRIX.md as executable proof; state the exhaustive truth (all 18-products × 6
  service families reachable from all 5 surfaces, with honest exceptions e.g. basket Greeks
  deliberately zeroed). Note the WS mirror is byte-identical across ALL services.
- **Must cover:** every service/RPC; every product arm + field number + engine + stderr flag; WS
  mirror; market-series feed; Heartbeat observability; SDK methods + InstrumentSpec; CLI tree;
  parity-matrix citation. **Do NOT claim XVA as an API capability** (not on the wire). Standardise
  on "price + 13 Greeks" (avoid the 13/14 count drift).
  Cite celnet.proto line ranges, celnet-client/src/{lib.rs,vocab.rs,error.rs,rfs.rs}, celnet-cli/src/cli.rs.
- **Parallel-safe:** YES.

### Ch 10 — Excel Integration — HIGH PRIORITY (must be exhaustive)
- **Rewrite brief:** Keep the three design principles verbatim (read-zero-click/write-two-phase,
  no-pricing-in-cell, per-cell convention+surface-version transparency). §10.2 — replace the 8-row
  table with the **complete 27-row table**: each CELNET.* function, its full argument signature
  (from @customfunction/@param JSDoc), what it does, and its spill shape (premium + std_error if MC
  + 13 Greeks + convention footer). Group by Pricing / Exotics / Surface / Stream / Risk. Note
  std-error disclosure for MC functions and BASKET's honestly-deferred (zeroed) Greeks. Note LSV
  booking-model selection on BARRIER (model=ANALYTIC|LSV). Add eSSVI to the MARKSURFACE/MARK
  smile-model lists and the shot-09 caption (VV/SABR/SVI/SSVI/eSSVI — Excel exposes it). §10.3 —
  extend the read-and-spill enumeration to the exotic + risk readers; rewrite the bespoke-risk
  workflow to lead with RISK/POSITIONS/LIMITS pulling the **server's hierarchical aggregate**
  (RiskService, entitlement-pruned, numeraire-converted, bit-identical to the GUI Book/Risk views) —
  not client-composed Greek grids; add an exotics-structuring workflow.
- **Must cover:** all 27 functions with signatures + spill shapes; eSSVI; std-error; server-side
  risk readers. Cite excel/src/functions/functions.ts.
- **Parallel-safe:** YES.

### Ch 11 — Trader GUI
- **Rewrite brief:** §11.2 — state the Ticket prices the full exotic catalogue (enumerate families
  incl. American/Bermudan and basket/best-of/worst-of), not just vanilla strategies. §11.4 — add the
  **eSSVI/extended-surface chip** (now 5 model chips — verified present in SurfaceWorkspace.tsx).
  Add a note on the `?` keyboard-shortcuts overlay (ShortcutsOverlay.tsx). Keep the honest
  provenance read-back framing. (Cross-reference Playwright e2e + axe a11y in ch14, not here.)
- **Must cover:** full exotic ticket; 5th (eSSVI) chip; shortcuts overlay. Cite gui/src/workspaces/
  {TicketWorkspace,SurfaceWorkspace}.tsx, gui/src/components/ShortcutsOverlay.tsx.
- **Parallel-safe:** YES.

### Ch 12 — Celer Integration — SOFT-OVERCLAIM FIX
- **Rewrite brief:** Keep the in-repo seams (FIX 4.4 engine acceptor+initiator, egress governor,
  resilient subscriber, normalization, three adapter-bound deployment modes). **Reword §12.3** so
  CelerIntegrated mode is the **designed estate-native binding**: the JVM distributor sidecar
  handshake / mailbox calibration / live quote-feed entitlement are deploy/live-gated, proven at
  deploy against the running estate — distinguish the in-repo seam (traits + adapter swap + FIX
  loopback) from the live-estate proof. Frame §12.4 vendor feeds as adapter targets.
- **Must cover:** built seams vs deploy-gated live JVM lifecycle. Cite the seam traits / DeployMode.
- **Parallel-safe:** YES.

### Ch 13 — Competitive Positioning — HIGH PRIORITY
- **Rewrite brief:** Rewrite §13.2 "Exotics" row into a multi-band catalogue claim naming the
  shipped, on-wire, parity-gated set (first-gen / structured / advanced mechanics = American +
  basket / booking = LSV), each clause citing the engine module + parity row + oracle. Add CVA/DVA/
  FVA (celnet-xva) as a quant-risk capability with the honest internal-only/synthetic caveat.
  Re-pitch §13.7: Celnet now matches Murex/Numerix/kACE on structured/path-dependent breadth WHILE
  retaining the open-SDK, microsecond, one-contract, deterministic edges they lack. Keep §13.4
  honest framing (network framing is the floor — no absolute SLO). Update the §13.1 reference to the
  re-rendered fig-12.
- **Must cover:** multi-band catalogue; American/basket/LSV; XVA caveat; the superset pitch; no
  deploy-gated absolutes. Cite POST-COMPLETION-AUDIT Class B, CLIENT-PARITY-MATRIX.md.
- **Parallel-safe:** YES.

### Ch 14 — Engineering Rigor & Assurance
- **Rewrite brief:** §14.1 — add the Heston QuantLib golden table (heston_fo.csv) and touch golden;
  consistent with ch04, state the validation method PER class (frozen QuantLib for
  vanilla/digital/barrier/Heston; closed-form limits + FD + hand-pinned published constants — cite
  the FRTB 0.75ρ circular-oracle defense — + honest stderr for the structured/MC/American/basket/LSV
  set). §14.2 — state the mutation kill-rate gate now spans vanilla + exotics + surface + risk-cube +
  xva (cite .config/mutants-*.toml); add the 6-target fuzz estate (vanilla_inputs, journal_recover,
  proto_convert, replog log-entry/snapshot/wire — no-panic/total-decode/bounded-alloc). §14.3 — cite
  CLIENT-PARITY-MATRIX.md + the ~26 parity rows vs independent oracles. Keep §14.4-§14.7 (accurate;
  non-absolute determinism/perf language).
- **Must cover:** Heston golden + per-class validation; widened mutation; 6 fuzz targets; parity
  matrix. Cite celnet-golden/data/*, .config/mutants-*.toml, fuzz/fuzz_targets/*.
- **Parallel-safe:** YES.

---

## 3. Figure re-authoring briefs (fig-01 … fig-13)

> **Workstream:** edit `_src/<fig>.html` + `_src/diagram-meta.json` (caption + alt + w/h), then
> re-render the PNG (§4), then sync the hub caption + the in-chapter caption. Figures are
> mutually disjoint (one HTML file each) → fully parallel-safe. Keep the brand CSS tokens verbatim.

- **fig-01 system-architecture-adaptability** — OPTIONAL touch. Structurally accurate. Optionally
  reword "partition-map scale-out" → "Raft-replicated scale-out". Low effort.
- **fig-02 api-first-parity** — Add "Unified exotic Instrument (18-products)" + "RiskService roll-up
  + limits" to the contract-carries panel. Smile-selector → "VV·SABR·SVI·SSVI·eSSVI". Expand the
  Excel tile from 8 to ~14-16 chips (exotic fns + RISK/POSITIONS/LIMITS/STATUS) or add a
  "+ exotic & risk fns" tag. Replace "first-generation exotics catalogue" → "full exotic & structured
  catalogue (incl. American & multi-asset basket), reachable from all five clients". Sync caption/alt.
- **fig-03 quant-coverage** — MOST STALE; full re-author. Column 3: 6 rows → full catalogue grouped
  (Vanilla-family: barriers×8+double, digitals, touches/DNT, window | Path-dependent: Asian
  arith/geo, lookback, fwd-start, cliquet, var/vol swap | Structured/MC: TARF, accumulator, quanto |
  Early-exercise: American/Bermudan PSOR-FD+LSM | Multi-asset: basket/best-of/worst-of Cholesky).
  Add a QMC method column (Sobol + Brownian bridge); note GPU path/Greeks. Column 2: add eSSVI +
  Dupire-local-vol + an LSV booking chip. Update the validation line to "~26 parity rows vs
  independent oracles + frozen QuantLib golden tables (vanilla/digital/barrier/Heston)". Label MC
  products (TARF/accumulator/discrete-lookback/basket) as MC w/ stderr, NOT machine-precision.
  Delete every "first-generation exotics catalogue" string.
- **fig-04 surface-pipeline** — Add eSSVI to the smile-model selector (5 models). Add eSSVI to the
  versioned-surface reference. Low effort.
- **fig-05 plugin-tiers** — OVERCLAIM FIX. Render Tier-0 + Tier-2 as solid "Built"; Tier-1 + Tier-3
  with a muted "Designed · deploy-gated" badge (dashed border + grey roadmap tag, same palette) +
  footnote: "Tier-0 native + Tier-2 wasmi sandbox are shipped & gated; Tier-1 signed shared-object
  and Tier-3 Landlock/seccomp are designed-and-seamed, proven at deploy." Update the desk-catalogue
  band to the full catalogue; drop "first-generation". Fix caption/alt so signed-.so/OS-isolated are
  not implied live.
- **fig-06 risk-architecture** — Note exotic positions roll up in the cube (W11). Optionally add an
  XVA tile labelled "CVA/DVA/FVA (synthetic netting; internal)". Add eSSVI to the versioned-surface
  reference. Medium-low effort.
- **fig-07 performance-ladder** — Keep the ladder + "illustrative bar widths" note. Optionally
  annotate the first two steps "measured, M4, in-core" (p50 42ns / p99 125ns) and the GPU side "GPU
  batch ratio up to ~53× on M4 (host-local ratio; absolute NVIDIA throughput deploy-gated)". ADD a
  muted honest-boundary banner (§11 absolute wire SLOs + NVIDIA absolute throughput deploy-gated;
  in-repo = §1.2 truth-gate + loopback + GPU ratios only). Reword caption from "qualitative" to
  "relative latency ladder (in-core measured nanosecond-scale; wire dominates)".
- **fig-08 scaleout** — Upgrade "Replicated state" tile → "Raft-replicated state
  (election·truncation·compaction·InstallSnapshot) + hot-standby takeover"; egress tile → "conflating
  egress governor over a lock-free SPMC fan-out ring (per-pair, exact conflation accounting)". Add a
  "Cross-fleet risk fan-out (HRW; additive merge / non-additive re-gather; fan-out==single-node to
  1e-12)" tile. ADD a muted honest-boundary .note: "Cross-host wire p99, cross-DC transport, dynamic
  membership and §11 absolute wire SLOs are deploy/live-gated; in-repo proofs are
  loopback/localhost-multiprocess."
- **fig-09 celer-lifecycle** — No capability change. Ensure it does not imply the live JVM estate is
  exercised in-repo (integration designed + seamed via DeployMode adapters).
- **fig-10 deployment-modes** — Accurate. Ensure CelerIntegrated is shown as the designed estate-
  native binding (consistent with ch12). Low/no effort.
- **fig-11 streamsession-clicktrade** — Accurate. Optionally add the market-series multiplex flow +
  the Heartbeat observability fields. Low effort.
- **fig-12 capability-landscape** — Update the pricing/analytics cluster to name the shipped tiers
  (first-gen exotics → structured → American/Bermudan → correlated basket → LSV booking) and add an
  XVA/quant-risk node. Keep it a vendor-neutral archetype map (no competitor product names). Re-render;
  update meta caption + alt. Ensure ch02 Fig 12 and ch13 Fig 13.1 captions match.
- **fig-13 engine-concurrency** — OPTIONAL: reword "partition-map scale-out" → "Raft-replicated
  scale-out". Otherwise accurate.
- **excel-grid-branded** — Expand the shown function set from ~8 toward the 27 (or add a "+ exotic &
  risk fns" tag) consistent with fig-02 and ch10.

---

## 4. Figure render command (reproducible — closes the guardrail #10 gap)

There is currently NO committed renderer (verified: no just recipe, no tools/scripts renderer, no
npm script). PNGs were produced ad-hoc via the Playwright MCP. **Land a reproducible recipe with
the figure fixes.**

**Ad-hoc per-figure (Playwright MCP), exact steps:**
1. `browser_navigate` → `file://<absolute-path-to-repo>/docs/assets/celnet-capabilities/_src/<fig>.html` (replace `<absolute-path-to-repo>` with the absolute path of your local checkout)
2. `browser_resize` to the figure's `diagram-meta.json` `w`×`h` (so `.canvas` fills the viewport and
   the Anaheim webfont loads).
3. Wait for `document.fonts.ready`.
4. `browser_take_screenshot` of the **`.canvas` ELEMENT** (not full page) →
   `docs/assets/celnet-capabilities/<fig>.png`.

**Committed reproducible recipe** — add `tools/render-figures.mjs` (uses the already-present
Playwright dep) wired as `just render-figures`:

```js
// tools/render-figures.mjs — render each _src/*.html to its PNG at the meta w×h
import { chromium } from 'playwright';
import { readFileSync } from 'node:fs';
const ROOT = new URL('../docs/assets/celnet-capabilities', import.meta.url).pathname; // repo-relative; adjust if the script moves
const meta = JSON.parse(readFileSync(`${ROOT}/_src/diagram-meta.json`, 'utf8'));
const figs = [...Object.keys(meta), 'excel-grid-branded']; // adapt to meta shape
const browser = await chromium.launch();
for (const name of figs) {
  const { width: w = 1680, height: h = 1000 } = meta[name] ?? {};
  const page = await browser.newPage({ viewport: { width: w, height: h } });
  await page.goto(`file://${ROOT}/_src/${name}.html`);
  await page.waitForFunction('document.fonts.ready');
  await page.locator('.canvas').screenshot({ path: `${ROOT}/${name}.png` });
  await page.close();
}
await browser.close();
```

`just render-figures` recipe (sources the env per the justfile convention):
```
render-figures:
    node tools/render-figures.mjs
```

`shot-01..10` are live GUI/Excel captures — **out of render scope** (re-shoot only if the UI
changed; the eSSVI chip + exotic ticket are already shipped, so shot-02/shot-03/shot-09 may want a
refresh capture but that is a separate manual step).

---

## 5. The new standalone HTML showcase — `docs/celnet-capabilities.html`

**Goal:** ONE self-contained, polished, shareable artifact reflecting the COMPLETE platform + the
honest boundary, openable with zero build step.

**Structure (in order):**
1. **Brand header** — left coral→indigo rail, Anaheim webfont, pinwheel mark (brand-kit viewBox
   `0 0 501 500` path) ONCE in the rail; mark-less "Celnet / a Celer Technologies product"
   wordmark (no traffic-light dots); build-hash · UTC footer.
2. **Executive summary** — the §1 thesis + 4-6 proof cards (catalogue depth, one contract / 5
   clients, nanosecond core, server-side risk, open SDK, honest evidence trail).
3. **Capability map** — fig-12 (re-rendered).
4. **Quant & catalogue coverage** — fig-03 (re-rendered) + the tiered catalogue table.
5. **Architecture + engine** — fig-01, fig-13.
6. **Risk** — fig-06 (+ FRTB/XVA-internal note).
7. **Extensibility** — fig-05 (re-rendered, Tier-1/3 designed) + built/designed tier table.
8. **Performance** — fig-07 (re-rendered, measured M4 numbers + boundary banner).
9. **Scale-out** — fig-08 (re-rendered, Raft + SPMC + cross-fleet + boundary).
10. **API & client parity** — fig-02, fig-11 + a compact CLIENT-PARITY-MATRIX table (18-products ×
    5 surfaces) + the **6-service / 18-product / WS-mirror reference table** and the **client-surface
    reference (27 Excel functions, ~20 SDK methods, 7 CLI commands)**.
11. **Excel reference** — excel-grid + shot-09/shot-10 + the full 27-function list with signatures.
12. **Celer integration** — fig-09, fig-10 (CelerIntegrated = designed estate-native binding).
13. **Competitive positioning** — a table distilled from CAPABILITIES-VS-COMPETITION.md (refreshed).
14. **Screen gallery** — shot-01 … shot-10.
15. **HONEST BOUNDARY** — the verbatim §6 lines, PROMINENT (its own section, not buried).

**Figure embedding:** for the portable single-file sales artifact, inline each PNG as a base64
`data:` URI (`<img src="data:image/png;base64,…">`) so it opens with zero deps. (A lighter in-repo
variant may use relative `<img src="assets/celnet-capabilities/<fig>.png">`.)

**Styling:** inline ALL CSS in `<style>`; reuse the figure CSS tokens VERBATIM so page + figures are
one visual system: `--brand:#ff7357; --brand-deep:#e85d42; --accent:#6b6bf5; --navy:#1B1F2A;
--raised:#282C3E; --muted:#979CB7; --line:#d9deec; --bg:#f4f6fb; --green:#1c8c5a`; Anaheim 400..800
with a system-font fallback if Google Fonts is offline.

**Reproducible build:** add `tools/build-showcase.mjs` wired as `just build-showcase` that
(a) runs `render-figures`, (b) reads the chapter `.md` → HTML, (c) inlines figures as base64,
(d) writes `docs/celnet-capabilities.html`. Closes guardrail #10 (assets stay in sync by construction).

---

## 6. Verbatim honest-boundary disclaimer lines (carry EVERYWHERE — docs + figures + showcase)

These are the canonical lines (from `docs/COMPLETION-PROGRAM.md` §4 / POST-COMPLETION-AUDIT class C).
Quote verbatim; never claim any of these as in-repo-proven.

- **Cross-host wire p99 / kernel-bypass NIC latency** — deploy-gated; in-repo proves the in-core
  §1.2 truth-gate + loopback benches only.
- **CUDA/NVIDIA ABSOLUTE GPU throughput + ≤50ms exotic + Workload-A/B absolute numbers** —
  deploy-gated. M4 Metal lacks f64 ⇒ in-repo proves **correctness + RATIOS only** (M4/Lavapipe).
  Never claim f64 on Metal.
- **The §11 ABSOLUTE wire-latency SLOs** — deploy-gated (in-repo proves the §1.2 truth-gate +
  loopback only).
- **The entire live JVM Celer estate lifecycle** (sidecar / FX_OPTION / inferred hops / tenant
  overlays) — deploy/live-gated; in-repo has the seams + adapters only.
- **Raft §6 dynamic membership / cross-DC transport / real network partitions** — deploy-gated;
  in-repo proves correctness/quorum/framing on localhost multi-process only.
- **Plugin Tier-1 signed-.so (stabby) + Tier-3 Landlock/seccomp** — designed-only; only Tier-0
  native + Tier-2 wasmi are shipped.
- **XVA (CVA/DVA/FVA, celnet-xva)** — internal-only, NO client/wire surface, **synthetic netting
  sets only**; live CSAs/collateral/wrong-way risk are deploy-gated.
- **Multi-source surface aggregation** — the blend/staleness/divergence ALGORITHM is built and
  gated; live multi-vendor quote VALUES are an integration/deploy target, not in-repo data.
- **MC-priced products** (TARF, accumulator, discrete lookback, basket/best-of/worst-of, American
  via LSM) carry a **price std-error** — never labelled "machine-precision"; that bar is reserved
  for analytic/PDE/golden-gated products.

**Strings to DELETE everywhere:** "first-generation exotics catalogue" (false — the catalogue is
full incl. American/Bermudan + multi-asset basket). **Strings to ADD everywhere a smile list
appears:** eSSVI (5 models).

---

## 7. Execution order for the orchestrator

1. **Figures first** (parallel, disjoint HTML files): re-author fig-02/03/05/06/07/08/12 (+ optional
   01/04/09/10/11/13/excel-grid); add `tools/render-figures.mjs` + `just render-figures`; re-render.
2. **Chapters in parallel** (Batch A: 01,02,04,13 · Batch B: 03,05,06,07,08 · Batch C:
   09,10,11,12,14) — each copies the §1 thesis, the §6 boundary lines, and the fixed catalogue/eSSVI
   strings verbatim.
3. **Competition docs** (COMPETITIVE-ANALYSIS.md re-stamp + close gaps; CAPABILITIES-VS-COMPETITION.md
   re-baseline to 34 crates, graduate built rows, label deploy-gated residuals).
4. **Hub last** (sync figure-index captions to the re-rendered figures + add boundary footer).
5. **Showcase** (`tools/build-showcase.mjs` + `just build-showcase` → `docs/celnet-capabilities.html`).
6. lodestar re-indexes automatically via the filesystem watcher (guardrail #10).
