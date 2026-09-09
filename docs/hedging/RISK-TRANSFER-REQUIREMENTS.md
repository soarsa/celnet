# Risk Transfer — Requirements / Design

> Status: **requirements + design for review** (2026-07-30). No code yet. Grounds a NEW
> Celnet capability on the existing booking/position/routing seams (file:line cited below,
> current as of this doc). Follows the shipped **risk-routing** and **pricing-groups**
> patterns (registry + resolver + proto/WS CRUD + provenance waterfall + visual surface) as
> the template. Vendor-neutral naming throughout (guardrail 8). OSS-only methodology
> (guardrail 7). Off the pinned zero-alloc pricing hot core (guardrail 11). No mocks /
> placeholders (guardrail 2).

## 1. Concept & goal — the complement to routing

**Risk routing** already ships and is live: at **book time** a fill's `RoutingContext`
walks a decision graph, `RiskRouter::route` resolves a **risk portfolio** (internally
`RiskBookDef`; user-facing "Risk Portfolio"), and the position store **stamps** the position
with that portfolio. Routing is the **AUTOMATIC** assignment of **NEW** fills.

**Risk transfer is the MANUAL move of EXISTING risk** — the exact complement. It is the
already-flagged follow-up in the routing doc: *"Re-routing existing positions: rules apply
to NEW fills. A manual 'move position to book' action … is a follow-up"*
(`docs/FI-RISK-ROUTING-REQUIREMENTS.md` §9). This doc specifies that action, at full depth.

A trader (or risk manager) selects one or more **existing positions** and moves the risk
(all or part) to:
- **(a)** a different **risk portfolio** *within the same desk* — a light **re-attribution**
  (an accounting/organisational move, same economics), OR
- **(b)** another **desk / desk-owned portfolio** — an **economic internal trade** (an
  internal cross at a transfer price that moves P&L and risk between two org units), OR
- **(c)** another **trader** — a **hand-off** that the recipient must **accept**.

Three deliverables, mounted under the **Fixed Income** GUI tab (beside the routing/dashboard
surfaces):
1. **Transfer ticket** — select position(s), choose target, quantity, transfer price → submit.
2. **Transfer inbox** — for desk-to-desk / trader-to-trader, the counterparty accepts/rejects.
3. **Transfer audit trail** — an immutable, queryable record of every transfer, surfaced on
   the blotter and the risk dashboard (mirrors the `PricingProvenance` waterfall discipline).

### The core economic distinction (locks the taxonomy — §4)
Industry practice separates two fundamentally different operations, and Celnet must too:
- **Pure re-attribution** — the position is re-labelled to a different book; **economics
  unchanged**, no P&L crosses, no new trade. This is a **re-stamp** of the routing dimension.
- **Economic internal trade** — an **internal cross** at a **transfer price**: two
  offsetting bookings (out of the source book, into the target book) that **realise P&L in
  the source** and **open risk in the target** at the agreed mark. This is a **book pair**,
  not a re-label. Getting the transfer price right (arm's-length / mark) is what protects
  P&L integrity and satisfies control/regulatory expectations (§3).

## 2. Architecture fit — the real seams (build ON these, don't reinvent)

Every seam below already exists and is cited. Risk transfer **reuses the booking sinks and
the routing dimension**; it does not add a parallel position store.

- **The routing dimension a re-attribution re-stamps.** The FX `PositionStore` keeps a
  per-position `risk_book` map (handle → book id), populated at book time by
  `RiskRouter::route` and read back via:
  - `PositionStore::book` — `crates/celnet-server/src/services/risk/store.rs:757`
    (resolves routing, runs the per-book hard-limit gate `project_risk_book_breach`
    (`store.rs:1274`), stamps `g.risk_book.insert(handle, book)`, then
    `self.risk_version.fetch_add(1, Ordering::Relaxed)` — the version the live per-book
    risk stream polls).
  - `PositionStore::risk_book_of` (`store.rs:508`), `positions_in_risk_book` (`store.rs:519`),
    `risk_book_tree` / `set_risk_book_tree` (`store.rs:474` / `:463`) — the read/write
    surface a **re-attribution** flips.
  - Ancestor roll-up for limits: `risk_book_chain` (`store.rs:1348`) +
    `project_risk_book_breach` (`store.rs:1274`) — a transfer INTO a book re-checks the
    **target's** (and its ancestors') hard caps, exactly like a routed fill.
