# Celnet Risk Model — Requirements & Gap Analysis

**What this document is.** A requirements specification and honest gap analysis for Celnet's
internal risk model, covering four threads the desk has asked for:

1. **DV01-based internal risk limits** — embedding a real DV01 budget in the platform.
2. **Tenor-bucketed hedging** — hedging each of 2/5/10/30 with its own future rather than
   covering a whole book with the 10Y, because a single-point hedge leaves **curve risk**.
3. **Credit risk** — what it actually is for a corporate-bond book, and whether **CDS** or
   **asset-swap** hedging is the right answer.
4. **A multi-tier, user-configurable risk model** — so a desk can configure each *type* of
   risk with its own limits, bands, and exit policy.

**What this document is not.** It is not a design that assumes the platform already works the
way the marketing does. Every statement about current behaviour is anchored to a file and line
in the working tree as of 2026-08-11, including the **large uncommitted change** (the hedge
**vehicle** model and the Treasury-futures reference data). Where a claim could not be
verified by reading code, it is marked **unverified** rather than asserted.

**Read alongside:**

| Document | Purpose |
| --- | --- |
| [`HEDGING-AND-RISK-EXIT.md`](HEDGING-AND-RISK-EXIT.md) | The as-built explainer for how risk lands, is measured, and exits. This document is its forward-looking counterpart. |
| [`RISK-HIERARCHY.md`](RISK-HIERARCHY.md) | The cube/limits architecture the multi-tier model must fit inside, not replace. |
| [`AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md`](AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md) | The cited academic basis for the warehouse/band/shed machinery. |
| [`FI-CREDIT-ENGINE-DESIGN.md`](FI-CREDIT-ENGINE-DESIGN.md) | The existing (design-only) credit-engine proposal. §4 below assesses and amends it. |

---

## 0. Executive summary

**The single most important finding.** Celnet has **two entirely disconnected rate-risk
measurement systems**, and the desk's real money runs on the wrong one.

- **System A — the live booking/hedging path.** `rates_linear_exposure`
  (`crates/celnet-server/src/services/rates_book.rs:2600`) reduces every position to a
  **single signed scalar**, and its **bond arm is `redemption × 1bp`**
  (`rates_book.rs:2647-2653`) — every bond treated as duration 1. This is what
  `book_net_dv01` (`rates_book.rs:2104`), the RAG band, the warehouse cap, the overflow, the
  hedge trigger and the pre-trade limit gate (`rates_pre_trade`, `rates_book.rs:2906`) all
  read. It has **no tenor dimension whatsoever**.
- **System B — a genuine, correct, tested key-rate engine.** `celnet-rates-risk` computes a
  real per-pillar key-rate DV01 by shocking individual curve pillars
  (`crates/celnet-rates-risk/src/ladder.rs:115-132`,
  `crates/celnet-rates-risk/src/curve_shock.rs:146-154`), and `celnet-limits` already carries
  **`LimitMetric::RateTenorBucket { tenor_years }`** (`crates/celnet-limits/src/limit.rs:85-89`)
  with a working evaluator (`crates/celnet-limits/src/check.rs:147-152`).

System B is **structurally unreachable from System A**. The booking path builds an
FX-Greeks `NodeAggregate` and stuffs the scalar proxy into `greeks.delta_base`
(`rates_book.rs:2917`); `exposure_of` returns **hard-coded `0.0`** for `Dv01`, `Pvbp` and
`RateTenorBucket` on such a node (`check.rs:113`). So a tenor-bucket limit configured today
would be **silently inert on every booked fill** — it can never breach, because the only node
the booking path constructs cannot carry the number it reads.

The tenor-bucketed DV01 the desk is asking for is therefore **~70% built and 0% connected**.
The work is an integration slice, not a green-field quant build — with one genuinely hard
piece (§3.5).

**Credit recommendation, up front (full argument in §4).** The user's framing — *"credit risk
= CDS? Or asset-swap hedging to lock in the credit spread"* — contains a common and
consequential conflation that should be corrected before any requirement is written:

> **An asset swap is not a credit hedge.** It removes the bond's *interest-rate* risk and
> converts the holding into a synthetic floating-rate note paying a spread. The issuer's
> default and spread-widening risk stays **100% with the holder**. What an asset swap "locks
> in" is the *carry*, not the exposure.
>
> **CDS is the credit hedge.** It is the only instrument in the set that actually transfers
> default and spread risk to someone else.

The recommendation is therefore **not** "CDS or asset swap" but a three-layer answer:

1. **Rates leg → tenor-bucketed government-bond futures.** This is what the uncommitted
   vehicle model is already building, and it is the cheaper, more liquid form of exactly what
   the asset swap's swap leg does. Build this. It is the correct first hedge.
2. **Credit leg → measure CS01 first, hedge with a CDS *index* (CDX.NA.IG / iTraxx Europe
   Main), beta-weighted.** Not single-name CDS: single-name liquidity is concentrated in a
   few dozen names and is not a dealable hedge for a general corp inventory (§4.3).
3. **Asset swap → keep it as a *calibration input and a quoting convention*, not a hedge
   vehicle.** The ASW spread is how the street quotes corp credit and is the natural input to
   bootstrap a credit curve. Celnet already has an (unwired) `asset_swap_spread`
   implementation at `crates/celnet-rates/src/bond.rs:229`.

**Top five gaps** and **sequencing** are in §6 and §7.

---

## 1. The as-built measurement chain, in one page

Everything below is a change to this chain, so it is worth stating precisely. A fill's journey
through risk measurement today:

```
fill books
  │
  ├─▶ rates_linear_exposure(position) ──────────────► ONE SIGNED SCALAR
  │     OIS/IRS : notional × tenor_years × 1bp       (rates_book.rs:2600-2655)
  │     FRA     : notional × window_years × 1bp
  │     Bond    : redemption × 1bp        ◄── DURATION-BLIND
  │
  ├─▶ book_net_dv01(book)  /  subtree_net_dv01(root, descendants)
  │     (rates_book.rs:2104 / :2117)                 — sum of the scalar. No tenor axis.
  │
  ├─▶ rates_pre_trade  →  NodeAggregate{ net_greeks.delta_base = scalar }
  │     (rates_book.rs:2906-2940)                    — scopes: Book → Entity → Firm only
  │     evaluated by celnet-limits::pre_trade_check against LimitMetric::Delta
  │     ⚠ LimitMetric::{Dv01, Pvbp, RateTenorBucket} read 0.0 here (check.rs:113)
  │
  ├─▶ WarehouseThreshold (band.rs:58-79) resolved Instrument → Book → Desk
  │     (resolve_hedge_threshold, rates_book.rs:2512-2527)
  │     utilization = |net| / cap  →  RAG band  →  breached  →  overflow
  │
  ├─▶ exit-policy graph, scoped Book → Bucket → Firm
  │     (HedgePolicyScope, config/hedge_policy.rs:413-420)
  │     → an ExitAction + (NEW) a HedgeVehicle + (NEW) a HedgeExitMode
  │
  └─▶ EXIT
        sizing numerator: genuine_position_dv01 (rates_book.rs:2390-2427)
          bond → celnet_bond analytic DV01, Dv01Basis::Analytic   ◄── CORRECT
          swap → notional × years × 1bp,    Dv01Basis::AnnuityPv01
          fallback → the duration-blind proxy, Dv01Basis::ExposureProxy (flagged)
        ratio: plan_hedge_ratio (vehicle.rs:563-594), whole-lot rounding + signed residual
        offsetting leg: offsetting_rates_leg (rates_book.rs:2558) — the fill's OWN security
```

**The load-bearing asymmetry, restated because everything in §2 and §3 follows from it:**
the *hedge size* is now computed on a genuine analytic DV01, but the *decision to hedge at
all* — cap, utilisation, band, overflow, and every pre-trade limit — is still computed on the
duration-1 proxy. Those are two different numbers and only one of them has been fixed. The
as-built explainer says this itself
([`HEDGING-AND-RISK-EXIT.md` §10 boundary 1](HEDGING-AND-RISK-EXIT.md)).

### 1.1 Three scope vocabularies, currently unreconciled

A multi-tier configurable model (§5) has to live inside these, and today there are three
different ones:

| Mechanism | Scope vocabulary | Precedence | Source |
| --- | --- | --- | --- |
| Warehouse threshold, LP panel, exit **mode** | `Instrument` / `Book` / `Desk` | most-specific-wins, instrument first | `config/hedge_policy.rs:195-200`; `rates_book.rs:2512-2527` |
| Exit **policy** graph | `Firm` / `Book` / `Bucket` (subtree root) | Book → Bucket → Firm | `config/hedge_policy.rs:413-420` |
| **Limit tree** | `Firm` / `Trader` / `Book` / `Desk` / `CcyPair` / `Location` / `Entity` | cascade, all nodes on the path checked simultaneously | `crates/celnet-limits/src/tree.rs:58-73` |

