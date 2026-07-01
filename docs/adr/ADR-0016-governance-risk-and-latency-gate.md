# ADR-0016 — Governance & pre-trade risk wiring + latency SLO gate + hot-core embargoes

- **Status:** Proposed / Accepted as a **design direction** (2026-07-01). **NOT yet
  implemented.** Records the intended wiring of the two "wire the guardrails" concerns
  (pre-trade risk enforcement; latency SLO enforcement + hot-core structural embargoes).
  The display-only limit path and the soft criterion baseline remain the current
  behaviour until the wiring lands.
- **Aligns with:** `docs/ARCHITECTURE-TARGET.md` §1 **D6** (governance + connectivity)
  and **D4** (ultra-low-latency + observability); target program **P1(a)** (⚠
  safety-first pre-trade wiring — should lead) and **P4** (harden latency). Honours
  CLAUDE.md guardrails #5 (gates), #6 (scale/perf), #9 (one unversioned contract), #11
  (zero-cost observability; pinned hot core stays alloc/lock/log-free).
- **Interlocks with:** ADR-0010 (term-structure unification) — the `Arc<*Curve>` embargo
  below is the **hard constraint ADR-0010 must honour**: the curve trait may NOT be
  hoisted into `MarketState`. ADR-0008 (carry seam) / ADR-0012 (unified gBSM kernel) —
  the hot-core drain path this ADR protects.

## Context (grounded in the code)

This ADR bundles two independent "the guardrail is built but not wired" defects that
share a theme: **Celnet has a sound mechanism that is not on the critical path.** (The
central finding of `docs/ARCHITECTURE-TARGET.md` §0 — SOTA capabilities as disconnected
islands.)

### A. Governance / pre-trade risk — the ⚠ safety-first island

- The pre-trade limit engine is **complete and correct but has zero execution-path
  callers**. `celnet_limits::check::pre_trade_check` (`crates/celnet-limits/src/check.rs:218`)
  implements the full hard/soft/RAG/hierarchical algorithm and returns a
  `PreTradeDecision` (`check.rs:172`; `Reject` at `check.rs:260`). Its **only** callers are
  unit tests (`crates/celnet-limits/src/lib.rs:401/457/469`). The limit taxonomy is rich —
  `LimitScope{Firm,Trader,Book,Desk,CcyPair,Location,Entity}` (`tree.rs:58`),
  `LimitMetric{Delta,Gamma,Vega,Vanna,Volga,VegaBucket,TenorVega,Concentration,Var,ES,
  StopLoss}` (`limit.rs:39`), `LimitSpec{cap,amber,red,Hard|Soft}` — but display-only.
- The deal-execution path **never consults it.** `TokenLedger::try_book`
  (`crates/celnet-server/src/services/clicktrade.rs:258`; ledger struct at `:153`) is the
  single source of truth for a last-look lift and returns `BookOutcome`
  (`clicktrade.rs:131`). It has **no `celnet-limits` import.** The FIX ingress lifts
  through the *same* ledger — `on_new_order` (`crates/celnet-server/src/services/fix.rs:476`)
  validates the keyed-MAC token against `TokenLedger::try_book` and emits
  `ExecutionReport(35=8, ExecType=8)` with a `Text(58)` reason (`fix.rs:28`) on rejection
  — but likewise never checks a limit. **Net defect (highest priority): a `NewOrderSingle`
  / `AcceptQuote` / RFS click-to-trade books even with a hard Delta/VaR/StopLoss limit
  BLOWN.** `LimitStatus` is a view-only RPC (`LimitTree.iter`). This is a *risk-control*
  gap, not a feature gap — Bloomberg SSEOMS / Murex / TriOptima all treat hard limits as
  **pre-trade blocking**, never post-trade reconciliation.
- **Cross-asset breadth is narrow.** `AssetClass`
  (`crates/celnet-entitlements/src/capability.rs:119`) has only `FxOptions | FixedIncome`
  — 2 variants. FIX `AcceptorKind` (`crates/celnet-server/src/config/fix_connections.rs`;
  wire mapping `fix_admin.rs:457`, capability gate `required_dialect_capability`
  `fix_admin.rs:491`) covers `Options | FixedIncomeQuote | FixedIncomeStream` only
  (Phase-2 `SpotFx` is commented). No equity/crypto/commodity dialect. Limit metrics are
  FX-centric (`Vanna/Volga/VegaBucket/CcyPair`); there is **no IR DV01 or tenor limit**.