- **The FX booking sink (where an economic transfer books the two legs).**
  - `PositionStore::book_from_attribution(booked, attribution)` — `store.rs:690` — the
    high-level entry every ESP/RFQ fill converges on; canonicalises a `BookedPosition` and
    calls `PositionStore::book`. An economic transfer books an **offsetting** `BookedPosition`
    into the source and a **new** `BookedPosition` into the target via this same sink — no
    new booking primitive.
- **The rates / bond booking sink.**
  - `RiskEdge::book_rates_position` — `crates/celnet-server/src/services/risk/mod.rs:1019` —
    already **capability-gated** on `RequiredAuthority::Capability(Action::Book,
    AssetClass::FixedIncome)`; delegates to `book_rates_position_impl` (`mod.rs:674`) →
    `self.rates.book(position)` on the `rates_book` monotonic ledger
    (`crates/celnet-server/src/services/rates_book.rs`; `RatesPosition { position_id, entity,
    book, instrument }`, cf. `bond_position` `rates_book.rs:1108`). A rates transfer books
    offsetting `RatesPosition`s through this ledger.
- **The desk RFQ / internal-cross precedent.** `RfqDeskEdge::accept_desk_quote`
  (`crates/celnet-server/src/services/desk/mod.rs:794`) → `book_family_and_return_position`
  (`desk/mod.rs:1244`) already books a **deal + position** from a desk interaction — the
  shape a desk-to-desk transfer's **acceptance** reuses (an inbox item the target desk lifts).
- **The portfolio/desk registries a transfer targets.** `RiskBookDef`
  (`crates/celnet-server/src/config/identity.rs:768` — `id`, `name`, `parent_id`, `desk_id`,
  `limits`, `enabled`) and `DeskDef` (`identity.rs:291`), with CRUD + tree ops
  (`create/update/delete_risk_book`, `risk_book_ancestors`/`_descendants`, `check_risk_book`,
  `mint_risk_book_id`) already in `IdentityStore`. Only **enabled** books are valid targets
  (same rule `check_risk_routing_graph` enforces).
- **The capability model that gates who can transfer.** `celnet-entitlements` `Action` enum
  (`crates/celnet-entitlements/src/capability.rs:47` — `View, Price, QuoteRespond,
  RfqRespond, IoiRespond, Stream, Execute, Book, Simulate, Administer`) × `AssetClass`
  (`capability.rs:119`). `Action::Book` is *"the booking-committing action"*; a transfer is
  a booking-class write, so it gates on `Book` (plus a new sub-gate — §7).
- **The provenance/audit template.** `PricingProvenance`
  (`crates/celnet-proto/proto/celnet.proto:4426`) — the RAW→constructed→tiered→outbound
  waterfall stamped on Quote→Execution→Deal. A `RiskTransferProvenance` mirrors this
  discipline (immutable, structured, carried on the record).
- **The dashboard that reflects the move.** `aggregate_risk_book`
  (`crates/celnet-server/src/services/risk/book_risk.rs`) rolls positions up per portfolio;
  after a transfer bumps `risk_version`, the per-book snapshot re-aggregates on the next tick
  (no timer thread — lazy, guardrail 11).

**Net:** a re-attribution = a `risk_book` re-stamp + version bump + limit re-check on the
target. An economic transfer = two `book_from_attribution` / `book_rates_position` bookings
(net-flat source, open target) at a transfer price, through the **existing** sinks. Nothing
touches the pinned pricing thread.

## 3. External research — how banks transfer risk internally (cited)