Three overlapping-but-different vocabularies is a real integration hazard, not a cosmetic
one: a desk that configures a "desk-level DV01 limit" today gets three different meanings
depending on which surface they used. Note also
[`HEDGING-AND-RISK-EXIT.md` §10 boundary 3](HEDGING-AND-RISK-EXIT.md): **`Desk`-scoped
bindings do not bind at all on the rates booking path** because the call site passes an empty
desk id. §5.4 proposes the reconciliation.

---

## 2. Requirement 1 — DV01-based internal limits

### 2.1 What the desk is asking for

> *"The system should embed our internal risk limits using DV01."*

A DV01 limit means: for a given scope (book, desk, firm, and — §3 — a given tenor bucket), the
**net present-value change of the book for a 1bp move in rates** must not exceed a configured
cap, with soft (warn/skew) and hard (block) enforcement, and utilisation surfaced as a
first-class number.

### 2.2 What exists

Almost all of the *shape*, none of the *substance*.

- **The limit taxonomy exists and already names DV01.** `LimitMetric::Dv01` ("net parallel
  DV01"), `LimitMetric::Pvbp` ("net analytic PV01") and
  `LimitMetric::RateTenorBucket { tenor_years }` are defined with careful sign conventions at
  `crates/celnet-limits/src/limit.rs:68-89`, and `LimitMetric::is_fixed_income()`
  (`limit.rs:105-110`) already separates the FI family from the FX-Greeks family.
- **Soft/hard enforcement, amber/red bands and utilisation exist** — `Enforcement`
  (`limit.rs:125-129`), `LimitSpec`, `RagStatus`, `Utilization` (`celnet-limits/src/lib.rs:84`).
- **A working FI evaluator exists.** `exposure_of_rates`
  (`crates/celnet-limits/src/check.rs:143-165`) reads `net_dv01`, `net_pv01` and the per-tenor
  bucket off a `celnet_risk_fleet::RatesNodeAggregate`, correctly and cheaply (O(ladder), no
  re-derivation).
- **It is enforced somewhere.** `enforce_fi_limits` in
  `crates/celnet-server/src/services/rates_risk/aggregate.rs:188-221` hard-rejects an
  `AggregateRatesRisk` request whose tenor bucket breaches.

### 2.3 What is missing — three defects, in order of severity

**Defect 1 (CRITICAL) — the DV01 the limits gate on is not a DV01.**
`rates_linear_exposure`'s bond arm is `redemption × 1bp` (`rates_book.rs:2647-2653`). A 10-year
corporate bond's true DV01 is roughly **8×** that. Consequences:

- A bond-heavy book's utilisation is understated by roughly its portfolio duration, so the
  hedge trigger fires far too late — or never.
- Worse, **bond and swap risk are not commensurable in the same book net**. A 10-year swap
  contributes `notional × 10 × 1bp`; a 10-year bond of equal face contributes
  `redemption × 1bp`. Netting them is arithmetic on two different units. A book that is
  economically flat can report large net risk, and vice versa. This is the defect that makes
  *any* DV01-based limit on a mixed book meaningless today.

**Defect 2 (CRITICAL) — the FI limit metrics are structurally inert on the booking path.**
`rates_pre_trade` (`rates_book.rs:2906-2940`) constructs a `NodeAggregate` (the FX-Greeks
shape) and assigns the scalar proxy to `greeks.delta_base` (`rates_book.rs:2917`). For that
node, `exposure_of` returns literal `0.0` for `Dv01`, `Pvbp` and `RateTenorBucket`
(`check.rs:113` — deliberately, and correctly, so a misconfigured rates limit cannot
spuriously breach an options booking). So today:

> A DV01 limit or a tenor-bucket limit set on a risk book **evaluates to zero exposure at
> booking and can never breach**. The only metric that actually gates a rates booking is
> `LimitMetric::Delta`, charged with the duration-blind proxy.

This is not a bug in `celnet-limits` — that crate is right. It is a missing integration: the
booking path never builds the `RatesNodeAggregate` that the FI metrics read.

**Defect 3 (HIGH) — the correct rates-risk aggregate is never fed from booked positions.**
Both genuine engines are **stateless inline calculators**: `AggregateRatesRisk`
(`services/rates_risk/aggregate.rs:32-35`) and `CombinedTailRisk`
(`services/risk/combined_tail.rs:19-21`) both document explicitly that the whole portfolio
travels on the request — *"no store read"*. The GUI panel that renders the key-rate ladder
(`gui/src/workspaces/RatesRiskWorkspace.tsx`, driving `gui/src/viz/KeyRateLadder.tsx`) is an
ad-hoc trader-editable position table, **not the live risk book**. So the platform can compute
a beautiful key-rate ladder for positions you type in, and computes nothing of the kind for
the positions it actually holds.

### 2.4 Requirements

| # | Requirement | Rationale |
| --- | --- | --- |
| **R1.1** | `rates_linear_exposure`'s bond arm MUST be replaced with a duration-correct DV01, so that bond and swap exposures are commensurable in one book net. | Defect 1. Without this nothing downstream means anything. |
| **R1.2** | The correct DV01 MUST be **cached on the position at book time**, not recomputed per query. It is a per-position analytic number that changes only when the curve moves. | §3.5 — the performance requirement. |
| **R1.3** | The booking path MUST construct a `RatesNodeAggregate` (net DV01, net PV01, key-rate ladder) for each scope on the position's path, so `exposure_of_rates` is reachable. | Defect 2. |
| **R1.4** | A DV01 limit MUST be settable at every scope in the reconciled vocabulary (§5.4) and MUST support soft (warn/skew) and hard (block) enforcement, with utilisation surfaced. | The taxonomy already supports this; only the wiring is missing. |
| **R1.5** | The warehouse `WarehouseThreshold.metric` MUST be able to be `Dv01` **and mean it** — i.e. resolve against the same duration-correct aggregate the limits read, not the proxy. | `band.rs:58-79` already carries a `LimitMetric`; today `Dv01` there silently means the proxy. |
| **R1.6** | Any surface presenting a DV01 MUST carry the `Dv01Basis` provenance already defined at `crates/celnet-hedge-routing/src/vehicle.rs:135-149`, extended from the hedge-sizing path to the *measurement* path. | Guardrail 2: a proxy number must never be presented as exact. |

---

## 3. Requirement 2 — tenor-bucketed DV01 and curve risk

### 3.1 The desk is right, and here is the precise reason

> *"we can work out a hedge ratio if we cover with 10y, but we would take a curve risk, so
> likely to try and stick to each tenor's future 2/5/10/30."*

This is correct and worth stating formally, because it is the exact blind spot in the current
model. Suppose a book is long 2-year corps and short 10-year corps such that the **net
parallel DV01 is zero**. Today `book_net_dv01` reports ~0, utilisation is ~0, the band is
Green, and no hedge fires. But the book is a large **steepener**: a curve flattening or
steepening move produces real P&L with no parallel component at all. The single-scalar model
is blind to it by construction.

Equally, hedging a whole book's net DV01 with the 10Y contract alone converts *outright* risk
into *curve* risk: you have removed the parallel exposure and created a
long-10Y-versus-short-everything-else spread position that nobody chose and no limit measures.

**A single-point hedge is not a smaller risk than no hedge — it is a different risk.** That is
the requirement's whole justification.

### 3.2 Bucketed DV01 or full key-rate duration? — an honest answer

These are not the same thing, and the platform needs **both, at different places**:

- **Full key-rate DV01 (fine grid)** — the sensitivity to each *calibrating instrument's* zero
  pillar, holding the others fixed. `celnet-rates-risk` already implements this properly
  (`ladder.rs:115-132`, over whatever pillar set `RatePillars` was built with). This is the
  right **measurement** grid: it is model-consistent, additive, and reconciles to the parallel
  DV01 by construction (their sum is the parallel bump, up to interpolation effects).
- **Bucketed DV01 (coarse grid)** — the exposure projected onto a small set of buckets. This
  is the right **hedging and limits** grid.

**Recommendation: measure on the fine key-rate ladder; hedge and limit on a coarse bucket set
projected from it.** The reason is bluntly practical: the hedge instrument set is
**six contracts** (ZT/ZF/ZN/TN/ZB/UB — `crates/celnet-refdata/src/futures.rs:232-316`). A risk
decomposition finer than the instrument set that will trade against it is **unactionable
precision**. Reporting 10 key-rate buckets and then telling a trader to hedge with 4 contracts
just relocates the aggregation decision from the platform to the trader's head, which is
exactly where it should not live.

Full key-rate duration in the strict textbook sense — bumping a piecewise-linear "key rate"
shape and re-bootstrapping — is **not** warranted here and should be explicitly declined. It
buys a marginally different interpolation attribution at the cost of a bootstrap solve per
pillar (`crates/celnet-rates/src/risk.rs:76-90` shows what that costs: bump a par quote,
`bootstrap_ois` again). `celnet-rates-risk`'s approach — shocking already-built zero pillars
without re-bootstrapping (`curve_shock.rs:95-109`) — is the right cost/accuracy trade for a
per-fill decision path, and it is already the one that is built.

### 3.3 The bucket set, and its mapping to a vehicle

The bucket set must be **configuration, not a constant**, because it is currency-specific and
because the hedge instrument set differs by market. But the shipped default for USD should be
the one the desk named, mapped 1:1 onto the contracts already defined in
`crates/celnet-refdata/src/futures.rs`:

| Bucket | Maturity span `[min, max)` | USD hedge vehicle | Contract |
| --- | --- | --- | --- |
| 2Y | `[1, 3)` | 2-Year T-Note future | `ZT` |
| 5Y | `[3, 7)` | 5-Year T-Note future | `ZF` |
| 10Y | `[7, 12)` | 10-Year T-Note future | `ZN` (or `TN` Ultra 10Y) |
| 30Y | `[12, 40)` | T-Bond / Ultra T-Bond future | `ZB` / `UB` |

Two things to notice about this table, both of which are **already half-built**:

1. **The `[min, max)` maturity buckets are exactly the shape the new `HedgeVehicleRegistry`
   already resolves on** (`crates/celnet-hedge-routing/src/vehicle.rs:191-197`,
   `resolve` at `vehicle.rs:420-441`). The registry's example rows in its own tests are
   literally `[3,7) → FV` and `[7,12) → TY` (`vehicle.rs:601-629`). The vehicle model has
   independently arrived at the right bucket geometry.
2. **But the registry buckets on the *position's own maturity*, not on where its risk sits.**
   `hedge_maturity_years` (`rates_book.rs:2437-2460`) returns a single maturity for a whole
   position. For a zero-coupon-like instrument that is fine. For a coupon bond it is an
   approximation — a 30-year bond has meaningful key-rate exposure at 5Y and 10Y from its
   coupon stream, not only at 30Y. The registry as built maps *the instrument* to a bucket;
   the requirement is to map *the risk* to buckets. This is the substantive design difference
   between what has been built and what is needed.

### 3.4 Is `LimitMetric::VegaBucket`/`TenorVega` the right precedent?

**Yes, and the precedent has already been followed — that is the good news.**
`LimitMetric::RateTenorBucket { tenor_years: u32 }` (`limit.rs:85-89`) is explicitly documented
as *"the linear fixed-income counterpart to `LimitMetric::TenorVega`"*, and its evaluator
(`check.rs:147-152`) mirrors `TenorVega`'s (`check.rs:96-101`) exactly: filter the additive
ladder to the bucket, sum. The shape is right, the sign convention is documented, the
additivity holds, and the pre-trade cost is O(ladder) with no re-derivation.

Two refinements are needed on top of the existing shape:

- **`tenor_years: u32` is too coarse a key for a bucket, and too fine for a pillar.** It
  currently means "the calibrating-instrument tenor in whole years", which cannot express a
  3-month or 18-month pillar and cannot express a `[7,12)` bucket. Requirement: introduce a
  **`TenorBucketId`** value type carrying the `[min, max)` span in years, so the limit key and
  the vehicle-registry key are **the same type** and cannot drift apart.
- **`RISK-HIERARCHY.md` §5.1's promised "tenor-bucket limit" is only half-delivered.** The
  document lists bucketed-vega and tenor limits as a first-class limit type; the rates
  counterpart exists as a metric but, per §2.3 Defect 2, is unreachable on the booking path.
  The document's own framing (`HEDGING-AND-RISK-EXIT.md` §10 boundary 1, and the inline
  comment at `rates_book.rs:2638-2646`) calls the exact curve-bootstrapped bond DV01 the
  **"ADR-0016 A3 breadth wave"**. **That wave was planned and is not built:** the actual
  ADR-0016 (`docs/adr/ADR-0016-governance-risk-and-latency-gate.md`) is about the pre-trade
  limit gate, hot-core embargoes and the latency SLO; A1 (wiring `pre_trade_check` into the
  execution path) landed, and the A3 "curve-bucketed DV01 / tenor pre-trade limits" item is
  referenced as still-outstanding in `docs/gui-redesign/SINGLE-FRONTEND-BUILD-PLAN.md:50` and
  `docs/plan/P1-LANES.md:26`. So the A3 label in the code comment is an accurate forward
  reference to unbuilt work, not a claim that it shipped.

