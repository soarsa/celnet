# Celnet Target Architecture

**Status:** ARCHITECTURAL TARGET SPECIFICATION — Baseline for multi-dimensional convergence plan (pricing, API, scale-out, latency, governance). Synthesized from a whole-platform, lodestar-anchored audit (5 dimensions × celnet-explorer agents; all 40 crates + gui + excel + deploy). Design-first — this defines the optimal target BEFORE implementation. Feeds a set of phased ADRs (0010 term-structure, 0012 done, + new 0013–0016 below). Honours CLAUDE.md guardrails #2/#6/#7/#8/#9/#10/#11.

## 0. The central finding — Celnet is SOTA capabilities built as disconnected islands

The audit's dominant theme, repeated in **every** dimension: Celnet has built genuinely
state-of-the-art capabilities to a high standard — and left them **unwired from the live
path**. The biggest architectural wins are therefore **integration and de-duplication, not
new capability**:

| Built & validated | Wired into the live path? | Evidence |
|---|---|---|
| GPU acceleration — 5 wgpu pipelines (batch/scenario/pathwise/MC/path) | ❌ **zero** non-test callers | `celnet-gpu/*`; `price_instrument` is CPU-only |
| Raft consensus — full `celnet-replog` (election/append/snapshot over TCP) | ❌ **zero** — no server dep | `RaftNode::boot` (`election.rs:433`) uncalled |
| FI analytics — Fra / VanillaSwap / CashBond / z-spread | ❌ only OIS served | `rates_pricing.rs:231` (1 arm) |
| Plugin dispatch — tiered native + wasmi host | ❌ only the vanilla-FX arm | `engines.rs:1442` |
| Generated wire codec — descriptor manifest (G INC1) | ❌ field codec still hand-written | 9,293 hand lines vs 4,484-line proto |
| FI risk / dealer-desk / notifications | ❌ GUI-only, no SDK/CLI | `celnet-client` gaps |
| **Pre-trade limit engine** (`celnet-limits`) | ❌ **display-only, not on the execution path** | `pre_trade_check` 0 non-test callers; `clicktrade` no `celnet-limits` import — a hard-limit-blown lift still books |
| FIX IOI dealer workflow | ❌ test fixture only | `services/fix.rs::handle_frame` has no `IOI(35=6)` handler |

The core pricing kernel (ADR-0012, gBSM unified) and the contract shape are already right.
The target is to **connect the islands, unify the substrates, and delete the duplication.**

## 1. Five dimensions — current state → target

### D1. Pricing / risk / cross-asset — *the missing `DiscountCurve` substrate*
- **Now:** options discount FLAT everywhere (`Carry` = 2 scalars); FI owns a real term-structure
  `Curve` in an isolated crate (deps: types+calendar only). ≥4 distinct paradigms; risk splits
  into `NetGreeks` vs `RatesNodeAggregate` with no summation path; `Underlying` has no
  `InterestRate` arm.
- **Target:** hoist a `DiscountCurve` trait into `celnet-core`; `Carry` = degenerate one-pillar
  curve, `celnet-rates::Curve` = general case; FX two-rate carry = 2 curves. Unify the
  discount substrate + risk org-hierarchy/envelope + contract vocabulary; keep payoff engines &
  the Greek-vector-vs-DV01-ladder representations domain-appropriate. **FX byte-identity
  preserved** (flat variant untouched). → **ADR-0010**, `docs/plan/TERM-STRUCTURE-UNIFICATION.md`.
- **Effective-zero-rate contract (curve tier → flat hot state) — required for exact forward reproduction:**
  the ADR-0016 embargo keeps `MarketState` FLAT (`f64` rates, no `Arc<Curve>`), so the discount
  *value* — not the curve object — crosses the seam. For that flat degenerate slice to reproduce the
  curve forward **exactly**, the surface-rebuild tier must publish **effective zero rates**
  `r_eff = −ln(df(t))/t` per slice, so that `S·e^{(r_dom−r_for)·t}` == `S·df_for(t)/df_dom(t)` holds
  byte-for-byte. This is the concrete contract between the curve tier (ADR-0010/0015) and the flat
  hot state (ADR-0016 embargo) — without it a slice priced off the flat scalars diverges from the
  same slice priced off the full curve.

