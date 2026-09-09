# Celnet — The Trading Universe at Scale (Breadth × Depth × the Vol Cube)

> **What this document is.** A SOTA (May-2026) design for scaling Celnet's *trading universe* —
> the set of pairs, the continuous expiry/date axis, the resulting per-pair surfaces and the
> firm-wide vol **cube** — to a global investment bank's FX-options business, and for the trader
> experience that must make that universe navigable. It formalises the **scale model**, sizes it,
> maps it onto the existing crates, and critiques the current `gui/` against the SOTA and the
> incumbents.
>
> **Relationship to other docs.** Risk *roll-up* across trader → book → desk → legal-entity is
> owned by `docs/RISK-HIERARCHY.md`; this document deliberately does **not** duplicate it — it
> defines the universe (breadth/depth) and the per-pair/cube *pricing & streaming* substrate that
> risk aggregation consumes, and threads a `book_id`/owner identity for that effort to roll up
> (§3, §4). The distributed topology lives in `docs/SCALE-OUT.md`; this doc *sizes* the cube
> against those budgets rather than re-specifying the fleet layer. Analytics breadth is in
> `docs/ANALYTICS-SPEC.md`.
>
> **Provenance discipline.** Every market-structure or competitor claim carries a
> `[source / confidence]` tag. **high** = directly on a primary/vendor page; **medium** = strongly
> implied or secondary; **low / inferred** = reasoned argument or arithmetic over sourced axes;
> **unverified** = could not be confirmed in a primary source (stated, not hidden). The document
> separates **verified facts (cited)** from **Celnet design proposal**. Negative claims about
> incumbents are flagged as arguments-from-absence (absence of public evidence ≠ proof of absence).
> Proposed Celnet identifiers are vendor-neutral and purpose-named per CLAUDE.md rule 8; vendor
> names appear only in research prose, never as shipped API identifiers.

---

## 1. Purpose & scope

Celnet today prices and risks at the level of **one instrument, on one pair, against one marked
surface** (`celnet-vanilla` + `celnet-surface` + the `celnet-engine` hot core). The `gui/` exposes
this through a **single global active pair** (`AppContext.pairCtx`), a **5-pair seeded watchlist**
(`seed.ts` `PAIRS`), a per-pair surface view, single-instrument risk and a streaming blotter. That
is a correct, honest *single-pair* product. It is **not yet a universe-scale product.**

A global-IB FX-options business does not trade one pair — it runs a continuously-quoted universe of
*hundreds* of pairs across G10, crosses, EM (deliverable and non-deliverable), and metals, each
with a full surface over a **continuous expiry line** (not a fixed 8-tenor ladder), owned by a
follow-the-sun desk structure. The unit of scale is therefore not "a surface" but a **vol cube**:
`pair × tenor × delta`, live, versioned, and streamed to many counterparties at the
`docs/ARCHITECTURE.md` §1.2 latency budgets.

This document specifies that universe along three formal dimensions (§2: **breadth**, **depth**,
**the cube**), the desk/ownership model the experience must reflect (§3), the architecture that
prices/risks/streams it in real time (§4), the SOTA experience at scale (§5), an honest critique
(§6), citations (§7) and a proposed backlog (§8).

---

## 2. The scale model — three dimensions, formalised

The trading universe scales along three orthogonal axes. Sizing them up-front turns "scale" from a
slogan into design parameters.

### 2.1 BREADTH — the pair universe

The universe is **not** uniform; it is Zipfian. USD is on one side of **~88% of all FX turnover**
(BIS Triennial 2022) [BIS-22 / high], rising to **89.2%** in the 2025 Triennial [BIS-25 / high],
with EUR ~29%, JPY ~17%, GBP ~13% (2022) and a long, cold tail of crosses and EM. Provisioning must
be **volume-weighted, not pair-count-uniform**: a handful of pairs carry most of the flow, but the
*count* of pairs that must be *priceable* (even if rarely traded) is large.

**Liquidity tiers (proposed Celnet model).** The universe registry should carry a per-pair
`liquidity_tier` that drives quoting cadence and the price source, not a flat list:

| Tier | Character | Quote source | Cadence | Illustrative count |
|---|---|---|---|---|
| **Streamed** | continuous two-way, internalised | live engine + own surface | sub-second RFS | tens (G10 + top crosses) |
| **RFQ** | quote-on-demand, warehoused selectively | engine + broker marks | on request | low hundreds |
| **Reference/proxy** | priceable but rarely traded; model-extrapolated surface | proxy/correlation surface | snapshot | the long tail |

The tier counts above are **illustrative design parameters, not market facts** [inferred / low] —
the real number of *options-streamed* pairs per bank is not publicly published, and Celnet must
treat the hot-core size as a **per-deployment configuration**, not a hard-coded number.

**Vendor coverage envelopes (sized, cited).** What banks *consume* gives an order of magnitude:
- Tradition "Enhanced FX Options": **140+ currency pairs**, ATM straddles + 5/10/15/25/35Δ RR/BF,
  surfaces **1W→30Y**, 3 snaps/day (London/NY/Tokyo) [Tradition / high]. **Caveat:** this is a
  **vendor-analytics-processed** surface count (a Tradition × Numerix joint product), *not* raw
  two-way broker liquidity — the long tail is model-extrapolated/proxy, which *strengthens* the
  cold-tail-dominates thesis [high].
- Fenics **FXO 2.0**: ML surfaces over **300+ pairs / 27 metals**, "more wing data points and
  long-dated tenors" [Fenics / high]; Fenics Direct tradeable feeds **100+ pairs** incl. G10,
  crosses, **NDF**, XAU/XAG, **20+ price-making banks**, FIX 4.4 [Fenics / high].
- Digital Vega Medusa: **60+ pairs**, 21 banks → 150+ clients [DigitalVega / high].
- 360T: **200+ liquidity providers** [360T / high].
- TraditionData real-time: ATM/RR/BF in real time [Tradition / high]; an exact *57-pair* count is
  **not** confirmable from the public page [unverified].

