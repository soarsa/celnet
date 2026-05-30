# Celnet — GA-Readiness Synthesis

> **Purpose.** A single, evidence-graded assessment of Celnet against a production /
> general-availability definition-of-done. Each criterion is marked **MET**, **PARTIAL**,
> or **OPEN**, with the concrete test / bench / doc that backs the verdict. The closing
> sections list the remaining must-do work before a GA tag and give an honest go/no-go.
>
> **Snapshot — 2026-05-30.** 19 crates (flat workspace), **38.5k LOC** of source.
> Full suite green and terminating: `cargo build --workspace --all-targets` clean;
> **`cargo nextest run --workspace` → 530 tests run, 530 passed, 0 skipped (20.5 s)**;
> `cargo clippy --workspace --all-targets` zero warnings/errors; `cargo deny check` →
> *advisories ok, bans ok, licenses ok, sources ok*. No `todo!`/`unimplemented!`/mock/
> placeholder in any source file (grep-verified, count = 0).
>
> This doc does not introduce new claims; it consolidates the evidence already in
> `docs/CAPABILITIES-VS-COMPETITION.md`, `docs/REVIEW-REMEDIATION.md`, the golden/parity
> test suites, and the `divan` bench README, and re-verifies the load-bearing ones live.

---

## 1. Definition-of-Done scorecard

