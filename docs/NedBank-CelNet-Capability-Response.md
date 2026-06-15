# CelNet — Response to NedBank Capability Requirements

**From:** Celer Trading — Celnet (FX Options) team
**To:** Justin, NedBank (cc Ben)
**Re:** Your follow-up to the CelNet capability walkthrough
**Date:** 2026-06-15

Thanks Justin — this is exactly the level of detail we wanted. Below we work through every
point in your note in order, mapped against where CelNet actually is today. Each item carries
an honest status, what exists now, and any clarification we need from you to lock scope. Status
key:

- **Shipped** — implemented and gated today.
- **Partial** — core is shipped; a defined, named gap remains.
- **In scope** — not built, but a clean fit on an existing seam; net-new but low-risk.
- **Integration** — depends on the Celer / FAFX / JSE estate or a vendor feed, not an in-CelNet build.
- **Clarify** — we need more detail from you before we commit a status.

---

## 1. Product

### 1.1 Asians — arithmetic average rate — **Shipped**
We ship **arithmetic** average-rate Asians, not only geometric. Two contract-selectable
closed-form engines are live and parity-gated: **Turnbull–Wakeman** (lognormal moment-matching
— your "lognormal approx") and **Curran** (conditioning integral). A variance-reduced
Monte-Carlo engine (antithetic + geometric control variate, with a std-error band) exists today
as the validation oracle and can be exposed as a third selectable engine if you want a
path-exact number. Discrete fixing schedules, the continuous limit, and seasoned (in-flight)
averages are all supported.
- **Clarify:** discrete named-fixing averaging vs continuous? And your preferred default engine
  for streaming (fast moment-matching) vs an MC std-error band on request.

### 1.2 Option on Futures (Black-76) — **Partial** (pricing shipped; STP/recon is integration)
The pricing and instrument are shipped: a full **ListedFutureOption** on the Black-76
(forward-price, zero net carry) engine, end-to-end through the API, server, SDK, GUI, Excel and
CLI. It supports **equity-style vs futures-style margining** (futures-style is correctly
undiscounted with a genuine zero discount-rho) and books the future symbol + exchange as
contract identity. What is **not** in CelNet's scope is the downstream STP: representing it as an
option-on-future to FAFX and the FAFX↔JSE two-way margin reconciliation — that is Celer-estate
plumbing.
- **Clarify:** the exact STP path/message from CelNet to FAFX, and who owns the FAFX↔JSE recon
  (does FAFX push back to Celer Options as you suggested?). Also: is the underlying a JSE-listed
  future, and is the option American/European and cash/physically settled into the future?

### 1.3 Pricers with solvers (spot/vega/delta/strike/expiry/premium) — **Partial**
We have a real root-finding layer. **Delta→strike** is fully productized end-to-end (bracketed
Brent/Newton with delta-convention discipline) and available on every vanilla and strategy leg.
The wire contract also carries a **solve-to-target-premium / target-strike** directive. Solving
for spot, expiry, or **implied vol from a quoted premium** is **not** productized yet — there is
no standalone premium→vol inverter today. These additional axes reuse the existing root-finder
and Solve seam, so they are bounded, low-risk extensions rather than new machinery.
- **Clarify:** which solve axes do you actually need at the ticket? A precise list
  (e.g. premium↔strike, premium→vol, delta→strike are the common desk set) lets us scope exactly.
  And by "vega" do you mean *solve the vol that reproduces a premium*, or *target a vega number*?

### 1.4 FX futures (the linear instrument) — **Partial**
CelNet ships the linear FX family — deliverable **forward/outright**, **FX swap**, and **NDF** —
end-to-end. There is **no distinct exchange-listed FX-future product arm** yet: economically it is
the forward plus exchange margining/daily MTM, so it lands cleanly on the existing linear inputs
with a new product arm, but the margin/clearing lifecycle is a JSE/Celer integration.
- **Clarify:** do you mean an exchange-listed, daily-margined FX future (e.g. JSE currency
  futures), or an OTC outright/forward? The latter is shipped; the former needs the new margined
  arm plus clearing integration.