**Deliverable vs non-deliverable, and cleared vs bilateral** are *first-class* breadth axes, not
afterthoughts — they change settlement mechanics, fixing reference data and legal definitions:

- **CLS** settles ~18 currencies — a clean, citable boundary for the *deliverable/settleable* leg
  of the universe [CLS / medium].
- **LCH ForexClear** clears exactly **25 NDF, 9 NDO, 8 deliverable** pairs [LSEG / high]. The NDF
  set splits **15 EM + 10 G10**; the NDO set is **5 G10** (AUD/EUR/GBP/CHF/JPY vs USD) **+ 4 EM**
  (BRL/KRW/INR/TWD vs USD), max tenor "2 years + 2 open business days" [LSEG / high].
- **Correction to a common framing:** NDO is **not** a subset of the non-deliverable-*currency*
  universe — **5 of the 9** NDO names are fully deliverable G10, cleared as *cash-settled* for
  clearing/capital efficiency. The honest takeaway is sharper: **liquid CCP-cleared
  non-deliverable-EM optionality exists in only ~4 names** (BRL/KRW/INR/TWD) [LSEG / high]. The
  larger bilateral/uncleared ND universe (per EMTA: CLP/COP/PEN/CNY/PHP/IDR/…) trades **off**
  clearing. **`clearing_eligibility` is therefore its own attribute, distinct from `settlement`
  and `liquidity_tier`** — what LCH *clears* ≠ what *trades*.
- **CNH ≠ CNY** as distinct underlyings (offshore vs onshore RMB) [BIS-25 / high]; USDCNY grew +59%
  to an 8.1% *pair* share (CNY *currency* share 8.5% — both correct, not a discrepancy) [BIS-25].
- **EMTA** publishes **per-currency** Template Terms, Settlement Rate Options and Disruption
  Fallbacks [EMTA / high] — the fixing source and disruption fallback differ *per currency*, so the
  universe registry's `Settlement::NonDeliverable` must carry `{ fixing_source, disruption_fallback }`.
- **Definition vintage (HIGH PRIORITY, forward-looking):** ISDA + EMTA published the **2026 FX
  Definitions** (3 Mar 2026), **effective 22 November 2027**, replacing the 1998 FX & Currency
  Option Definitions and folding Valuation Postponement, Price Source Disruption and Unscheduled
  Holiday into a common Main Book [ISDA-2026 / high]. Celnet's ND/fixing/disruption logic should
  build to the **2026 Definitions** and record which vintage it targets plus the Nov-2027 cutover.

**Metals on the FX desk.** XAU/XAG (and minor XPT/XPD) options sit on the FX-options desk via the
LBMA OTC wholesale market; XAU/XAG materiality is high, XPT/XPD low [LBMA / medium]. They are a
breadth axis with their own conventions (troy-ounce notionals, metal-vs-USD quoting).

### 2.2 DEPTH — the expiry/date axis as a continuous line over discrete pillars

The second axis is **time to expiry**, and it is **not** a fixed ladder. A real book's expiries are
a *continuous line* populated mostly by **broken (odd) dates**:

- On a major venue, **51% of FX *forward* trades are broken-dated, ~20% of volume sits on turns**,
  turn impact 1–15 pips (year-end largest), across a venue doing **>$460bn/day** [LSEG-turns /
  high]. **Scope label:** that $460bn is **Spot + Forwards + NDFs + FX Options combined**, on the
  LSEG venue — *not* forwards-only and *not* market-wide (BIS total FX ≈ $7.5tn/day 2022; $9.6tn/day
  2025) [BIS / high]. The 51%/20% are venue-specific. The lesson holds regardless: **the broken-date
  population dominates the round-tenor population.**
- **Pre-spot tenors** (ON/TN/SN) and **spot = T+2** (T+1 for USDCAD, USDTRY, USDRUB, USDPHP — a
  *subset*; offshore USDKZT/USDPKR also T+1; USDCAD moved to T+1 only in **May 2020** — conventions
  are *dated*, not eternal) [date-conventions / high]. Settlement adjusts modified-following with an
  end-of-month rule, on the **joint + USD** calendar.
- **IMM dates** = 3rd Wednesday of Mar/Jun/Sep/Dec [CME / high]; CME FX options aligned their expiry
  to **10:00 NY** on **2019-06-09** to match OTC [CME / high].
- **Cuts as fixings, not just times:** the **NY 10:00** and **Tokyo 15:00** cuts define *which fixing*
  settles the option; NDO are cash-settled 1–2 business days post-expiry on the expiry-date fixing
  [CME / high]. A broken/event date needs a **fixing registry keyed by (pair, cut)** — the cut is a
  methodology choice (e.g. WM/Refinitiv 4pm London vs ECB vs a CME 60s VWAP), not merely a clock time.
- **Time interpolation is in total variance, not calendar time:** linear total variance ≡ flat
  forward vol between pillars, with **weekend/holiday day-weighting** (the "sawtooth"); using
  calendar time alone is wrong [Clark / rateslib / high]. Healy, *Counterexamples for FX Options
  Interpolations* (Part I arXiv:2512.19621; Part II arXiv:2512.19625, Dec 2025) documents the
  interpolation pathologies, incl. broker-quote divergence at 10Δ/25Δ — directly relevant to the
  surface RR axis and the trend-series choice [Healy / medium].
- **Event/jump dates** (central-bank meetings, NFP, CPI, elections, referenda) create **term-structure
  kinks**: the variance contributed by an event day exceeds a normal day, producing a non-smooth ATM
  term structure. Note **turn liquidity ≠ turn vol**: the LSEG turn material is a forward-points
  funding phenomenon; an event/turn *vol-clock kink* is a distinct, weaker, **inferred** claim [low].