- **Two governance sub-paths are test-only.** FIX `handle_frame` (`fix.rs:364`) dispatches
  only `QuoteRequest(R)` → `on_quote_request` (`fix.rs:391`) and `NewOrderSingle(D)` /
  `NewOrderMultileg(AB)` → `on_new_order` (`fix.rs:379`) — there is **no `IOI(35=6)`
  arm** (IOI exists as a fixture only). And the per-connection FIX desk scope is checked
  at *list-connections* admin time, not re-enforced when a bound session answers a
  `QuoteRequest`.
- **What is genuinely good and MUST be kept:** the deny-by-default `Capability{Action,
  AssetClass}` kernel (`capability.rs`), the deny-wins info-barrier `Principal`, the
  7-variant `AccessReason` audit taxonomy, session-aware `authorize_caller`, and the
  **uniform-across-every-client** enforcement (gRPC + WS share the trait;
  `EntitlementPrincipal` lives in gui/excel/cli). The capability-deny test template
  already exists — `admin_without_fi_quote_capability_is_denied`
  (`crates/celnet-server/src/services/fix_admin.rs:743`).

### B. Ultra-low-latency — the sacred hot core, ungated and structurally exposed

- The two-tier hot path is correctly zero-alloc and lock-free: async edge
  `price_instrument` (`crates/celnet-server/src/pricer.rs`) → pinned busy-poll core
  `PricingCore::drain` (`crates/celnet-engine/src/core.rs:189`; struct `:94`): rtrb pop →
  `price()` → `StateReader::load` (`arc_swap::Cache`, refcount-free steady state) →
  `CalibratedSmile::implied_vol` (5-arm enum, **static** dispatch,
  `crates/celnet-surface/src/calibrate.rs:104`; enum `:58`) → `celnet_vanilla::greeks` →
  cache-line-isolated single-writer seqlock store (`crates/celnet-engine/src/rt.rs`,
  seqlock module `:198`) → rtrb push. Proven allocation-free under concurrent publish:
  `hot_pricing_under_concurrent_publish_allocates_zero`
  (`crates/celnet-engine/tests/zero_alloc.rs:221`). No logs on the path
  (`docs/ARCHITECTURE.md:264`). Telemetry is correctly offloaded — `HotProbe::publish`
  (`crates/celnet-observability/src/channel.rs:58`) pushes a `Copy` POD `HotSample`
  (`record.rs:180`) into a bounded ring, drop-on-full, drained off-thread into an
  HdrHistogram `LatencyRecorder`.
- **`MarketState` is FLAT `f64` — the zero-pointer-deref property.**
  `crates/celnet-engine/src/rt.rs:53` stores `r_dom` (`:57`), `r_for` (`:59`) as bare
  `f64`; the forward is `S·e^{(r_dom−r_for)·t}` (`rt.rs:76`) with no indirection. **If the
  rates layer puts `Arc<YieldCurve>`/`Arc<Curve>` (or `Arc<*Surface>`) into `MarketState`,
  it destroys the zero-deref hot path** and imports refcount traffic + a term-structure
  interpolation into the pinned loop. The `ParametricSurface::implied_vol` arm
  (`crates/celnet-surface/src/parametric.rs:212`) already risks O(log N) tenor search per
  call; streaming must pin the O(1) `CalibratedSmile::Parametric`.
- **No hard SLO assertion — green CI ≠ p99 met.** The budgets are `docs/ARCHITECTURE.md:61`
  (vanilla p50≤2µs / p99≤10µs / p99.9≤25µs), `:62` (surface rebuild p99≤150µs), `:63`
  (exotic p99≤50µs). `FleetSloReport` (`crates/celnet-bench/src/fleet_slo.rs:267`) is
  produced by `measure` (`:780`) which **prints, does not assert**; the only regression
  guard is a soft criterion baseline. A p99 regression can ship on a green pipeline.
