# Celnet — Leadership Execution Program

> The dependency-ordered program to take Celnet to **the world's most performant,
> fully-integrated, intuitive, clean, state-of-the-art FX-options product**. Produced
> by a multi-agent assessment (performance / scalability / functionality / integration /
> experience) + architect synthesis, 2026-06-05. Executed as **gated implement→verify
> waves** — every item below becomes real only when its exact gate is green in
> `just check` / CI; nothing is asserted. The §"Honest boundary" is the canonical
> *never-claim-from-this-repo* set and is reproduced verbatim from the synthesis.

## Where we are (honest headline)

The in-core compute floor (vanilla ~8.85 ns / +14 Greeks ~19–23 ns, zero-alloc lock-free
hot core, AAD adjoint Greeks), the arb-free multi-family surface (VV/SABR/SVI/SSVI +
broker→smile), QuantLib-gated 1st/2nd-gen exotics + LSV, single-node hierarchical risk
(VaR/ES/FRTB-SbM), and the correctness-proven loopback fleet (HRW routing, cross-shard
aggregation, real-gRPC federation, per-node WAL) are **all built and gated** (`celnet-parity`
rows 1–19 + `just check`). Celnet is **not yet world-leading on four measurable fronts, all
fully closable in this repo** (no NVIDIA / cross-host / live-JVM dependency):

1. **Perf is proven only as in-core medians + a relative loopback gate** — no in-core
   p99/p99.9 distribution, no absolute-budget assertion, no `iai-callgrind` instruction gate.
2. **The fleet is correctness-grade, not SLO-grade** — every §11 fleet SLO is unbaselined;
   no replicated log / hot-standby pre-warm / SPMC fan-out; federation fan-out is sequential.
3. **The integration seams are built + self-gated but have zero reverse-deps** — no FIX RFQ
   or vendor-feed loop runs through the live edge; no `DeploymentMode` is bound at boot.
4. **The GUI (most-used client) has zero automated tests**; the CLI lacks risk/stream parity;
   the catalogue misses variance/vol swaps, arithmetic Asian, forward-start/cliquet, eSSVI,
   standalone Heston, Sobol QMC.

## The leadership bar (every item committed as a gate, not asserted)

- **Performance:** in-core pinned price+13-Greek loop emits a HdrHistogram under sustained
  tick injection with **p99 ≤ 10 µs / p99.9 ≤ 25 µs** asserted *absolutely* (ARCH §1.2);
  surface-rebuild (all-tenors, VV/SSVI/eSSVI) **p99 ≤ 150 µs**; committed `iai-callgrind`
  instruction baselines CI-gated; `bench_gate` asserts the §1.2 *absolute* budgets (not just
  2× relative); `wide` SIMD batch bit-identical to scalar with a measured win; GPU perf
  harness (`gpu_load`/`gpu_batch`/`gpu_gate`) headless on M4/Lavapipe with f32↔f64 reconcile.
- **Scale:** every §11 fleet SLO has a committed loopback baseline gated by `bench_gate`
  (cross-shard routing p99 ≤ 25 µs, publish→snapshot lag p99 ≤ 150 µs, ≥1M/s/core under
  fan-out, fan-out tail to 100/1000 subs, conflation-correctness, per-tick-never-crosses-router
  architectural test, **concurrent** federation fan-out, a **replicated** event log with
  deterministic bit-identical cross-node replay, hot-standby pre-warm with measured failover).
- **Functionality:** parity rows extended to eSSVI, variance/vol swaps, arithmetic Asian,
  forward-start/cliquet, standalone Heston (FFT/COS), Sobol+Brownian-bridge QMC (measured
  variance reduction), FRTB-SA DRC/RRAO, + pair-universe breadth — each a gated row.
- **Integration:** `celnet-fix` wired as a live FIX acceptor on `celnet-server` (external
  RFQ→Quote repricing the golden price to 1e-12→fill, stale/forged-token reject); EgressGovernor
  + ResilientSubscriber + vendor adapter bound behind a `CELNET_DEPLOY` knob (Standalone
  default, byte-identical when no feed); an OS-process harness proving vendor-feed→surface→
  FIX-RFQ→fill with 14-Greek parity.