### 3.5 What has to change — and the part that is genuinely hard

**Measurement.**

- Replace the scalar `rates_linear_exposure` with a **`RatesExposure` value carrying both** a
  signed parallel DV01 and a bucket ladder. Keep the scalar as a derived `parallel()` accessor
  so the ~162 existing call sites that legitimately want one number keep working.
- Populate the ladder from `celnet_rates_risk::key_rate_ladder`
  (`crates/celnet-rates-risk/src/ladder.rs:115-132`) at book time, then **project** the fine
  pillars onto the configured bucket set.
- Cache the ladder **on the position**. This is the load-bearing performance decision.

**This is the genuinely hard part, and it should not be understated.** A key-rate ladder is
not a match-arm addition:

- **Cost.** `key_rate_ladder` builds one shocked `Curve` per pillar and reprices every position
  under each (`ladder.rs:119-122`). For `N` positions and `P` pillars that is `N × P`
  repricings. Doing that per fill, for an investment-banking-sized book, on the booking tier,
  is not viable — it is precisely the bump-and-revalue trap `RISK-HIERARCHY.md` §3.3 identifies
  and says the platform must not fall into.
- **The architecture that makes it viable** is the one `RISK-HIERARCHY.md` §3.3 already
  specifies for the FX cube and which has simply never been applied to rates: the ladder is
  **additive**, so compute each position's ladder **once** at book time (cost `P` reprices for
  *that one position*, not the book), store it, and net the ladders on roll-up in `O(depth)`.
  Recompute on **curve refresh**, not on fill. Under that design the per-fill cost is `O(P)`
  for the new position plus `O(depth × P)` ancestor updates — genuinely cheap.
- **The curve dependency is a new coupling.** Today `rates_linear_exposure` is deliberately
  **curve-free** and deterministic — that is why it is safe to call on the booking path
  (`rates_book.rs:2529-2539` documents this as an intentional conservative choice: undiscounted
  PV01 is an upper bound on the true annuity PV01, hence fail-safe for a hard limit). Making
  the risk measure curve-dependent means the booking gate now depends on live market data, on
  a bootstrapped curve being available, and on what happens when it is **not**. That is a real
  design question with a real failure mode, and it needs an explicit answer:
  **requirement — when no curve is available the measure MUST fall back to the conservative
  proxy and stamp `Dv01Basis::ExposureProxy`, never block silently and never present the
  fallback as exact.** The `Dv01Basis` machinery for exactly this already exists at
  `vehicle.rs:135-169`.
- **Reconciliation is mandatory, not optional.** The sum of the bucket ladder must reconcile
  to the parallel DV01 within tolerance, and to the analytic `celnet_bond::dv01`
  (`crates/celnet-bond/src/risk.rs:35-122`) for a single bond. Without that gate the two
  systems will drift and nobody will notice until a hedge is wrong.

**Thresholds and bands.** `WarehouseThreshold` (`band.rs:58-79`) is per-`(scope × metric)` and
already carries a `LimitMetric`. A tenor-bucket budget is then just a threshold whose metric is
`RateTenorBucket{bucket}` — no new primitive needed. But the **band classification must run
per bucket**, producing a per-bucket RAG, and the book-level band becomes an aggregation over
bucket bands (worst-bucket, plus the parallel band). That is a real change to
`stamp_internalise`'s single-band assumption.

**Exit policy.** `HedgeField` (per `HEDGING-AND-RISK-EXIT.md` §6.2) exposes `NetDv01`,
`Utilization`, `Overflow`, `Breached` as scalars. Requirement: add a **bucket-qualified**
form so a rule can say *"if the 10Y bucket is breached, submit a market order hedged into
`ZN`"*. The natural expression is a `Tenor`-valued condition field the rule already almost
has (`RouteField::Tenor` exists on the routing side).