**The scheduling convention divergence (real, to be made an explicit policy flag).** The market-
canonical chain is **horizon → spot → add-tenor-to-spot → adjust *delivery* → back-derive expiry**
by subtracting the spot lag. Celnet's `celnet-calendar` `expiry_for_tenor` currently adjusts the
**expiry** directly and derives delivery from it (`delivery = expiry + lag`) — **expiry-led, not
delivery-led**. The two can differ by a day at month-ends/holidays. Some banks *do* run expiry-led
scheduling, so this is a **convention divergence requiring an explicit policy flag**, not an
outright bug [verified in `crates/celnet-calendar/src/fx.rs` / high]. A concrete failing case (a
1M trade on a month-end where delivery-led EOM and expiry-led EOM land on different days) should be
captured as a golden test.

### 2.3 The per-pair SURFACE and the firm-wide vol CUBE — order-of-magnitude sizing

**One per-pair surface (quoted inputs).** The market convention is **5 benchmark instruments per
tenor** — ATM (DNS or fwd) + **25Δ & 10Δ Risk-Reversal & Butterfly** [vol-surface refs / high].
Over ~10–20 quoted tenors that is **~50–100 quote inputs per pair** [arithmetic / verified-arith].
Materialised to a dense `(tenor × delta)` grid for pricing/scenario, a pair is **~135–375 nodes**
[inferred / low] — a sampling choice, not a sourced figure.

**The firm-wide cube.** `pairs × tenors × deltas`:
- Conservative streamed core: ~30 pairs × ~12 tenors × ~7 delta pillars ≈ **~2,500 live nodes**.
- Full IB coverage incl. RFQ + reference tail: **several-hundred pairs** × continuous tenors × delta
  pillars ⇒ **O(10⁴–10⁵) materialised surface nodes** firm-wide [inferred / low].
- The **risk-evaluation** surface is larger still: `nodes × ~14 Greeks × scenario shocks` pushes the
  per-revaluation work to **O(10⁵)→O(10⁹–10¹¹)** floating-point evaluations across a stress run
  [inferred / low — arithmetic over sourced axes]. This is why **recompute-cheaply ≫ store-everything**
  (§4): you cache the *parametric* surface (a handful of calibrated parameters per smile), not the
  dense grid.

**Intraday change rate is the real driver.** The cube's *static* size matters less than how often
quoted surfaces *change* — the per-pair quote-update rate is what gives incremental recalibration
its value (§4.1). The real rate is deployment-dependent and **not publicly published**; treat it as
an illustrative, measured parameter (a Celnet bench), not a market fact [gap, §7].

---

## 3. Who is trading — desk ownership, routing & follow-the-sun

The experience must reflect *who owns* each slice of the universe. (Risk roll-up across this
hierarchy is `docs/RISK-HIERARCHY.md`; here we cover ownership/attribution/coverage so the GUI can
attribute a streamed line, a quote and a trade.)

**Desk taxonomy — three orthogonal axes; a "book" is their intersection** [industry-standard /
medium]:
1. **Franchise/region** — G10 vol vs EM vol, EM sub-split by region (driven by NDF/fixing mechanics).
2. **Role-on-flow** — market-making/streaming vs structuring/RFQ vs correlation/exotics. (Product
   sub-desks exist *within* G10 vol — e.g. a real Citi "FX Options G10 Correlation Trading Desk Head"
   posting [Citi / high].)
3. **Complexity/channel** — auto-priced electronic flow vs voice-brokered large/exotic tickets.

**Scale & electronification.** Top-20-dealer daily FX-options volume rose **+136% in the 5 years to
Oct 2024** (US FX Committee survey) [FXC / high]; separately, BIS reports options turnover **+108%
Apr-2022→Apr-2025**, reaching ~7% of total FX turnover (≈$0.67tn/day — the report's own arithmetic;
BIS publishes no standalone $ figure) [BIS-25 / high — do not conflate the two growth numbers].
Qualitatively, **most tickets are auto-priced/electronic but most *notional* is still voice**, with
large/exotic tickets staying voice [FX-Markets/e-Forex/FOW / high]. Precise "66–90% electronic /
10–40% of volume" and "$50–200m ticket" figures are **unsourced and dropped** as false precision.

**Internalise vs externalise, skew-toward-axe.** Dealers warehouse vs hedge and **skew the
bid/offer toward their axe** to attract offsetting flow [Risk.net / academic / high]. **Asset-class
caveat:** the widely-cited ">80% internalisation" figure is a **G10 *spot*** statistic (~63%
aggregate, >80–90% in majors at top e-FX houses) [BIS / LSE-Oomen / high] — **options
internalisation is structurally lower and less electronic** (gamma/vega is harder to warehouse and
offset). Treat 80% as a **spot upper-bound analogue, not an options fact.**

**Axe distribution as a product opportunity.** An MTF operator (OptAxe, FCA-authorised) states that
**only ~25% of bank-to-client FX-options axes are distributed successfully** via traditional
channels [OptAxe / high — but a **single-vendor self-published** figure with a commercial interest;
triangulate before sizing it as a product opportunity].

**Follow-the-sun.** A single global risk book is passed **HK/SG/Tokyo → London → NY**, handing over
*both* quoting responsibility and live position/axe context [industry practice / high]. Celnet's
blue-green drain/pin/cut-over is a structural *analogy* for the regional engine handover — flagged
as a design analogy, not a claimed feature.

**Attribution & governance (proposed).** Thread a `book_id` + `owner` (human seat *or* auto-pricer,
treated uniformly) and an `AttributionRecord` (who quoted, against which surface_version, with what
LP competition) onto the `celnet-observability` audit committer — *outside* the zero-alloc hot core.
The auto-pricer identity needs **governance**: kill-switch, max-risk envelope, quote throttle (a
real desk + regulatory expectation under algorithmic-trading rules). **Regulatory scope (precise):**
the attributability requirement rests on MiFID II **Art. 27 best-ex + RTS 6 order/decision
record-keeping** (and RTS 6 for algo trading) — **not** on the abolished RTS 27/28 *reporting*
regime; FX **spot is out of MiFID scope** (only deliverable forwards/options are in); the **FX
Global Code** (internalisation, last-look, information-handling principles) and US Dodd-Frank/CFTC
swap reporting also apply [ESMA/FCA/Global Code / high with caveat]. **Prime-brokerage / give-up &
credit lines** gate *who can trade with whom* at scale — a counterparty/attribution dimension the
current model omits [gap, §7].

