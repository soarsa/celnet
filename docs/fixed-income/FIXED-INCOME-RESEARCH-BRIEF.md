# Fixed Income — Deep-Research Brief (agent charter & requirements)

**Status:** Draft base — 2026-06-21 · Branch `feature/fixedincome`
**Owner:** (operator) · **Executor:** a deep-research agent (see §1)
**Bar:** the same SOTA, validated, OSS-only standard the FX-options platform already meets.

---

## 0. What this document is (and how to use it)

This is the **charter for a deep-research agent**: a self-contained brief that tells an
autonomous research agent (or a human + agent loop) *what to investigate*, *under what
constraints*, and *what to produce* so that Celnet can add a **fixed-income (rates & credit)
capability** to the same standard as its FX-options core.

It is **not** itself the design. It is the requirements that *form the agent* and bound its
output. The agent consumes this brief and produces the **fixed-income design corpus** (§6) —
a parallel set of spec/plan docs mirroring the existing FX corpus (`docs/ANALYTICS-SPEC.md`,
`docs/CONVENTIONS.md`, `docs/ARCHITECTURE.md`, `docs/COMPETITIVE-ANALYSIS.md`,
`docs/VERIFICATION-CONTRACT.md`, the `W*-PLAN.md` workstream docs, and structured findings
under `docs/_research/`).

**How to run it** is in §7. Read §1–§3 first (mission + guardrails), then §4–§5 (scope +
the research-question map), then §6 (deliverables) and §8 (acceptance).

---

## 1. Mission & the agent

**Mission.** Determine, with evidence, exactly what it takes to build a state-of-the-art
fixed-income trading-analytics capability into Celnet — products, curves, models,
conventions, market data, risk, validation, performance, and the integration into the single
Celnet contract — and produce a buildable, gated, phased design corpus that a parallel
implementation effort can execute crate-by-crate.

**The agent.** A deep-research agent is a long-running, tool-using researcher that:
1. decomposes the question map (§5) into investigations,
2. gathers primary evidence (standards bodies, vendor docs, academic papers, open-source
   reference implementations, code search) — **research-first**, citing every source,
3. cross-checks numerics against **open reference engines** (QuantLib, ORE — see §3.1 / §7),
4. synthesises findings into the corpus (§6) and surfaces operator decisions (§9).

Pattern to mirror: the existing research/design workflow that produced
`docs/_research/findings.{md,json}` and the `W*-PLAN.md` workstream plans. Fixed income is the
next workstream family (propose `W-FI-1…n`).

---

## 2. Context — the platform today (what the agent must build *into*)

Celnet is a SOTA FX-**options** pricing platform in Rust (edition 2024), with multi-asset
breadth (FX, metals, equity, commodity, crypto) and a **single, unversioned wire contract**
(`celnet-proto`, the `celnet.wire` schema) consumed identically by **five clients**: the React
GUI, the Excel add-in, the Rust SDK (`celnet-client`), the FIX acceptor, and the federation
fan-out. Salient facts the agent must respect and reuse:

- **Crate-split architecture** (`docs/ARCHITECTURE.md`, `docs/INTERFACES.md`): small crates,
  one-way dependencies. Relevant existing crates: `celnet-types`, `celnet-conventions`,
  `celnet-calendar`, `celnet-core` (the pricing-trait seam / carry model), `celnet-vanilla`
  (FX Garman-Kohlhagen leaf), `celnet-linear` (FX forward/swap/NDF — *already* discounted-
  cashflow products), `celnet-surface`, `celnet-risk-*`, `celnet-limits`, `celnet-server`
  (the edge), `celnet-golden` (the oracle harness).
- **Golden-oracle validation** (`docs/VERIFICATION-CONTRACT.md`): numerical code is validated
  against an **independent** reference (QuantLib / published prices), never merely asserted.
  Circular oracles are explicitly forbidden (the W2 "don't re-derive `F = S·e^{(r_d−r_f)t}`"
  lesson).
- **Server-owned aggregation & hierarchical risk** (`RiskService`, `docs/RISK-HIERARCHY.md`):
  clients never loop-and-sum; the server rolls up over org dimensions in a reporting numeraire.
- **Conventions are first-class per-(instrument, tenor)** config, never global defaults
  (`docs/CONVENTIONS.md`).