- **Experience:** GUI vitest + Playwright headless e2e + axe-core a11y gates; CLI at full
  four-client parity; saved-views/deep-link state; design corpus reconciled to shipped code.
- **Clean:** zero mocks/placeholders; docs-in-sync; integration ADRs via `manage_adr`;
  lodestar auto-indexes.

## Waves (dependency-ordered, leverage-first)

| Wave | Theme | Key workstreams | Exit gate (summary) |
|---|---|---|---|
| **1** | **Truth gates & integration wiring** (immediate, all buildable here, low conflict) | in-core p99/p99.9 + absolute §1.2 assertion; absolute-budget `bench_gate`; `iai-callgrind` gate; **wire `celnet-fix` live acceptor**; bind `CELNET_DEPLOY` Standalone + EgressGovernor + ResilientSubscriber; **GUI vitest harness** | `just check` green; `core_load` exits non-zero on §1.2 breach; FIX RFQ→Quote(==golden 1e-12)→fill over loopback; Standalone byte-identical + vendor-replay resync; `npm test` green in gui/ |
| **2** | **Fleet SLO truth + experience parity** | full §11 SLO bench suite + baselines; **concurrent** federation fan-out; GUI Playwright e2e + axe; CLI risk/stream parity; `wide` SIMD batch; surface-rebuild p99 bench | `bench_gate` fails on any §11 regression; per-tick-never-crosses-router test; concurrent fan-out ≈ slowest-backend; Playwright+axe green; CLI==SDK==GUI totals |
| **3** | **Distributed correctness: replicated log + hot-standby** (XL) | new `celnet-replog` (leader-replicated/thin-Raft over the journal, deterministic replay); hot-standby pre-warm; new `celnet-fanout` SPMC ring; IB-cardinality federation | kill-leader → follower replays **bit-identical** prices (f64 oracle); standby takeover ≤ target, zero in-flight loss; SPMC at 100/1000 consumers; `just check` green |
| **4** | **Functionality catalogue & surface depth** (parallel to W3) | eSSVI; variance/vol swaps; arithmetic Asian; forward-start/cliquet; standalone Heston FFT/COS; Sobol+bridge QMC; FRTB-SA completeness; pair-universe breadth | each a gated `celnet-parity` row vs QuantLib/quadrature; Sobol variance-reduction ≥3× + CPU↔GPU KAT |
| **5** | **GPU perf at scale** (ratios here, NVIDIA headline gated) | GPU perf harness G1; batch many-instrument (G2) + multi-step path (G3) + payoff kernels (G4); GPU pathwise/LR Greeks (G6) | `gpu_load` headless on M4/Lavapipe; f32↔f64 reconcile; three-way GPU-MC≈CPU-MC≈golden; **NVIDIA absolute headline explicitly deferred** |
| **6** | **Deploy/live-estate proof tracks** (designed+seamed here, proven at deploy; never blocks/claims in-repo) | cross-host/kernel-bypass wire p99; CUDA deploy-gate baselines; live JVM CelNet estate lifecycle; integration ADRs + docs reconcile | deploy-gated; in-repo only the tree/fairness unit tests, the ADRs, and the docs-anchor lint close |

