# Celnet — Capabilities vs Competition

**Status as of 2026-06-08 · 34 crates · full suite green via `just check` (fmt · clippy `-D` ·
nextest · cargo-deny) · the complete platform: full exotic & structured catalogue, 5 smile
families, server-side firm risk, FRTB-SA, distributed-correctness substrate, open quant SDK.**

> **Celnet is not a thin challenger closing gaps — it is a functionally complete, evidence-backed
> superset of what a derivatives desk stitches together today, proven by a runnable parity matrix
> against independent oracles, behind one unversioned contract reachable identically from five
> clients.**

This document maps Celnet's **actually-built, test-verified** capabilities against the FX-options
incumbents, and states the deploy-gated residuals honestly. Every "✅ built" row is backed by
passing tests in the repo (cited by crate/path); "🟡 designed" means a committed design + seam but
not in-repo-proven; "⛔ deploy-gated" means correctness is proven in-repo but the absolute claim is
proven only at deploy (reason noted). No capability is claimed that isn't in the code.

> **Legend** — ✅ built & test-verified · 🟡 designed/seamed (not in-repo-proven) ·
> ⛔ deploy-gated (in-repo proves correctness/ratios/loopback; absolute proven at deploy) ·
> ✱ deliberate non-goal.

---

## 1. Executive summary

The FX-options stack splits into four archetypes, none of which combines a complete exotic &
structured catalogue, open extensibility, true microsecond-class latency, zero-downtime upgrades,
native trade-lifecycle integration, and GPU acceleration in one product:

| Archetype | Examples | Core limitation Celnet exploits |
|---|---|---|
| Closed desktop terminal | Bloomberg OVML / BVOL / MARS | Seat-licensed, closed analytics, no embeddable microsecond API, convention opacity |
| Front-to-back platform | Murex MX.3, Numerix CrossAsset | Heavy, multi-year upgrades, batch/EOD risk, costly, no open quant SDK |
| Data / venue player | Fenics (FMD FXO 2.0, kACE), SynOption, 360T, Digital Vega | RFQ/seconds-scale, closed construction, **no customer-extensible quant SDK**, no typed microsecond streaming contract |
| Modern entrant | Quantifi, RustQuant, libraries | Library, not a low-latency service; no estate integration |

**Celnet's seam:** a Rust, microsecond-class, **open** FX-options engine carrying a **complete
catalogue** — vanilla → the full first-generation exotics → structured & path-dependent products →
American/Bermudan early exercise → correlated multi-asset basket → an LSV booking model + standalone
Heston — over an **arbitrage-free surface (5 smile families incl. eSSVI)**, with GPU acceleration, a
**trader-shaped typed gRPC contract + byte-identical WebSocket mirror** reachable from a client SDK,
CLI and Excel, server-side hierarchical risk + FRTB-SA, a sandboxed open quant SDK, zero-downtime
blue-green upgrades, zero-cost observability, and seamed Celer trade-lifecycle integration — every
numeric row gated against an independent oracle (QuantLib 1.42.1 / closed-form limits / FD /
hand-pinned published constants / honest MC std-error bands).

> **Three pillars (each grounded):**
> 1. **Catalogue depth that matches the front-to-back platforms.** Vanilla → the full
>    first-generation exotics → structured & path-dependent (var/vol swaps, Asians,
>    forward-start/cliquet, quanto, TARF, accumulator, lookback) → American/Bermudan early exercise
>    → correlated multi-asset basket/best-of/worst-of → a particle-calibrated LSV booking model +
>    standalone Heston — all on the **one wire** (`celnet.proto` `Instrument` oneof, 18-product
>    arms), parity-gated (`celnet-parity`), reachable from **all five clients**. This is the breadth
>    Murex MX.3 / Numerix CrossAsset / Fenics kACE charge for, delivered open and microsecond-class.
> 2. **Edges the deep-catalogue incumbents structurally lack:** an open quant SDK (run private IP
>    in-engine, sandboxed and deterministic); one clean unversioned contract with bit-identical
>    values across GUI / SDK / CLI / Excel / WS; a pinned zero-alloc nanosecond hot core;
>    server-side hierarchical risk (clients never loop-and-sum); and an honest evidence trail
>    (runnable parity matrix + frozen golden tables + mutation + fuzz).
> 3. **Honesty as a differentiator.** Every Celnet figure is labelled (in-core / M4 / loopback);
>    every competitor claim is a stated inference; deploy-gated absolutes are never claimed in-repo.
>    A reviewer doing diligence finds the proof, not marketing fiction.