---

## 4. Architecture at scale — pricing, risking & streaming the cube in real time

Governing rule (from `docs/SCALE-OUT.md`): **scale *around* the hot path, never *through* it.** The
latency-critical work stays on a fat, NUMA-pinned, thread-per-core single node; distribution exists
for capacity, fault-tolerance and fan-out only.

### 4.1 Incremental / dirty recalibration — the biggest lever

The dominant cost is surface recalibration. A market tick touches a handful of broker quotes for
**one tenor of one pair**; naively re-marking the whole cube is wasteful. The SOTA pattern is a
**dirty-bit dependency graph** (QuantLib's `LazyObject`/observer pattern is the canonical reference
— research prose only; the *named pattern must not leak into Celnet identifiers* per rule 8): a
broker-quote change sets a dirty bit on exactly the dependent `(pair, tenor)` smile; recalibration
runs lazily, on demand, and caches. This collapses an O(cube) reval into O(touched smiles).

**Proposed:** a `celnet-surface` **`SurfaceCube`** store keyed `(pair, surface_version)` holding the
**parametric** calibrated smiles (a few parameters per tenor, not the dense grid), with a dirty
dependency graph from broker inputs → smile → cube node. The dense grid is **materialised on demand**
and cached, never the source of truth (§4.3 separates *authoritative* parametric from *materialised*
grid).

### 4.2 Vectorised + GPU pricing, AAD for Greeks

- **Batch-vectorise** the materialise/scenario path: price a whole `(tenor × delta)` slice, or a
  many-instrument batch, with SIMD on CPU and the `celnet-gpu` `PricingBackend` (wgpu/Metal +
  CUDA + f64 CPU oracle; note Metal lacks f64) for the embarrassingly-parallel scenario grids.
  Existing bench: vanilla price ~8.85 ns, +14 Greeks ~19 ns, 64-strike batch ~6.75 µs.
- **AAD (adjoint algorithmic differentiation)** for the full Greek set in one reverse sweep is the
  textbook-standard SOTA for risk at portfolio scale [established practice / high] — research prose
  only; `AAD` must not appear in shipped identifiers.
- **Columnar in-memory** (Apache Arrow-shaped) layout for the risk grid keeps the scenario reval
  cache-friendly and zero-copy to the GPU [established practice / high] — again, `Arrow` stays out of
  identifiers.

### 4.3 Surface/cube caching — authoritative vs materialised

Keep two layers strictly separate:
- **Authoritative**: the versioned parametric surface (`MarkedSurface` / `surface_version` already
  in the contract). Small, cheap to store, the single source of truth; pricing/RFS/RFQ **pin**
  against a `surface_version` (already enforced server-side — unknown version ⇒ `failed_precondition`,
  never a silent live fallback).
- **Materialised**: the dense `(tenor × delta)` grid + scenario tensors, computed on demand from the
  authoritative surface and cached with the dirty graph. Disposable; rebuildable; never streamed as
  the source of truth.

This is the "recompute ≫ store" lever made concrete: cache the O(parameters) authoritative layer;
recompute the O(10⁴–10⁵) materialised layer lazily.

### 4.4 Conflated fan-out streaming + sharding

- **Shard by currency-pair** (all tenors of a pair co-resident on one pinned `celnet-engine` shard,
  because surface rebuild is per-pair-all-tenors); sub-shard hot pairs by book/tenant; assign via
  **HRW/rendezvous** hashing (per `docs/SCALE-OUT.md`). Region sharding aligns with follow-the-sun.
- **Conflation**: the edge already conflates (the blotter footer says "Conflated 60Hz"); at universe
  scale, fan-out must conflate per-subscription so a slow counterparty cannot back-pressure the core,
  and coalesce deltas (already `Snapshot`/`Update` + resync in the contract).
- **Many→many fan-out**: the in-proc LMAX-disruptor SPMC ring (`celnet-fanout`), the replicated
  log + full Raft consensus (`celnet-replog`), and the HRW router tier (`celnet-router`) that
  `docs/SCALE-OUT.md` specifies are now **built** — the SPMC ring is wired under the edge so the
  per-pair price path fans out through one producer/pair → N session consumers (the per-session
  tokio channel now carries only control/lifecycle frames). What remains designed-only is the
  cross-DC datapath hardening and the absolute cross-host fan-out SLO (deploy-gated).
  A cross-fleet multicast tree (the research literature's **Jasper**, arXiv:2402.09527, SIGCOMM'24,
  reports median ~129 µs to 100 receivers / ~238 µs to 1000, with Huygens clock-sync + hold-and-
  release fairness) is a **proposal**, and those are **Jasper's published *cloud-multicast*
  benchmarks, not Celnet measurements** [Jasper / high-for-the-paper, external]. Adoption risk: Jasper
  is a 2024 *research artifact*, not a hardened library, and solves *cloud* multicast — if Celnet's
  tier-1 IB deployment is **colo/on-prem with hardware multicast or Aeron**, the Jasper differentiator
  may not apply. State the deployment assumption that makes it relevant, and gate the headline on a
  **Celnet-measured cross-fleet fan-out bench**, not Jasper's numbers.

### 4.5 Budgets

Tie everything to `docs/ARCHITECTURE.md` §1.2: p50 ≤ 2 µs vanilla on a pinned core, surface rebuild
p99 ≤ 150 µs per-pair-all-tenors, ≥1M price updates/s/core. At cube scale these are **per-shard**
budgets; the universe scales by adding shards (capacity), not by making one price cross a node
(disqualifying — kernel net stack ~20–50 µs is 10–25× over the 2 µs budget). The **client-side
render budget** must also be stated (§5): a 150k-update/s server stream is meaningless if the GUI
re-renders the DOM per tick.

---

## 5. The SOTA experience at scale

The experience must let a trader navigate *hundreds* of pairs × a continuous date axis × a delta
cube without drowning. The references below are sourced; the Celnet mapping is proposal.

