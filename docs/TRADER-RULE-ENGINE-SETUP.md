# Trader Setup Guide — Acceptance, Risk Routing & Hedging

This guide walks a trader through configuring the three composable rule engines that
govern the full life of an incoming client lift, end to end:

1. **Acceptance** — *do we take this trade at all?*
2. **Risk routing** — *which risk book does the resulting position land in?*
3. **Hedging** — *do we warehouse the risk internally, or hedge it back-to-back into the
   aggregated liquidity?*

All three are edited with the same builder idiom (an ordered, first-match **rule list**
that compiles to a decision graph, with a live trace panel), so once you have set one up
the others feel identical.

---

## 1. How a lift flows through the three engines

When a client accepts one of our quotes, the engine runs these steps **in order**. Each
rule engine owns exactly one step:

```
Client RFQ / RFS
      │
      ▼
Engine prices it ──────────────►  Quote (carries a QuoteID)
      │
      ▼
Client lifts ─────────────────►  NewOrderSingle referencing that QuoteID
      │
      ▼
┌───────────────────────────────────────────────────────────────────┐
│ 1. LAST-LOOK (always on, not configurable)                        │
│    Is the QuoteID still live? (unknown / expired / already-lifted  │
│    → reject)                                                       │
└───────────────────────────────────────────────────────────────────┘
      │  quote valid
      ▼
┌───────────────────────────────────────────────────────────────────┐
│ 2. ACCEPTANCE RULES        ← you configure this                    │
│    Accept / Reject(reason) / Hold-for-desk-review                  │
└───────────────────────────────────────────────────────────────────┘
      │  Accept
      ▼
   Book the deal
      │
      ▼
┌───────────────────────────────────────────────────────────────────┐
│ 3. RISK ROUTING            ← you configure this                    │
│    Which risk book does the booked position roll up into?          │
└───────────────────────────────────────────────────────────────────┘
      │
      ▼
┌───────────────────────────────────────────────────────────────────┐
│ 4. HEDGING RULES           ← you configure this                    │
│    Warehouse internally (up to the DV01 cap) vs back-to-back to    │
│    the market. Includes the "are we making money" edge tolerance.  │
└───────────────────────────────────────────────────────────────────┘
      │
      ▼
   Deal visible in the Deal blotter (with an internalise badge) and the
   Risk Dashboard (rolled up under its risk book).
```

Key distinction worth internalising:

- **Acceptance** decides *whether* we do the trade. Reject here and there is no deal.
- **Hedging** decides *what we do with the risk* once we have already accepted it. The
  profitability / edge check that decides warehouse-vs-hedge lives **here**, not at
  acceptance — unless you deliberately add an edge rule to the Acceptance engine to turn
  unprofitable lifts away up front.

Every engine ships a **no-op default** (Acceptance = accept everything; Routing = the
seeded firm warehouse book; Hedging = warehouse with an advisory external overflow), so
the platform behaves exactly as before until you write rules.

---

## 2. Prerequisites

You need the capability for each surface you intend to edit. Ask an administrator to grant
these on the **Administration → Permissions** grid (they are all per-user, per-asset):

| Engine        | Capability          | Where to configure it (GUI)                              |
|---------------|---------------------|----------------------------------------------------------|
| Acceptance    | `manage_acceptance` | Fixed Income → **Risk → Acceptance**                      |
| Risk routing  | `risk_manage`       | Fixed Income → **Risk → Risk Routing**                    |
| Risk books    | `risk_manage`       | Fixed Income → **Risk → Risk Dashboard → Portfolios tab** |
| Hedging       | `hedge`             | Top-level **Hedging** tab → Policy / Thresholds / LP Panels / Monitor |

These are independent grants — you can give a desk head `risk_manage` + `hedge` without
`manage_acceptance`, or a credit/onboarding role only `manage_acceptance`.

---

## 3. Step 1 — Acceptance rules

**Where:** Fixed Income → Risk → **Acceptance**.

The Acceptance engine runs at the moment of the lift, *after* last-look validity and
*before* the deal books. It answers one question per rule and stops at the first match.