- **Scale is a requirement**: investment-banking-sized portfolios; streaming to HPC
  counterparties at the latency/throughput budgets in `docs/ARCHITECTURE.md §1.2` and
  `docs/SCALE-OUT.md`; zero-alloc pinned hot core.

The agent must produce designs that **extend** this estate (new crates, additive proto arms,
the same five-client parity), not a parallel silo.

---

## 3. Guardrails & constraints (non-negotiable — from `CLAUDE.md`)

These bound every recommendation. A design that violates one is out of scope.

1. **OSS / permissive only.** MIT/Apache-2.0/BSD software and free, academically-grounded
   methods. **No commercial products** — no Numerix/Murex/FINCAD/Bloomberg terminal/MKL as
   runtime deps. QuantLib / ORE (both modified-BSD/permissive) as oracles is fine. **NB
   (research pass 1): `rateslib` is source-available NON-commercial — NOT OSS — and FinancePy
   is GPL-3.0, so neither is dependency-eligible** (FinancePy only as an out-of-process oracle).
   `cargo-deny` enforces the license set.
2. **Vendor-neutral, purpose-named identifiers.** No vendor/competitor/person/paper names in
   product crate/type/trait/fn identifiers (e.g. `SwaptionInputs`, not `HullWhiteInputs`).
   Mathematical-method provenance lives in **doc comments only**.
3. **Single, current contract.** No versioned APIs, no `schema_version`, no back-compat shims.
   FI arms are **additive** to the one `celnet.wire` contract; all five clients evolve together.
4. **Validate against references, never assert.** Every model/number reaches a reference by an
   **independent route** (a genuinely different engine + a structural identity).
5. **Scale & performance from day one.** Algorithms/data structures chosen for IB-sized books
   and the streaming budgets; telemetry must not touch the pinned hot path.
6. **Leverage current academic research**; cite the paper/method in code comments and docs.

---

## 4. Scope of "fixed income" (the universe to cover — and phase)

The agent must define **in/out and phasing**, but cover at least the following so nothing is
silently dropped. Tag each with a recommended phase (P0 foundation → P3 advanced).

### 4.1 Curves & term structure (the foundation — almost certainly P0)
- Multi-curve framework: **RFR/OIS discounting** + projection curves; the post-LIBOR world
  (SOFR, €STR, SONIA, TONA, SARON), term-RFR vs compounded-in-arrears.
- Curve construction: deposits, FRAs, futures (with convexity adj.), OIS, IRS, basis swaps,
  cross-currency basis; bootstrapping vs **global** (smooth/spline/ tension) calibration;
  interpolation (log-linear DF, monotone-convex forwards, Hagan-West).
- Discount-factor / zero / instantaneous-forward representations; turn-of-year & central-bank
  meeting-date jumps; multi-currency / collateral (CSA) discounting & cheapest-to-deliver
  collateral.

### 4.2 Cash instruments
- Money market: deposits, CDs, CP, T-bills; day-count & settlement conventions.
- Government bonds: fixed-coupon, accrued interest, clean/dirty, yield↔price, conventions per
  market (UST, Gilt, Bund, JGB, OAT, …); ex-div, settlement, T+1/T+2.
- Credit/corporate bonds: Z-spread, asset-swap spread, OAS; callable/putable (see 4.4).
- Inflation-linked: index-linked bonds (real curve, indexation lag, deflation floors).
- Floating-rate notes; amortising/sinking structures.

### 4.3 Linear rates derivatives
- FRA; STIR futures & bond futures (CTD, conversion factor, basis); IRS (fixed-float),
  OIS, tenor **basis swaps**, **cross-currency** swaps (mark-to-market & constant-notional),
  inflation swaps (ZC & YoY).
- PV, par rate, DV01/PV01, key-rate / bucketed deltas, carry & roll-down.

### 4.4 Rates volatility / optionality
- Swaptions (physical/cash, European; Bermudan as advanced); caps/floors; **SABR** smile
  calibration; the normal (Bachelier) vs lognormal world (negative rates ⇒ normal/shifted-
  lognormal); the swaption **vol cube** (expiry × tenor × strike).
- CMS & CMS spread options (convexity/replication); callable bonds & range accruals (advanced).