- **Two measured/known hot-path taxes.** (1) reader-thread dealloc jitter: when a
  superseded `Arc<MarketState>` snapshot's last reference is dropped **on the hot reader
  thread**, the `CalibratedSmile` `Vec`s free inline → p99.9 dealloc jitter (currently
  UNMEASURED). (2) batch pricing allocates: `cpu_batch_with_as_erf`
  (`crates/celnet-gpu/src/batch.rs:481`) and `price_batch` (`:210`) return a fresh
  `Vec<f64>` per call, and the CPU batch is scalar AoS ~52 ns/opt — an IB 10k×5-tenor
  book is ~2.6 ms/core with no `wide`/SoA SIMD.

## Decision

Wire both guardrails onto the critical path. The two halves are additive, low-blast-radius,
and independent, so they can land as separate gated phases (A leads — it is the risk
control).

### A. Governance / pre-trade risk wiring (⚠ safety-first — leads)

**A1 (P0) — Wire `pre_trade_check` into the deal-execution path.** Both execution entry
points build a `ScopePath` from the lifting token's `TokenBinding` (desk/book/entity/
ccy-pair already carried on the token) and call `pre_trade_check` **before** the book
commits:

- `TokenLedger::try_book` (`clicktrade.rs:258`) consults the limit tree; a
  `PreTradeDecision::Reject` short-circuits to a new `BookOutcome::LimitBreached`
  (`clicktrade.rs:131`) carrying the breached scope/metric and RAG state. The book state
  is **never** mutated on a reject.
- FIX `on_new_order` (`fix.rs:476`) maps `BookOutcome::LimitBreached` to
  `ExecutionReport(35=8, ExecType=8)` with `Text(58) = "limit breached: <scope>/<metric>"`
  — reusing the existing reject wire encoding (`fix.rs:28`).