### The building blocks

**Condition fields** you can test:

| Field            | Type     | Meaning                                              |
|------------------|----------|------------------------------------------------------|
| Counterparty     | name     | The lifting counterparty / party id                  |
| Notional (USD)   | number   | The trade notional                                   |
| Tenor (years)    | number   | The instrument tenor                                 |
| Instrument       | text     | The curve / security symbol                          |
| Side             | enum     | Pay / receive (or buy / sell)                        |
| Edge (bps)       | number   | Signed dealer edge of this lift vs the engine mid    |
| Quote age (ms)   | number   | How long since we published the quote                |
| Asset class      | enum     | FX options / fixed income                            |
| Desk             | enum     | The desk the flow belongs to                         |

Operators follow the usual matrix — numeric fields take `= ≠ > ≥ < ≤ between`; text takes
`= ≠ contains in`; enums take `= ≠ in`.

**Decisions** (the leaf of each rule):

- **Accept** — proceed to book (then routing + hedging run downstream).
- **Reject(reason)** — the lift is turned away; the counterparty gets an execution reject
  carrying your reason. No deal.
- **Hold for review(reason)** — the lift is **not** auto-filled; instead the request drops
  into the desk inbox so a human can accept it manually (this is the natural place for a
  future credit-hold). The quote is not consumed by the hold.

### Worked example

A conservative starter policy — turn away unprofitable and oversized lifts, hold a
watch-list name for manual review, accept the rest:

```
1.  Edge (bps)      <  0.5           →  Reject  "below edge floor"
2.  Notional (USD)  >  250,000,000   →  Reject  "over single-ticket limit"
3.  Counterparty    in  {ACME, ZenCap}  →  Hold  "counterparty on manual review"
4.  Otherwise                        →  Accept
```

Add rules top-to-bottom; the first one that matches wins. The **trace panel** on the right
shows, for a sample context, which rule fires and the path taken through the graph — use it
to sanity-check ordering before you **Save**. Saving installs the graph immediately; it is
persisted and survives a restart.

> **Extending later:** credit checks and richer quote validations slot in as new condition
> fields on this same engine — the graph model and builder do not change.

---

## 4. Step 2 — Risk routing

**Where:** Fixed Income → Risk → **Risk Routing**.

Once a lift is accepted and books, routing decides **which risk book** the position rolls
up into. It is a decision graph of conditions (counterparty, currency, tenor, desk, …) with
**risk-book leaves**.

### Set up the destination books first

Routing only *picks* a destination; the book itself lives in the portfolio registry:

1. Go to Fixed Income → Risk → **Risk Dashboard → Portfolios** tab.
2. The platform seeds one enabled book — **Firm Warehouse** (`warehouse`) — so routed fills
   have a home out of the box. Create additional books here (name, parent for a hierarchy,
   desk, limits) and mark each **enabled**.
3. **Shortcut:** when you save a routing graph, any risk-book id its leaves reference that
   does not yet exist is **auto-created as an enabled book** — so you can build the routing
   tree first and the portfolios follow.

### Worked example

```
if  Counterparty  =  "Marex"        →  Book  MAREX-OIS
elif Currency     =  "EUR"          →  Book  EUR-RATES
else                                →  Book  warehouse   (Firm Warehouse)
```

Save, then confirm on the **Risk Dashboard** tab: each enabled book shows its rolled-up
NET / GROSS / POSITIONS / DV01 / LIMITS, aggregating its own routed fills plus every
descendant book.

---

## 5. Step 3 — Hedging

**Where:** top-level **Hedging** tab (next to Analytics). Four sub-tabs:

- **Policy** — the hedge decision graph (this is the equivalent of the Acceptance/Routing
  rule builder).
- **Thresholds** — the warehouse DV01 caps and utilisation bands.
- **LP Panels** — which liquidity providers a back-to-back hedge is worked into.
- **Monitor** — a live trace of band utilisation against the caps.