### D2. Integrated API + SDKs + Excel — *the contract is right; the hand codec is the debt*
- **Now:** ONE clean cross-asset contract (9 services, 25-arm `Instrument.oneof`, 5 asset
  classes, no versioning cruft). But the WS codec is **~9,293 hand-maintained lines** (Rust
  `codec.rs` 4,378 + 2 TS codecs + 2 TS contract mirrors) tracking one 4,484-line proto — a
  proto field needs 5 manual edits, none compile-checked. Parity gaps: dealer-desk, FI risk,
  notifications are GUI-only.
- **Target:** **generate the codec from the proto descriptor** (complete the G deferral →
  −4,378 Rust lines, drift structurally impossible); `buf`-generate TS types + a shared codec
  package (−~6.7k TS lines); typed `DeskClient`/`NotificationClient` + FI risk in SDK & CLI;
  `=CELNET.SCENARIO`. Cross-asset non-vanilla arms need **zero API shape change** (the
  `Instrument.oneof` + `Price` is already universal). → **ADR-0014 (generated wire contract)**.

### D3. Scaling / nodes / distribution — *activate the dormant consensus*
- **Now:** sound principle ("distribute for capacity, not speed" — the router is provably never
  on the per-tick path; SPMC seqlock fanout ring; HRW routing). But **Raft is fully built and
  dormant** (no server wiring) → no hot-standby failover, no quorum surface/curve distribution,
  no cross-shard deterministic replay. Static env-var fleet membership (no gossip/versioned
  map). HFT counterparty fan-out is unicast over the stock OS stack.
- **Target:** **activate `celnet-replog`** — wire `RaftNode::boot` into `start_on_with_topology`,
  route `PositionStore` book writes + authoritative surface/curve updates through the leader-
  append log (`BookState`/`Journal` seams already exist); use the **Raft log itself as the
  versioned membership + surface-distribution feed** (one source of truth, no separate gossip);
  Raft §6 dynamic membership; HFT fan-out tier (`SO_REUSEPORT`+`io_uring`, then Jasper
  proxy-multicast) gated on a measured bottleneck. → **ADR-0015 (replicated state + elastic fleet)**.

### D4. Ultra-low-latency + observability — *hold the sacred core; gate it hard*
- **Now:** zero-alloc, lock-free hot core is correctly implemented (arc_swap::Cache + rtrb SPSC +
  cache-line-isolated seqlock + static enum smile dispatch; no logs; telemetry offloaded to a
  bounded ring + off-thread HdrHistogram). Budgets: vanilla p50≤2µs/p99≤10µs/p99.9≤25µs.
- **Target / constraints:** (a) **Arc<*Curve>/Arc<*Surface> EMBARGO inside `MarketState`** —
  `MarketState` keeps FLAT `f64` rates; the term-structure `Curve` stays in the surface-rebuild
  tier; streaming always pins a `CalibratedSmile::Parametric` (O(1)). This is the **hard design
  constraint the D1 unification must honour.** (b) **Hard SLO gate in `t2`** (assert p50/p99/p99.9,
  not just criterion baselines). (c) reader-thread drop isolation (off-thread reclamation to kill
  p99.9 dealloc jitter); (d) batch SoA + `wide` SIMD (~52ns→~13ns/option) with caller-supplied
  output buffers. → **ADR-0016 (latency SLO gate + hot-core embargoes)**.

### D5. Analytics / acceleration / plugins / config — *wire the GPU, open the plugin surface*
- **Now:** 5 validated GPU pipelines with **zero live callers**; plugin dispatch only the
  vanilla-FX arm (`calibrate`/`check-no-arbitrage` defined in WIT but unreachable); no declarative
  config (env-var proliferation); Metal-f64 handled correctly (f32 math, f64 reduction); no
  CUDA/CubeCL despite the doc claim.