**Offsetting leg.** This is the part that must change most, and it is where the current
uncommitted design has a stated hole. Today `offsetting_rates_leg` (`rates_book.rs:2558`)
books the offset **in the fill's own security** scaled by a factor. Under a vehicle hedge the
platform trades a *future* on the street but books a risk-equivalent in the *underlying*
(documented honestly at `HEDGING-AND-RISK-EXIT.md` §8.2.1 and §10 boundary 7). That is
tolerable for a parallel-DV01 model. **It is not tolerable for a bucketed model**, because
scaling down the underlying reduces the risk in *that bond's* buckets, whereas selling a 10Y
future removes risk from the *10Y bucket only*. The bucket ladder will be wrong in a way the
parallel number hides.

Fixing this properly means giving `RatesInstrument` a **futures arm**. The proto oneof today
has exactly four arms — `ois`, `irs`, `fra`, `bond`
(`crates/celnet-proto/proto/celnet.proto:7955-7967`) — and there are **162 match sites** on
`rates_instrument::Instrument::` across the workspace (measured). That is a large but
mechanical change; the non-mechanical part is that a futures position needs its own pricing
and its own DV01, which is where §3.6 comes in.

### 3.6 `dv01_per_unit`: configured vs derived

The registry's `dv01_per_unit` is admin-entered (`vehicle.rs:209-210`), documented as a stated
boundary (`HEDGING-AND-RISK-EXIT.md` §10 boundary 8), while the new
`celnet-refdata::futures::dv01_per_contract(reference_yield)`
(`crates/celnet-refdata/src/futures.rs:441-472`) **can derive one**. These should meet — but
the honest position is that meeting them fixes *staleness*, not *accuracy*:

- The derivation builds a synthetic **6% semi-annual notional deliverable** maturing at the
  midpoint of the published deliverable window and runs `celnet_bond::dv01` on it, scaled by
  contract point value. The 6% figure is the CBOT conversion-factor coupon (cited to Rules
  18101.B/19101.B/20101.B/21101.B/26101.B/40101.B in the file's own header table,
  `futures.rs:27-34`).
- The module is explicit that this is **not** the exchange's live basis-point value: the
  cheapest-to-deliver security and the delivery option are not modelled, and at sub-6% market
  yields the standardised figure **understates the live DV01 by roughly 10–20%**
  (`futures.rs:74-83`).

**Requirement:** the registry's `dv01_per_unit` should become **derived-with-override** — the
platform derives from the futures reference data at the live curve yield, an administrator may
pin a value, and the resolved number carries provenance (`derived` vs `configured`) and the
yield it was taken at. A genuine CTD-and-conversion-factor model is a **further, separate**
step and should be scoped as such rather than implied. Note `crates/celnet-rates/src/futures.rs`
already imports `celnet_bond::{CashBond, price_at_yield}` for CTD-flavoured pricing
(reported by audit; not personally opened) — that is the likely home.

---

## 4. Requirement 3 — credit risk

### 4.1 The decomposition: what a corporate bond's risk actually is

A corporate bond's P&L decomposes, to first order, into:

```
dP  ≈  −DV01_rates × d(risk-free curve)      ← the rates leg   (§3 handles this)
       −CS01       × d(credit spread)        ← the credit leg  (NOT handled anywhere)
       + carry/roll + convexity + FX
       − LGD × (jump to default)             ← the discrete leg
```

- **DV01 (rates)** — sensitivity to the benchmark curve, hedgeable with futures/swaps.
- **CS01 / spread DV01** — sensitivity to a 1bp parallel widening of the issuer's credit
  spread over the benchmark. For an investment-grade bond, CS01 is close in magnitude to DV01
  (both are approximately spread-duration × price), so **hedging only the rates leg leaves
  roughly half the mark-to-market risk of the position uncovered, and all of the risk that
  actually kills credit desks.**
- **JTD (jump to default)** — the discrete loss on default, `notional × (1 − recovery)`. Not
  captured by any spread sensitivity, and the reason issuer concentration limits exist as a
  separate tier from spread limits (§5).

**The point the desk needs to internalise:** rates and credit are *different risk factors*
that happen to be transported by the same instrument. Hedging the rates leg with futures — the
thing the platform is currently building — leaves the **credit leg completely naked**. In a
2008 or a March-2020, the rates hedge would have made money while the unhedged credit leg lost
multiples of it.

### 4.2 What exists in Celnet today: essentially nothing, plus two useful fragments

Verified by code audit:

- **`celnet-credit` does not exist.** `docs/FI-CREDIT-ENGINE-DESIGN.md:3` states plainly:
  *"Status: Proposed design (2026-07-01). Design only — no credit code yet."* Its companion
  `docs/adr/ADR-0019-credit-pricing-leaf.md:5` is *"Status: Proposed"*. The design is coherent
  and specifies the right things — a hazard-rate `SurvivalCurve`, single-name CDS legs and
  `par_spread`, `bootstrap_survival_curve`, credit-risky bond pricing, `z_spread`, and `CR01`
  and `JTD` as new risk-cube dimensions. **None of it is built.**
- **`celnet-bond` has no spread input at all.** Its entire public surface is
  `Bond`, `accrued_interest`, `clean_price`, `dirty_price`, `price_from_curve`, `bond_risk`,
  `convexity`, `dv01`, `macaulay_duration`, `modified_duration`, `yield_to_maturity`
  (`crates/celnet-bond/src/lib.rs:53-66`), and its own module doc is explicit
  (`lib.rs:45-47`): *"Single (risk-free) discount curve. Credit spreads / Z-spread
  discounting live in the sibling `celnet-credit` leaf."* This is the crate every server
  pricing and risk path actually consumes. **The entire shipped bond-pricing path is
  credit-blind.**
- **Fragment 1 — orphaned spread analytics.** `crates/celnet-rates/src/bond.rs` contains real,
  tested implementations of `z_spread` (`:195`), `g_spread` (`:216`) and `asset_swap_spread`
  (`:229`). They have **no production caller anywhere** and operate on a different, simpler
  `CashBond` type than the settlement-aware `celnet_bond::Bond` the server prices. Genuine
  code, genuinely unwired.
- **Fragment 2 — hazard machinery, for the wrong risk.** `celnet-xva` has a real
  piecewise-constant-hazard `SurvivalCurve` with `survival`, `cumulative_hazard`,
  `marginal_default` (`crates/celnet-xva/src/survival.rs:1-113`), fully wired to server, WS,
  GUI and Excel. But it models **counterparty** default for CVA/DVA/FVA on FX netting sets —
  not **issuer** credit on a bond. Notably, `FI-CREDIT-ENGINE-DESIGN.md` proposes a *parallel*
  `SurvivalCurve` in a new crate and **does not mention the existing one at all**. That is a
  duplication the design should resolve before it is built.
- **Refdata cannot express an issuer taxonomy.** The one bond reference record, `GovBondSpec`
  (`crates/celnet-refdata/src/model.rs:64-104`), carries `issuer` as a bare `String` and
  hardcodes `sub_asset_type = "government"` (`model.rs:80`). There is **no sector, no rating,
  no seniority, no credit-curve identifier, and no corporate-bond record type at all** — the
  shipped universe is sovereign-only (US/UK/DE/FR/IT). **Issuer and sector concentration
  limits are therefore not expressible today, at all.**

### 4.3 CDS vs asset swap — the recommendation, argued

**First, correct the framing.** These are not two alternative credit hedges.

**An asset swap does not hedge credit risk.** In a standard **par–par asset swap** the investor
buys the bond at par (funding the difference between par and the market price inside the
package) and enters an interest-rate swap paying the bond's fixed coupons and receiving
floating plus the **asset-swap spread (ASW)**. The result is a synthetic floating-rate note.
What has been removed is the **interest-rate duration**. What remains, entirely and
undiminished, is the **issuer's credit risk**: if the issuer defaults, the bond stops paying,
but the swap does not terminate — the investor is left holding a mark-to-market swap position
against a defaulted asset, which in a falling-rate environment can be a *second* loss on top of
the first. (This "asset swap unwind" exposure is precisely why the par–par ASW's embedded
funding and the swap MTM are treated as distinct risks in the standard treatments — see
O'Kane & Sen, *Credit Spreads Explained*, Lehman Brothers Quantitative Credit Research, 2004,
and Choudhry, *The Credit Default Swap Basis*, 2006, both openly circulated.)

The **market-value asset swap** sizes the swap notional to the bond's market price rather than
par, removing the upfront/funding mismatch — but it changes nothing about the credit exposure.

**A CDS does hedge credit risk.** Buying protection transfers the default and spread-widening
exposure to the protection seller. It is the only instrument in this set that does.