### 3.1 Two operations: re-attribution vs economic internal trade
Position transfer is defined as *"the movement of an open financial commitment … from one
administrative environment to another **without liquidating the underlying asset**"*,
preserving *"continuity of interest"* rather than realising a market gain/loss like an
outright trade ([marketclutch — Mechanics of Position
Transfer](https://marketclutch.com/the-mechanics-of-position-transfer-a-comprehensive-framework/)).
At the institutional level a **Transfer of Trade (T.O.T.)** occurs *"when two different legal
entities or desks under the same umbrella move risk between them"* — routine in prime
brokerage (ibid). Celnet's **(a)** re-attribution is the lightweight administrative move; its
**(b)** desk-to-desk is the T.O.T. that must carry economics.

### 3.2 Mechanics: mirror / back-to-back, novation, give-up
- **Mirror / back-to-back bookings.** An internal transfer is executed as **two offsetting
  bookings** — a sell out of the source book matched by a buy into the target — so the
  source flattens and the target opens; the pair nets to zero at the firm level while moving
  risk and P&L between books ([nikstehr — Risk System Concepts: Trade Booking &
  Pricing](https://nikstehr.medium.com/risk-system-concepts-3f259804d87c)). Product-control
  practice is explicit that improper matching of the offsetting legs *"can create a break in
  P/L"* — the two legs must be booked as a linked pair
  ([nikstehr](https://nikstehr.medium.com/risk-system-concepts-3f259804d87c)).
- **Novation.** *"One party in a multi-party contract is replaced by another"* while the
  originating party stays market-neutral
  ([marketclutch](https://marketclutch.com/the-mechanics-of-position-transfer-a-comprehensive-framework/)).
  In FX, ISDA's novation framework works via **equal and offsetting pass-through trades that
  get terminated**, with the remaining party continuing to maintain its books and records
  ([ISDA — Overview of the FX Novation and Cancellation
  Protocol](https://www.isda.org/a/KfTDE/overview-of-fx-novation-and-cancellation-protocol.pdf)).
  For an **internal** transfer the "third party" is another internal desk — the same
  offsetting-and-terminate mechanic, kept inside the firm.
- **Give-up / allocation.** A trade executed by one party is "given up" to another for
  clearing/booking — the allocation of an already-done trade to a different book/account,
  the desk-to-desk analogue of allocating a block fill to sub-accounts (ibid, "Institutional
  Novation" / allocation).

### 3.3 Transfer pricing — at what price, and why it matters
The transfer price is the internal cross level. Practice draws on two bodies of doctrine:
- **Arm's-length principle.** Intra-group transactions should be priced *"as if they
  occurred between independent, unrelated parties under comparable circumstances"*
  ([OECD / TPguidelines — Arm's Length
  Principle](https://tpguidelines.com/category/transfer_pricing_case_laws/arms_length_principle/);
  [Archipel — Transfer Pricing
  Guide](https://www.archipeltaxadvice.nl/insights/transfer-pricing-guide/)). The transfer
  price *"creates revenues for the selling division and purchase costs for the buying
  division, affecting each group's operating income"* — i.e. it determines **how P&L splits
  across the transfer** (ibid). Choosing an off-market internal price would shift P&L between
  desks improperly, which is exactly what controls guard against.
- **Mark-to-market reference.** Product control reviews mark-to-market P&L against
  independent marks; an internal cross at **mid / current mark** (rather than an off-market
  level) keeps each book's realised/unrealised split defensible
  ([O'Reilly — *Effective Product Control*, Ch.10 "Review of Mark-to-Market
  P&L"](https://www.oreilly.com/library/view/effective-product-control/9781118939819/c10.xhtml)).
- **Funds Transfer Pricing (FTP) analogue.** Banks run a central "internal bank" that prices
  internal transfers off a maintained curve; the point is a **single, governed reference**
  everyone transacts against, not ad-hoc bilateral levels
  ([CostPerform — Funds Transfer
  Pricing](https://www.costperform.com/what-is-funds-transfer-pricing-a-complete-guide-for-banks/)).

**Design consequence:** Celnet offers the transfer price as **mid / mark-to-market (default,
auto-filled from the live composite)** or an **agreed override** (with reason), and records
which was used — never a silent free number.

### 3.4 P&L, risk, inventory implications
- **P&L attribution.** The source book **realises** P&L to the transfer price at the moment
  of transfer; the target opens at that price and carries it forward. A mis-set transfer
  price mis-attributes P&L between the two desks (§3.3).
- **Risk (DV01 / greeks / notional).** The full risk vector (Δ/Γ/Vega/Θ for FX-vanilla, DV01
  and per-tenor buckets for rates) moves from source to target — the target's limits and
  roll-up must absorb it (Celnet re-checks target caps, §6).
- **Inventory / axe.** Moving inventory changes each desk's axe/position skew inputs; the
  position-feature stages (`POSITION`/`AXE`) at each desk see the new book state on the next
  aggregation tick.

### 3.5 Regulatory / control context (conceptual, cited)
- **Internal risk transfer is a recognised, regulated construct.** A banking-book GIRR
  exposure hedged into the trading book must go through a **dedicated, approved internal
  risk-transfer desk** and be subject to **trading-book capital on a stand-alone basis**
  ([QFCRA BANK 6.1.16 — Capital effect of internal risk
  transfer](https://qfcra-en.thomsonreuters.com/entiresection/14678)). Banking-book
  positions can only move into the trading book if the risk is offset via **separate matched
  external hedges** — internal transfers alone don't relieve capital
  ([SIFMA — FRTB Introductory
  Guide](https://www.sifma.org/news/blog/the-fundamental-review-of-the-trading-book-frtb-an-introductory-guide);
  [Risk.net — FRTB: banks fearful of risk-transfer
  missteps](https://www.risk.net/risk-management/2476693/frtb-banks-fearful-of-risk-transfer-missteps)).
- **Desk boundaries make "which desk owns this risk" a regulated fact.** FRTB defines a
  trading desk as *"a group of traders … that implements a well-defined business strategy
  operating within a clear risk-management structure,"* subject to supervisor approval; the
  Volcker rule defines a trading desk as *"the smallest discrete unit of organization … that
  purchases or sells financial instruments for the trading account"*
  ([Accenture — Trading Desk Definitions under FRTB and
  Volcker](https://financeandriskblog.accenture.com/regulatory-insights/regulatory-compliance/trading-desk-definitions-under-frtb-and-volcker);
  [Risk.net — Final Volcker rule spurs rethink on FRTB trading
  desks](https://www.risk.net/regulation/7196436/final-volcker-rule-spurs-rethink-on-frtb-trading-desks)).
  Because inter-desk transfers change which regulated desk owns a position, they must be
  **attributable, priced, and auditable** — which drives §7.
- **Four-eyes / maker-checker + immutable audit.** An inter-desk transfer is a
  P&L-moving action, so it warrants dual control: *"one person prepares or requests an action
  and another independent person reviews and approves it before the action is completed"*
  ([chequedb — Four-Eyes
  Principle](https://chequedb.com/resources/blog/four-eyes-principle-foundations);
  [opcito — Maker-Checker
  Implementation](https://www.opcito.com/blogs/maker-checker-implementation-guide-for-secure-fintech-systems)).
  The approval is only as good as its trail: audit trails must be **immutable, timestamped
  from a trusted source, attributable to a named authenticated user, tied to the specific
  state approved, and produced from a log the actors cannot edit**
  ([auditingauthority — Audit Trail Requirements in Financial
  Services](https://auditingauthority.com/audit-trail-requirements-financial-services/);
  [Stampli — Immutable Audit
  Trail](https://www.stampli.com/resources/immutable-audit-trail/)). *If logs can be altered
  by the same personnel who execute transactions, the control environment is materially
  deficient* (ibid).

## 4. Transfer taxonomy

| Type | Scope | Economics | Mechanism | Approval |
|---|---|---|---|---|
| **(a) Re-attribute** | portfolio → portfolio, **same desk** | **unchanged** (no P&L crosses) | **re-stamp** `risk_book` + version bump; re-check target caps | initiator's `Book` capability; no counterparty (same owner) |
| **(b) Desk-to-desk** | portfolio/desk → **other desk's** portfolio | **economic** — P&L realised in source, opened in target at transfer price | **two offsetting bookings** through the existing sinks + transfer price + provenance | initiator + **target desk acceptance** (four-eyes across the org boundary) |
| **(c) Trader-to-trader** | trader → trader (may be same or different book) | either — light hand-off, or economic if books differ | re-attribution or booking pair per whether the target book differs | initiator + **recipient accept/reject** |

- **(a)** is the fast path — it is literally the routing re-stamp applied on demand instead
  of at fill time. Same-desk, so no P&L integrity question and no acceptance (you own both
  ends).
- **(b)** is the heavyweight economic cross — the T.O.T. of §3.1, priced per §3.3, dual-
  controlled per §3.5. This is where the offsetting-booking mechanic and transfer price live.
- **(c)** is a hand-off with **acceptance semantics** (mirrors the desk RFQ inbox at
  `accept_desk_quote`): the recipient must lift it, or it expires/rejects — never a silent
  push onto someone else's book.

## 5. The transfer object + lifecycle

### 5.1 `RiskTransfer` (the request/record)
```
RiskTransfer {
  id: String,                    // stable slug (audit key)
  kind: ReAttribute | DeskToDesk | TraderToTrader,
  source: TransferLeg,           // { risk_book_id, desk_id, trader, position_ids: Vec<u64> }
  target: TransferLeg,           // { risk_book_id, desk_id?, trader? }
  quantity: Full | Partial(f64), // notional_base to move (Full = the whole position(s))
  price: TransferPrice,          // Mid | MarkToMarket | Agreed(f64)  (§3.3)
  reason: String,                // free-text rationale (required for Agreed price)
  initiated_by: String,          // authenticated user
  initiated_at: i64,             // trusted-source timestamp
  state: Draft | Pending | Accepted | Rejected | Booked | Cancelled,
  approver: Option<String>,      // the accepting/approving user (four-eyes)
  decided_at: Option<i64>,
  provenance: Option<RiskTransferProvenance>,  // stamped on Booked (§5.3)
}
```
Invariants (validated server-side, mirror `check_risk_book` / `check_pricing_group`):
- every `position_id` exists and is currently in `source.risk_book_id`;
- `source` and `target` books are **enabled** and resolve to real desks;
- `Partial(q)` has `0 < q <= |remaining notional|`;
- `Agreed` price requires a non-empty `reason`;
- `ReAttribute` ⇒ `source.desk_id == target.desk_id` (else it is really a DeskToDesk);
- `DeskToDesk` / `TraderToTrader` start `Pending` (need acceptance); `ReAttribute` may go
  straight to `Booked` (same owner, no counterparty).

### 5.2 Lifecycle
```
initiate ──▶ [Pending] ──accept──▶ book ──▶ [Booked] ──▶ audit
    │              │
    │              └──reject / expire──▶ [Rejected]
    └── ReAttribute (same desk) ─────────▶ book ──▶ [Booked] ──▶ audit   (no Pending)
    └── cancel (initiator, pre-accept) ──▶ [Cancelled]
```
- **initiate** — validate (§5.1); resolve the default transfer price (mid/mark from the live
  composite) unless `Agreed`.
- **accept** (b/c only) — target desk/trader with the right capability confirms; four-eyes
  satisfied by a **different authenticated principal** than the initiator (server-enforced).
- **book** — apply the move atomically (§6); stamp provenance; bump `risk_version`.
- **audit** — append the immutable record; surface on blotter + dashboard.

### 5.3 `RiskTransferProvenance` (immutable audit record — mirrors `PricingProvenance`)
Stamped on the `Booked` transfer and carried on both booking legs / the deal record:
```
RiskTransferProvenance {
  transfer_id, kind, initiated_by, initiated_at, approver, decided_at,
  source_book_id, target_book_id, position_ids, quantity,
  transfer_price, price_basis (Mid|MarkToMarket|Agreed), reason,
  realized_pnl_source,          // P&L crystallised in the source at the transfer price
  risk_moved { delta, gamma, vega, theta, dv01, buckets… },  // the moved risk vector
}
```
Same discipline as the pricing waterfall (`PricingProvenance`,
`crates/celnet-proto/proto/celnet.proto:4426`): structured, additive, carried on the record —
so the blotter and dashboard can show **exactly what moved, at what price, by whom, approved
by whom**.

## 6. Server model — re-stamp vs offsetting bookings

A single `celnet-risk-transfer` module owns the operation; it calls the **existing** sinks.

### 6.1 Re-attribution (kind = ReAttribute) — the light path
Pure re-stamp of the routing dimension, no new trade:
1. For each `position_id`: verify it is in `source.risk_book_id` (`risk_book_of`,
   `store.rs:508`).
2. Run the **target** book's hard-limit gate over a read snapshot — `project_risk_book_breach`
   (`store.rs:1274`) walking `risk_book_chain` (`store.rs:1348`) so ancestor caps compose;
   reject BEFORE mutation if the target (or an ancestor) would breach (identical discipline
   to the routed-fill gate in `PositionStore::book`, `store.rs:757`).
3. Under the write lock, re-point `g.risk_book.insert(handle, target_book_id)` for each
   handle (the same map `book` stamps), then `risk_version.fetch_add(1, …)`.
4. **Partial:** split the fact — reduce the source position's `notional_base` and insert a
   new position handle in the target book for the moved quantity (canonicalised through the
   same `canonical_vanilla_fact` path `book` uses). No economics change: same instrument,
   same marks, just two smaller line items.

Economics are untouched (same instrument/inputs/marks) — this is the administrative move of
§3.1, and the analogue of re-running routing on an open position.

### 6.2 Economic transfer (kind = DeskToDesk / TraderToTrader-with-different-book)
Two linked offsetting bookings through the existing sinks (§3.2 mirror mechanic):
1. Resolve the **transfer price** (mid/mark from the live composite, or `Agreed`).
2. **Source leg** — book an **offsetting** `BookedPosition` (opposite side, `quantity`
   notional) into the source book via `book_from_attribution` (`store.rs:690`) so the source
   **net-flattens** the moved quantity and **realises P&L** to the transfer price.
3. **Target leg** — book a **new** `BookedPosition` (same side as original, `quantity`
   notional) into the target book via the same sink, **opening** at the transfer price. The
   target's hard-limit gate (`project_risk_book_breach`) runs on this leg exactly as for any
   fill — a transfer that would blow the target's cap is **refused** before either leg
   commits (both legs are staged, then applied atomically; a refused target leg rolls back
   the source leg — never a half-transfer).
4. **Rates / bonds** — the identical pattern over `RiskEdge::book_rates_position`
   (`mod.rs:1019`) / the `rates_book` ledger (`rates_book.rs`): offsetting `RatesPosition` in
   the source book, opening `RatesPosition` in the target book.
5. Stamp `RiskTransferProvenance` (§5.3) on both legs / the deal; bump `risk_version` once.

Both legs reuse the **exact** booking primitives fills use — no new position store, no
bypass of limits, no bypass of the consensus/`Strong`-book path in `PositionStore::book`
(`store.rs:757`) for books configured for quorum-replicated durability.

### 6.3 Off the hot core (guardrail 11)
The whole operation runs on the **async booking tier**, never the pinned pricing thread —
same tier `book_from_attribution` and `book_rates_position` already run on. The dashboard
reflects the move **lazily** via the `risk_version` bump + on-tick re-aggregation
(`aggregate_risk_book`, `book_risk.rs`); no timer thread, no allocation on the pricing path.

## 7. Permissions & controls

- **Capability gate.** A transfer is a booking-class write, so it gates on the existing
  `Action::Book` (`crates/celnet-entitlements/src/capability.rs:47`) × the position's
  `AssetClass` — the same authority `book_rates_position` already requires
  (`RequiredAuthority::Capability(Action::Book, AssetClass::FixedIncome)`, `mod.rs:1019`). A
  caller who may `Execute`/`RfqRespond` yet lacks `Book` cannot transfer. We add a narrow
  **`risk_transfer` sub-capability** (or a `Book`-with-transfer flag) so a desk can grant
  booking without granting cross-desk transfer — decided at review (§12).
- **Four-eyes across the org boundary (b/c).** A `DeskToDesk` / `TraderToTrader` transfer is
  `Pending` until a **different authenticated principal** on the target side accepts —
  server-enforced (initiator ≠ approver), the maker-checker pattern of §3.5. `ReAttribute`
  within one desk is single-control (same owner both ends).
- **Immutable audit trail.** Every transfer appends a `RiskTransferProvenance` record
  (§5.3): system-generated, trusted-source timestamp, attributable to the authenticated
  initiator + approver, tied to the exact positions/price/quantity, in an **append-only** log
  the actors cannot edit (§3.5). Surfaced on the blotter and dashboard; queryable.
- **Transfer-price integrity.** Default is mid / mark-to-market auto-filled from the live
  composite; an `Agreed` override **requires a reason** and is flagged in the audit record —
  so off-mark internal crosses are visible to control (§3.3).
- **Limits are never bypassed.** The target book's hard caps + ancestor roll-up are
  re-checked on the incoming leg (§6), identical to a routed fill.

## 8. Risk dashboard reflection

No new aggregation engine — the transfer moves positions between the books
`aggregate_risk_book` (`book_risk.rs`) already rolls up:
- source book's net position / notional / greeks / DV01 / PnL **drops** by the moved risk;
- target book's **rises**;
- the `risk_version` bump makes the per-book `RiskBookSnapshot` stream re-aggregate on the
  next tick (lazy, §6.3).
The dashboard's drill-into-contributing-fills view shows the transfer provenance (the
"why it moved here" beside routing's "why it landed here").

## 9. GUI (Fixed Income tab)

New rail entries (`gui/src/lib/commands.ts`, `assets:["fixed_income"]`), gated on the
transfer capability for initiate/accept, view for any FI trader. Slots beside the existing
`RiskDashboardWorkspace` / routing surfaces.

### 9.1 Transfer ticket — `RiskTransferWorkspace.tsx`
- **Select positions** from a portfolio (or the Book / blotter) — multi-select, with the
  live risk each line carries.
- **Choose target** — a typed dropdown of the **book tree** (`RiskBookDef` tree) for a
  portfolio, or a **desk** (`DeskDef`) / **trader** picker; the UI infers `kind` from the
  selection (same desk ⇒ re-attribute; other desk ⇒ desk-to-desk; a trader ⇒ hand-off).
- **Quantity** — full or partial (a notional slider/input bounded by the line).
- **Transfer price** — **mid / mark auto-filled** from the live composite (read-only unless
  overridden); `Agreed` override reveals a required **reason** field.
- **Preview** — a before/after strip: source book risk ↓, target book risk ↑, realised P&L in
  source at the shown price (the economic legs, computed client-side for display, server-
  authoritative on submit). Compositor-friendly motion only.
- **Submit** → `InitiateRiskTransfer`. Re-attribution books immediately; desk/trader
  transfers land in the counterparty's inbox as `Pending`.

### 9.2 Transfer inbox — `RiskTransferInboxWorkspace.tsx`
Mirrors the desk RFQ inbox (`accept_desk_quote` shape): incoming `Pending` transfers with
source/target/quantity/price/reason and the moved-risk preview; **Accept** (books the legs)
/ **Reject** (with reason) / auto-expire. Server enforces approver ≠ initiator.

### 9.3 Transfer audit trail
A blotter view of `RiskTransferProvenance` records (who/when/what/price/approver), filterable
by book/desk/trader/date; each row links to the moved positions and the resulting deal legs.

## 10. Proto / WS / CRUD (mirror pricing-groups / risk-routing)

New proto messages + `AuthService`/`RiskService` RPCs (descriptor-driven codec auto-adapts;
add hand + generated codec entries per the pricing-groups / risk-routing precedent in
`crates/celnet-proto/proto/celnet.proto`, `ws/codec.rs`, `ws/generated_codec.rs`):
- `InitiateRiskTransfer` (returns the `RiskTransfer` in `Pending`/`Booked`).
- `AcceptRiskTransfer` / `RejectRiskTransfer` (target side; four-eyes enforced).
- `CancelRiskTransfer` (initiator, pre-accept).
- `ListRiskTransfers` (blotter/audit query; by book/desk/trader/state).
- `RiskTransferProvenance` carried on the `RiskTransfer` and on the booked deal legs.
- `RiskTransferInbox` stream message (per-desk/trader pending items) — beside
  `RiskBookSnapshot`.
All capability-gated (`Action::Book` × asset, + the transfer sub-gate). No versioned APIs
(guardrail 9): one current contract.

## 11. Crate / workstream breakdown (parallel-safe, disjoint files)

1. **`celnet-risk-transfer`** (NEW leaf crate): `RiskTransfer`, `TransferLeg`,
   `TransferPrice`, `RiskTransferProvenance`, the validation (`check_transfer` — position
   membership, enabled books, partial bounds, agreed-reason, kind↔desk consistency), and the
   pure "compute the two offsetting legs at a transfer price" function. No server deps →
   independent oracle = a hand-written truth table of (positions, price, quantity) → (source
   leg, target leg, realised P&L).
2. **`services/risk/` + `services/rates_book.rs`**: the apply path — re-stamp (§6.1) and the
   staged offsetting-booking pair (§6.2) through `book_from_attribution` /
   `book_rates_position`; atomic two-leg apply with rollback; `risk_version` bump.
3. **`config/identity.rs`**: transfer-target validation reusing `check_risk_book` /
   `risk_book_ancestors`.
4. **`celnet-entitlements`**: the `risk_transfer` sub-capability (or `Book` transfer flag).
5. **proto + `ws/` codecs + `RiskService`/`AuthService`**: the §10 messages/RPCs + inbox
   stream.
6. **GUI**: the three surfaces (§9) + contract/transport + tests.

Each phase gated (`just t1` per crate; `just t2` at land). Numerical claims (realised P&L at
the transfer price, moved-risk vector) validated against the independent oracle, never merely
asserted (guardrail 5). GUI verified live.

## 12. Open items for the next review

- **Transfer capability granularity.** New `risk_transfer` action vs a `Book`-with-transfer
  flag vs gating desk-to-desk on `Administer`. (Leaning: a narrow `risk_transfer` sub-gate so
  booking ≠ cross-desk transfer.)
- **Partial-transfer position identity.** Splitting one position into source-remainder +
  target-new: preserve the original `position_id` on the remainder and mint a new id for the
  moved slice (leaning yes) vs mint both.
- **Transfer-price policy per desk.** Whether `Agreed` overrides are allowed at all for some
  desks, or mid/mark is mandatory (a per-desk policy flag) — the FTP "single governed
  reference" stance (§3.3).
- **Accept SLA / expiry.** Timeout for `Pending` desk/trader transfers before auto-reject;
  whether an un-accepted transfer parks the risk anywhere (leaning: risk stays in source
  until accepted — no limbo).
- **Rule-change re-bucketing.** Whether editing the routing graph offers a **bulk
  re-attribute** of already-open positions to the new routing (a batch of kind-(a) transfers)
  — a natural follow-on now that manual transfer exists.
- **Cross-entity / cross-`AssetClass` transfers.** Whether a transfer may cross legal entity
  (`EntityId`) or asset class, and the added controls if so (the FRTB desk-boundary /
  banking-book-vs-trading-book concerns of §3.5 bite hardest here).