- **Target:** wire GPU into the live path by **batch size** (small→CPU analytic; large→`BatchPricer`
  GPU); make **`ScenarioPricer` the risk-cube repricer backend** (the single biggest throughput win
  — risk-cube is the portfolio-scale bottleneck); GPU `PathwiseGreeksPricer` for streaming; plugin
  hooks on **every** `ProductEngine` arm + a `calibrate` callback for user smile models; a typed
  `PlatformConfig` (celnet.toml) for GPU/MC-paths/plugin-wasm/fleet/feed; resolve CUDA (implement
  via CubeCL or drop the claim). → folded into **ADR-0013 (acceleration & extensibility wiring)**.

### D6. Governance + connectivity — *sound kernel, unwired pre-trade risk, FX-centric breadth*
- **Now:** the entitlement kernel is genuinely good — deny-by-default `Capability{Action, AssetClass}`,
  deny-wins info-barrier `Principal`, a 7-variant audit taxonomy, session-aware `authorize_caller`,
  **uniform across every client** (gRPC + WS same trait; `EntitlementPrincipal` in gui/excel/cli).
  FIX is a **first-class ingress** that proves parity (same `price_instrument`, same last-look
  `TokenLedger`). **But** three gaps: (a) **the pre-trade limit engine is display-only** —
  `pre_trade_check` (`celnet-limits/src/check.rs:218`, full hard/soft/RAG/hierarchical) has **zero
  execution-path callers**; `clicktrade`/FIX `on_new_order` never consult it, so a **hard Delta/VaR
  breach still books** (`LimitStatus` RPC is view-only). (b) **Cross-asset breadth:** `AssetClass`
  has only `FxOptions|FixedIncome`; FIX dialects cover FX+FI only (no equity/crypto/commodity);
  limits are FX-centric (Vanna/Volga/CcyPair, no IR DV01/tenor). (c) FIX IOI is a test fixture, not
  a live server path.
- **Target:** **wire `pre_trade_check` at the POSITION SINKS** — `PositionStore::book` +
  `RatesPositionStore::book`, where **every** booking converges, since `AcceptQuote`
  (`quote.rs:815-1036`), `AcceptDeskQuote` (`desk/mod.rs:523`) and `BookRatesPosition`
  (`risk/mod.rs:665`) book **directly** and bypass `clicktrade::try_book` — plus FIX `on_new_order`;
  `Reject → BookOutcome::LimitBreached → ExecutionReport(ExecType=8)`
  — hard limits are **pre-trade, not post-trade** (Bloomberg SSEOMS / Murex / TriOptima canonical);
  extend `AssetClass` + `AcceptorKind` per leaf (with the existing capability-deny test as the
  template); add `LimitScope::Tenor` + `LimitMetric::Dv01` (additive); activate the IOI path; enforce
  the per-connection FIX desk scope at quote-request time. → folded into **ADR-0016 (governance &
  pre-trade risk wiring)** alongside the latency embargoes (both are "wire the guardrails" work).

## 2. Cross-asset as the opportunity multiplier

The carry-seam (ADR-0008) + the unified kernel (ADR-0012) already route every asset class through
one `price_greeks`. That makes these **structurally low-impedance** (the audit confirmed each):
1. **One curve-risk cube** — FX + rates + equity delta/DV01 aggregate through the seam, but **not**
   for free "in one pass": it needs a **tagged `Measure` enum** (`Options(NetGreeks)` |
   `Rates(RatesNodeAggregate)`) plus a shared **`RateKey{ccy,tenor}`** to net rho against DV01. The
   two representations **do not collapse** into one vector — the `NetGreeks` vs `RatesNodeAggregate`
   split has no free summation path (§1 D1); netting is an explicit keyed reduction, not an implicit
   sum. Needs the D1 `Underlying::InterestRate` arm + risk envelope.
2. **GPU cross-asset batch Greeks** — `BatchPricer`/`ScenarioPricer` over a mixed-asset portfolio in
   one dispatch (needs a `GpuInstrument` carrying `CostOfCarry`).
3. **Universal product coverage** — barrier/Asian/variance-swap on equity/crypto/commodity needs only
   new dispatch arms, **zero** new RPCs/proto/messages.
4. **One term-structure** feeding FX forwards (`S·df_for/df_dom`), rates discounting, and STIR
   convexity from a shared vol surface.

## 3. Legacy / replication to remove (DRY — guardrail #10)