So the correct statement of the desk's options is:

| Instrument | Removes rates risk? | Removes credit risk? | What it is really for |
| --- | --- | --- | --- |
| Government-bond **future** | **Yes**, bucket-wise | No | The cheap, liquid, exchange-cleared rates hedge. |
| **Asset swap** | **Yes** | **No** | Converting a fixed bond into a synthetic FRN and *isolating* the credit spread as carry. A funding/format decision, plus a quoting convention. |
| **Single-name CDS** | No | **Yes**, for that name | The precise issuer hedge — where it trades. |
| **CDS index** (CDX.NA.IG, iTraxx Europe Main) | No | **Yes, systematically** — leaves idiosyncratic residual | The practical portfolio credit hedge. |

**The recommendation, in priority order:**

1. **Build the rates leg first, tenor-bucketed** (§3). It is the largest single risk, the
   platform is already 70% of the way there, and the futures are the most liquid and
   operationally simplest instruments in the entire set.
2. **Measure CS01 before hedging anything credit.** An unmeasured risk cannot be limited,
   banded, or hedged. CS01 with an issuer/sector breakdown, plus JTD, is the deliverable — and
   it is independently valuable even if the desk never executes a credit hedge, because it
   feeds inventory limits, quote skewing, and internalisation, which are how a market maker
   actually manages credit inventory most of the time.
3. **When a credit hedge vehicle is needed, make it a CDS index, beta-weighted.** The
   liquidity argument is decisive and should be stated plainly: since the 2009 "Big Bang" and
   "Small Bang" conventions (standardised 100bp/500bp running coupons with upfront exchange,
   the ISDA Standard Model, quarterly IMM roll dates on the 20th of Mar/Jun/Sep/Dec, hardwired
   auction settlement, and Determinations Committees), **single-name CDS activity concentrated
   into a relatively small set of names while index and index-option volume grew**. A general
   corporate-bond inventory will contain many issuers with no dealable single-name CDS at all.
   An index hedge is executable, cleared, and continuously two-way. Its cost is **idiosyncratic
   residual** — an index hedge protects against systemic spread widening and does nothing about
   one name gapping. That residual must be measured and limited (§5, tier 4), not assumed away.
   Single-name CDS should be supported as a vehicle for the handful of largest, most liquid
   concentrated exposures, not as the default.
4. **Model the asset swap, but as a spread and a format — not a hedge.** The ASW spread is how
   the corporate market quotes credit, and `asset_swap_spread` already exists unwired at
   `crates/celnet-rates/src/bond.rs:229`. Wire it as a **quoting and calibration** input.
5. **Model the CDS–bond basis explicitly.** `basis = CDS spread − bond spread` (ASW or
   Z-spread). It is persistently non-zero and its drivers are well documented — the CDS
   cheapest-to-deliver option, counterparty risk, funding and balance-sheet costs, repo
   specialness, and the bond's distance from par. A platform that hedges bond credit risk with
   CDS and reports zero residual credit risk is lying; the basis is the residual and must
   appear as its own measured, limitable exposure.

### 4.4 What Celnet would need to model either

| Capability | Needed for | Status | Where it goes |
| --- | --- | --- | --- |
| **Issuer / credit-curve identity on refdata** — issuer id, sector, rating, seniority, and a credit-curve key | *Everything* credit, including concentration limits | **Absent** (`refdata/src/model.rs:64-104`) | `celnet-refdata` — a corporate-bond record type alongside `GovBondSpec` |
| **A spread-aware discounting seam** on the bond pricer | Z-spread, credit-risky PV, CS01 | **Absent by explicit design** (`celnet-bond/src/lib.rs:45-47`) | `celnet-bond` gains a spread input, OR `celnet-credit` wraps it |
| **Z-spread / ASW / I-spread** | Quoting, calibration, basis | **Built but orphaned + wrong bond type** (`celnet-rates/src/bond.rs:195/216/229`) | Re-home onto `celnet_bond::Bond` |
| **Survival / hazard curve** | CDS pricing, credit-risky bond, JTD | **Exists for counterparty XVA only** (`celnet-xva/src/survival.rs`) | Resolve the duplication with `FI-CREDIT-ENGINE-DESIGN.md`'s proposed parallel type |
| **Recovery assumption** | Upfront↔spread conversion, JTD | **Absent** | Configuration, per seniority. The market convention for the standard-model conversion is 40% senior unsecured / 20% subordinated for corporates; must be **data, never a constant** |
| **CDS instrument + par-spread/upfront** | Any CDS hedge | **Absent** (designed at `FI-CREDIT-ENGINE-DESIGN.md:69-85`) | `celnet-credit` |
| **CS01** | The credit limit tier | **Absent** (designed as `CR01`) | `celnet-credit` + a new `LimitMetric::Cs01` |
| **JTD per issuer** | Concentration/default tier | **Absent for bonds** — the FRTB `fx_default_risk_charge` (`celnet-risk-cube/src/frtb.rs:538-568`) is a correct, documented *zero* for deliverable FX and is not reusable | `celnet-credit` + `LimitMetric::IssuerJtd` |
| **Index composition + beta** | Index hedge sizing | **Absent** | `celnet-refdata` + `celnet-credit` |

**Naming note (guardrail 8):** every identifier proposed here is purpose-named —
`SpreadSensitivity`/`Cs01`, `SurvivalCurve`, `IssuerId`, `SeniorityClass`,
`CreditIndexDefinition`, `TenorBucketId`. No vendor, index-provider, or person names in API
identifiers; market conventions are cited in doc comments only.

---

## 5. Requirement 4 — a multi-tier, user-configurable risk model

> *"Risk should be multi-tiered so users can configure the different types."*

### 5.1 The tiers

Each tier is a distinct **risk factor** with its own measure, its own budget, its own bands,
and its own exit policy — because each is hedged with a different instrument, and because
netting across tiers is meaningless.

| # | Tier | Measure | Hedge vehicle | Status today |
| --- | --- | --- | --- | --- |
| **1** | **Outright / directional rate risk** | Net parallel DV01 (signed) | Any single future or swap | **Partial** — exists as the duration-blind proxy (§2) |
| **2** | **Curve / bucket risk** | Bucketed DV01 at 2/5/10/30 | Per-bucket future (ZT/ZF/ZN/ZB) | **Metric exists, unreachable** (§3) |
| **3** | **Credit spread risk** | CS01, by issuer and by sector | CDS index; single-name where liquid | **Absent** (§4) |
| **4** | **Issuer / sector concentration & default** | Gross exposure and JTD per issuer/sector; index-hedge idiosyncratic residual | Position limits; no hedge — this tier is *limited*, not hedged | **Absent, and not expressible** (refdata has no taxonomy) |
| **5** | **Liquidity / inventory age** | Gross notional (turnover brake), inventory age, days-to-liquidate | Not hedged — governs skew and clip size | **Partially exists**: `GrossNotional` threshold metric and `InventoryAgeSecs` as a `HedgeField` (`HEDGING-AND-RISK-EXIT.md` §5.0, §6.2) |

Tiers 1 and 2 are **the same factor at different resolutions** and must reconcile: the bucket
ladder must sum to the parallel DV01. Tiers 3 and 4 are **different factors** and must never be
netted against 1/2 — a credit-spread DV01 and a rates DV01 are both "per basis point" and are
both denominated in currency, which makes them dangerously easy to add together and completely
wrong to do so. This is exactly the mistake §2.3 Defect 1 already makes between bonds and
swaps, one level down; the tier model exists to make it structurally impossible to repeat.

### 5.2 Composition, not bolting-on

The critical design constraint is that a tier must **not** be a new parallel subsystem. The
existing machinery already generalises if the right thing is parameterised:

```
                     ┌───────────────────────────────────────────┐
   For each TIER  ──▶│ Measure    → a signed exposure in the      │
   in scope          │              tier's own units              │
                     │ Budget     → WarehouseThreshold{metric,cap}│  band.rs:58-79
                     │ Band       → utilisation → RAG → breached  │  (unchanged)
                     │ Limit      → LimitSpec{soft|hard}          │  limit.rs
                     │ Policy     → HedgeGraph → ExitAction       │  graph.rs
                     │ Vehicle    → HedgeVehicleRegistry row      │  vehicle.rs:374-457
                     │ Mode       → HedgeExitMode{Auto|Suggest}   │  mode.rs:27-34
                     └───────────────────────────────────────────┘
```

Every box in that column **already exists and is already keyed by a `LimitMetric`**. A tier is
therefore not a new concept — it is **a `LimitMetric` plus the exposure function that computes
it**. The requirement is:

- **R4.1** — `WarehouseThreshold`, the RAG band, the exit-policy graph, the vehicle registry
  and the exit mode MUST all be resolvable **per `(scope × metric)`**, not per scope alone.
  `WarehouseThreshold` already carries a `metric` field (`band.rs:59-61`) — the resolver
  (`resolve_hedge_threshold`, `rates_book.rs:2512-2527`) does not key on it. That is the single
  smallest change that unlocks the whole tier model.
- **R4.2** — a fill MUST be evaluated against **every** tier whose measure it moves, and the
  band/decision MUST be produced **per tier**. A book-level RAG becomes an aggregation
  (worst-tier), never a sum.
- **R4.3** — each tier's exit policy MUST be independently authorable, so a desk can express
  *"warehouse outright risk to 90%, but hedge any 30Y-bucket breach immediately, and escalate
  any credit-spread breach to a human"* — three different actions on three different tiers of
  the same book.
- **R4.4** — cross-tier netting MUST be structurally impossible. Exposures carry their metric;
  an aggregate is per-metric. (`LimitMetric::is_fixed_income()` at `limit.rs:105-110` is the
  existing precedent for this discipline and should be generalised to a full
  `metric.family()`.)

### 5.3 Configurability

The desk configures, per tier and per scope:

- the **cap**, amber/red bands, target fraction, min/max clip, ramp (all already on
  `WarehouseThreshold`, `band.rs:58-79`);
- **soft vs hard** enforcement (already `Enforcement`, `limit.rs:125-129`);
- the **exit policy graph** (already authored as data in the GUI);
- the **vehicle** and its bucket→instrument mapping (already `HedgeVehicleRegistry`);
- **auto vs suggest** (already `HedgeExitMode`, `mode.rs:27-34`).

Per the standing rule that every trader-facing configuration control ships with in-app help,
each tier needs its own help entry and the multi-tier configuration flow needs a tutorial —
this is a multi-step configuration and will not be discoverable otherwise.

### 5.4 Reconciling the three scope vocabularies

Per §1.1 there are currently three. The requirement is **one** vocabulary with a documented
precedence, adopted by all five resolvers:

```
Instrument  ▸  Book  ▸  Bucket (book-subtree root)  ▸  Desk  ▸  Entity  ▸  Firm
   most specific ───────────────────────────────────────────────► least
```

This is a strict superset of all three existing orders and preserves every current behaviour:
the threshold/panel/mode resolvers gain `Bucket`/`Entity`/`Firm` above their current
`Instrument ▸ Book ▸ Desk`; the policy resolver's `Book ▸ Bucket ▸ Firm` is already a
subsequence; the limit tree's cascade already evaluates every node on the path (which is a
different and complementary semantic — *all* limits on the path bind, whereas thresholds and
policies pick *one* most-specific). Both semantics must be kept and clearly named:
**"resolve one" (threshold, policy, vehicle, mode)** vs **"check all" (limits)**.

**Also fix `HEDGING-AND-RISK-EXIT.md` §10 boundary 3 while here**: the rates booking path
passes an empty desk id, so `Desk`-scoped bindings silently never bind. A unified vocabulary
that still doesn't populate `desk` is a unified vocabulary with a hole in it.

---

## 6. Gap table

Severity: **C**ritical (unsafe or meaningless without it) / **H**igh / **M**edium.

### 6.1 DV01 limits (tier 1)

| # | Requirement | Exists today | Gap | Concrete change |
| --- | --- | --- | --- | --- |
| G1 | **C** — Duration-correct bond exposure | `redemption × 1bp` (`rates_book.rs:2647-2653`) | Bond risk understated ~8× at 10y; **bond and swap not commensurable** | Replace the bond arm with analytic DV01 (`celnet_bond::dv01`, already used at `rates_book.rs:2420`); cache on the position |
| G2 | **C** — FI limit metrics reachable at booking | `exposure_of` returns `0.0` for all three FI metrics on a Greeks node (`check.rs:113`) | **Any `Dv01`/`RateTenorBucket` limit is silently inert on every fill** | `rates_pre_trade` (`rates_book.rs:2906`) must build a `RatesNodeAggregate` and route FI metrics to `exposure_of_rates` (`check.rs:143`) |
| G3 | **H** — Live book feeds the rates aggregate | Both engines are inline-only (`rates_risk/aggregate.rs:32-35`; `risk/combined_tail.rs:19-21`) | The correct engines never see the real book | New slice: `PositionStore` → `RatesRiskFact` → `RatesNodeAggregate` |
| G4 | **H** — Warehouse `Dv01` metric means DV01 | `WarehouseThreshold.metric` exists (`band.rs:59-61`) but resolves against the proxy | A "DV01 cap" caps the proxy | Point band classification at the G1/G3 aggregate |
| G5 | **M** — Measurement carries `Dv01Basis` | Only the *hedge sizing* path carries it (`vehicle.rs:135-169`) | A proxy-based *utilisation* can be presented as exact | Extend `Dv01Basis` to the exposure measure and onto the wire/GUI |

### 6.2 Tenor buckets / curve risk (tier 2)

| # | Requirement | Exists today | Gap | Concrete change |
| --- | --- | --- | --- | --- |
| G6 | **C** — Book risk has a tenor axis | Single scalar (`book_net_dv01`, `rates_book.rs:2104`) | **Curve risk is invisible; a flat-DV01 steepener reads Green** | New `RatesExposure{parallel, ladder}`; keep `parallel()` for the ~162 existing call sites |
| G7 | **H** — Key-rate ladder computed for booked positions | `key_rate_ladder` exists and is correct (`celnet-rates-risk/src/ladder.rs:115-132`) but is never called on booked positions | The engine is right and unused | Compute per position at book time (`O(P)`), cache, net additively on roll-up (`O(depth)`) — **not** per fill over the whole book |
| G8 | **H** — Bucket set is configuration keyed identically to the vehicle registry | Limit key is `tenor_years: u32` (`limit.rs:85-89`); registry key is `[min,max)` f64 (`vehicle.rs:191-197`) | **Two independent bucket vocabularies that will drift** | One shared `TenorBucketId{min,max}` value type used by both |
| G9 | **H** — Band + policy evaluate per bucket | One band, one decision per fill | A 30Y breach inside a flat book never fires | Per-bucket RAG; book band = worst-bucket ∨ parallel band; bucket-qualified `HedgeField` conditions |
| G10 | **H** — Offsetting leg is bucket-truthful | Offsets in the fill's own security (`rates_book.rs:2558`); no futures arm in the proto oneof (`celnet.proto:7955-7967`) | Net parallel risk right, **bucket ladder wrong** under a vehicle hedge | Add a futures arm to `RatesInstrument` (**162 match sites**, mechanical) + a futures DV01 |
| G11 | **M** — `dv01_per_unit` derived, not typed in | Admin-entered (`vehicle.rs:209`); derivation exists (`refdata/src/futures.rs:441-472`) | Static DV01 vs a CTD/curve that moves | Derived-with-override + provenance. **Honest caveat:** the derivation is a 6%-notional approximation understating live DV01 by ~10–20% at sub-6% yields (`futures.rs:74-83`) — CTD modelling is a further step |
| G12 | **M** — Curve-unavailable fallback | Measure is deliberately curve-free today (`rates_book.rs:2529-2539`) | New dependency on live market data in the booking gate | Fall back to the conservative proxy, stamp `ExposureProxy`, never block silently |
| G13 | **M** — Ladder↔parallel↔analytic reconciliation gate | None | The two systems will drift undetected | CI gate: `Σ buckets ≈ parallel ≈ celnet_bond::dv01` within tolerance |

### 6.3 Credit (tiers 3 & 4)

