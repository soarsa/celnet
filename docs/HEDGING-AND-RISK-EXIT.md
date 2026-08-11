# How Hedging Works — Risk Management and the Exit of Risk

**What this document is.** The *as-built* explanation of what actually happens to risk in
Celnet: how it arrives, how it is measured, how the firm decides to keep or shed it, and —
the part most often misunderstood — the precise mechanism by which risk **leaves** a book.

It is deliberately anchored to the running code, not to intent. Where the implementation is
coarser than the design, this document says so in [§10 Honest boundaries](#10-honest-boundaries)
rather than describing the design as though it were shipped.

**Related documents — read these for a different purpose:**

| Document | Purpose |
| --- | --- |
| [`HEDGING-CONFIGURATION-GUIDE.md`](HEDGING-CONFIGURATION-GUIDE.md) | **Trader-facing**: how to *configure* hedging, tab by tab, in the GUI. |
| [`AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md`](AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md) | The requirements/design and the cited academic basis for each choice. |
| [`FI-RISK-ROUTING-REQUIREMENTS.md`](FI-RISK-ROUTING-REQUIREMENTS.md) | How a fill is *routed* into a risk book. |
| [`RISK-HIERARCHY.md`](RISK-HIERARCHY.md) | The hierarchical risk-aggregation model and limits framework. |
| [`RISK-TRANSFER-REQUIREMENTS.md`](RISK-TRANSFER-REQUIREMENTS.md) | Moving risk *between* books (a different operation from hedging). |

This document is the connective tissue between them.

---

## 1. The idea in one paragraph

A dealer makes money by **warehousing** risk, not by flattening it. Every client fill is
captured at some edge to fair mid; if the desk immediately hedged every fill on the street it
would pay away that edge in spread and impact and earn nothing. So the firm deliberately holds
risk — up to a **budget**. Hedging is the *controlled exit* that happens when one of two
things becomes true: the warehouse is too full (a risk-budget decision), or the flow was not
profitable enough to be worth holding (a price-quality decision). Everything below is the
machinery that makes those two judgements precise, auditable, and automatic.

The governing principle, applied at every stage: **internalise before you externalise.**
Holding costs nothing. Crossing internally costs the consolidated mid. Only going to the
street pays real money.

---

## 2. The spine — six stages

Every unit of risk follows the same six stages. The rest of this document is one section per
stage.

```
   client fill
        │
   ┌────▼─────────────────────────────────────────────────────────┐
   │ 1. LAND      risk routing graph  →  a risk book              │  §3
   ├──────────────────────────────────────────────────────────────┤
   │ 2. MEASURE   book (or subtree) net exposure                  │  §4
   ├──────────────────────────────────────────────────────────────┤
   │ 3. BUDGET    warehouse cap → RAG band → overflow → size      │  §5
   ├──────────────────────────────────────────────────────────────┤
   │ 4. DECIDE    exit-policy decision graph → an ExitAction      │  §6
   ├──────────────────────────────────────────────────────────────┤
   │ 5. SPLIT     price-edge verdict → internal vs external DV01  │  §7
   ├──────────────────────────────────────────────────────────────┤
   │ 6. EXIT      execute on a venue → BOOK THE OFFSETTING LEG    │  §8
   └────┬─────────────────────────────────────────────────────────┘
        │
   provenance + analytics + trace                                    §9
```

Stages 2–6 all run inside `RatesPositionStore::stamp_internalise`
(`crates/celnet-server/src/services/rates_book.rs`), off the pinned pricing core, on the async
booking tier. Nothing here touches the zero-alloc hot path (guardrail 11).

---

## 3. Stage 1 — risk lands in a book

A fill does not simply "belong to a desk". It is routed by a **firm-wide decision graph**
(`celnet-risk-routing`) that walks condition nodes until it reaches a `Book` leaf. The
available condition fields (`RouteField`) are:

`InstrumentId`, `Ccy`, `Product`, `Side`, `Notional`, `Tenor`, `Strike`, `Counterparty`,
`User`, `Desk`, `Price`.

So a firm can express *"bonds over 50mm from CLIENT-X land in BOOK-A, everything else in
DEFAULT"* as data, not code. The resolved book id is stamped onto the `Deal` as
`risk_book_id`, which is what makes the blotter and the risk dashboard reconcile.

Risk books are **hierarchical**. A book has ancestors and descendants, and a hedge decision may
be scoped to a whole subtree (a *bucket*) rather than a single leaf book — see §5.

> If no routing graph is installed the fill books unrouted. A fill that never lands in an
> enabled risk book will never appear in the risk dashboard — historically the single most
> common cause of "my risk isn't showing up".

### 3.1 The precondition for everything that follows

Stages 2–6 run **only** when all three of these hold:

1. a hedge policy is primed,
2. the fill routed to a book, **and**
3. the booking path supplied both a priced reference **mid** and a **dealt** price.

Condition 3 is satisfied by the RFQ-desk accept and FIX-lift paths. It is *not* satisfied by
the manual `BookRatesPosition` path, which has no dealt-vs-mid to compute edge from — so **a
manually booked position carries no hedge decision at all**. It still contributes its full
exposure to the book net (and therefore to the next priced fill's decision), but it will never
itself trigger, or be recorded as, a hedge.

---

## 4. Stage 2 — how risk is measured

Two **different and deliberately non-interchangeable** measures are computed over the same
positions. Conflating them is the classic source of wrong hedge sizes, so the code keeps them
separate and so does this document.

### 4.1 `rates_linear_exposure` — the signed PV01 proxy (the risk measure)

This is what the warehouse cap, the RAG band, and hedge sizing all read.

| Instrument | Magnitude | Sign |
| --- | --- | --- |
| OIS | `notional × tenor_years × 1bp` | pay-fixed `+`, receive-fixed `−` |
| IRS | `notional × tenor_years × 1bp` | pay-fixed `+`, receive-fixed `−` |
| FRA | `notional × (end−start)/12 × 1bp` | pay-fixed `+`, receive-fixed `−` |
| Bond | `redemption × 1bp` | long (Buy) `−`, short `+` |

Signs are chosen so that **equal-and-opposite positions net to zero at a node**. A long bond
carries long-duration exposure, which is the same IR sign as a *receive*-fixed swap — hence the
apparently inverted bond row. This is correct and intentional.

The measure is deliberately **curve-free**: no discount factors, no bootstrapping. Discount
factors are `≤ 1`, so the undiscounted PV01 is an *upper bound* on the true annuity PV01 —
conservative, i.e. fail-safe, for a hard limit. See §10 for what this costs.

### 4.2 `rates_signed_notional` — the trade-direction face notional

Magnitude is the instrument's `notional` (or a bond's `redemption` face), signed by trade
direction: `SIDE_BUY` `+`, `SIDE_SELL` `−`. Note this signs by *trade direction* while
`rates_linear_exposure` signs by *duration direction*, so **a long bond is `+` here and `−`
there**. They are honestly different measures — notional versus PV01 — and a hedge rule's
`NetNotional` / `NetDelta` condition reads *this* one, so such a rule measures true face
notional and never the DV01 proxy.

### 4.3 Scope — book or subtree

The risk state a decision is measured against is either:

- `book_net_dv01(book)` — the fill's own book; or
- `subtree_net_dv01(root, descendants)` — the whole bucket roll-up, when a `Bucket`-scoped
  policy governs. This is the "portfolio" a bucket-level decision actually manages.

---

## 5. Stage 3 — the warehouse budget and the RAG bands

The budget is a `WarehouseThreshold` (`celnet-hedge-routing/src/band.rs`) resolved for the
fill by **most-specific-wins** precedence: `Instrument` → `Book` → `Desk`. If no threshold is
configured at any scope, a firm default applies, so **every booked fill carries a real cap and
a real band — never an empty `—`**.

### 5.0 The budget basis — net, gross, or DV01

A threshold carries a **metric**, and that metric selects which roll-up everything downstream
measures against — the band, the utilisation, the overflow, and the shed size:

| Metric | Measured against | Character |
| --- | --- | --- |
| `Dv01` (default) | signed net PV01 proxy | first-order rate risk |
| `NetNotional` / `NetDelta` | signed net face notional | direction-aware size |
| `GrossNotional` | `Σ\|notional\|` | **turnover brake — never nets down** |
| `NetVega` | `0` on a linear-rates cell | honestly never breaches (rates carry no vega) |

`GrossNotional` behaves fundamentally differently from the others and is worth choosing
deliberately: gross only *grows* with activity, so a gross cap stays breached until the
positions themselves roll off. It brakes churn; it does not measure exposure.

The metric is a single basis end-to-end — `book_risk` (the scope roll-up) and `fill_risk` (this
fill's contribution) are both taken in it. That matters because the offsetting leg is scaled by
`filled / fill_risk`: numerator and denominator must share a metric or the ratio is meaningless.
Both `rates_linear_exposure` and `rates_signed_notional` are linear in the instrument's
notional, so the *ratio* is identical whichever is chosen — only the consistency matters.

Firm default: metric `Dv01`, `amber = 0.8`, `red = 0.9`, `target_fraction = 0.8`,
`min_clip = 0`, `max_clip = ∞`, `ramped = false`.

### 5.1 The derived quantities

```
target       = target_fraction × cap        the band edge we hedge back to
utilization  = |net_risk| / cap             the RAG ratio
overflow     = max(0, |net_risk| − target)  how far past the edge we are

band         = Green   if utilization <  amber
               Amber   if utilization >= amber
               Red     if utilization >= red
               Breach  if utilization >  1.0

breached     = band ∈ {Red, Breach}         ← THE HEDGE TRIGGER
```

**The trigger fires at the red band, not at the cap.** `breached` is true from
`utilization ≥ red` (0.9 by default), so the desk begins shedding *before* the budget is
actually blown. This is the intended behaviour and a common point of confusion.

### 5.2 Sizing

When breached, the default hedge size is the **overflow** — hedge back to the band edge, not
to flat. This is the transaction-cost-optimal choice (Whalley–Wilmott 1997): flattening
completely pays spread on risk you are permitted and paid to hold.

The overflow is then optionally **ramped** (`fraction = clamp(ramp_k × (utilization − 1), 0, 1)`,
so the shed scales in as the breach deepens) and finally clamped to `[min_clip, max_clip]`.
Because `min_clip` is a *floor*, a breach whose overflow is below the minimum economic ticket
still hedges exactly `min_clip` rather than sending a trivially small order.

A policy leaf may override the size choice entirely:

| `HedgeSize` | Magnitude |
| --- | --- |
| `Overflow` (default) | hedge to the band edge |
| `Full` | `\|net_risk\|` — flatten |
| `Fixed(x)` | `\|x\|`, capped at `\|net_risk\|` (you cannot hedge more than you hold) |

### 5.3 Worked example

Book holds **net 5,000 DV01**. Threshold: `cap = 4,000`, `red = 0.9`, `target_fraction = 0.8`.

```
target      = 0.8 × 4,000 = 3,200
utilization = 5,000 / 4,000 = 1.25        → 1.25 > 1.0  ⇒ Breach
overflow    = 5,000 − 3,200 = 1,800       → shed 1,800, warehouse 3,200
```

---

## 6. Stage 4 — the exit policy decides *what kind* of exit

The band tells you *how much*. The **exit policy graph** tells you *what to do about it*. It is
the same decision-graph primitive as risk routing: `Condition` nodes walking to `Action` leaves,
authored as data in the GUI, validated as a total function.

### 6.1 Policy scope precedence

A fill's governing graph is the most specific one: its own **Book** policy → the nearest
ancestor **Bucket** policy → the **Firm** policy → failing all of those, a built-in default
graph (`breached == false ? Warehouse : SubmitMarketOrder{Overflow, Immediate}`).

### 6.2 What a condition can test (`HedgeField`)

| Group | Fields |
| --- | --- |
| Identity | `InstrumentId`, `Ccy`, `Product`, `Book`, `Desk`, `Counterparty` |
| Risk state | `NetDv01`, `NetNotional`, `NetVega`, `NetGamma`, `InventorySign` |
| Budget state | `Threshold`, `Utilization`, `Overflow`, `Breached` |
| Flow quality | `CounterpartyToxicity`, `InventoryAgeSecs` |
| Market state | `InternalOffsetAvailable`, `HedgeCostBp` |

`Counterparty` is a property of the **incoming flow**, not a per-counterparty net position: a
rule `counterparty == "X"` matches when *the fill that triggered this evaluation* came from X.
That is what lets a desk back-to-back one toxic client's flow while warehousing everyone else's.

### 6.3 The eight terminal actions (`ExitAction`)

Ordered by cost to the firm — the engine always prefers the cheap ones:

| Action | External? | What it does |
| --- | --- | --- |
| `Warehouse` | no | Hold the risk. The green-band leaf. Costs nothing. |
| `Skew` | no | Lean the two-way quote to **attract** the offsetting side — passive internalisation. No trade, just a quote lean. The amber-band lever. |
| `CrossInternal` | no | Net against opposing internal flow in the Agg Book at the consolidated mid. |
| `Escalate` | no | Fire an alert and hand to a human desk. Places no order. |
| `Split` | **yes** | Net internally up to the available offset, externalise the residual. The composite "internalise then hedge the overflow" primitive. |
| `SubmitMarketOrder` | **yes** | Back-to-back: one clip (`Immediate`) or Almgren–Chriss slices (`Worked`) onto the RFQ/FIX panel. |
| `RfqOut` | **yes** | Request two-way prices from named LPs and lift the best. |
| `ClearRisk` | **yes** | **Flatten the net to zero.** Where `SubmitMarketOrder{Overflow}` hedges only to the band edge, this sheds the *entire* position. The hard-breach / end-of-day-flatten leaf. |

`is_external()` is the flag that gates real street orders behind the desk's live-hedging arm —
and, as §7 shows, it also protects internal-mandate books from being force-shed.

---

## 7. Stage 5 — the internalise-vs-shed split

The graph has now produced a book-level intent. But there is a second, independent question:
**was this particular fill any good?**

### 7.1 The price-edge verdict

`internalise::verdict` computes the dealer edge in bp of the fill against fair mid, signed so
that positive means the desk made money (paying *below* mid on a pay-fixed swap is `+`). It
compares against the policy's `min_edge_bps` floor:

```
edge_bps         = dealer_edge_bps(desk_side, dealt, mid, kind)
within_tolerance = edge_bps >= max(min_edge_bps, 0)
```

### 7.2 The three-branch split

```rust
if verdict.within_tolerance {
    // Made money: warehouse it, shedding the BOOK's over-cap overflow (bounded by the
    // book's own risk, not by this fill — that is what lets a breach actually unwind).
    let ext = shed.min(book_risk.abs()).max(0.0);
    ((fill_risk - ext).max(0.0), ext, ext == 0.0)
} else if graph_is_external {
    // Lost money AND the policy is a shed policy: hand the whole fill back to the street.
    (0.0, fill_risk, false)
} else {
    // Lost money BUT the policy is an internal hold: HONOR IT. Warehouse the fill.
    (fill_risk, 0.0, true)
}
```

The third branch matters more than it looks. A **wash book** or any internal-only mandate
captures approximately zero edge on essentially every fill by construction. Without that
branch, a thin-edge fill would be force-shed to the street, emitting a back-to-back that
directly contradicts the book's own internal mandate. The policy's own resolved action is
consulted *before* the split precisely so this cannot happen.

A useful consequence the code then relies on: **`external_dv01 > 0` implies
`graph_is_external`** in both shedding branches, so anything externalised always carries the
graph's own action rather than a synthesised market-order stand-in.

---

## 8. Stage 6 — how risk actually exits

This is the stage that answers "how does an exit of risk happen", and it has **two distinct
halves that must both succeed**. Historically they were conflated, which is exactly how a bug
hid here for as long as it did.

### 8.1 Half one — execute on a venue

`execute_external` resolves the shed against a venue per the policy's `HedgeExecutionMode`:

| Mode | Behaviour |
| --- | --- |
| `Advisory` | Shadow-run. Computes and stamps the intent, **never trades**. |
| `LpPanel` | Best executable LP price for the instrument on the required side. No fill ⇒ an honest miss; no composite fallback. |
| `Composite` | Always fills against the Agg Book composite mid with the configured spread applied. |
| `LpPanelThenComposite` | LP panel first; on a miss, fall back to the composite. **The live default.** |

The result (`ExternalHedgeFill`) carries the realised economics: `filled`, `residual`,
`hedge_price`, `mid_at_fire`, signed `slippage_bp`, the winning `lp_won`, and the `venue`.

**The LP lookup is asked for a *security*, not a product family.** A rates cell's
`instrument_id` is a family label (`"BOND"`, `"OIS"`), which is not tradeable — asking a panel
for a security called "BOND" can only ever miss, silently backstopping every shed to the
synthetic composite and leaving the street-side league table with nothing to attribute. So
`HedgeContext.execution_instrument_id` carries the canonical security id when the cell resolves
one, and the executor prices the lookup off that. It is `None` — honestly absent, never
fabricated — for an OIS/IRS/FRA cell or a bond whose id never resolved against refdata, in
which case the executor falls back to the family label.

Which LPs are eligible is resolved from a standing **include/exclude panel** (`HedgeLpPanel`)
bound at desk / book / instrument scope; both lists empty means the unrestricted "all known
LPs" panel.

### 8.2 Half two — book the offsetting leg (**this is where risk leaves**)

A fill on the street does **not**, by itself, reduce your book. The book is an aggregation over
positions; until an offsetting position exists in it, the aggregation still returns the full
risk. The exit is only real when `offsetting_rates_leg` constructs a leg and
`book_into_risk_book` accepts it.

The leg is a copy of the fill with its **side flipped** and its notional scaled by

```
factor = filled_dv01 / fill_dv01        ∈ (0, 1]
```

so its signed exposure is exactly the negation of the shed portion.

| Arm | Construction |
| --- | --- |
| OIS / IRS / FRA | `notional = \|notional\| × factor`, side flipped |
| **Bond** | `redemption = \|redemption\| × factor`, side flipped — **the same security sold back** |

The bond case deserves its own sentence, because it was long believed to require a duration
model. It does not. The offsetting leg is the *identical security* — same coupon, maturity,
day-count, security id — so **the DV01 ratio between leg and fill is identically 1**. Selling
`factor` of the face you are long sheds exactly `factor` of the risk under *any* duration
measure. No curve, no duration input, and it stays exact both under today's coarse proxy and
under the exact curve DV01 that will replace it. A DV01 ratio would only be needed to hedge a
bond with a *different* instrument.

`book_into_risk_book` re-runs the full pre-trade and per-book hard-cap gates (a *reducing* leg
never breaches) and assigns a fresh `position_id`. It does not recurse into `stamp_internalise`,
so booking a hedge cannot trigger a hedge.

> **Failure mode, now instrumented.** If the street fill succeeds but the offsetting leg is
> *rejected*, that is an operational break, not a no-op: the trade really happened, the book did
> not reduce, and the next fill would compound risk the blotter already reports as hedged. The
> `Result` is no longer discarded — the divergence is rung at `ERROR`.

---

## 9. Stage 7 — provenance, analytics, and trace

**Exactly one ring record per fired hedge.** The engine rings a *decision* record when it
resolves a non-hold action; when that shed then executes, the realised record **supersedes the
decision record in place**, matched on its minted `hedge_id`, rather than appending a second
row. Otherwise a single fired hedge would double-count in the blotter. (Defensively, if no
decision record was rung, a fresh realised record is appended so a fill is never lost.)

`HedgeProvenance` carries the full "why and what happened": the book, instrument, fire time,
the metric and threshold that tripped, signed net risk, utilisation, RAG band, **the exact
graph path walked** (`policy_path` — the machine-checkable "why this action"), the action, the
internal/external/residual decomposition, realised price, mid at fire, signed slippage, winning
LP, the targeted LP set, and `parent_position_id` — the reconciliation key back to the
originating `Deal`, which carries the same id.

`advisory` is `true` exactly when nothing filled. **A real venue fill is never marked
advisory, and a miss is never marked as a fill** (guardrail 2).

**Street-side attribution.** A hedge that fills on a *named* LP records a won deal and won
notional for that LP into the shared flow log the LP league table folds, keyed on the security
actually dealt. A composite fill is deliberately **not** attributed to a named LP — `COMPOSITE`
is a pseudo-venue, not a street counterparty. No last-look or cover is honestly known for a
hedge fill, so those stay absent rather than being invented.

**Trace.** `HEDGE_DECIDED` is emitted for every fill (the internalise-vs-shed verdict and RAG
band); `HEDGE_FIRED` is emitted additionally when risk actually sheds externally — the terminal
stage of a hedged lift.

---

## 10. Honest boundaries

Read this section before relying on the numbers.

1. **The bond exposure proxy is not duration-scaled.** `rates_linear_exposure` computes
   `notional × tenor_years × 1bp` for swaps but `redemption × 1bp` for bonds — i.e. it treats
   every bond as though it had a duration of 1. A 10-year bond's true DV01 is roughly 8× what
   the proxy reports. **Consequence: bond and swap risk are not commensurable in the same book
   net today**, and a bond-heavy book's utilisation is understated. The exact
   curve-bootstrapped DV01 is the ADR-0016 **A3** breadth wave. Note this does *not* affect
   hedge-leg correctness (§8.2): the same-security ratio is 1 under any measure.

2. **The PV01 proxy is curve-free and undiscounted.** It is an upper bound on the true annuity
   PV01 — conservative for a hard limit, but not a mark.

3. **`Desk`-scoped warehouse thresholds do not bind on the rates booking path.** The resolver
   supports `Instrument → Book → Desk` precedence, but the rates call site passes an empty desk
   id, so only instrument- and book-scoped thresholds (and the firm default) actually resolve.

4. **`ExecStyle::Worked` does not yet slice.** The Almgren–Chriss schedule is modelled in the
   action but a worked order currently executes as a single clip.

5. **The composite venue always fills.** It is a synthetic backstop off the Agg Book mid, not a
   real counterparty. A book whose hedges all show `venue=Composite` / `lp_won=COMPOSITE` is
   telling you the LP panel never matched — usually a security-id mismatch between the book's
   instruments and what the LPs actually quote.

6. **Hedging evaluates per fill, not on a timer.** The decision runs when a fill books. Risk
   that drifts past a band through *market* movement alone, with no new flow, is not
   re-evaluated until the next fill lands. A breach now unwinds the book's own overflow when a
   fill does arrive (it used to be able to neutralise only the fill itself), so a quiet book
   still needs one fill — or a manual `ClearRisk` — to shed.

---

## 11. Where the code lives

| Concern | Location |
| --- | --- |
| Booking sink, split, execution, offsetting leg | `crates/celnet-server/src/services/rates_book.rs` (`stamp_internalise`, `offsetting_rates_leg`, `book_into_risk_book`) |
| Exposure measures | same file — `rates_linear_exposure`, `rates_signed_notional`, `rates_dealt_level` |
| Warehouse bands and sizing | `crates/celnet-hedge-routing/src/band.rs` |
| Exit policy graph and actions | `crates/celnet-hedge-routing/src/graph.rs` |
| Condition fields / risk state | `crates/celnet-hedge-routing/src/{field,context}.rs` |
| LP include/exclude panel | `crates/celnet-hedge-routing/src/lp_panel.rs` |
| Venue execution | `crates/celnet-server/src/services/auto_hedge/executor.rs` |
| Decision engine + provenance ring | `crates/celnet-server/src/services/auto_hedge/engine.rs` |
| Price-edge verdict | `crates/celnet-server/src/services/internalise.rs` |
| Risk routing into books | `crates/celnet-risk-routing/` |
| RAG classification | `crates/celnet-limits/src/limit.rs` |
| Firm defaults | `crates/celnet-server/src/config/identity.rs` (`default_hedge_policy_graph`, `default_warehouse_threshold_def`) |

---

## 12. The invariants worth defending

These are the properties the test suite pins. If you change this subsystem, keep them true.

1. **A filled shed reduces the book by exactly the hedged amount.** Whatever provenance reports
   as `external_hedged` must have left the book. Violating this is unbounded position growth
   that the blotter reports as hedged.
2. **A hedge is never fabricated.** A miss stamps an honest advisory record; it never stamps a
   fill. A composite fill is never attributed to a named LP. An unresolved security id is
   absent, never invented.
3. **Exactly one ring record per fired hedge** — realised supersedes decision in place.
4. **A shed never exceeds the book's own risk** (`ext = shed.min(book_risk.abs())`). It is
   NOT bounded by the incoming fill: a breached book sheds its own overflow, which is what
   lets auto-hedging unwind a standing position rather than merely neutralise new flow.
   `max_clip` is the control for bounding a single ticket.
5. **An internal-mandate policy is never force-shed** by a thin-edge fill.
6. **Every booked fill carries a real threshold, band, and decision** — never an empty `—`.
7. **The two exposure measures stay distinct.** Notional rules read notional; the DV01 budget
   reads the PV01 proxy. Never cross them.