### 1.5 TARFs — **Shipped** (you flagged low priority)
A complete **TARF** is already live: a variance-reduced Monte-Carlo engine with gearing on the
adverse leg, cumulative-gain target redemption, full-gain vs capped-gain (gap-risk) styles, and a
discrete fixing schedule — a first-class product arm through the one contract. A **Pivot-TARF**
variant is also present.
- **Clarify:** leave as-is for now, or are specific variants wanted later (KO/EKI barriers,
  range-accrual, proportional fixings)?

---

## 2. Risk Management

### 2.1 Spot / vol / rates / time shift scenario reporting — **Shipped**
The scenario engine reprices any instrument across user-defined shift axes for **spot, vol, both
rate legs, and calendar-time (theta) roll** — these four are the native axis vocabulary. Every
shocked node full-reval reprices exactly against the closed-form pricer (gated to 1e-10).

### 2.2 Two-factor spot×vol grid — **Shipped**
Both forms exist: a full Cartesian **spot-axis × vol-axis** PV/Greek grid (the classic shock
matrix), and an explicit **cross-gamma** measure via a four-corner mixed-partial stencil over any
two factors, reconciled to a direct finite-difference. Surfaced through the SDK's bucketed-risk
request.
- **Clarify:** do you want the raw grid the desk pivots itself, a single netted cross-gamma per
  factor pair, or both?

### 2.3 Other operational reporting — **Clarify**
We expose structured risk output today (scenario grids, bucketed vega/theta/cross-gamma,
reporting-currency node aggregates, FRTB SBM capital), but "operational reporting" is open-ended
and we won't over-claim.
- **Clarify:** which reports specifically — position blotter, STP/trade-exception report, EOD risk
  pack, limit-utilisation, P&L sign-off? A short list lets us map each to the right seam.

### 2.4 PnL attribution per risk category — **Partial**
The **market-driven** explain is shipped: a Taylor decomposition attributes P&L into delta/gamma
(spot), vega/volga (vol), vanna (spot-vol cross), and discount/carry rho (rates), validated
against a hand expansion. The **lifecycle layer you describe — amendments / events / new-trade,
day-over-day with a zero residual — is not built**; it needs a persisted prior-day position+market
snapshot we don't yet store.
- **Clarify:** confirm your attribution buckets and ordering (e.g. new-trade & amendments isolated
  *before* the market legs; T-1→T spot → smile → rates → theta), and whether "events" means
  scheduled-event vol marks or a generic manual-mark bucket — so the explain ties out your way.

### 2.5 Vega per vol-curve (ATM/25d-RR/10d-RR/25d-fly/10d-fly), vega/volga/vanna by tenor — **Partial**
We compute a **per-tenor vega ladder** that reconciles to the node's total vega, and carry
**vanna and volga** as netted book Greeks — so the tenor-pillar and volga/vanna requirements are
met. The gap is the **market-quote basis**: vega is bucketed by tenor, not yet *projected onto the
ATM / 25d-RR / 10d-RR / 25d-fly / 10d-fly quote instruments*. The surface layer already calibrates
from exactly those RR/fly quotes, so the delta-pillar→RR/fly transform is mechanical net-new work.
- **Clarify:** confirm you want vega re-expressed in those five market buckets, and at which tenor
  pillars (1W/1M/2M/3M/6M/1Y…).

### 2.6 Rates risk per tenor per ccy — **Partial**
Rate sensitivities exist as **discount-rho and carry-rho** (the FX two-rate decomposition) per
position, rolled up per node in the reporting numeraire — i.e. per-leg/per-pair rate risk. The gap
is **per-tenor key-rate bucketing** along the SOFR/depo pillars: rho today is a single point-shift
per leg, not a KRD vector.
- **Clarify:** do you need full key-rate (per-tenor) rho along each curve, or is per-leg
  (discount vs carry) point rho per ccy sufficient for hedging? The per-tenor ladder lands on the
  existing pillar-grid + curve seam.