---

## 2. Capability inventory (built & test-verified)

### Pricing & conventions
- ✅ **Garman-Kohlhagen vanilla + price + the full 13-Greek set** (delta spot, delta forward, gamma,
  vega, theta, two rhos, vanna, volga, charm, speed, zomma, color) — every Greek
  finite-difference-validated and **gated against QuantLib 1.42.1 to ~1e-10** (`celnet-vanilla`;
  golden `crates/celnet-golden/data/vanilla_gk.csv`). Reverse-mode AAD adjoint Greeks available
  (`celnet-vanilla::adjoint_greeks`), gated to the analytic block and a central-FD oracle.
- ✅ **Full convention correctness as first-class data**: all four delta conventions
  (spot/forward × premium-adjusted/unadjusted), ATM-forward & delta-neutral-straddle, premium
  styles, NY-10am / Tokyo-3pm cut, deliverable & NDF settlement, a real **business-day calendar /
  spot-lag / delivery engine** (`celnet-calendar`, modified-following + EOM). Convention travels on
  **every priced message** — the single biggest correctness edge over Bloomberg/Fenics, where
  convention ambiguity causes real mismarks.
- ✅ **Convention-aware strike↔delta solver** (guarded for the non-monotone premium-adjusted call
  delta) — the load-bearing primitive competitors treat as opaque internal machinery.
- ✅ **Documented 19-pair universe** — 7 G10 majors, EM deliverable crosses, EM NDF/NDO USD-cash
  fixings, precious metals XAU/XAG (`celnet-conventions` / `celnet-calendar`; parity
  `pair_universe.rs` resolves conventions == published EMTA/ISDA tables).

### Volatility surface
- ✅ **Five selectable, arbitrage-free smile families** behind one `VolSurface`: Vanna-Volga
  (market-hedge), SABR (stochastic-vol), SVI (parametric), SSVI (parametric surface), and
  **eSSVI / extended surface** (maturity-dependent correlation, `SMILE_MODEL_EXTENDED_SURFACE=4`,
  `celnet-surface/extended_surface.rs`; parity `essvi.rs` + `essvi_hardening.rs`).
  **No competitor lets the user choose the parameterization.**
- ✅ **Broker → smile strangle calibration** (iterative, not the naive arithmetic average — the
  documented "#1 production bug") so the smile reprices broker ATM/RR/BF exactly; regression-tested
  on a high-RR EM case (parity `broker_smile.rs`).
- ✅ **Static + calendar arbitrage gates** (butterfly density ≥ 0, vertical monotonicity, monotone
  total variance in business time) exposed as a consolidated report; continuously re-strikable.
  Auditable arb-freeness is a genuine IPV/FRTB differentiator.

### Exotics & structured catalogue (the complete catalogue, parity-gated)
- ✅ **First-generation exotics**: European digitals (cash/asset), one-/no-/double-no-touch, all
  eight single-barrier flavours + double-KO + **window barrier**, closed-form + a survival-weighted
  Vanna-Volga overlay (`barrier.rs`/`digital.rs`/`touch.rs`/`market_hedge_overlay.rs`). **Barriers
  & digitals & touches gated against QuantLib** (golden `barrier_gk.csv`, `double_barrier_gk.csv`,
  `digital_gk.csv`, `touch_gk.csv`).