- The gRPC/WS `AcceptQuote` / RFS lift surfaces the same `LimitBreached` outcome uniformly
  (guardrail #11 cross-client parity), so every client renders an identical rejection.

Hard limits are enforced **pre-trade, blocking** — never post-trade reconciliation
(Bloomberg SSEOMS / Murex / TriOptima norm). Soft (`Warn`) decisions book but raise the
RAG amber/red state and an audit event.

**A2 (P1) — Extend `AssetClass` + FIX `AcceptorKind` per new leaf.** Add
`Equity | Crypto | Commodity` to `AssetClass` (`capability.rs:119`) and the matching
`AcceptorKind` dialect variants + `required_dialect_capability` arms
(`fix_admin.rs:491`), in lockstep with the entitlement grants — following the existing
capability-deny test template (`admin_without_fi_quote_capability_is_denied`,
`fix_admin.rs:743`) as the acceptance pattern for each new leaf. Additive to the enum; no
contract reshape (guardrail #9).

**A3 (additive) — `LimitScope::Tenor` + `LimitMetric::Dv01` for rates.** Add
`LimitScope::Tenor` (`tree.rs:58`) and `LimitMetric::Dv01` (`limit.rs:39`) so rates/FI
positions carry a curve-bucketed IR limit alongside the FX Greek limits. Purely additive
variants — the hierarchical roll-up in `pre_trade_check` already generalizes over
scope/metric.

**A4 (P2) — Activate the FIX `IOI(35=6)` server path.** Add an `IOI` arm to
`handle_frame` (`fix.rs:364/378`) gated on the `Action::IoiRespond` capability — promoting
the existing test-only IOI fixture to a live handler. No new proto (FIX-side only).

**A5 (P2) — Enforce the per-connection FIX desk scope at quote-request time.** Re-check the
bound session's `DeskScope` in `on_quote_request` (`fix.rs:391`), not only at
list-connections admin time, so a session cannot price outside its desk after bind.

**Keep unchanged (sound + uniform):** the deny-by-default `Capability{Action, AssetClass}`
kernel, the deny-wins info-barrier `Principal`, the 7-variant `AccessReason` audit
taxonomy, and cross-client uniform enforcement. This ADR **wires** and **broadens** the
kernel; it does not redesign it.

### B. Latency SLO gate + hot-core embargoes

**B1 (P4) — Hard SLO gate in `just t2`.** Add an assertion step that runs
`fleet_slo::measure` (`fleet_slo.rs:780`) and **asserts** the `FleetSloReport`
(`fleet_slo.rs:267`) against the `docs/ARCHITECTURE.md:61` budgets — vanilla **p50≤2µs /
p99≤10µs / p99.9≤25µs** (and, as they come online, surface-rebuild p99≤150µs / exotic
p99≤50µs). A breach **fails the landing gate**. This closes the "green CI ≠ p99 met" gap:
the SLO becomes a blocking correctness contract, not a soft criterion baseline. (Runs
once per push milestone at T2 per the tiered-gate law — `docs/PARALLEL-SESSIONS.md` §4.2.)

**B2 (P2/D1 constraint) — `Arc<*Curve>` / `Arc<*Surface>` EMBARGO inside `MarketState`.**
A clippy/`cargo-deny` lint forbids any `Arc<…Curve>` / `Arc<…Surface>` (or other heap
term-structure handle) as a field of `MarketState` (`rt.rs:53`). `MarketState` stays **flat
`f64`**. The term-structure `Curve` (ADR-0010) lives in the **surface-rebuild tier**; the
hot engine sees only a **pre-interpolated flat slice**, and streaming **always pins a
`CalibratedSmile::Parametric`** (O(1), `calibrate.rs:104`) — never the O(log N)
`ParametricSurface`/`ExtendedSurface` tenor-search arm (`parametric.rs:212`). **This is the
hard structural constraint the ADR-0010 term-structure unification MUST honour** — the
`DiscountCurve` trait may be hoisted into `celnet-core` but may NOT enter `MarketState`.

**B3 (P4) — Reader-thread drop isolation.** Move the drop of a superseded `MarketState`
snapshot **off** the pinned reader thread via epoch-based deferred reclamation
(`crossbeam-epoch`; add to the workspace alongside the existing `arc-swap`) or a bounded
off-thread drop queue, so the `CalibratedSmile` `Vec` frees never run inline on the hot
loop. Kills the p99.9 dealloc jitter at its source. Validated by extending the zero-alloc
harness (`zero_alloc.rs:221`) with a "reader thread performs zero frees under publish
churn" assertion.

**B4 (P4) — Batch SoA + wide SIMD with caller-supplied output buffers.** Re-lay the CPU
batch (`batch.rs:481`) from scalar AoS to SoA + `wide` SIMD lanes (~52 ns → ~13 ns/opt,
LMAX/Disruptor-shaped data layout) and change `price_batch` (`:210`) /
`cpu_batch_with_as_erf` to write into a **caller-supplied `&mut [f64]`** output buffer
instead of returning a fresh `Vec<f64>` — removing the per-call allocation on the batch
tier. (GPU `BatchPricer` remains the correct tier for large books; this closes the CPU
tier's 52× gap for the small/interactive path.)

## Consequences + invariants

- **A hard-limit breach can no longer book.** After A1, every execution entry point
  (`clicktrade`, FIX, gRPC/WS) is gated by `pre_trade_check`; a `Reject` cannot mutate book
  state. This is the load-bearing risk-control invariant of this ADR.
- **The pinned pricing thread stays alloc/lock/log-free.** B2 (embargo) + B3 (drop
  isolation) + the existing no-log rule (`ARCHITECTURE.md:264`) are jointly enforced by the
  lint, the epoch reclamation, and `hot_pricing_under_concurrent_publish_allocates_zero`.
  **No `Arc<Curve>` on the hot path, ever** (also `docs/ARCHITECTURE-TARGET.md` §4).
- **The SLO is a gate, not a hope.** B1 makes p50≤2µs/p99≤10µs/p99.9≤25µs a `t2` blocking
  assertion; a tail regression fails the landing.
- **Numerical invariant preserved (≤1e-12).** None of A/B changes a pricing float form;
  the SoA/SIMD relayout (B4) is validated against the independent oracle at ≤1e-12
  (guardrail #5), and the FX byte-identity contract (ADR-0008/0010/0012) is untouched — the
  flat-slice guarantee (B2) is *why* it stays intact.
- **One contract, uniform parity (guardrails #9/#11).** A1–A5 are additive to enums and
  handlers — no `schema_version`, no versioned negotiation; the `LimitBreached` outcome and
  the new asset classes surface identically across gRPC/WS/FIX/SDK/CLI/GUI/Excel.
- **Cost:** A1 adds one limit-tree read on the *booking* path (not the per-tick pricing
  path — booking is already off the sacred core, so the SLO budget is unaffected). B3 adds
  a `crossbeam-epoch` dependency (MIT/Apache-2.0 — guardrail #7 clean).

## Alternatives rejected

- **Post-trade limit checking / reconciliation instead of pre-trade blocking** — rejected.
  A hard Delta/VaR/StopLoss breach must be *refused at the point of execution*; detecting it
  after the book has moved is a risk-control failure, not a control. Industry canon
  (Bloomberg SSEOMS, Murex, TriOptima) is pre-trade hard limits. Post-trade monitoring
  (the existing `LimitStatus` view) is complementary, not a substitute.
- **Leave the latency SLO a soft criterion baseline** — rejected. A criterion baseline
  catches *relative* drift on a tuned box but never asserts the *absolute* budget, so a p99
  regression ships green. Tail latency is the product (`ARCHITECTURE.md:28`); it must be a
  blocking gate.
- **Allow `Arc<Curve>` in `MarketState` (let the term-structure unification hoist the curve
  onto the hot path)** — rejected. It re-imports pointer-chase + refcount traffic + an
  O(log N) tenor interpolation into the pinned zero-deref loop, breaking the p99.9 budget
  and the zero-alloc proof. The curve belongs in the surface-rebuild tier; the hot engine
  consumes a pre-interpolated flat slice. This is precisely the constraint ADR-0010 accepts.
- **Return-a-`Vec` batch API + scalar AoS** — rejected for the batch tier: per-call
  allocation and 52 ns/opt scalar math miss the throughput budget by ~52×; SoA + `wide`
  SIMD + caller-supplied buffers is the SOTA layout (LMAX Disruptor data-oriented design).
- **Redesign the entitlement kernel** — rejected. The deny-by-default capability kernel +
  deny-wins info-barrier + 7-variant audit taxonomy + uniform cross-client enforcement are
  sound; the defect is *wiring* (pre-trade limits) and *breadth* (asset classes), not the
  model. This ADR keeps the kernel and wires/broadens it.

## Supporting verified claims (lodestar knowledge layer)

Authored graph-anchored, lifecycle **draft** (proposed direction; promotion to `active`
awaits implementation + a Stage-2 review):

- **(invariant)** A hard-limit breach cannot book: every execution entry point
  (`TokenLedger::try_book`, FIX `on_new_order`, gRPC/WS `AcceptQuote`/RFS) calls
  `pre_trade_check`; `PreTradeDecision::Reject → BookOutcome::LimitBreached → ExecutionReport
  (ExecType=8)` and book state is never mutated. Anchors: `pre_trade_check`
  (`check.rs:218`), `PreTradeDecision` (`check.rs:172`), `TokenLedger::try_book`
  (`clicktrade.rs:258`), `BookOutcome` (`clicktrade.rs:131`), `on_new_order` (`fix.rs:476`).
- **(decision)** Governance breadth extends additively per asset leaf: `AssetClass`
  (`capability.rs:119`), FIX `AcceptorKind` / `required_dialect_capability`
  (`fix_admin.rs:491`), `LimitScope::Tenor` (`tree.rs:58`), `LimitMetric::Dv01`
  (`limit.rs:39`) grow in lockstep with the entitlement grants; the deny-by-default kernel
  is kept, not redesigned. Template: `admin_without_fi_quote_capability_is_denied`
  (`fix_admin.rs:743`).
- **(invariant)** `MarketState` stays flat `f64` — `Arc<*Curve>`/`Arc<*Surface>` is
  embargoed on the hot path (clippy/deny lint); the curve lives in the surface-rebuild
  tier; streaming pins `CalibratedSmile::Parametric` (O(1)). This is the hard constraint
  ADR-0010 must honour. Anchors: `MarketState` (`rt.rs:53`), `CalibratedSmile`
  (`calibrate.rs:58/104`), `ParametricSurface::implied_vol` (`parametric.rs:212`).
- **(decision)** The latency SLO is a hard `t2` gate: assert the `FleetSloReport`
  (`fleet_slo.rs:267`) against p50≤2µs/p99≤10µs/p99.9≤25µs (`ARCHITECTURE.md:61`); a breach
  fails the landing (replaces the print-only `measure`, `fleet_slo.rs:780`). Reader-thread
  drop isolation (`crossbeam-epoch`) + SoA/`wide` SIMD batch with caller-supplied buffers
  (`batch.rs:481`) hold the zero-alloc core (`zero_alloc.rs:221`).