### 2.7 Strike topography — **Partial**
The building blocks are there — the scenario engine sweeps the spot axis for a P&L/Greek profile,
and the surface layer maintains a calibrated strike grid the desk can sample. A first-class
**strike-axis exposure report** (net delta/gamma/vega laid along the strike continuum) is a thin
new aggregation over those primitives, not yet a named report.
- **Clarify:** define it precisely — net Greek vs strike across the book at a fixed tenor, a
  pin-risk/barrier-proximity map near expiry, or a P&L-at-expiry payoff profile?

### 2.8 Option-on-Future / NDO delta replaced on fixing vs total-portfolio risk — **Integration**
The pricing/risk leaf is shipped (Black-76 option-on-future with equity/futures-style margining;
NDF/NDO fixing source and settlement style are first-class metadata). What you describe —
**replacing delta with the realised fixing at the fixing event** and re-folding into total-book
risk — is a trade-lifecycle/fixing-event behaviour driven by the Celer/FAFX/JSE fixing feed.
CelNet supplies the risk-on-fixing reprice; the estate supplies the fixing and the booking
transition.
- **Clarify:** confirm the fixing-delta-replacement is owned by the STP/fixing layer (FAFX fixing →
  Celer Options), i.e. CelNet *receives* the fix and reprices. Which fixing source is authoritative
  per NDO pair?

### 2.9 Sales margin capture & reporting per portfolio — **In scope**
Not built. We have a maker-side spread model (risk-scaled half-spread → two-way price) and a
multi-dealer RFQ panel that books an external winner, but **no sales-credit/margin capture,
attribution, or reporting by salesperson/portfolio**. It is a clean fit: a margin field on the
booked trade (mid vs dealt) aggregated through the risk cube's existing portfolio/grouping
dimension.
- **Clarify:** how is sales margin defined (dealt vs internal mid vs auto-price)? Reporting
  dimensions (salesperson / client / portfolio / product) and period (per-ticket / daily / monthly)?

### 2.10 Cash balances — rolling balance / settlement diary — **Integration**
Largely the estate's domain. CelNet has the calendar/value-date machinery to *project* cashflow
dates (spot/value-date schedules, USD-settled NDF metadata, holiday roll), but a rolling cash
balance, settlement diary and nostro reporting are a back-office / FAFX responsibility fed by
booked trades and their SSIs.
- **Clarify:** confirm cash balances/diary sit with FAFX / Celer back office, with CelNet
  contributing premium + exercise cashflows and their settlement dates. If you instead want CelNet
  to host the diary, we'd scope a cashflow-projection report off the calendar seam.

### 2.11 Expiry reporting with greeks (vega/theta/gamma) — **Partial**
The Greeks you prioritise are all computed and netted (theta and gamma in the netted book Greeks;
vega both netted and laddered by tenor), and the scenario engine has a native time/theta-roll
axis. The missing piece is the **report shape**: grouping the book's vega/theta/gamma by expiry
date/bucket into an expiry ladder — the same aggregation as the existing vega ladder, extended to
theta+gamma.
- **Clarify:** bucket by exact expiry date, standard tenor pillar, or expiry-cut day? Netted by
  pair, or drilled to the trade?

---

## 3. Market Data

### 3.1 Spot & forwards from the Celer spot/forwards/NDF stack — **Partial**
The resilient ingress is built: a passive WS subscriber enforcing
subscribe → snapshot → sequenced deltas → on-gap full resync, with reconnect/resubscribe and a
bounded multiplexed connection pool, normalising vendor frames (spot mid, forward points/outright,
NDF fixing) to a canonical market context. The gap is a **dedicated decoder + message-type
dispatch for the Celer spot/forwards/NDF wire shapes** — net-new decoder work, not a pipeline
change.
- **Clarify:** which Celer channels/message types carry spot mid/bid/ask, forward points/outrights,
  and NDF fixings, and are they on the WS price service or the distributor? Does the stack deliver
  per-tenor forwards directly, or does CelNet build forwards from spot + curves?