Hedging runs *after* the deal is booked and routed. For each fill it decides how much risk
to **warehouse** (hold internally) versus **shed** back-to-back into the aggregated
liquidity.

### The Policy tab (rule builder)

Same idiom as the other two engines — conditions → an **exit action** leaf. The exit
actions express the internalise-vs-hedge vocabulary:

| Exit action        | Meaning                                                        |
|--------------------|---------------------------------------------------------------|
| Warehouse          | Hold the risk internally.                                     |
| Cross internal     | Net the fill against existing internal risk.                  |
| Submit market order| Back-to-back: hedge the clip into the market / LP panel.      |
| RfqOut             | Fan a hedge RFQ out to multiple LPs.                          |
| Split              | Internalise first, hedge only the overflow above the cap.     |
| Escalate           | Hand to a human.                                              |

The condition side keys on the **netted risk-state** — including the identity fields
`book` and `counterparty`. Hedging **by book** scopes a rule to one risk book; hedging
**by counterparty** scopes it to one client's flow (the originating party-id, matched
exactly as the blotter shows it). For example `Counterparty = CITADEL → Submit market
order` back-to-backs **all** of Citadel's flow. Right-clicking a filled deal on the Deals
blotter → **Change hedging strategy** drops you into this builder with the counterparty
condition (plus currency/product/desk) already filled in.

### The Thresholds tab (the "internalise up to 100" control)

Set the **warehouse DV01 cap** — the budget you are willing to hold before hedging out
(the "100" in "internalise up to 100, then back-to-back"). You also set the **amber/red**
utilisation bands and the min/max clip sizes. A fill that fits under the cap and passes the
edge tolerance is warehoused; the overflow (or the whole clip if the price is not making
money) is shed externally.

### The edge tolerance ("are we making money")

The hedge config carries a **minimum edge (bps)** floor (default 0.5bp). On each fill the
engine computes the dealer edge versus the engine mid; **within tolerance** → internalise,
**below tolerance** → back-to-back. This is the profitability gate — it decides *how to
manage* the risk, not whether to accept it. (If you want unprofitable lifts turned away
entirely, add an `Edge (bps) < …` **Reject** rule to the *Acceptance* engine as well — see
Step 1.)

### External hedging is advisory by default

Out of the box the external (back-to-back) leg is **advisory** — it records the hedge
decision and provenance without firing a live order into a venue. Flip that in the hedge
config when you are ready to route real hedges.

---

## 6. Seeing the result

After a lift runs the full chain:

- **Deal blotter** (Fixed Income → Risk → Deals) shows the deal with an **internalise
  badge** — *Internalised* vs *B2B*, tinted by the hedge band (green / amber / red /
  breach), with a "losing" marker when the fill was below the edge tolerance. The deal
  ticket expands to the full internalise detail: edge (bps), within-tolerance, band, and
  the internal / external DV01 split.
- **Risk Dashboard** (Fixed Income → Risk → Risk Dashboard) shows the position rolled up
  under the risk book routing sent it to.
- **Analytics** (top-level Analytics tab) aggregates the client-flow and street-side
  liquidity view over time.

---

## 7. Quick-start checklist

1. Get `manage_acceptance`, `risk_manage`, and `hedge` granted (Administration →
   Permissions).
2. **Acceptance** (Risk → Acceptance): add an edge floor + a size cap + `Otherwise →
   Accept`. Save.
3. **Portfolios** (Risk → Risk Dashboard → Portfolios): confirm *Firm Warehouse* is
   enabled; add any desk/counterparty books you want.
4. **Risk Routing** (Risk → Risk Routing): route by counterparty/currency to those books,
   else `warehouse`. Save.
5. **Hedging** (Hedging → Thresholds): set the warehouse DV01 cap and the min-edge floor.
   (Hedging → Policy): warehouse under cap, submit-market-order on the overflow. Save.
6. Watch a lift flow through: **Deal blotter** for the internalise badge, **Risk Dashboard**
   for the rolled-up book.

Every engine defaults to a no-op, so you can adopt them one at a time.