**Dependency notes:** the absolute-budget `bench_gate` (W1) precedes the §11 SLO baselines (W2);
the Standalone edge + `demo_edge` (W1) precede CLI/GUI integration tests (W2); `celnet-replog`
(W3) is a one-way dep → {journal, types} (verify acyclic) and precedes hot-standby + SPMC; W4 is
independent of the fleet waves (parallel to W3) — build Sobol CPU-first then reuse the bridge in
W5; shared `celnet-proto`/`celnet-types` deltas (FX_OPTION projection, eSSVI/var-swap wire enums)
are stabilized first, batched, reviewed via the protobuf skill (guardrail #9, one current contract).

## Wave 1 — immediate increments (each a gated implement→verify increment)

1. **In-core p99/p99.9 + absolute §1.2 budget** — `celnet-bench` `src/bin/core_load.rs` +
   `baselines/core_path.json`: HdrHistogram of the pinned price+13-Greek loop under injected
   ticks; **exits non-zero if p99 > 10 µs or p99.9 > 25 µs**. *(the #1 honesty correction —
   today's headline figures are divan medians.)*
2. **Absolute-budget `bench_gate`** — extend `bench_gate` + wire harness to assert the §1.2
   *absolute* budgets, not just a 2× relative slowdown.
3. **`iai-callgrind` instruction-count gate** — deterministic, jitter-free per-PR perf gate for
   `price()`/`greeks()`/`batch` (the §1.2-named, currently-absent gate).
4. **Wire `celnet-fix` as a live FIX acceptor** — `celnet-server` FIX edge: QuoteRequest→
   `surface_book`→Quote/MassQuote, NewOrderSingle/Multileg→ExecutionReport reusing the
   click-to-trade token path; behind `CELNET_FIX_ADDR`. *(turns a 42-test leaf crate with zero
   reverse-deps into a real venue capability — no new math.)*
5. **Bind `CELNET_DEPLOY` Standalone edge** — `DeploymentMode` at `Edge` boot (Standalone
   default, byte-identical when no feed); `PriceSink`→EgressGovernor; MarketDataSource =
   loopback vendor replay→ResilientSubscriber→normalize→surface. *(mirrors the proven
   `CELNET_FLEET_MODE` knob; composes with #4 for the full feed→price→FIX loop.)*
6. **GUI automated test harness** — `gui/` vitest + @testing-library/react + jsdom (codec
   round-trips, TrendMode, virtual-window math, scope reducer, surface-edit transport).
   *(the highest-traffic client is the only un-gated one.)*

## Honest boundary (the never-claim-from-this-repo set — verbatim)

These CANNOT be proven in this repo and must remain flagged gates, **NEVER claimed done**:
(1) Absolute cross-host wire p99/p99.9 — loopback is an upper bound on compute+framing and a
lower bound on real cross-host wire latency; the counterparty-observed p99 is provable only on a
tuned LAN. (2) Kernel-bypass NIC sub-µs tick-to-trade (DPDK/Onload/VMA/XDP) + eBPF SO_REUSEPORT
handoff — only the transport seam + tree/fairness logic are in-repo. (3) GPU ≤50 ms exotic +
Workload-A/B throughput headline — M4 Metal lacks f64; the production baseline must come from the
CUDA deploy-gate CI; in-repo we prove only correctness + **ratios** on M4/Lavapipe. (4)
isolcpus/nohz_full/rcu_nocbs + mlock + huge-pages p99.9 benefit — realizable only on tuned
bare-metal Linux. (5) The §11 fleet **wire-latency** SLOs under real NIC / inter-DC — loopback
baselines prove routing/conflation/fan-out arithmetic + relative regression, not absolute wire
latency. (6) The entire JVM CelNet estate lifecycle: distributor sidecar handshake + mailbox
drain-rate, FX_OPTION enum/netting/risk/dialect rollout, the 4 inferred orderrouting→risk→
destination→clearing→positionmanager hops (inferred from signatures, NOT runtime traces), tenant
overlays, webtrader — all live-estate-gated; each estate-side hop stays tagged
**[inferred — live-staging gate]**. (7) Live EM/NDF vendor market-data feeds + real entitlement
principals / attribution seat identities — the convention/settlement/pruning *code* is gateable
here, the live data is consumed from the estate. (8) XVA (CVA/FVA/MVA) — designable on synthetic
netting sets vs an analytic benchmark, but firm-wide exposure/collateral/CSA inputs live in the
estate. (9) Sustained ≥1M updates/s/core under real fan-out to high-performance counterparties —
provable in principle on loopback, but the production headline needs real concurrent network
connections; the egress governor's contract is proven against the *documented* CelNet silent-skip
behaviour, not a live JVM distributor.

## Top risks

Overclaim (highest — every gate is phrased as the exact in-repo assertion; the Honest boundary is
the canonical never-claim set); shared-contract churn (stabilize `celnet-proto`/`celnet-types`
deltas first, batch, review via the protobuf skill, keep all four clients in lockstep);
replicated-log scope (CPU-oracle-reconciled multi-process loopback first, thin leader-replicated
before full Raft, never fake a cluster); determinism regression from SIMD/GPU (ship a
bit-identity / f32↔f64-within-bound test *before* claiming any win); GUI e2e flakiness (vitest
unit layer carries correctness, e2e is a thin bounded smoke+a11y gate); catalogue effort
misestimate (each product its own gated row, land one fully before the next — guardrail #2).