### 3.2 USD SOFR / depo curve from FAFX — **Integration**
CelNet does not bootstrap rate curves itself today — the pricing core consumes a discount factor
per slice (and implies the foreign rate from the forward to machine precision). The clean design
is to **consume an already-bootstrapped USD SOFR/depo curve from FAFX (or Celer staticdata)** and
feed its DF into the carry. The piece to build is the curve-ingest adapter (plus a bootstrap step
only if FAFX supplies par/depo instruments rather than DFs).
- **Clarify:** does FAFX deliver a bootstrapped curve (zeros/DFs) or raw depo/par instruments? What
  message/format and node tenors? Single-curve (SOFR discount = projection) or dual-curve for USD?

### 3.3 Vol curves for sourced pairs from vendors — **Shipped** (mechanism), per-vendor mapping is config
The vendor vol-surface ingest is built and **vendor-neutral**: it decodes a per-(pair,tenor)
delta-space smile (ATM + 25d/10d RR & fly, spot, forward, optional NDF fixing, timestamp) and a
**declared convention descriptor** that we resolve and **cross-check against CelNet's canonical
convention for the pair — flagging a mismatch rather than silently corrupting strikes**. A new
vendor (SpectraXe / DigiVega / Bloomberg / other) is a **thin per-source decoder/config**, not a
pipeline change. To be precise: the generic decoder is shipped; named per-vendor decoders are
config we add per source at onboarding, not pre-built today.
- **Clarify:** which vendor(s) at go-live for EURUSD/GBPUSD/USDJPY, and each one's exact
  delta/ATM/premium convention, day-count and cut? (Feeds rarely publish a convention spec — we
  need it explicitly to avoid strike corruption.)

### 3.4 Independent curves — base / correlation / driven pairs — **Partial**
For **independent sources of the same quoted pair**, CelNet ships a real consolidation layer:
it blends N feeds into one fair mid with staleness-decay weighting and robust median-consensus
divergence gating (outliers excluded from the mid but reported). The gap is a **cross/triangulation
engine** that *derives* a driven pair (e.g. EURGBP from EURUSD×GBPUSD plus a correlation) — every
priced pair today is marked from its own quoted surface. Correlation already exists in our quanto
path, so a base-vs-driven build reuses that input.
- **Clarify:** EURUSD/GBPUSD/USDJPY are all directly quoted — do you also need driven/cross pairs
  (EURGBP, EURJPY…) from base pairs + correlation, or will every traded pair have its own sourced
  surface? If driven: source of the correlation matrix, and which pairs are base vs driven?

### 3.5 Spread management — vanilla AND exotic — **Partial**
A working maker spread model exists and is **structure-agnostic**: it turns a mid into a two-way
price from a risk-proportional half-spread keyed off Greeks (vega/gamma) with a floor — so it
already applies uniformly to vanillas **and** exotics (exotics surface real vega/gamma through the
same Greeks strip). The gap is the **mark-up policy/config layer**: per-pair/per-tenor/per-product
calibration of the charges, a configurable spread schedule, and any distinct exotic (wing/barrier-
aware) policy. Coefficients are global defaults today.
- **Clarify:** desired spread-policy model — per-pair/tenor/product schedule, a sales-margin add-on
  on top of the maker risk charge (ties to 2.9), or trader override? Do exotics need a distinct
  wider/wing-aware policy, or is the Greek-proportional charge with exotic-calibrated coefficients
  enough?

### 3.6 Event management (NFP/CPI/FED/MPC) — **In scope**
Not built. The vol time-axis is currently a calendar-exact day-count with no event kinks. It is a
clean fit on the surface term-structure seam, which already separates **vol-time from
accrual-time** — an event-aware vol-time map concentrating variance on scheduled event dates per
pair slots in there and feeds the existing smile/term-structure construction unchanged.
- **Clarify:** source of the event calendar (Celer/FAFX feed, vendor, or desk-maintained)? Required
  mechanic — per-event added variance / overnight-vol bump, or a richer event-weighting model — and
  must each event's weight be desk-tunable per pair?