| # | GA criterion | Verdict | Evidence |
|---|---|---|---|
| 1 | **Analytics correctness (QuantLib-gated)** | **MET** | `crates/celnet-golden/` — five frozen QuantLib 1.42.1 reference tables (`vanilla_gk.csv` 3200 rows + 6 Greeks, `barrier_gk.csv` 288 / all 8 flavours, `digital_gk.csv` 192, `touch_gk.csv` 1224, `double_barrier_gk.csv` 432), each actively gated (`tests/{vanilla,barrier,digital,touch,double_barrier}_grid.rs`) to ~1e-10. Plus put-call parity (proptest), finite-difference validation of every Greek (`celnet-vanilla/tests`). |
| 2 | **Arbitrage-free surface** | **MET** | `celnet-surface` (3.8k LOC, 11 src) — selectable smile (Vanna-Volga / SABR / SVI raw / SSVI) behind one `VolSurface`; iterative broker→smile strangle calibration (avoids the naive-average "#1 production bug"); static + calendar arb gates (butterfly density ≥ 0, vertical monotonicity, monotone total variance in business time) as a consolidated report. Gated by `tests/surface_properties.rs` and the `celnet-parity` `surface.rs`/`broker_smile.rs` rows. |
| 3 | **Exotics + LSV** | **MET (core); PARTIAL (breadth)** | `celnet-exotics` (7.4k LOC, 15 src): digitals (cash/asset), one-touch/no-touch/DNT, all 8 single-barrier + double-KO closed form + survival-weighted VV overlay; Crank-Nicolson + Rannacher PDE; Philox counter-based MC with Broadie-Glasserman-Kou barrier correction + control variates; Asian (Kemna-Vorst CV) + window/partial barrier (2-D ADI). **LSV booking model** — Andersen-QE variance backbone + particle-calibrated local leverage repricing the arb-free surface (`lsv.rs`), local-vol limit recovers Dupire. Live cross-checks pass: `lsv::tests::window_barrier_pde_matches_mc`, `mc_barrier_bgk_shift_matches_analytic`, `touch::tests::dnt_survival_matches_monte_carlo`, `lsv::calibrates_to_celnet_surface_and_reprices`. **Breadth gap:** no TARF / accumulator / pivot / quanto / lookback (deferred; substrate ready). |
| 4 | **Latency budgets (measured)** | **MET (single-node, micro-bench)** | `crates/celnet-bench` (`divan`) vs `ARCHITECTURE.md` §1.2. Reference run (Apple M4, single core, `bench` profile): vanilla price 40.6 ns; **price + full 13-Greek set 23.4 ns**; 64-strike batch price 3.33 µs (~52 ns/opt); **64-strike batch price+Greeks 6.75 µs (~105 ns/opt)** — the conservative, cache-realistic headline is ~19× inside the 2 µs p50 per-option budget; batch throughput ~9.5× the 1M/s/core target. Bench README is the single source of truth (docs cannot drift from code). **Caveat:** these are micro-benchmark medians, not an end-to-end wire-path p99/p99.9 under load, and not the production-`HdrHistogram`-in-the-loop measurement §1.2 ultimately calls for. |
| 5 | **Determinism** | **MET** | All transcendentals routed through `celnet_core::math` (libm, correctly-rounded, cross-platform-identical); float compares via `is_close`/`assert_close`, never `==`/`!=` (grep-verified, lint-enforced); MC uses bit-reproducible counter-based Philox. `celnet-parity/tests/determinism.rs` asserts bit-identical reprice; CI runs the gate on Linux/macOS/Windows so cross-platform determinism is exercised, not asserted. |
| 6 | **Trader API + SDK** | **MET (gRPC); PARTIAL (WS mirror, v2 ergonomics)** | `celnet-proto` (single current contract, **no version field** — guardrail 9), `celnet-server` (16 src, 5.8k LOC): RFQ lifecycle (two-way quote, last-look, caller idempotency), per-subscription RFS streaming (snapshot + sequenced deltas + heartbeat + exactly-once resync + lag recovery), surface read/mark/scenario, click-to-trade. `celnet-client` typed async SDK with auto gap-detect/resync/reconnect, validated by real-like workflow tests (`rfq_workflow.rs`, `rfs_workflow.rs`, `surface_workflow.rs`) + server-side `rfq.rs`/`rfs.rs`/`rfs_clicktrade.rs`/`surface_scenario.rs`/`readiness.rs`. **Gaps:** WebSocket JSON mirror is designed not built; API-v2 ergonomics (multiplex RFS session, book-shaped risk, surface versioning) captured as a critique, not yet wired (task #20). |
| 7 | **Observability** | **MET** | `celnet-observability` (7 src, 2.0k LOC): tracing + `metrics` + `HdrHistogram` p50/p99/p99.9, lossless audit sink, telemetry over a bounded SPSC ring so the pinned core stays log/lock/alloc-free. **Proven**, not asserted: `celnet-observability/tests/zero_alloc.rs` and `celnet-engine/tests/zero_alloc.rs` use a counting-allocator guard. **Gap (operational):** dashboards/alerts and end-to-end tracing across the server edge are not yet wired into a deployment. |
| 8 | **Competitive parity (beats SynOption)** | **MET as an executable matrix; PARTIAL as a product** | `celnet-parity` — the competitive matrix is *executable*: `tests/{conventions,greeks,surface,broker_smile,exotics,determinism}.rs` turn parity claims into gated tests. `docs/CAPABILITIES-VS-COMPETITION.md` is the honest map. Celnet **out-functions** on user-selectable arb-free smiles, particle-calibrated LSV, GPU MC as a service, published microsecond latency, zero-downtime upgrades — none of which any single incumbent combines. **Where SynOption still leads:** ~75 pairs + crypto vs Celnet's ~9 G10 + few EM NDFs; structured products (TARF/accumulator); a shipping trader GUI; an enforced multi-bank RMO venue (a deliberate non-goal). |
| 9 | **Supply-chain clean** | **MET** | `cargo deny check` green (advisories/bans/licenses/sources). OSS / permissively-licensed deps only; wasmtime **excluded** by policy for open 2026 RUSTSEC advisories (documented in `Cargo.toml` and `PLUGIN-HOST-ALT.md`). `cargo-audit` + cargo-deny gate the whole tree in CI; fuzz lane (cargo-fuzz, nightly, time-boxed) exercises `vanilla_inputs`. **Mutation testing**: `cargo-mutants` → 412/466 caught (~88%), 54 missed (all in `celnet-vanilla` delta/solver comparison-operator and arithmetic mutants — a backlog, not a correctness regression). |
| 10 | **Docs in sync (no overclaim)** | **MET** | A full adversarial audit (98 findings) + remediation closed the doc-vs-code drift: latency claims corrected to verified bench numbers, seqlock UB fixed, Philox naming corrected, versioned-protocol drift removed, exotics under-claims fixed (see `REVIEW-REMEDIATION.md`, commits `c558eab`/`e824194`). `CAPABILITIES-VS-COMPETITION.md`, `SCALE-OUT.md`, and `PLUGIN-HOST-ALT.md` each open with an explicit "built today vs designed" split. Bench README is the single source of latency truth. |
| — | **Plugin / SDK runtime (sandbox host)** | **OPEN** | `celnet-plugin-api` (WIT + traits) is **built and frozen**, but **`celnet-plugin-host` does not exist** (no crate). The headline "open quant SDK" differentiator is contract-only. Decision is made and ADR-grade (`PLUGIN-HOST-ALT.md`): tiered host = native trait registry + **`wasmi` v1.0.9** (pure-Rust, fuel-metered, twice-audited) sandbox + signed-native `stabby` partner tier + optional Landlock/seccomp ring. Not yet implemented. |
| — | **Trader GUI** | **OPEN** | `docs/GUI-DESIGN.md` is a complete design spec (Apple-grade, out-intuit SynOption Optimus) but **no GUI crate/app exists**. Designed-only. |
| — | **Distributed / horizontal scale-out** | **PARTIAL** | Single-shard substrate exists and is validated (engine hot path, server edge). The cross-node fleet layer (router tier, HRW partition map, replicated log, hot-standby) is **designed, not built** — no `celnet-router`/`celnet-cluster` crate, no cross-node routing/consensus code (`SCALE-OUT.md` §0). IB-portfolio scale is argued from per-node throughput headroom, not demonstrated across a fleet. |
| — | **Celer estate / FIX integration** | **PARTIAL** | `celnet-integration` (5 src, 2.1k LOC): FMD-style ATM/RR/BF normalization through the convention layer + multi-source aggregation with time-weighted staleness decay and divergence detection. FIX STP and live Celer trade-lifecycle wiring are **mapped** (`CELER-INTEGRATION.md`) but not wired to a running estate. |
| — | **GPU backend** | **MET (functional); PARTIAL (perf-at-scale)** | `celnet-gpu` (5 src, 1.3k LOC): wgpu (Metal/Vulkan/DX12) MC with f64 CPU reconciliation oracle and bit-stable Philox. CI exercises the Vulkan path headless via Mesa lavapipe (software). **Gap:** no on-GPU large-batch latency/throughput numbers vs the §1.2 ≤ 50 ms booking-grade budget on real hardware (Metal lacks f64; the CUDA path is CI-only). |

---

## 2. What is solidly GA-grade today

- **The numerical core.** Vanilla + 13 Greeks, first-generation exotics, and the arb-free
  surface are QuantLib-gated to ~1e-10, finite-difference-validated, property-tested, and
  mutation-tested at ~88% kill. This is the part a bank's IPV/model-validation function would
  sign first, and it is the part most ready.
- **Determinism + supply chain.** libm-routed, `is_close`-only, Philox-reproducible, gated
  on three OSes; cargo-deny/audit/fuzz clean. The hardest-to-retrofit properties are in place
  from day one.
- **The hot path + edge service.** Zero-alloc pinned core (allocator-guard-proven), blue-green
  zero-downtime handoff, a single-current typed gRPC contract with a tested client SDK, and
  zero-cost observability. The single-node service is real and exercised.
- **Honesty.** Every doc separates built from designed; the audit→remediation loop demonstrably
  catches and fixes overclaim. This is itself a GA asset.

---

## 3. Remaining must-do before a GA tag

**Blockers (a GA tag should not ship without these):**

1. **Build `celnet-plugin-host`** on the chosen `wasmi` tiered architecture
   (`PLUGIN-HOST-ALT.md`): fuel-metered sandbox, capability isolation (no ambient authority),
   deterministic replay harness (R1–R5), native + signed-partner tiers behind one
   `ModelRegistry`. Until this ships, the headline "open quant SDK" differentiator is
   contract-only. *(Tracked task #10/#22.)*
2. **Build the trader GUI** to `GUI-DESIGN.md` (the `frontend-design` lane), consuming the
   existing `celnet-client` SDK / wire contract. FIX STP and a shipping GUI are table-stakes
   the matrix marks as gaps vs incumbents. *(Task #23.)*
3. **End-to-end latency under load.** Replace the micro-bench medians with a wire-path
   p50/p99/p99.9 measurement (production `HdrHistogram` in the loop, sustained streaming load),
   proving §1.2 budgets through the server edge, not just in-core. Add a CI bench-regression
   gate (criterion/iai-callgrind baseline) so latency cannot silently regress.

**Should-do (raises GA from "single-node service" to "platform"):**

4. **WebSocket JSON mirror** of the gRPC contract (designed; required for browser/GUI parity)
   and **API-v2 ergonomics** (multiplex RFS session, book-shaped risk, surface versioning) —
   task #20.
5. **Cross-node fleet layer** — at minimum a router tier + HRW partition map + hot-standby,
   validated against §1.2 budgets, to substantiate the IB-portfolio scale claim beyond
   per-node headroom (`SCALE-OUT.md`; task #19).
6. **GPU perf-at-scale** numbers on real hardware against the ≤ 50 ms booking-grade budget.
7. **Live Celer / FIX integration** wired to a (staging) estate, not just mapped.
8. **Close the 54 missed mutants** in `celnet-vanilla` delta/solver (comparison-operator and
   arithmetic mutants) and stand up coverage + mutation as CI gates with thresholds.

**Nice-to-have (post-GA roadmap):**

9. Structured products (TARF / accumulator / pivot) and quanto / lookback breadth on the
   existing LSV+MC+PDE substrate; pair/asset-class breadth toward incumbent coverage.

---

## 4. Honest go / no-go

**Verdict: NO-GO for a full-platform GA tag today; GO for a scoped "pricing-core + single-node
service" GA (call it a controlled / limited availability).**

The numerics, determinism, supply chain, single-node hot path, typed API + SDK, and
observability are GA-grade and evidence-backed. What blocks a *full* GA is that two of the
product's headline differentiators are not yet code — the **open quant SDK runtime**
(`celnet-plugin-host`, decided but unbuilt) and the **trader GUI** (designed-only) — and the
latency story, while excellent in micro-benchmarks, is **not yet measured end-to-end under
load** with a CI regression gate. The scale-out, GPU-at-scale, and live-estate-integration
stories are designed/partial, acceptable as post-GA roadmap but not claimable as GA.

Recommended path to a full GA tag, in order: (1) `celnet-plugin-host` on `wasmi`; (2)
end-to-end latency measurement + CI bench gate; (3) the trader GUI; (4) WS mirror + API-v2;
then tag. Items 5–9 are explicit post-GA roadmap. None of the blockers is research-risk — each
has a decided design and a ready substrate — so the path to GA is execution, not discovery.