| # | Requirement | Exists today | Gap | Concrete change |
| --- | --- | --- | --- | --- |
| G14 | **C** — Issuer taxonomy on refdata | `issuer: String`, `sub_asset_type` hardcoded `"government"` (`refdata/src/model.rs:64-104`, `:80`) | **No corporate-bond record type exists; issuer/sector limits are not expressible at all** | Corporate-bond record: issuer id, sector, rating, seniority, credit-curve key |
| G15 | **C** — Spread-aware bond discounting | None; explicitly deferred (`celnet-bond/src/lib.rs:45-47`) | **The whole shipped bond path is credit-blind** | Spread input on `celnet-bond`, or a `celnet-credit` wrapper |
| G16 | **H** — CS01 measured | Absent (designed as `CR01`, `FI-CREDIT-ENGINE-DESIGN.md:106-116`) | The credit leg of every bond is unmeasured | `celnet-credit` + `LimitMetric::Cs01` |
| G17 | **H** — Z-spread / ASW wired | Real but orphaned, on the wrong bond type (`celnet-rates/src/bond.rs:195/216/229`) | Quoting and calibration inputs unavailable | Re-home onto `celnet_bond::Bond`; wire to quoting + curve calibration |
| G18 | **H** — Survival curve for issuers | Exists for **counterparty** XVA only (`celnet-xva/src/survival.rs:1-113`); design proposes a *parallel* type without referencing it | Duplication designed in before a line is written | Resolve in ADR-0019 before building: extract/share or justify the split |
| G19 | **H** — JTD per issuer | FRTB DRC is a documented, correct **zero** for FX (`risk-cube/src/frtb.rs:538-568`) and not reusable | Default risk unmeasured | `celnet-credit` + `LimitMetric::IssuerJtd` |
| G20 | **M** — CDS / index instrument + hedge vehicle | Absent | No credit hedge can be executed or sized | `celnet-credit` CDS; index definition + beta in refdata; new `HedgeVehicle` arm |
| G21 | **M** — CDS–bond basis + idiosyncratic residual measured | Absent | An index-hedged book would report zero residual credit risk — **untrue** | Basis as a first-class measured exposure with its own limit |
| G22 | **M** — Recovery assumption as data | Absent | Cannot convert upfront↔spread or compute JTD | Configuration per seniority; **never a compiled constant** |

### 6.4 Multi-tier framework (tier framework + tier 5)

| # | Requirement | Exists today | Gap | Concrete change |
| --- | --- | --- | --- | --- |
| G23 | **H** — Threshold/policy/vehicle/mode resolve per `(scope × metric)` | Resolve per scope only (`rates_book.rs:2512-2527`) despite `WarehouseThreshold` carrying a `metric` | Only one tier can be configured per scope | Key every resolver on `(scope, metric)` — **the smallest change that unlocks the tier model** |
| G24 | **H** — One scope vocabulary | Three (§1.1) | "Desk-level limit" means three different things | Adopt `Instrument ▸ Book ▸ Bucket ▸ Desk ▸ Entity ▸ Firm`; keep "resolve-one" vs "check-all" distinct and named |
| G25 | **M** — `Desk` scope actually binds on rates | Empty desk id passed (`HEDGING-AND-RISK-EXIT.md` §10 boundary 3) | Desk bindings silently no-op | Populate desk id at the rates call site |
| G26 | **M** — Cross-tier netting structurally impossible | `is_fixed_income()` (`limit.rs:105-110`) is the only guard | A CS01 could be added to a DV01 | Generalise to `metric.family()`; aggregates are per-metric |
| G27 | **M** — Per-tier in-app help + tutorial | Hedging help exists | Multi-tier config is not discoverable | Help entry per tier + a configuration tutorial |

---

## 7. Sequencing

### 7.1 The ordering law — state it before anything else

> **Nothing that consumes a DV01 ratio may be broadly enabled on mixed bond/swap books while
> the warehouse band still classifies off the duration-1 proxy.**

The reasoning is precise. The uncommitted vehicle work made the hedge **size** correct
(`genuine_position_dv01` → `Dv01Basis::Analytic`, `rates_book.rs:2390-2427`) while leaving the
hedge **trigger** on the proxy. Those combine badly:

- On a **bond-only** book, the proxy understates risk uniformly, so the system under-triggers.
  Conservative in the wrong direction, but not incoherent.
- On a **mixed bond + swap** book, the proxy nets a duration-1 bond against a duration-correct
  swap. The net is not a risk number in any unit. It can be near zero on a book carrying large
  real risk, or large on a book that is genuinely flat. The band, the overflow and therefore
  the *size* passed into the now-correct ratio are all derived from that number. **A correct
  ratio applied to a meaningless target produces a precisely-sized wrong trade.**

Therefore: **G1 (duration-correct exposure) gates everything.** It is also the single smallest
change in the entire programme — the analytic call is already being made twenty lines away.

### 7.2 The waves

**Wave 0 — Make the number true. (Small. Unblocks everything.)**
`G1` + `G12` + `G5`. Replace the bond arm of `rates_linear_exposure` with the analytic DV01
already computed at `rates_book.rs:2420`; fall back to the proxy with an honest
`Dv01Basis::ExposureProxy` stamp when no curve or contract resolves; carry the basis onto the
wire. *Unblocks:* all DV01 limits, all vehicle hedging on mixed books, every downstream wave.
*Risk:* every existing threshold cap on a bond book is now measured on a number ~8× larger —
**caps must be re-based as part of this change or every bond book breaches on day one.** This
is a migration, not a drop-in, and must ship with the re-basing.

**Wave 1 — Make the limits reachable. (Medium.)**
`G2` + `G3` + `G4` + `G23`. Build a `RatesNodeAggregate` on the booking path from the live
`PositionStore`; route FI metrics through `exposure_of_rates`; key the threshold resolver on
`(scope, metric)`. *Unblocks:* real DV01 limits (tier 1), and the tier framework itself —
after this, adding a tier is adding a metric and its exposure function.

#### 7.2.1 Waves 0 and 1 — AS BUILT (2026-08-11), and the cap re-basing

Waves 0 and 1 are **implemented**. What actually landed, and what an operator must do:

**Wave 0 — the bond arm is re-based.** `rates_linear_exposure`'s bond arm
(`crates/celnet-server/src/services/rates_book.rs`) is now
`celnet_bond::dv01(contract, coupon_yield) × face`, memoized per `(security, settlement
date)` as a DV01 **per unit face** (DV01 is exactly linear in `redemption`, so one cached
number is exact for any size). The retired proxy survives as
`rates_linear_exposure_proxy`, used only as the migration's reference measure and as the
honest fallback when no contract can be constructed (missing/unparseable maturity,
unmapped frequency or day-count). Bond and swap exposure are now commensurable in one
book net.

> **The curve question, answered.** §3.5 warns that making the measure curve-dependent
> couples the booking gate to market-data availability. **Wave 0 does not do that.**
> `celnet_bond::dv01` is the closed-form derivative `−∂P/∂y · 1bp` of the bond's *own*
> cashflow schedule (`crates/celnet-bond/src/risk.rs`) — no discount curve, no bootstrap,
> no live market data. The gate stays deterministic and market-data-free, exactly as
> before. The genuine curve dependency belongs to the Wave-2 **key-rate ladder** (G7/G12)
> and has deliberately **not** been introduced. The one honest approximation is the yield
> the derivative is taken at: a stored position has no dealt price, so the par (coupon)
> assumption is used; the sizing path (`genuine_position_dv01`) still prefers the dealt
> yield when the booking path supplies one, so sizing is never less precise than
> measurement.

**Cap re-basing — mandatory, and what each cap becomes.** Every cap that gates on
`rates_linear_exposure` changed units for bonds and **only** for bonds:

```
new_cap  =  old_cap  ×  (portfolio-weighted average modified duration of the BONDS in that scope)
```

Equivalently, `new_cap = old_cap × (Σ|analytic DV01| / Σ|redemption × 1bp|)` over the
scope's bond inventory. Indicative multipliers for a par bond: **≈1.9× at 2y, ≈4.5× at
5y, ≈7.7× at 10y, ≈16× at 30y**. A scope holding **no bonds** needs **no change** —
swap/FRA exposure was never duration-blind.

The affected caps are:

| Cap | Where | Re-base? |
| --- | --- | --- |
| `LimitSpec{metric: Dv01 \| Pvbp}` at `Book`/`Entity`/`Firm` | `RatesPositionStore::set_limit` | **Yes**, by the scope's bond duration |
| `LimitSpec{metric: Delta}` on the **rates** limit tree | same | **Yes** — the rates sink charges the linear IR proxy against `Delta`, so it changed units too |
| `HedgeThresholdDef{metric: Dv01}` (warehouse cap, amber/red, `min_clip`/`max_clip`) | hedge policy, `Book`-scoped | **Yes** — and note `min_clip`/`max_clip` are in the same units |
| `HedgeThresholdDef{metric: NetNotional \| GrossNotional \| NetDelta \| NetVega}` | hedge policy | **No** — these read `rates_signed_notional`, untouched |
| `RiskLimits{max_net_notional, max_gross_notional}` per risk book | risk-book tree | **No** — notional, untouched |
| Any cap on a swap/FRA-only scope | anywhere | **No** |

**The migration announces itself.** Three mechanisms, so no operator discovers this as an
unexplained breach:

1. **Boot audit** — `RatesPositionStore::audit_proxy_era_caps()` runs at server start
   (`celnet-server/src/lib.rs`, after the limit/policy prime) and measures every affected
   cap against the *actual* loaded inventory under both measures. Each cap whose meaning
   changed is rung at `WARN` (`class=risk`) with `legacy_exposure`, `rebased_exposure`,
   `multiplier`, `suggested_cap` and `breaching_now`. It returns the findings so an admin
   surface can render them.