---

## 4. EOD Processes

### 4.1 Lock market data for EOD flash (all systems reval off one snapshot) — **Partial**
The core is shipped: marking a surface stamps an **immutable, monotonically-versioned snapshot**,
and every CelNet pricing/RFQ/RFS/FIX path can **pin to that exact version**, so all CelNet
consumers revalue off byte-identical inputs; an unknown version hard-fails rather than silently
using the live mark. **One integrity gap to flag honestly:** if a pinned version exists but did not
mark a *particular pair*, the current behaviour falls back to that pair's live vol (echoing the
version) instead of hard-failing — a flash run needs a **strict-mode hard-fail** on that path,
which is a small, well-localised fix we would land before go-live.
- **Clarify:** does "all systems" include FAFX and external risk/reporting tools — do they consume
  the same published version, or need a flat-file snapshot export of the locked marks? And is the
  flash snapshot per-pair/desk (as today) or a single **firm-wide atomic close** (a thin
  orchestration layer over the existing per-pair marks)?

### 4.2 Start/stop market data for risk while pricing stays live — **Partial**
The **dual-rail behaviour is achievable per request today**: a risk/repricing run can pin a frozen
version while live pricing continues unpinned, concurrently, with no interference (the pin is a
per-request data selector; frozen marks are immutable). What does **not** exist is a stateful global
**"freeze/resume the risk feed"** toggle, and the scenario/risk-grid RPC currently reprices off an
explicitly-supplied base market rather than resolving a pinned snapshot.
- **Clarify:** which operating model — (a) stateless per-request pin (shipped: risk names a frozen
  version, live pricing keeps streaming), or (b) a stateful operator freeze/unfreeze switch (new
  control-plane verb)? Should the scenario/risk RPCs accept a pinned version so a risk report is
  reproducibly tied to the flash snapshot?