### 4.5 Credit (likely a later phase, but scope it)
- Single-name CDS, the ISDA Standard Model, hazard-rate/survival-curve bootstrapping; index
  CDS; recovery; CVA/DVA touchpoints (cross-ref the existing XVA work if any).

### 4.6 Cross-cutting
- Repo / financing / funding curves; collateral & CSA discounting; settlement & lifecycle.
- **Risk**: PV01/DV01, key-rate durations, convexity, cross-gamma, scenario/curve shifts,
  VaR/ES inputs, **FRTB** sensitivities (GIRR/CSR) — mapped onto the existing
  `RiskService`/`docs/RISK-HIERARCHY.md` hierarchy.
- Market conventions & standards: **ISDA** definitions (2006/2021), day-count fractions,
  business-day conventions, **RFR** observation/lookback/lockout/payment-delay conventions,
  fixing sources, holiday calendars, rounding.

For each scope item the agent answers: *what it is, the market-standard payoff/convention, the
SOTA model & numerical method, the open reference to validate against, the data it needs, and
its place in the Celnet contract/crate map.*

---

## 5. Research-question map (the agent's investigation backbone)

Organise the work into these tracks; each yields a section of the corpus (§6).

1. **Products & payoffs.** Definitive, market-standard payoff/cashflow definitions for every
   §4 instrument; the conventions that materially change the number; edge cases (stubs,
   broken dates, ex-div, deflation floors, negative rates).
2. **Curves & calibration.** The SOTA multi-curve methodology; which instruments build which
   curve; interpolation choices and their arbitrage/locality trade-offs; collateral/CSA
   discounting; how QuantLib/ORE/Rateslib do it (and where they differ).
3. **Models & numerics.** Per product family: the market-standard model(s) (e.g. Hull-White
   1F/2F, LMM/LFM, SABR, Bachelier, G2++), calibration targets, and the numerical method
   (analytic, tree, PDE, Monte-Carlo, LSM) — with convergence/perf characteristics for
   IB-scale.