### 5.1 Navigating breadth — watchlist / favourites / search / region trees
- A **single global active pair + a static 5-tile strip cannot scale** to hundreds of pairs. The
  SOTA is a **configurable watchlist/favourites + a region/liquidity tree + a command-palette
  search** (the ⌘K palette already exists as the escape hatch). Incumbents lean on saved layouts,
  pop-out windows and shared/role-based desk views (Bloomberg Launchpad; Murex "desk managers have
  the same view") [general UX / medium].
- The pair navigator should be a **virtualised, searchable, groupable** list (by region / G10-vs-EM /
  liquidity tier / deliverable-vs-ND / metals), with favourites pinned and multi-pair monitoring.

### 5.2 High-density blotters with server-side aggregation
- The current blotter maps `app.stream.rows` straight to the DOM (not virtualised) — fine for 6 rows,
  fatal for thousands. The SOTA is **row+column virtualisation + a server-side row model**: a
  data-grid like AG Grid demonstrates **150k+ updates/s** (independent benches ~178k/s) with the
  Server-Side Row Model and async transaction batching (`asyncTransactionWaitMillis`) [AG Grid /
  high]. The client coalesces ticks to a frame budget; the server pre-aggregates.

### 5.3 The vol-cube pivot / heatmap
- The cube (`pair × tenor × delta`) needs **pivot/heatmap navigation**: pick two axes, fix the third,
  render a colour-mapped grid (e.g. ATM-vol or RR by pair×tenor; or smile by tenor×delta for one
  pair). Incumbents do point-by-point smile + barrier topography (Murex) and arbitrage-free surfaces
  even at low strikes/delta (ICE Data Derivatives, ex-SuperDerivatives) [Murex/ICE / high].
- **WebGPU substrate (specific to *this* GUI):** the existing `viz/SurfaceMesh` (WebGPU) is the
  natural substrate for a **GPU-rendered, instanced heatmap** of cube cells — far better than a DOM
  grid for a dense `pair × tenor` matrix. The scale-UX recommendation is *not* "add AG Grid
  everywhere"; it is virtualised DOM for the **blotter** and GPU-instanced cells for the **cube**.

### 5.4 The calendar/event-aware broken-date pricer in the ticket
- The ticket must accept a **broken/odd date** (explicit expiry date) and resolve the full date chain
  (horizon → spot → expiry → delivery → cut/fixing), show the resolved expiry/delivery/cut, and mark
  **IMM / end-of-month / event** dates on the date picker. Total-variance interpolation with
  day-weighting (incl. event-day uplift) must drive the priced vol — not a nearest-pillar lookup.

### 5.5 TREND / SPARKLINE semantics — the central fix

**The problem (verified in source).** Today the sparkline is **axis-agnostic** (`Sparkline.tsx` takes
a bare `number[]`, no label) and is fed two different unlabelled things:
- In the **blotter**, `row.midHistory` — the streamed *option-premium* mid history (`fmtPremiumPct`).
- In the **pair strip**, `aggregateActivity()` rebases each contributing row's `midHistory` to its
  own first point and **averages across structures** into a "unitless, comparable activity index"
  (the code's own comment, `PairStrip.tsx` L90). It conflates several structures' premiums into one
  abstract index — *not* spot, *not* vol, and not configurable.
- The **direction cues are also inconsistent**: `PairTile` reads direction from the *last two points*
  of the rebased average (L96–98); `Sparkline` colours up/down from the *first-vs-last* endpoint
  (L49). Two different definitions of "up".

**Honesty note (important):** the underlying data is **honest real streamed premium**, not fabricated
— the defect is **metric semantics + non-configurability + inconsistent direction**, *not* a
data-honesty bug. `PairStrip` even shows spot **statically** and admits in its own comment that
**there is no live spot feed** in this transport. So the fix is a labelled, configurable series — and
the choice of default is **constrained by what the transport can actually deliver today.**

**The recommendation — a labelled, configurable `TrendMode`:**

| `TrendMode` | Series plotted | Available today? | Sensible default |
|---|---|---|---|
| `PREMIUM` | the instrument's own streamed premium mid | **yes** (`midHistory`) | **blotter-row default** |
| `ATM_VOL` | the pair's ATM vol history | only once the transport streams a vol history | pair-strip default *once feed exists* |
| `RR` / `BF` | risk-reversal / butterfly (skew/wings) in vol terms | once streamed | trader-selectable |
| `SPOT` | spot mid | **NO** (no live spot feed in this transport) | needs a new feed first |
| `FORWARD` | outright forward | once streamed | selectable |
| `VEGA` / `PNL` | position-weighted vega / mark-to-market P&L | from position store | book/blotter selectable |

**Recommended defaults, honestly sequenced:**
1. **Now (no new feed):** the trend defaults to the instrument's **own premium mid, explicitly
   labelled "Premium"** on the tile, with a **single, consistent direction definition** (first-vs-last
   over the displayed window) shared by the line *and* the tick glyph. Stop the cross-structure
   premium-blend in the pair strip — show the active/representative structure's labelled premium, or
   nothing ("—"), not an unlabelled abstract index.
2. **Target default = `ATM_VOL`** for the **pair strip** (a vol trader watches vol, not premium) —
   but only **after** the transport streams an ATM-vol history. Until then, defaulting the strip to
   "ATM vol" would draw an empty/static line, trading one ambiguity for another, so it stays
   "Premium (labelled)" until the feed lands.
3. `TrendMode` is an **open, per-context enum**: pair-strip, blotter-row and cube-cell may each carry
   a different sensible default, and the tile **always shows the unit/label** (vol pts, %, pips).
4. **Prerequisite, sequenced:** a streamed **ATM-vol / spot / RR time-series** in the wire contract is
   the gate for the vol/spot/RR modes — this is a backlog dependency (§8 #11), not assumed present.

### 5.6 Multi-monitor / workspace persistence & front-end budget
Saved layouts, pop-out windows and shared role-based desk views are a real scale-of-universe need
(incumbent table-stakes). And the scale-UX claim must reconcile to a **client render budget**: tick
coalescing to frames, async transaction batching, and GPU-instanced cube rendering — stated against
the server stream rate, not aspirational.

---

## 6. Critique — current Celnet GUI vs SOTA vs incumbents

**All `gui/` findings below are verified against source this session.** Cleanly separating verified
facts from proposal:

> **Reconciliation note (status update — this critique is a point-in-time record).** Several of
> the "Celnet GUI today" gaps in the table below have since been **closed** and are preserved here
> only as the original critique record, not as current state: the static 5-tile strip → a
> **UniverseNavigator** (⌘B, searchable/grouped/favourites); the not-virtualised blotter →
> **virtualised** windowing; per-pair-surface-only → a **vol-cube pivot/heatmap**; no broken-date
> picker → a **broken/IMM/event-aware ticket**; the unlabelled premium-blend sparkline → a
> labelled, configurable **`TrendMode`** with one consistent direction (commit `547b7bd`). The
> **Tenor enum has also been overhauled** (now `Overnight`/`TomNext`/`SpotNext`/`Imm`/`BrokenDate`
> + Weeks/Months/Years) and the **ON-resolves-as-SN bug is fixed** (ON = next good business day
> after the horizon) — so the "Depth (date axis)" row's enum/bug claim is **no longer true**. The
> SOTA-target and incumbent columns remain the standing design reference.

| Dimension | Celnet GUI today (verified — see reconciliation note above; several rows now superseded) | SOTA target (proposal) | Incumbents (cited / confidence) |
|---|---|---|---|
| **Breadth** | single global active pair (`AppContext.pairCtx`); static 5-pair seed (`seed.ts PAIRS`); no search/virtualisation/region tree | hundreds of pairs; watchlist/favourites + region/liquidity tree + ⌘K search; tier-driven cadence; ND/metals first-class | Tradition 140+ surfaces [high]; Fenics FXO 2.0 300+ pairs/27 metals [high]; Digital Vega 60+ pairs [high] |
| **Depth (date axis)** | fixed 8-tenor ladder (`seed.ts TENOR_LADDER`); blotter shows ON/W/M/Y only; **no broken-date/IMM/event picker**; Tenor enum has only Overnight/Weeks/Months/Years (and **ON resolves as SN — T+3** — a real bug-class) | continuous expiry line; broken/IMM/EOM/event dates; total-variance day-weighted interp; cut-as-fixing | Murex what-if **date shifts** [high]; broken-date pricing is standard FX practice [Clark / high] |
| **The cube** | per-pair surface view only (`SurfaceWorkspace`); no firm-wide cube/pivot/heatmap | `pair × tenor × delta` pivot + GPU-instanced heatmap on the existing `SurfaceMesh` | Murex point-by-point smile + barrier topography [high]; ICE arb-free even at low strike/delta [high] |
| **Trend clarity** | sparkline axis-agnostic & **unlabelled**; pair-strip plots an **abstract cross-structure rebased premium index**; direction defined two inconsistent ways (PairTile L96–98 vs Sparkline L49) | labelled, configurable `TrendMode` (Premium/ATM-vol/RR/BF/Fwd/Vega/PnL); one consistent direction; default Premium-labelled now → ATM-vol once a vol feed exists | Bloomberg BVOL real-time vols 200+ pairs [high]; RR/BF series are standard vol-trader views |
| **Virtualisation** | blotter maps `stream.rows` straight to DOM (not virtualised) | row+column virtualisation + server-side row model + async tx batching | AG Grid 150k+ upd/s [high] |
| **Who's-trading / attribution** | **no** book/trader/region/client/auto-pricer identity anywhere in `src/`; RFS rows carry no routing/attribution; Book view sums **all** positions as one **anonymous** aggregate (no owner dimension, no numeraire normalisation — deferred to RISK-HIERARCHY) | `book_id` + owner (human/auto-pricer) + AttributionRecord; follow-the-sun/region awareness; PB/credit context | Murex single-screen aggregate-by-pair + per-pair Greeks [high]; OptAxe: only ~25% of axes distributed [single-vendor / high] |
| **What's *right* (keep)** | RFS-as-resting-state; no-fabricated-data honesty (real streamed premium, honest "—" empty state); click-to-trade + last-look (`TradableToken`); surface_version pinning; book-risk decomposition (bucketed vega / cross-gamma / theta-roll) | keep all of the above | streaming-first is the structural inversion of incumbent RFQ-per-click |

**Incumbent reference notes (cited, with confidence):** Bloomberg BVOL real-time vols on **200+
pairs** over B-PIPE, OVDV vol-surface fn / OVML multi-leg pricer with ATM/RR/BF inputs [Bloomberg /
high] (the "synthetic surfaces from correlation" mechanism is **unverified** — concept plausible,
exact wording not primary-sourced [low]); 360T RFS multi-bank ranked by factor models (price/vol/
spreads), 200+ LPs [360T / high]; Digital Vega Medusa = first FX-options MDP (2013), up to 5 banks /
5-min RFS / dynamic best-bid-offer / 4 simultaneous requests, 60+ pairs, FIX 4.4 [DigitalVega /
high]; Fenics Direct 100+ pairs incl. NDF + XAU/XAG, 20+ banks [high]; ICE Data Derivatives arb-free
surfaces even at low strike/delta [high]; Murex MX.3 single-screen FX options, P&L banner,
aggregate-by-pair + per-pair Greeks, what-if spot/vol/**date-shift**, barrier topography, SLV [Murex
/ high]. **Correction:** a "Murex **GPU Monte-Carlo**" claim does **not** appear in the cited Murex
FX-options spotlight — Murex has GPU MC elsewhere, but it is **mis-attributed** there and should be
re-sourced or marked low.

---

## 7. Citations & honest gaps

Verified facts are cited; design proposals are not. Confidence tags inline above.

- **BIS** Triennial Surveys: 2022 (rpfx22) and 2025 (rpfx25 / press p250930) — turnover, currency &
  pair shares, options share, CNY/CNH. https://www.bis.org/statistics/rpfx25_fx.htm ·
  https://www.bis.org/press/p250930.htm · https://www.bis.org/statistics/rpfx22_fx.htm
- **LSEG / LCH ForexClear** "What we clear" — 25 NDF / 9 NDO / 8 deliverable, splits, NDO 2y tenor.
  https://www.lseg.com/en/post-trade/clearing/lch-services/forexclear/what-we-clear
- **LSEG** "The case for turn-impact-adjusted FX forward curves" — 51% broken-dated, 20% on turns,
  1–15 pips, >$460bn/day (all-products venue figure).
  https://www.lseg.com/en/insights/fx/the-case-for-turn-impact-adjusted-fx-forward-curves
- **EMTA** FX & currency-derivatives documentation — per-currency Template Terms / Settlement Rate
  Options / Disruption Fallbacks.
  https://www.emta.org/documentation/emta-standard-documentation/fx-and-currency-derivatives-documentation/
- **ISDA + EMTA** 2026 FX Definitions (3 Mar 2026; effective 22 Nov 2027).
  https://www.isda.org/2026/03/03/isda-and-emta-publish-revised-definitions-for-fx-derivatives-market/
- **CME** FX-options expiration-time change to 10:00 NY (2019-06-09); IMM 3rd-Wed convention; cuts.
  https://www.cmegroup.com/trading/fx/expiration-time-change-for-cme-fx-options.html
- **CLS** settled-currency coverage (deliverable-universe anchor). https://www.cls-group.com/
- **Tradition** Enhanced FX Options (140+ pairs, 1W→30Y, 5/10/15/25/35Δ RR/BF, 3 snaps/day; Tradition
  × Numerix). https://www.traditiondata.com/products/tradition-enhanced-fx-options/ ·
  https://www.traditiondata.com/products/fx-options/
- **Fenics** FXO 2.0 (300+ pairs / 27 metals) & Fenics Direct (100+ pairs incl. NDF + XAU/XAG, 20+
  banks, FIX 4.4). https://home.fenicsdirect.com/
- **Digital Vega** Medusa (first FX-options MDP 2013; 60+ pairs; execution rules).
  https://www.digitalvega.com/ · https://www.digitalvega.com/execution
- **360T** (200+ LPs; RFS factor-model ranking — SEF user guide / 360T.com). https://www.360t.com/
- **Bloomberg** Real-Time Volatilities (BVOL 200+ pairs over B-PIPE); OVDV/OVML.
- **ICE Data Derivatives** (ex-SuperDerivatives) arb-free surfaces.
- **Murex** MX.3 FX-options product spotlight (single screen, P&L banner, date-shift what-if, barrier
  topography, SLV). https://www.murex.com/
- **OptAxe** — "~25% of bank-to-client axes distributed successfully" (single-vendor figure, via FX
  News Group / The Full FX).
- **US FX Committee** semi-annual volume survey (+136% top-20 dealer options vol, 5y to Oct 2024; via
  Risk.net / FX-Markets "Eyes on automation as FX options volumes surge").
- **BIS / LSE (Oomen)** FX spot internalisation (~63% aggregate, >80–90% majors) — applied as a *spot*
  analogue only.
- **MiFID II** Art. 27 best-ex + RTS 6 record-keeping/algo-trading (RTS 27/28 reporting abolished — UK
  FCA Dec-2021, EU under the MiFIR review); **FX Global Code** (internalisation, last-look).
- **Clark**, *FX Option Pricing* (total-variance / flat-forward-vol interpolation, day-weighting), via
  rateslib temporal-vol docs. https://rateslib.readthedocs.io/en/latest/z_fxvol_temporal.html
- **Healy**, *Counterexamples for FX Options Interpolations* — arXiv:2512.19621 (Part I),
  arXiv:2512.19625 (Part II), Dec 2025.
- **Jasper** cloud-multicast (proxy tree, Huygens sync, hold-and-release fairness; 129 µs@100 /
  238 µs@1000) — arXiv:2402.09527, SIGCOMM'24 (external benchmarks; research artifact).
- **AG Grid** (150k+ updates/s, Server-Side Row Model, virtualisation, async tx batching).
  https://www.ag-grid.com/
- **QuantLib** `LazyObject`/observer dirty-recalc (incremental-recalibration reference; research
  prose only — name must not enter Celnet identifiers).
- **GUI source (verified this session):** `gui/src/components/PairStrip.tsx`,
  `gui/src/components/Sparkline.tsx`, `gui/src/workspaces/StreamWorkspace.tsx`,
  `gui/src/workspaces/SurfaceWorkspace.tsx`, `gui/src/app/AppContext.tsx`, `gui/src/data/seed.ts`,
  `gui/src/data/contract.ts`; `crates/celnet-types/src/lib.rs` (Tenor enum),
  `crates/celnet-calendar/src/fx.rs` (`expiry_for_tenor`),
  `crates/celnet-surface/src/termstructure.rs` (`BusinessClock`/`CalendarClock`).

**Honest gaps & confidence flags.** (1) Liquidity-tier counts, ~135–375 nodes/pair, O(10⁵–10¹¹)
reval, and the 50–150-pair universe sizing are **inferred/low**, arithmetic over sourced axes, not
market facts. (2) The *real* options-streamed-pair count per bank and the *real* intraday
surface-update rate are **not publicly published** — treated as per-deployment config / measured
benches, never asserted. (3) Jasper figures are **external cloud benchmarks**, not Celnet-measured,
and gated on a colo/cloud deployment assumption. (4) "Internalisation >80%" is a **spot** statistic
used only as an options upper-bound analogue. (5) The OptAxe 25% is a **single-vendor self-published**
figure. (6) Bloomberg "synthetic surfaces from correlation" exact mechanism and a "Murex GPU MC"
attribution are **unverified/mis-attributed** and flagged. (7) Negative claims about incumbents are
arguments-from-absence. (8) Prime-brokerage/give-up credit modelling, listed/exchange FX-options
turnover sizing, and CLS settlement-risk-at-cutoff for turn/broken dates are **acknowledged missing**
market-structure angles. (9) The event/turn *vol-clock kink* is an inferred claim, kept distinct from
the verified turn-liquidity (forward-points) phenomenon.

---

## 8. BACKLOG (proposed)

> Proposed task list for this facet. **NOT** added to `docs/ROADMAP.md` (owned by a concurrent
> workflow) — to be merged by the orchestrator. Each task: scope · crate/area · dependency. All
> proposed identifiers are vendor-neutral; method/vendor names (LazyObject, AAD, Arrow, Jasper, HRW)
> stay in research prose only, never in shipped identifiers (rule 8).

**Breadth — pair universe & liquidity tiers**
1. **Pair-universe registry + liquidity tiers.** A `PairUniverse` model carrying per-pair
   `liquidity_tier {Streamed|Rfq|Reference}`, region/franchise group, and a metals flag. · `celnet-types`
   (+ `celnet-conventions`) · dep: none (extends frozen vocab — coordinate, it is an interface crate).
2. **Settlement & clearing attributes.** Extend `Settlement::NonDeliverable{fixing_source,
   disruption_fallback}` keyed to **EMTA per-currency** terms and the **2026 ISDA/EMTA Definitions**
   (record vintage + Nov-2027 cutover); add an *independent* `clearing_eligibility {Cleared|Bilateral}`
   attribute (LCH-clearable ≠ traded). · `celnet-types` · dep: #1.
3. **CNH≠CNY & metals underlyings.** Distinct underlyings for offshore RMB and XAU/XAG/XPT/XPD with
   their quoting/notional conventions. · `celnet-types`/`celnet-conventions` · dep: #1.

**Depth — continuous expiry / broken / event dates**
4. **Tenor model overhaul.** Replace the Overnight/Weeks/Months/Years enum with ON/TN/SN, IMM,
   end-of-month, and an explicit **broken (date) tenor**; **fix the ON-resolves-as-SN bug**. ·
   `celnet-types` + `celnet-calendar` · dep: #1.
5. **Delivery-led scheduling policy flag.** Add canonical **delivery-led** (add-to-spot → adjust
   delivery → back-derive expiry) alongside the current expiry-led path, behind an explicit policy
   flag; golden test for the month-end divergence case. · `celnet-calendar` · dep: #4.
6. **Event/turn/fixing registries + business clock.** Implement event-date / turn-date / fixing
   registries and a non-identity `BusinessClock` (weekend/holiday day-weighting + event-day uplift)
   behind the existing `BusinessClock` seam; total-variance interpolation with the sawtooth. ·
   `celnet-surface` (+ `celnet-calendar`) · dep: #4, #5.

**The cube — store & incremental recalibration**
7. **`SurfaceCube` store + dirty dependency graph.** Versioned `(pair, surface_version)` parametric
   store with a broker-input→smile→node dirty graph; recalibrate only touched smiles; materialise the
   dense grid on demand + cache. · `celnet-surface` · dep: #6.
8. **Vectorised/GPU materialise & scenario.** Batch the materialise/scenario path over SIMD + the
   `celnet-gpu` `PricingBackend`; reverse-mode (adjoint) Greeks for the batch. · `celnet-gpu` +
   `celnet-engine` · dep: #7.

**Streaming the universe**
9. **Conflated multi-pair fan-out at scale.** Per-subscription conflation + delta coalescing across
   the pair universe so a slow counterparty cannot back-pressure the core; tie to the SCALE-OUT SPMC
   ring / router tier (**now built** — `celnet-fanout` wired under the edge + `celnet-router`; this
   task is the universe-wide conflation/coalescing layer on top). · `celnet-server` + `celnet-engine`
   (+ `celnet-router`) · dep: SCALE-OUT.
10. **Cross-fleet fan-out bench (gate the latency headline).** A Celnet-measured many→many fan-out
    bench under the §1.2 budgets; record the deployment assumption (colo vs cloud) before adopting any
    multicast-tree transport. · `celnet-bench` + `celnet-server` · dep: #9.
11. **Streamed market-series feed (trend prerequisite).** Add ATM-vol / spot / RR / forward
    *time-series* to the wire contract so the GUI trend modes have real data. · `celnet-proto` +
    `celnet-server` · dep: none (interface crate — coordinate).

**Who's trading — attribution (defer roll-up to RISK-HIERARCHY.md)**
12. **Book/owner identity + attribution record.** Thread `book_id` + owner (human seat | auto-pricer,
    uniform) onto quotes/streams/trades; `AttributionRecord` (quoter, surface_version, LP competition)
    on the `celnet-observability` audit committer (outside the hot core); auto-pricer governance
    (kill-switch/max-risk/throttle). · `celnet-observability` + `celnet-server` · dep: none; feeds
    `RISK-HIERARCHY.md` roll-up.

**GUI (do NOT touch `gui/` in this workflow — these are downstream tasks)**
13. **Watchlist manager + region/liquidity tree + search.** Replace the static 5-tile strip with a
    configurable, virtualised, groupable pair navigator (favourites, multi-pair monitoring). · `gui/`
    · dep: #1.
14. **Virtualised blotter + server-side aggregation.** Row+column virtualisation, async tx batching,
    server-side row model; coalesce ticks to a frame budget. · `gui/` + `celnet-server` · dep: #9.
15. **Pair×tenor×delta pivot/heatmap.** GPU-instanced cube heatmap on the existing `viz/SurfaceMesh`
    (WebGPU); axis-pick pivot. · `gui/` · dep: #7.
16. **Broken-date / event-aware ticket.** Explicit date entry, resolved expiry/delivery/cut display,
    IMM/EOM/event markers on the date picker, total-variance day-weighted priced vol. · `gui/` ·
    dep: #4, #5, #6.
17. **Configurable, labelled `TrendMode` + consistent direction.** Replace the unlabelled
    premium-blend with a labelled, per-context `TrendMode` enum (default **Premium-labelled now →
    ATM-vol once #11 lands**); one consistent direction definition shared by line + glyph; stop the
    cross-structure premium average in the pair strip. · `gui/` · dep: #11 (for vol/spot/RR modes).