### 4.3 Other EOD processes — **Partial**
Building blocks exist: durable journalling/recovery of book + marks (snapshot survives restart and
replays), an immutable versioned-surface basis for re-mark P&L diffing, and a full risk-cube for
batch portfolio revaluation/aggregation. What is **not** yet a first-class CelNet feature is an
**EOD orchestrator**: a scheduled close/cut-off job, explicit valuation-date rollover, and a
publish/IPV sign-off workflow.
- **Clarify:** which EOD processes do you need beyond flash/reval — official close-mark publish +
  sign-off, valuation-date rollover, settlement/expiry/fixings processing, P&L sign-off, FRTB/IPV
  packs, STP reconciliation to FAFX/JSE? And should the orchestration live **inside** CelNet as a
  scheduled job, or be driven by the Celer estate's EOD scheduler calling CelNet's lock/snapshot/
  revalue verbs? (We'd recommend the latter to avoid duplicating estate scheduling.)

---

## 5. MI (nice-to-haves)

### 5.1 Historical data for back-dated scenario tooling — **Partial**
The bump-and-revalue engine that back-dated history would feed is shipped and full-reval-correct,
including a **historical-simulation VaR/ES** that reprices a supplied scenario set through the
pricer. What is **not** built is a **persisted historical market-data archive** — today's streamed
series are live/throttle-conflated and not retained; our durable store journals the trade
lifecycle, not market history.
- **Clarify:** retention/granularity — EOD curve/surface snapshots only, or intraday tick history?
  Should back-dated scenarios reuse the EOD-locked snapshots (one archive feeding both EOD reval and
  scenario tooling), and over what depth?

### 5.2 Seeing what clients are pricing / RFQ-ing — **Partial**
The **data is already captured losslessly**: every quote/RFQ/book event flows through a
totally-ordered audit stream tagged with the counterparty and the resolved attribution chain
(maker seat/book). What is **not** shipped is a **sales/trading-facing cross-client blotter** that
aggregates and renders that flow — today's GUI blotter shows only the viewer's own live
subscriptions. The audit stream has a drain/consumer seam that such a view taps into.
- **Clarify:** a read-only franchise-flow view across all clients/tenants — what entitlement model
  gates which users see which counterparties? Live streaming, periodic refresh, or both? Do you need
  won/lost/missed outcomes and hit-ratio, not just outstanding requests?

---

## 6. Summary scorecard

| # | Item | Status |
|---|------|--------|
| 1.1 | Asians — arithmetic average rate | **Shipped** |
| 1.2 | Option on Futures (Black-76) | **Partial** — pricing shipped; STP/recon = integration |
| 1.3 | Solvers (spot/vega/delta/strike/expiry/premium) | **Partial** — delta→strike shipped; others bounded extensions |
| 1.4 | FX futures (linear) | **Partial** — fwd/swap/NDF shipped; listed future = new arm |
| 1.5 | TARFs | **Shipped** |
| 2.1 | Spot/vol/rates/time scenario reporting | **Shipped** |
| 2.2 | Two-factor spot×vol grid + cross-gamma | **Shipped** |
| 2.3 | Other operational reporting | **Clarify** |
| 2.4 | PnL attribution per risk category | **Partial** — market legs shipped; lifecycle legs to build |
| 2.5 | Vega per RR/fly bucket, vol/volga/vanna by tenor | **Partial** — tenor ladder + vanna/volga shipped; RR/fly projection to build |
| 2.6 | Rates risk per tenor per ccy | **Partial** — per-leg rho shipped; key-rate ladder to build |
| 2.7 | Strike topography | **Partial** — primitives shipped; named report to build |
| 2.8 | Option-on-Future/NDO delta on fixing | **Integration** |
| 2.9 | Sales margin capture & reporting | **In scope** |
| 2.10 | Cash balances / settlement diary | **Integration** |
| 2.11 | Expiry reporting with greeks | **Partial** — Greeks shipped; expiry-bucketed report to build |
| 3.1 | Spot/forwards from Celer stack | **Partial** — ingress shipped; Celer decoder to build |
| 3.2 | USD SOFR/depo curve from FAFX | **Integration** — curve-ingest adapter |
| 3.3 | Vendor vol curves | **Shipped** (mechanism); per-vendor map = config |
| 3.4 | Independent curves (base/correlation/driven) | **Partial** — multi-source blend shipped; cross/triangulation to build |
| 3.5 | Spread management (vanilla + exotic) | **Partial** — Greek-proportional model shipped; policy/config layer to build |
| 3.6 | Event management (NFP/CPI/FED/MPC) | **In scope** |
| 4.1 | Lock market data for EOD flash | **Partial** — mark/pin shipped; strict-mode hard-fail to add |
| 4.2 | Start/stop risk feed vs live pricing | **Partial** — per-request dual-rail shipped; global toggle to add |
| 4.3 | Other EOD processes | **Partial** — building blocks shipped; orchestrator to build |
| 5.1 | Historical data for scenarios | **Partial** — reval engine shipped; market archive to build |
| 5.2 | Client pricing/RFQ visibility | **Partial** — capture shipped; cross-client blotter to build |

**Net:** of 27 points, the analytics/pricing core you asked about is overwhelmingly shipped or a
bounded extension. The genuinely net-new product work clusters in **reporting *shapes*** (vega
RR/fly bucketing, key-rate rho, strike/expiry topography, sales margin) and **EOD orchestration**;
the remainder is **Celer-estate integration** (FAFX curves, STP/recon, cash/settlement, fixings).
None of it is blocked on CelNet's architecture — it sits on seams that already exist.

We suggest a short working session to close the clarifications above (especially the FAFX/JSE
integration boundary and your exact attribution buckets), after which we can put dates against each
item.