2. **Rejection-time advice** — when a booking is rejected by a cap the old measure fitted
   inside and the new one does not, the `failed_precondition` message carries a
   `MIGRATION NOTE … PROXY-ERA` explanation with the observed multiplier and the re-based
   cap. A cap blown under *both* measures is a real breach and gets no such note, so the
   guard never cries wolf.
3. **Tenor-bucket honesty** — a configured `RateTenorBucket` limit still cannot bind at
   booking (no ladder, Wave 2). Rather than let it read a silent zero, the gate warns once
   per process that curve risk is unprotected there.

**Wave 1 — the FI limits are reachable.** `rates_pre_trade` now runs
`celnet_limits::pre_trade_check_mixed`, which routes each limit to the node its own family
is measured on: FX-Greeks metrics against the projected `NodeAggregate`, FI metrics
(`is_fixed_income()`) against a real `RatesNodeAggregate` built from the projected position
set. Each limit is charged **exactly once**, so the coarse `Delta` proxy and a `Dv01` cap
cannot double-charge. Two honest limitations of the booking-gate aggregate, both
documented at `rates_risk_fact_of`:

- it is keyed on ISO 4217 **`XXX`** ("no currency involved"), because a `RatesPosition`
  carries no settlement currency — the gate nets one currency-agnostic set, exactly as
  every existing rates roll-up already does. G3 replaces this with a per-currency
  aggregate;
- `pv` is **NaN** (there is no curve here, so there is no present value, and a `0.0` would
  read as "flat"), and `key_rate_ladder` is **empty**. No limit metric reads either.

**Still open from these waves:** G3 (feeding the aggregate from the live `PositionStore`
per currency, with a real PV), G4 (`WarehouseThreshold`'s `Dv01` still resolves against the
scalar, though that scalar is now duration-correct), G5 (`Dv01Basis` on the *measurement*
wire, not just the sizing path), G23 (`(scope × metric)` threshold resolution), G25 (the
empty desk id).

**Wave 2 — Add the tenor axis. (Large. This is the real work.)**
`G6` + `G7` + `G8` + `G9` + `G13`. `RatesExposure{parallel, ladder}`; per-position ladder
computed once at book time and cached; one shared `TenorBucketId` for limits and the vehicle
registry; per-bucket bands and bucket-qualified policy conditions; the reconciliation gate.
*Unblocks:* tier 2 and the desk's actual ask. *Hard parts:* the caching architecture (§3.5),
the new curve dependency in a previously curve-free gate, and per-bucket band aggregation
changing `stamp_internalise`'s single-band assumption.

**Wave 3 — Make the hedge instrument-truthful. (Medium-large, mostly mechanical.)**
`G10` + `G11`. Futures arm on `RatesInstrument` (162 match sites) so a bucket hedge books as a
real futures position; derived-with-override `dv01_per_unit`. **Can run in parallel with Wave
2** — they touch different files — but must land after it, since the bucket ladder is what
makes instrument-truthfulness matter.

**Wave 4 — Measure credit. (Large. Independent of Waves 2–3.)**
`G14` + `G15` + `G17` + `G18` + `G16`. Corporate-bond refdata with issuer taxonomy; a
spread-aware discounting seam; re-home the orphaned Z-spread/ASW analytics; **resolve the
`SurvivalCurve` duplication in ADR-0019 before writing code**; then CS01. *Unblocks:* tier 3
measurement and tier 4 concentration limits. Note `G14` alone unblocks issuer/sector
concentration limits **without any credit pricing at all** — a cheap, high-value early slice.

**Wave 5 — Hedge credit. (Large.)**
`G19` + `G20` + `G21` + `G22`. CDS/index instruments, index composition and beta, JTD, the
CDS–bond basis and idiosyncratic residual as measured exposures, recovery as configuration.
**Must not precede Wave 4** — hedging an unmeasured risk is worse than not hedging it, because
it produces a confident number where there was previously an honest blank.

**Continuous.** `G24`/`G25` (scope reconciliation) and `G26` (family separation) are small and
should ride with Wave 1. `G27` (help) rides with each user-facing wave.

### 7.3 What is genuinely hard, stated plainly

| Item | Why it is hard |
| --- | --- |
| **Per-position key-rate ladder caching** (Wave 2) | Real architecture work, not a match arm. Naive per-fill computation is `N × P` repricings and will not meet the booking-tier budget. Requires an incremental additive roll-up with curve-refresh invalidation — the pattern `RISK-HIERARCHY.md` §3.3 specifies for FX and which has never been applied to rates. |
| **Introducing a curve dependency into the booking gate** (Wave 2) | The current measure is deliberately curve-free and fail-safe. Every failure mode of "no curve available" must be answered before it ships, or booking becomes coupled to market-data availability. |
| **Re-basing existing caps** (Wave 0) | Not technically hard, operationally sharp. Every configured bond-book cap silently changes meaning by ~duration×. Ship the migration with the change. |
| **The futures arm** (Wave 3) | 162 match sites is large but mechanical. The genuinely non-mechanical part is that a futures position needs its own pricing and DV01, and the available derivation is a 6%-notional approximation, not a CTD model. |
| **Credit, end to end** (Waves 4–5) | The largest single body of new work in this document. A credit curve, a recovery convention, a survival model, a CDS instrument, index composition and beta, and the basis — none of which exist. `FI-CREDIT-ENGINE-DESIGN.md` is a sound starting point but needs the amendments in §4.4 and the `SurvivalCurve` duplication resolved first. |
| **Not hedging what you cannot measure** | The hardest thing here is not code. It is the discipline to build Wave 4 before Wave 5, and to publish the idiosyncratic and basis residuals of an index hedge rather than reporting a hedged book as flat. |

---

## 8. Sources

Market-convention material is cited to open sources only (guardrail 7):

- **Asset swaps, Z-spread, I-spread, and the CDS–bond basis** — D. O'Kane & S. Sen,
  *Credit Spreads Explained*, Lehman Brothers Quantitative Credit Research (2004);
  M. Choudhry, *The Credit Default Swap Basis* (2006).
- **CDS valuation and the hazard-rate/survival framework** — D. O'Kane & S. Turnbull,
  *Valuation of Credit Default Swaps*, Lehman Brothers Quantitative Credit Research (2003);
  D. Lando (1998); Brigo & Mercurio (2006) — the latter two already cited in
  `crates/celnet-xva/src/survival.rs`.
- **Standardised CDS conventions** (fixed 100/500bp coupons with upfront exchange, quarterly
  IMM roll dates, hardwired auction settlement, Determinations Committees) — the ISDA 2009
  "Big Bang" and "Small Bang" protocols and the openly-published **ISDA CDS Standard Model**,
  which is released as open-source reference code.
- **Treasury futures contract specifications** — CBOT rulebook chapters, cited per-contract
  inline at `crates/celnet-refdata/src/futures.rs:27-34` and `:232-316`; CME Group
  *Understanding Treasury Futures* for the listed-months cycle.
- **Key-rate duration** — the standard partial-DV01 construction; Celnet's implementation and
  its GIRR vertex set are at `crates/celnet-rates-risk/src/{ladder,girr,curve_shock}.rs`, with
  the MAR21 GIRR vertices at `girr.rs:61`.
- **Warehouse bands, overflow sizing and ramped shedding** — Whalley–Wilmott (1997),
  Zakamouline (2006), Almgren–Chriss, Barzykin et al. (2021), as already cited in
  `crates/celnet-hedge-routing/src/band.rs` and
  [`AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md`](AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md).

**Unverified / stated as such:**

- I did not personally open `crates/celnet-rates-risk/*`, `crates/celnet-rates/src/bond.rs`,
  `crates/celnet-bond/src/lib.rs`, `crates/celnet-refdata/src/{model,futures}.rs`,
  `crates/celnet-xva/src/survival.rs`, `crates/celnet-risk-cube/src/frtb.rs`,
  `crates/celnet-server/src/services/rates_risk/aggregate.rs`,
  `crates/celnet-server/src/services/risk/combined_tail.rs`, or
  `docs/FI-CREDIT-ENGINE-DESIGN.md`. Those citations come from a read-only code audit with
  file+line references, and should be spot-checked before implementation planning.
- The claim that `crates/celnet-rates/src/futures.rs` is the natural home for a CTD model is
  **inference** from its importing `celnet_bond::{CashBond, price_at_yield}`, not a design
  I verified.
- Whether any GUI surface consumes `CombinedTailRisk` was reported as "no hits found"; treat
  the negative as an argument-from-absence.