4. **Conventions & standards.** The authoritative tables (ISDA/ICMA/local), expressed as the
   *config schema* a `celnet-fixedincome-conventions` crate would carry (mirroring
   `docs/CONVENTIONS.md`'s per-(pair,tenor) record).
5. **Market data.** What inputs each capability needs (curve quotes, vol cubes, fixings,
   bond reference data, recovery), and **vendor-neutral feed shapes** to ingest them (no
   commercial terminal as a runtime dep) — cross-ref `docs/CELER-INTEGRATION.md`.
6. **Risk & analytics.** The sensitivities/measures and how they fold into the server-owned
   hierarchical-risk contract; FRTB GIRR/CSR mapping.
7. **Competitive landscape.** What Murex/Numerix/FINCAD/Bloomberg/ION cover (capability
   benchmark only — never a dependency); and the **OSS reference set** (QuantLib, ORE,
   Rateslib, FinancePy) as oracle + gap analysis. Mirror `docs/COMPETITIVE-ANALYSIS.md`.
8. **Validation & oracles.** A per-product oracle plan: which open engine + which independent
   structural identity validates each number (the anti-circular discipline). Mirror
   `docs/VERIFICATION-CONTRACT.md` (a)–(g).
9. **Performance & scale.** Algorithm/data-structure choices for IB-sized rates books and the
   streaming budgets; what must be SIMD/GPU vs CPU; curve-build & risk-cube cost at scale.
10. **Celnet integration.** The additive `celnet.proto` arms, new crates + dependency arrows,
    the five-client surface (GUI workspace, Excel `CELNET.*` functions, SDK, FIX, federation),
    and how FI reuses the carry/discount seams `celnet-core`/`celnet-linear` already expose.

---

## 6. Deliverables — the fixed-income design corpus (what the agent produces)

All under `docs/fixed-income/` (this dir) unless noted, mirroring the FX corpus shape:

| Deliverable | Mirrors | Contents |
|---|---|---|
| `FI-ANALYTICS-SPEC.md` | `ANALYTICS-SPEC.md` | Definitive products, payoffs, models, numerical methods, sensitivities — market-standard, cited. |
| `FI-CURVES-SPEC.md` | (new) | Multi-curve construction: instruments→curves, interpolation, OIS/CSA discounting, calibration. |
| `FI-CONVENTIONS.md` | `CONVENTIONS.md` | The convention config schema (day-count, BDC, RFR obs, fixings, calendars) per market/tenor. |
| `FI-ARCHITECTURE.md` | `ARCHITECTURE.md`/`INTERFACES.md` | New crates + dependency arrows, additive proto arms, the five-client surface, hot-path/scale plan. |
| `FI-COMPETITIVE-ANALYSIS.md` | `COMPETITIVE-ANALYSIS.md` | Capability benchmark vs vendors + OSS reference/gap analysis. |
| `FI-VERIFICATION-CONTRACT.md` | `VERIFICATION-CONTRACT.md` | Per-product oracle + independent-identity validation plan. |
| `FI-ROADMAP.md` | `ROADMAP.md`/`W*-PLAN.md` | Phased workstreams `W-FI-1…n` with tracks, crates, gates, parallel-lane map. |
| `docs/_research/fixed-income-findings.{md,json}` | `docs/_research/findings.*` | The raw, cited research findings the specs are synthesised from. |
| `OPEN-QUESTIONS.md` | (new) | Operator decisions surfaced by the research (see §9). |

Each spec doc states its **status**, **scope**, and **sources**, and is internally consistent
with the existing corpus (cross-reference, don't duplicate).

---

## 7. Methodology — how to run the agent

1. **Research-first, cite everything** (`rules/.../development-workflow.md` §0): GitHub code
   search + primary vendor/standards docs first (ISDA, ICMA, CME, local DMOs, ECB/Fed RFR
   pages), then academic papers, then broader web. Prefer adopting/porting a proven approach.
2. **Open reference engines as oracle, not dependency:** read QuantLib / ORE / Rateslib /
   FinancePy to learn the canonical method and to **cross-check numbers** — never to embed a
   commercial or GPL-incompatible runtime dep. **Research pass 1 confirmed:** QuantLib + ORE
   are modified-BSD (permissive, usable); `rateslib` is source-available **non-commercial (NOT
   OSS)** and FinancePy is **GPL-3.0** — both excluded as dependencies (FinancePy usable only as
   a disposable out-of-process oracle, never linked).
3. **Structured findings → synthesis.** Capture every finding with its source in
   `docs/_research/fixed-income-findings.json` (claim, evidence, source URL, confidence), then
   synthesise the human-readable specs. This is the same shape as the existing findings corpus.
4. **Numerical proof of understanding.** For each headline product, the agent computes a worked
   example and reconciles it to an open engine + a structural identity (e.g. a par swap has PV
   0; receiver + payer swaption = forward swap via put-call parity; a bond = sum of discounted
   cashflows), documenting the reconciliation.
5. **Decompose & parallelise.** Treat §5's tracks as independent investigations; fan out, then
   a synthesis/completeness pass ("what modality/source/claim is missing?").
6. **Iterate with the operator** at the §9 decision points before locking phasing.

---

## 8. Acceptance criteria (when a deliverable is "done")

- **Complete:** every §4 scope item is covered or explicitly deferred with a reason.
- **Cited:** every non-obvious claim/convention/number has a primary source.
- **Validated:** each headline product has a worked example reconciled to an open engine + an
  independent identity (no circular oracle).
- **Buildable:** `FI-ARCHITECTURE.md` names concrete crates, dependency arrows, additive proto
  arms, and the five-client surface — not prose aspirations.
- **Guardrail-clean:** no commercial runtime deps; vendor-neutral identifiers; single additive
  contract; scale addressed.
- **Phased:** `FI-ROADMAP.md` sequences `W-FI-*` workstreams with per-workstream gates aligned
  to `docs/VERIFICATION-CONTRACT.md`.

---

## 9. Open questions for the operator (the agent must surface, not assume)

The research will hit decisions only the business can make. The agent collects these in
`OPEN-QUESTIONS.md`; seed list:

- **Market & currency scope of P0** (which RFR curves/currencies first: USD-SOFR, EUR-€STR,
  GBP-SONIA …?).
- **Cash vs derivatives first** (bond analytics vs the swap/curve engine as the P0 wedge).
- **Credit in scope now or later** CDS/ISDA model is a distinct workstream
- **Collateral/CSA discounting depth** for v1 single-curve OIS and full multi-CSA
- **Real-time vs analytics-first** FI need the streaming RFS and streaming esp
- **Reference-data sourcing** (bond static, calendars, fixings) given the no-commercial-feed
  guardrail.

---

## 10. Additional notes from client conversation

Core components

1. Current starting point ( Existing Marex Infrastructure)
 
Selective Principal Risk:

 

Paired with agency back-to-back riskless execution, we rely on our balance sheet to provide targeted liquidity during market hours. Given our small balance-sheet risk appetite we do not compete with institutional tier-1 banks, rather tier-2 and tier-3 banks. We do not stream prices or axes; we do not engage with automation protocols such as auto-respond or auto-RFQ. Given the high level of automation at most of the top clients globally, these are non-negotiable technology requirements given the rise in electronic trading flows across all bond types.

 

On a positive, we can offer cross-asset clearing and provide margin efficiencies and risk minimization across related financial products. We can cross-sell and benefit from unique liquidity access within the Capital Markets trading hubs, cash and derivatives.

 

2. Modular technology embedded in Celer infrastructure

 

Front-end pricing and IOIs aggregation:

 

Providing a highly customizable graphical user interface (GUI) which can integrate order management (OM), execution management (EM), and post-trade (PT) allocation functionalities. We need an algorithmic engine to process automated market-making and execution workflows across multiple regional jurisdictions for a global offering.

 

The GUI can be the client-facing solution in MX1, where price, axes, protocols, trade ideas can be advertised, used and executed. Highly modular. Highly customisable.

 

The core trading engine (CORE) must be built around key principles, transparency, liquidity, resiliency, efficiency and security.

 

It connects external price-makers via FIX protocols and APIs to identify price differentials and asset mispricing in real-time. Celer infrastructure can program algorithmic execution tools which can be tailor-made depending on products, regions and liquidity environment (Smart Order Routers).

 

CORE must stream and contribute to the main fixed-income trading venues globally, namely Tradeweb, MarketAxess, Trumid and Bloomberg (+Brokertec and CME)

MX1 must also connect and interact with all available liquidity pools globally.

 

As discussed, a few weeks ago, we are looking at 2 proposals for MX1 :

a highly automated electronic market-making workflow we can apply locally first then globally as we scale, and
an opportunistic alpha-generating trading engine which identify mispricing, yield anomalies, and price arbitrage in both cash and FX for EM bonds.
A third option would be to offer composite strategies such as basis trades, asset swap packages, CDS versus single name or ETF share versus underlying constituents’ arbitrage.

 

We would also add some pre-trades anchors, such a real-time pricing grids that continuously scan the basis between local EM bonds, FX forwards, and offshore bonds to lock in the basis arbitrage with minimal slippage. Another product would be to aggregate illiquid bond quotes across diverse electronic venues and dark pools to identify arbitrage opportunities and create executable trade signals.

 

Of course we must feed Celer with various data sources including bond data, referential and real-time (quotes and axes) and historical data so we can build real-time curves and price skewing capabilities.

 

Critical to MX1 is the ability to contribute and code up to all trading venues as trading protocols have been heavily focused on low-touch to no-touch/robotic execution as well as the use of Smart Order Routing capabilities to access all liquidity pockets.

Another critical point will be the internalisation of flows, especially where strategies are built on multi-legged executions which could benefit internal desks.

Last but not least, our prime and clearing solutions should give us a clear edge as we work on growing MX1 into a cross-asset interface within Marex Group.

 

 

Key Features  summary

Low latency connectivity to external venues 
Data is being ingested, normalised and presented as a synthetic price (bid / ask + depth)  from multiple feeds in the GUI  
Users can interact with a price feed to skew / mark up from the raw feed  
Pre-trade added value, tradable ideas
Marex Market makers can view real time curve, with data feed-back Loop 
Marex Market makers can construct synthetic contracts  
Marex Market makers can view and manage risk on executed contracts 
Contribution capabilities (Streaming Bid / Ask)  on Bonds Universe + auto pricing of composite / strategies 
End-users can view and build synthetic products 
RFQ workflow with autoquote capabilities (price to trade), including on synthetic products 
Full Data collection  
Trades can be passed to a post trade platform for booking purposes  
Dashboard for MI  
Pre- and post-trade analytics (TCA, audit trail)