Ranked by lines eliminated: (1) hand WS codec generation −4,378 Rust + ~6.7k TS; (2) dual TS
contract mirrors; (3) `gui/pricing.ts` client-side pricing engine scoped strictly to mock/display
(drift risk vs server); (4) `CountingAlloc` → `celnet-testkit` (deduped across 3 crates); (5) WGSL
math primitives via `include_str!` composition (deduped across 5 shaders); (6) CPU/GPU parallel
path-payoff impls → single tested-equivalent description; (7) drop or implement the CUDA/CubeCL
doc claim. No versioning/back-compat cruft exists (ADR-0007 already clean).

## 4. Hard invariants (non-negotiable through every phase)
- **Hot core sacred:** no alloc/lock/log on the pricing thread; **no `Arc<Curve>` in `MarketState`**.
- **FX byte-identity** (`to_bits`) across the term-structure generalization (ADR-0010).
- **`to_bits` byte-identity is a CPU-only oracle:** **no test may pin a GPU FX result to `to_bits`** —
  the GPU (f32 math, f64 reduction) stays a **≤1e-12 batch-tier** result, while the CPU flat-carry
  path (`carry.rs:525`) remains the **sole** `to_bits` byte-identity oracle (ADR-0013 §invariants).
- **Numerical vs INDEPENDENT oracles ≤1e-12** (QuantLib/multi-curve); never a self-referential regen.
- **One unversioned contract** (ADR-0007); every capability lives in the one API, uniform client parity.

## 5. Prioritized target program (each = an ADR + a gated phase; design-first, operator-confirmed)

Ordered by leverage × independence (so lanes parallelize across sessions without shared-crate churn):

- **P1 — Wire the islands (highest ROI, genuinely low blast radius — additive dispatch/client/codec
  wiring, NO shared-enum change):**
  (a) **⚠ SAFETY-FIRST: wire `pre_trade_check` into the execution path** [ADR-0016 A1] — the only
  *risk-control* gap (a hard-limit-blown lift currently books); small, self-contained, should lead.
  **The gate sits at the POSITION SINKS** (`PositionStore::book` + `RatesPositionStore::book`), where
  **every** booking converges — **not** merely at `clicktrade::try_book` — because `AcceptQuote`
  (`quote.rs:815-1036`), `AcceptDeskQuote` (`desk/mod.rs:523`), and `BookRatesPosition`
  (`risk/mod.rs:665`) book **directly** and today bypass any limit check (a hard-limit-blown lift
  books straight through them);
  (b) GPU into the live path (batch-size branch + risk-cube repricer) [ADR-0013];
  (c) serve the built FI analytics (Fra/Swap/Bond) + FI risk/desk/notification SDK+CLI parity [ADR-0014 clients];
  (d) generate the WS codec from the descriptor — complete G [ADR-0014 codec].
  These four are additive at the dispatch/client/codec boundary and change **no shared enum and no
  cross-client contract**, so they parallelize without coordinator gating.
- **P2 — Unify the substrate + broaden the shared vocabulary (shared interface crates — coordinator-gated):**
  the `DiscountCurve` trait + curve-backed carry (curve OFF the hot path) + `Underlying::InterestRate`
  + the risk-cube envelope [ADR-0010 → build]; **and the `AssetClass` / `AcceptorKind` breadth
  broadening [ADR-0016 A2/A3]** — a **shared-enum** change consumed by **every** client (gui/excel/cli)
  + the FIX dialect + risk, so it is coordinator-gated cross-cutting that belongs here, **not** in the
  low-blast P1. FX byte-identity gate every step.
- **P3 — Activate consensus & elastic scale-out** [ADR-0015]: replog wiring + Raft-log membership +
  surface distribution; then HFT fan-out tier on a measured bottleneck.
- **P4 — Harden latency & extensibility** [ADR-0016]: hard SLO gate, reader-thread drop isolation,
  batch SoA/SIMD, all-arm plugin dispatch + `calibrate`, declarative `PlatformConfig`.

P1 is independent, additive, and unlocks the most cross-asset value immediately; P2 is the deepest
(shared crates) and must be coordinator-sequenced; P3/P4 harden scale + latency for IB/HFT.