- ✅ **Closed-form structured**: variance & volatility swaps (log-contract static replication /
  Carr-Lee convexity, `var_swap.rs`/`vol_swap.rs`; parity `var_vol_swap.rs`), arithmetic Asians
  (Turnbull-Wakeman + Curran, `asian.rs`; parity `asian.rs`), forward-start & cliquet (Rubinstein,
  `forward_start.rs`; parity `forward_start.rs`), quanto (`quanto.rs`), lookback (floating/fixed,
  `lookback.rs`).
- ✅ **Monte-Carlo / path-dependent** — each carries a **price std-error** (never "machine
  precision"): TARF (`tarf.rs`), accumulator (`accumulator.rs`), discrete lookback, and
  **correlated multi-asset basket / best-of / worst-of** over N pairs by Cholesky-correlated GBM MC
  on the scrambled-Sobol / Brownian-bridge engine (`multiasset.rs`; parity `basket.rs`, `exotics.rs`,
  `structured.rs`). Philox counter-based RNG, BGK barrier correction, control variates;
  PDE ≈ MC ≈ analytic cross-validated.
- ✅ **Early exercise (American / Bermudan)**: projected-SOR free-boundary finite difference
  (default) **and** Longstaff-Schwartz regression Monte-Carlo (`american.rs`; parity via
  `exotics.rs`). LSM path carries a std-error.
- ✅ **LSV booking model**: stochastic-variance backbone (Andersen QE) + **particle-calibrated local
  leverage** repricing the arb-free surface on a 2-D ADI PDE; local-vol limit recovers Dupire
  (`lsv.rs`/`adi.rs`/`particle.rs`/`leverage.rs`; parity `lsv.rs`). **Standalone Heston**
  (`celnet-heston`: Carr-Madan + Fang-Oosterlee COS over a branch-cut-free CF) gated to the **frozen
  QuantLib Heston golden** (`heston_fo.csv`; parity `heston.rs`).
- ✅ **Sobol + Brownian-bridge QMC** (`celnet-qmc`: gray-code Joe-Kuo Sobol, Owen scramble,
  principal-bisection bridge) — **measured** variance reduction ≈37.7× (geometric Asian) / ≈88.5×
  (European) vs fair plain MC (parity `qmc.rs`, `qmc_highdim.rs`).

### Engine, GPU, latency
- ✅ **Core-pinned, zero-allocation, lock-free hot path** (wait-free SPSC rings, arc-swap/seqlock
  publication, CachePadded) with **blue-green zero-downtime state handoff** + a standalone durable
  WAL with crash recovery (`celnet-engine`, `celnet-journal`).
- ✅ **Cross-platform GPU acceleration** (wgpu → Metal/Vulkan/DX12) with an f64 CPU reconciliation
  oracle and bit-stable counter RNG — a live-service capability **no competitor exposes**:
  a multi-step path kernel (`path.wgsl`/`path.rs`), pathwise/LR Greeks (`greeks.wgsl`/`pathwise.rs`;
  parity `gpu_greeks.rs`, `gpu_path.rs`), a batch closed-form Garman-Kohlhagen kernel (`batch.wgsl`),
  and a spot×vol scenario grid (`scenario.wgsl`). Reconciled GPU-f32 ≈ CPU-f64 ≈ golden element-wise.
  ⛔ **M4 Metal lacks f64 ⇒ in-repo proves correctness + RATIOS only.** Measured dispatch-amortization
  RATIO 0.68×@4k → 53×@1M paths (M4, host-local ratio). **NVIDIA absolute throughput / ≤50ms exotic /
  Workload-A/B absolutes are deploy-gated, never claimed in-repo.**
- ✅ **Measured §1.2 in-core truth-gate** (`crates/celnet-bench/src/bin/core_load.rs`,
  HdrHistogram): asserts **p50≤2µs / p99≤10µs / p99.9≤25µs ABSOLUTE** on the pinned price+13-Greek
  loop; measured on the **M4 dev host (host-local, single-core)** ~42ns / 125ns / ~1µs — a 24–80×
  margin. iai-callgrind instruction-count gate on the Linux CI lane. **The only published,
  reproducible FX-option latency figures among this competitor set** — labelled in-core/M4, never
  converted to a wire number. ⛔ **Absolute cross-host wire p99 / kernel-bypass NIC / §11 absolute
  wire SLOs are deploy-gated** (in-repo proves the §1.2 in-core gate + loopback benches —
  `wire_load.rs`, `fleet_slo.rs` — only).

### API, clients, integration, observability
- ✅ **One unversioned trader-shaped wire contract** — `celnet.proto`, **5 gRPC services**
  (PricingService.Price; QuoteService.{RequestQuote, AcceptQuote, RejectQuote};
  StreamService.StreamSession; RiskService.{ListPositions, AggregateRisk, DrillRisk, LimitStatus};
  SurfaceService.{GetSmile, MarkSurface, Scenario}) over a **unified `Instrument` (18-product arms)**
  + a **byte-identical WebSocket JSON mirror** of the same contract (`celnet-server`). Each MC
  product reports `price_std_error`. Multiplexed RFS `StreamSession` (snapshot + sequenced deltas +
  heartbeat + exactly-once resync + lag recovery + click-to-trade keyed-MAC tokens), market-series
  feed, and Heartbeat observability (conflation drops + server-price HdrHistogram percentiles).
- ✅ **Five clients, bit-identical, in lockstep** — GUI (React/WebGPU; eSSVI chip + full exotic
  ticket incl. American & basket), `celnet-client` SDK (typed async, auto gap-detect/resync/
  reconnect), `celnet-cli` (price/surface/exotic/basket/convention/risk/stream), and **27 Excel
  `CELNET.*` functions** (PRICE, GREEKS, SURFACE, MARKSURFACE, MARK, RFQ, SUBSCRIBE, SERIES, BARRIER,
  WINDOWBARRIER, DIGITAL, TOUCH, VARSWAP, VOLSWAP, ASIAN, FORWARDSTART, CLIQUET, QUANTO, TARF,
  ACCUMULATOR, LOOKBACK, AMERICAN, BASKET, RISK, POSITIONS, LIMITS, STATUS — `excel/src/functions/
  functions.ts`). Proven by the runnable **`docs/CLIENT-PARITY-MATRIX.md`**.
- ✅ **Server-side hierarchical firm risk** — `RiskService` rolls entitled position-facts over an org
  dimension into netted additive + re-derived non-additive (VaR/ES/curvature) measures in a reporting
  numeraire, entitlement-pruned, with limit RAG; **clients never loop-and-sum**
  (`celnet-risk-cube` / `-normalize` / `-limits` / `-entitlements`; cross-shard fan-out
  `celnet-risk-fleet`, fan-out == single-node to 1e-12). Aggregates **exotic legs**
  (`celnet-risk-cube/exotic.rs`; parity `exotic_risk_cube.rs`).
- ✅ **Regulatory capital — FRTB-SA** (`celnet-risk-cube/frtb.rs`; parity `frtb.rs`): SbM within/
  cross-bucket K_b, the three correlation scenarios → max with the **MAR21.6 0.75ρ low-corr floor**
  (hand-pinned BCBS constants — the circular-oracle defense), curvature, RRAO, honest DRC=0 for
  deliverable FX.
- ✅ **Counterparty valuation adjustments — XVA** (`celnet-xva`: CVA/DVA/FVA over EPE/ENE; parity
  `xva.rs`). ⛔ **INTERNAL-ONLY: no client/wire surface, synthetic netting sets only** — live CSAs /
  collateral / wrong-way risk are deploy-gated. (Verified: `celnet.proto` exposes no XVA RPC.)
- ✅ **Distributed-correctness substrate** — a leader-replicated event log with **full Raft**
  (election + Pre-Vote, conflicting-tail truncation, snapshot compaction, **InstallSnapshot** reseed)
  proven **bit-identical (`f64::to_bits`)** on real loopback-socket multi-node replication
  (`celnet-replog`; parity `raft_election.rs`, `raft_compaction.rs`, `raft_snapshot.rs`); a lock-free
  **SPMC broadcast ring under the edge** fanning each pair's tick to all subscribers, zero-alloc with
  exact skip-accounting conflation (`celnet-fanout`, wired `celnet-server/src/services/
  pricefanout.rs`). ⛔ **Cross-host wire p99 / cross-DC transport / real partitions / Raft §6 dynamic
  membership are deploy-gated** (in-repo proves correctness/quorum/framing on localhost
  multi-process only).
- ✅ **Customer-extensible quant SDK** — `celnet-plugin-api` (WIT + traits) frozen + the
  `celnet-plugin-host` ships **two tiers**: Tier-0 native registry + Tier-2 **wasmi** fuel-metered,
  no-WASI, deterministic sandbox behind one registry with a bit-identical replay harness (all four
  WS-G gates green). wasmtime was rejected for open RustSec advisories; wasmi is the advisory-clean
  pure-Rust replacement (`docs/PLUGIN-HOST-ALT.md`). 🟡 **Tier-1 signed-`.so` (stabby) + Tier-3
  Landlock/seccomp OS-sandbox are DESIGNED-ONLY** — seamed to slot behind the same frozen contract,
  not shipped in-repo.
- ✅ **Seamed Celer trade-lifecycle integration** — FIX 4.4 engine (acceptor + initiator,
  loopback-tested, `celnet-fix`), egress governor / resilient subscriber / normalization
  (`celnet-integration`), and three adapter-bound deployment modes (`celnet-server/services/
  deploy.rs`). ⛔ **The live JVM Celer estate lifecycle** (distributor sidecar handshake / mailbox
  calibration / live quote-feed entitlement) is **deploy/live-gated** — in-repo has the traits +
  adapter swap + FIX loopback only, not a live-estate connection.
- ✅ **Vendor market-data integration** — FMD-style ATM/RR/BF normalization through the convention
  layer + **multi-source aggregation with time-weighted staleness decay and divergence detection**.
  ⛔ The blend/staleness/divergence ALGORITHM is built and gated; **live multi-vendor quote VALUES
  are an integration/deploy target**, not in-repo data.
- ✅ **Zero-cost observability** — tracing + metrics + HdrHistogram p50/p99/p99.9, with the hot core
  staying log/lock/alloc-free (telemetry over a bounded SPSC ring; **proven zero hot-path
  allocations** by a counting-allocator guard) + a separate lossless audit sink (`celnet-observability`).

---

## 3. Feature comparison matrix

| Capability | **Celnet** | SynOption | Fenics (FMD/kACE) | Bloomberg (OVML/BVOL) | Murex / Numerix |
|---|---|---|---|---|---|
| Vanilla + price + 13 Greeks | ✅ QuantLib-gated, FD-validated, AAD adjoint | ✅ (closed) | ✅ kACE (closed) | ✅ OVML (closed) | ✅ |
| Convention transparency (4 delta / ATM / cut / NDF, on every msg) | ✅ first-class, documented | ⚠️ hidden | ⚠️ no public spec | ⚠️ ambiguous | ⚠️ |
| User-selectable smile model (VV/SABR/SVI/SSVI/**eSSVI**) | ✅ 5 families | ❌ single closed | ❌ single closed | ❌ closed | ❌ |
| Broker→smile calibration (exact reprice) | ✅ tested | ⚠️ opaque | ⚠️ opaque | ⚠️ opaque | ⚠️ |
| Auditable arbitrage-free guarantee | ✅ butterfly+calendar+vertical | ⚠️ heuristics | ❌ ML fill, no guarantee | ⚠️ closed | partial |
| First-gen exotics (digital/touch/DNT/barrier/window) | ✅ QuantLib-gated | ✅ (closed) | ✅ kACE | ✅ | ✅ |
| Structured (var/vol swap, Asian, fwd-start/cliquet, quanto, lookback) | ✅ closed-form, parity-gated | ✅ | ✅ kACE | ✅ MARS | ✅ |
| Path-dependent MC (TARF, accumulator) | ✅ MC w/ std-error, gated | ✅ | ✅ | ✅ MARS | ✅ |
| Correlated multi-asset (basket/best-of/worst-of) | ✅ Cholesky MC w/ std-error (`multiasset.rs`) | ⚠️ | ⚠️ kACE | ⚠️ | ✅ |
| American / Bermudan early exercise | ✅ PSOR FD + Longstaff-Schwartz LSM (`american.rs`) | ⚠️ | ✅ kACE | ✅ | ✅ |
| LSV booking model + standalone Heston | ✅ particle-calibrated ADI + Heston golden-gated | ⚠️ closed | ⚠️ closed | ⚠️ closed | ✅ |
| Sobol + Brownian-bridge QMC | ✅ ~38×/88× measured variance reduction | ⚠️ | ⚠️ | ⚠️ | ✅ |
| GPU-accelerated MC/PDE/Greeks as a service | ✅ wgpu + CPU reconcile (ratios; abs. deploy-gated) | ❌ | ❌ | ❌ | ❌ |
| Microsecond pricing, **published in-core numbers** | ✅ ~42ns p50 / 125ns p99 price+13-Greeks (M4, in-core; §1.2 gate) | ❌ none | ❌ snapshot feed | ❌ terminal | ❌ batch/EOD |
| One unversioned typed gRPC + WS-mirror contract | ✅ 5 services, byte-identical WS | ❌ FIX/UI | ❌ data feed | ❌ BLPAPI/terminal | ❌ |
| RFQ lifecycle + caller idempotency | ✅ | ✅ venue | ⚠️ | ⚠️ | ✅ |
| Quote-accept → execution booking | ✅ idempotent | ✅ venue | ⚠️ | ⚠️ | ✅ |
| Multiplexed RFS streaming + resync + click-to-trade | ✅ | ⚠️ indicative | ❌ | ❌ | ❌ |
| Five-client bit-identical parity (GUI/SDK/CLI/Excel/WS) | ✅ CLIENT-PARITY-MATRIX | ❌ | ❌ | ❌ | ❌ |
| Server-side hierarchical firm risk (no client loop-and-sum) | ✅ RiskService + cross-shard fan-out | ⚠️ | ❌ | ⚠️ | ✅ batch |
| Regulatory capital FRTB-SA (SbM + curvature + RRAO) | ✅ BCBS-pinned (`frtb.rs`) | ❌ | ❌ | ⚠️ MARS | ✅ |
| XVA (CVA/DVA/FVA) | ✅ internal-only, synthetic netting (no wire) | ❌ | ❌ | ⚠️ | ✅ |
| Customer-extensible quant SDK (own models, sandboxed) | ✅ Tier-0 native + Tier-2 wasmi (Tier-1/3 designed) | ❌ | ❌ | ❌ | ❌ |
| Zero-downtime (blue-green) hot upgrade | ✅ | n/a SaaS | n/a | n/a | ❌ multi-year upgrades |
| Distributed correctness (Raft replicated log + SPMC fan-out) | ✅ bit-identical replay, loopback-proven (abs. wire deploy-gated) | n/a SaaS | n/a | n/a | ⚠️ |
| Multi-source surface aggregation + divergence | ✅ algorithm (live feed VALUES = integration target) | ✅ Primus (broad LP panel) | partial | ❌ | ⚠️ |
| Asset-class / pair breadth | ⚠️ 19-pair universe (G10 + EM NDF + XAU/XAG; no crypto) | ✅ 75 pairs + crypto | ✅ 300+ pairs + 27 metals (data) | ✅ 200+ pairs | ✅ |
| Zero-cost observability (HdrHistogram, audit) | ✅ proven | ⚠️ | ⚠️ | ⚠️ | ⚠️ |
| Native Celer trade-lifecycle integration | ✅ seamed (FIX 4.4 + DeployMode adapters); ⛔ live JVM estate deploy-gated | ❌ | ❌ | ❌ | n/a |
| On-prem / in-process embeddable | ✅ Rust crates + service | ❌ SaaS/venue | feed only | ❌ terminal | on-prem heavy |
| Regulated multi-bank RFQ venue (RMO) | ✱ non-goal (sit alongside) | ✅ Optimus | ❌ | ❌ | ❌ |
| FIX connectivity | ✅ FIX 4.4 engine (loopback-tested); live estate deploy-gated | ✅ | ✅ | ✅ | ✅ |

---

## 4. How Celnet out-functions / out-intuits / out-performs

- **Out-functions** — a **complete exotic & structured catalogue** (first-gen → structured →
  American/Bermudan → correlated multi-asset basket → LSV booking + Heston) on one wire, five
  selectable arbitrage-free smile families incl. eSSVI, GPU acceleration as a live service, FRTB-SA
  capital, and an open sandboxed quant SDK: a breadth-plus-openness combination no single incumbent
  offers, all behind one typed contract reachable identically from five clients.
- **Out-intuits** — convention transparency on every message, an auditable arbitrage report, a
  broker→smile calibration that reprices the desk's own quotes exactly, server-side hierarchical risk
  the desk reads 1:1, and a trader-shaped RFQ/RFS/click-to-trade API — versus convention-opaque,
  closed-construction terminals where clients loop-and-sum their own risk.
- **Out-performs** — a core-pinned, zero-allocation Rust hot path with a **published, reproducible
  §1.2 in-core truth-gate** (price + 13 Greeks ~42ns p50 / 125ns p99 on M4, gated absolute) and
  blue-green zero-downtime upgrades, versus RFQ/seconds-scale, terminal, or batch/EOD architectures
  with no latency numbers and painful upgrade cycles. (In-core, host-local — the absolute wire p99
  and NVIDIA GPU throughput are deploy-gated, never claimed here.)

---

## 5. Verification posture (why the claims hold)

- **Full suite green & terminating via `just check`** (fmt · clippy `-D warnings` · nextest ·
  cargo-deny); the exact test count is whatever `cargo nextest run` reports on the day, not a
  hard-coded integer. Numerics validated **per product class** (the honest split):
  - **Frozen QuantLib 1.42.1 golden tables** — vanilla, both digital styles, all 8 barrier flavours,
    double-barrier, touch, **Heston** (`crates/celnet-golden/data/`), to ~1e-10.
  - **Closed-form limits + finite-difference + hand-pinned published constants** — structured,
    Asian, forward-start/cliquet, swaps, FRTB-SA (BCBS constants — the 0.75ρ circular-oracle
    defense), eSSVI butterfly/calendar.
  - **Honest MC std-error bands** — TARF, accumulator, discrete lookback, basket/best-of/worst-of,
    American via LSM. These are **never** labelled "machine precision"; that bar is reserved for
    analytic/PDE/golden-gated products.
- **~26 parity rows vs independent oracles** (`celnet-parity/tests/`) — each capability is a runnable
  test against a code-disjoint oracle, not a self-check.
- **Mutation + fuzz + the CLIENT-PARITY-MATRIX** — mutation kill-rate gates span vanilla + exotics +
  surface + risk-cube + xva (`.config/mutants-*.toml`); a multi-target fuzz estate (`fuzz/`);
  client parity proven executable by `docs/CLIENT-PARITY-MATRIX.md` (all 18-products × the service
  families reachable from all five surfaces, with honest exceptions e.g. basket Greeks deliberately
  zeroed).
- **Adversarial review every gate** — has caught and forced fixes for real defects (a sign-inverted
  charm, an RFS lag-recovery hole, an idempotency key-collision, the FRTB 0.75ρ circular-oracle bug,
  an SPMC torn-read) before each milestone committed. Nothing unverified ships.
- **OSS-only** supply chain (cargo-deny advisories + licenses); dependencies vetted before use
  (wasmtime **rejected** for open RUSTSEC advisories — the plugin host runs on the advisory-clean,
  pure-Rust **wasmi** interpreter instead).

---

## 6. Honest boundary — deploy-gated residuals (verbatim, never claimed in-repo)

These are the canonical boundary lines. In-repo proves correctness / ratios / loopback; the
**absolute** claim is proven only at deploy. None of the following is claimed as in-repo-proven:

| Residual | In-repo proof | Deploy-gated absolute |
|---|---|---|
| Cross-host wire p99 / kernel-bypass NIC latency | §1.2 in-core truth-gate + loopback benches only | Absolute cross-host wire p99 |
| §11 ABSOLUTE wire-latency SLOs | §1.2 truth-gate + loopback only | Absolute wire SLOs |
| CUDA/NVIDIA GPU throughput + ≤50ms exotic + Workload-A/B | **correctness + RATIOS only** (M4/Lavapipe; Metal lacks f64) | Absolute NVIDIA throughput |
| Live JVM Celer estate lifecycle (sidecar / FX_OPTION / tenant overlays) | seams + adapters + FIX loopback only | Live estate connection |
| Raft §6 dynamic membership / cross-DC transport / real partitions | correctness/quorum/framing on localhost multi-process only | Cross-DC / membership / partitions |
| Plugin Tier-1 signed-`.so` (stabby) + Tier-3 Landlock/seccomp | **designed-only** (only Tier-0 native + Tier-2 wasmi shipped) | OS-isolated tiers at deploy |
| XVA (CVA/DVA/FVA, `celnet-xva`) | **internal-only, no client/wire surface, synthetic netting sets** | Live CSAs / collateral / wrong-way risk |
| Multi-source surface aggregation | the blend/staleness/divergence ALGORITHM (built + gated) | Live multi-vendor quote VALUES |
| MC-priced products (TARF, accumulator, discrete lookback, basket, American LSM) | carry a **price std-error** — never "machine-precision" | analytic/PDE/golden bar reserved for closed-form products |

---

## 7. Remaining breadth gaps (honest)

| Gap | Status | Plan |
|---|---|---|
| Pair/asset-class breadth (vs SynOption 75 + crypto) | ⚠️ 19-pair universe (G10 + EM NDF + XAU/XAG) | Add pairs via the convention registry; crypto out of current scope |
| Pivot + remaining structured catalogue | ⚠️ core structures shipped; pivot pending | Extend the catalogue via the same parity-gated pattern |
| Plugin Tier-1 signed-`.so` + Tier-3 OS-sandbox | 🟡 designed/seamed (`PLUGIN-HOST-ALT.md`) | Wire behind the frozen contract; prove at deploy |
| Cross-DC / dynamic-membership distributed deploy | ⛔ deploy-gated (correctness proven loopback) | Prove absolute wire p99 / membership at deploy |
| Live Celer JVM estate + multi-vendor feed VALUES | ⛔ deploy/live-gated (seams + algorithm built) | Integration/deploy wave against the running estate |
| Regulated multi-bank RFQ venue (RMO) | ✱ deliberate non-goal | Sit alongside venues, not replace them |

---

*Sources: `crates/celnet-proto/proto/celnet.proto`, the `celnet-exotics` / `celnet-heston` /
`celnet-qmc` / `celnet-risk-cube` / `celnet-xva` / `celnet-replog` / `celnet-fanout` /
`celnet-plugin-host` crates, `celnet-golden/data/`, `celnet-parity/tests/`,
`celnet-bench/src/bin/core_load.rs`, `excel/src/functions/functions.ts`,
`docs/CLIENT-PARITY-MATRIX.md`, `docs/COMPETITIVE-ANALYSIS.md`, `docs/PLUGIN-HOST-ALT.md`, the
per-wave competitor critiques in `docs/_research/`, and the verified test suite. Kept in sync as
capabilities land (zero-legacy); rows graduate 🟡/⛔ → ✅ only when test-backed.*
