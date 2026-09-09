# FI Risk Routing & Risk Books — Requirements / Design

> Status: **requirements + design for review** (2026-07-28). No code yet. Grounds a NEW
> Fixed-Income capability on the existing booking/position seams (file:line cited below,
> current as of this doc). Follows the shipped **pricing-groups** pattern (registry +
> resolver + proto/WS CRUD + visual builder) as the template.

## 1. Concept & goal

When an incoming order/RFQ is **accepted and filled**, the resulting **risk** must be
routed into a trader-defined **risk book (portfolio)** so that risk management (limits,
greeks, PnL) is applied **per book**. Traders define, visually, *which* book a fill's risk
drops into via **IF `<field> <op> <value>` THEN book** rules, arranged as a **decision-tree
flow**. A **risk-visualisation view** then shows aggregated risk per book.

Three deliverables, all under the **Fixed Income** GUI tab:
1. **Risk Routing rule builder** — a node/flow decision-tree canvas.
2. **Risk Book management** — a **hierarchical** portfolio tree (desk → book → sub-book).
3. **Per-book risk dashboard** — net position/notional, greeks, rates DV01/curve buckets,
   live PnL + limit usage, rolled up the book tree.

### Decisions locked (2026-07-28 review)
- **Builder visual:** node/flow **decision-tree canvas** (condition nodes + book leaves).
- **Book model:** **hierarchical** — desk → book → sub-book, with risk roll-up up the tree.
- **Risk view:** all four metric families (position/notional, greeks, DV01/buckets, PnL+limits).
- **Rules-engine scope:** **ONE firm-wide graph.** An accepted trade **enters the graph at a
  single entry node**; the rules **direct its risk to different DESK PORTFOLIOS** (the leaf
  books, each owned by a desk). Not per-desk isolated graphs — a single graph decides which
  desk's portfolio each trade's risk lands in. So the graph's early conditions typically
  branch by asset/ccy/counterparty and its leaves are the desk portfolios.

## 2. Architecture fit — the real seams (build ON these, don't reinvent)

- **Where fills book positions today (the interception point):**
  - ESP fills → `record_booked_position` (`crates/celnet-server/src/services/stream.rs:1042`).
  - RFQ fills → `QuoteEdge::accept_quote` (`services/quote.rs:1443`) and
    `RfqDeskEdge::accept_desk_quote` (`services/desk/mod.rs:784`).
  - All converge on **`PositionStore::book_from_attribution(booked, attribution)`**
    (`services/risk/store`). A booked fill is already the structured
    **`BookedPosition { position_id, pair, option, notional_base, inputs, quoted_delta,
    premium_style, surface_version }`** carrying an **`attribution`** (desk/user/session).
    *This call is exactly where the risk router decides the target book.*
- **Existing risk/position layer to extend:** `services/risk/` (the `PositionStore` +
  pre-trade **limits**, `celnet_limits::PreTradeDecision`) and the **`rates_book`**
  (`services/rates_book.rs`, monotonic booking ids). Positions currently bucket by
  attribution only; risk routing adds a **`risk_book`** bucketing dimension.
- **Direct precedent for the whole shape (mirror it):** pricing groups —
  `PricingGroupDef` registry + `PricingGroupResolver` (ordered, first-match) in
  `config/identity.rs`; proto/WS CRUD in `celnet.proto` + `ws/codec.rs` +
  `ws/generated_codec.rs` (descriptor-driven) + `AuthService` RPCs; the drag-and-drop
  builder `gui/src/workspaces/PricingGroupsWorkspace.tsx`. Risk routing = the same
  registry + resolver + builder, with a graph resolver instead of a flat list.

## 3. Data model

### 3.1 Risk books — hierarchical (`RiskBookDef`)
A new registry beside `PricingGroupDef`/`AggregatedBookDef` in `config/identity.rs`.
Purpose-named (guardrail 8): a **risk book** is a portfolio risk lands in — distinct from
an **aggregated book** (liquidity consolidation) and the **rates book** (booking ledger).

```
RiskBookDef {
  id: String,                 // stable slug (store key)
  name: String,               // trader label, unique case-insensitive
  parent_id: Option<String>,  // None ⇒ a top-level book; else a sub-book (tree edge)
  desk_id: Option<String>,    // owning desk (top-level books tag a desk)
  description: String,
  limits: Option<RiskLimits>, // per-book pre-trade limits (reuses celnet_limits)
  enabled: bool,
}
```
Invariants: acyclic parent chain; a leaf (childless) book is where positions actually
sit; parent books **aggregate** their descendants (roll-up). One **default/unrouted**
book per desk (or firm) catches fills no rule targets — never drop a fill on the floor.

### 3.2 Routing context — the fields a rule matches
Extracted from the `BookedPosition` + `attribution` + the originating order/quote at the
booking seam. The **`RoutingContext`** (one struct, the rule-evaluation input):

| Field | Source | Type |
|---|---|---|
| instrument_id / symbol | `sub.instrument` | string |
| ccy / pair | `BookedPosition.pair` / underlying | enum/string |
| product | `instrument.product` (vanilla/swap/bond/…) | enum |
| side | fill `Side` | Buy/Sell |
| notional_base / &#124;notional&#124; | `BookedPosition.notional_base` | f64 |
| tenor / expiry_years | `instrument` | duration/f64 |
| strike | `priced.resolved_strike` | f64 |
| counterparty / session | `attribution` (FIX session) | string |
| user | `attribution` | string |
| desk | `attribution.desk` | string |
| price / premium | fill | f64 |

Each registry entry declares a **`kind`** that drives both server type-checking and the
GUI editor (§6.1): `Enum{source}` (a fixed enum like `side`/`product`, or a **registry-
backed** list — `desk`→`DeskDef`, `counterparty`→`FixConnectionDef`) → renders a
**dropdown** of valid values; `Numeric{unit}` → numeric input + `> ≥ < ≤ between`;
`String` → text + `= ≠ contains`. The palette drag-chip, the operator list, and the value
editor are all derived from this one `kind` — so the field vocabulary, valid operators, and
allowed enum values live in a single typed registry the GUI reads (never free-typed, never
drifting). New routable fields are added by extending `RoutingContext` + this registry.

### 3.3 Routing rules — a decision graph (`RiskRoutingGraph`)
The node/flow canvas is backed by a **directed acyclic decision graph**, evaluated from a
single **entry** node per incoming fill:

```
RiskRoutingGraph {
  scope: RoutingScope,           // firm-wide or per-desk (see §7 open item)
  entry: NodeId,
  nodes: Vec<RoutingNode>,
}
RoutingNode =
  | Condition { id, field: RouteField, op: RouteOp, value: RouteValue,
                on_true: NodeId, on_false: NodeId }   // a decision node (yes/no edges)
  | Book      { id, risk_book_id: String }            // a terminal leaf → route here
RouteOp = Eq | Ne | Gt | Ge | Lt | Le | Contains | In | Between
```
- Evaluation: start at `entry`; at each `Condition`, test `field op value` against the
  `RoutingContext`, follow `on_true`/`on_false`; stop at the first `Book` leaf → that book.
- **Well-formedness (validated server-side, like `check_pricing_group`):** acyclic; every
  path terminates at a `Book` leaf; every referenced `risk_book_id` exists; every
  `field/op/value` is type-consistent. A graph with any non-terminating path is rejected —
  guaranteeing every fill routes deterministically (fallback = the default book leaf).
- A multi-condition "AND" in the UI is just a chain of `Condition` nodes sharing the same
  `on_false`; "OR" fans several conditions to the same `on_true`. The flat ordered-list
  mental model (pricing-groups resolver) is the degenerate right-spine of this graph.

## 4. `RiskRouter` (the resolver) & booking integration

- **`RiskRouter::route(ctx: &RoutingContext) -> RiskBookId`** — pure, deterministic graph
  walk; O(depth), allocation-free on the hot path (guardrail 11 — booking is off the
  pinned pricing core but still bounded).
- **Integration:** at `book_from_attribution`, resolve the book id via `RiskRouter`, then
  book into `PositionStore` keyed by **(risk_book_id, instrument)** (new dimension) instead
  of attribution alone. Pre-trade **limits** check runs against the *target book's* limits
  (and its ancestors' roll-up limits) — so a hard-limit-blown booking is refused at the
  book that would breach (extends the existing `PreTradeDecision` path,
  `services/quote.rs`/`services/risk`).
- **Provenance:** stamp the chosen `risk_book_id` + the winning rule path onto the
  execution/deal (mirrors the pricing `PricingProvenance` waterfall) so the blotter and the
  risk view can show *why* a fill landed where it did.

## 5. Risk aggregation & the per-book dashboard

Per **leaf** book: aggregate its positions; per **parent** book: roll up descendants.
Four metric families (all requested):
1. **Net position + notional** — signed net + gross/net notional, per book and per instrument.
2. **Greeks (Δ/Γ/Vega/Θ)** — aggregated from the FX-vanilla risk already marked via
   `VanillaInputs` in `record_booked_position`; summed across the book subtree.
3. **Rates DV01 / curve buckets** — DV01 + per-tenor bucket risk for FI positions (ties
   into `rates_book` + the `celnet-bond` risk leaf).
4. **Live PnL + limit usage** — mark-to-market PnL per book, and each book's usage vs its
   `RiskLimits` (green/amber/red), reusing `celnet_limits`.
Streamed to the GUI over the existing WS mirror (a `risk_book_snapshot` message), memoised
per book-version like the aggregation composite (lazy, not a timer thread).

## 6. GUI (Fixed Income tab)

New rail entries (`gui/src/lib/commands.ts`, `assets:["fixed_income"]`) mounted in
`Shell.tsx` `WORKSPACE_VIEW`, gated on a trader capability (`risk_manage·fixed_income`, new)
for edit; view for any FI trader.

### 6.1 Risk Routing canvas (the cool visual) — `RiskRoutingWorkspace.tsx`
A **decision-tree flow canvas** with **drag-and-drop rule building**: an **incoming-fill**
source node on the left; drag **condition nodes** and **book leaf** nodes onto the canvas;
wire `yes`/`no` edges between them. Features:

- **Drag-and-drop field palette (the primary interaction).** A left-hand **palette of the
  routable fields** (driven by the routing-field registry, §3.2) — ccy, notional, product,
  side, tenor, counterparty, desk, strike, … each a draggable chip. A trader **drags a
  field onto the canvas** to spawn a condition node pre-bound to that field (or drops it
  onto an existing node to set/replace its field). No typing field names — the palette IS
  the field vocabulary, so a rule can only reference real fields and never drifts from the
  registry. The palette groups fields (instrument / trade / counterparty / desk) and is
  searchable.
- **Typed value editors — enum fields render as DROPDOWNS.** Once a field is on a node, the
  operator list and value editor are **typed to that field** (§3.2 `kind`):
  - **Enum fields → a dropdown of the enum's valid values** (never free text): `side`
    (Buy/Sell), `product` (Vanilla/Swap/Bond/…), `ccy`/`pair` (the FX/ccy enum), and
    **registry-backed** fields — `desk` (dropdown of `DeskDef`s), `counterparty`/`session`
    (dropdown of `FixConnectionDef`s), `risk_book` on the leaf (dropdown of the book tree).
    Multi-value ops (`in`) present a multi-select of the same enum.
  - **Numeric fields** (`notional`, `strike`, `tenor`, `price`) → a numeric input with unit,
    and ops `> ≥ < ≤ between`; `between` shows two inputs.
  - **String fields** → text input with `= ≠ contains`.
  The operator dropdown only offers ops valid for the field `kind`, so a malformed rule
  (e.g. `side > 5`) is unrepresentable in the UI.
- **Book leaf** nodes pick their target from the hierarchical book-tree dropdown (§6.2).
- **Live "trace a sample fill"**: pick/compose a sample fill (or replay a recent real fill)
  → the canvas **lights up the traversed path** node-by-node and shows the landing book —
  the same trace the server `RiskRouter` would produce (parity-tested).
- Validation surfaced inline (unreachable node, non-terminating path, unknown book, type
  mismatch) before save; save calls `UpdateRiskRoutingGraph`.
- Auto-layout (layered DAG) so the tree stays readable; pan/zoom; minimap for big graphs.
- Implementation note: reuse React state + SVG edges (no heavy graph lib needed for the
  MVP; a layered auto-layout is a small dependency-free pass). Compositor-friendly motion
  only for the path-trace highlight.

### 6.2 Risk Book tree editor — `RiskBooksWorkspace.tsx`
A collapsible **tree** (desk → book → sub-book): create/rename/nest/disable books, set
per-book limits, mark the default book. Drag to re-parent (validated acyclic).

### 6.3 Per-book risk dashboard — `RiskDashboardWorkspace.tsx`
Select a book in the tree → the four metric panels (§5) for that book **and its roll-up**;
a heat-strip of limit usage across books; drill into contributing fills (links to the
blotter, showing the routing provenance).

## 7. Proto / WS / CRUD (mirror pricing groups)

New proto messages + `AuthService` RPCs (descriptor-driven codec auto-adapts; add hand +
generated codec entries per the pricing-groups precedent):
- `ListRiskBooks` / `CreateRiskBook` / `UpdateRiskBook` / `DeleteRiskBook` (admin structure).
- `ListRiskRoutingGraph` / `UpdateRiskRoutingGraph` (trader `risk_manage·fixed_income`).
- `RiskBookSnapshot` stream message (per-book risk, §5).
No versioned APIs (guardrail 9): one current contract.

## 8. Crate / workstream breakdown (parallel-safe, disjoint files)

1. **`celnet-risk-routing`** (NEW leaf crate): `RoutingContext`, `RouteField`/`RouteOp`/
   `RouteValue`, `RiskRoutingGraph`, `RiskRouter::route`, well-formedness check — pure,
   fully unit-tested (graph walk + validation), no server deps. *(Numerically/logically
   self-contained → independent oracle = a hand-written truth table of fills→books.)*
2. **`config/identity.rs`**: `RiskBookDef` (tree) + `RiskRoutingGraph` registry +
   validators (mirror `check_pricing_group`).
3. **`services/risk/`**: `PositionStore` gains the `risk_book` dimension + tree roll-up;
   wire `RiskRouter` into `book_from_attribution`; per-book limit check.
4. **proto + `ws/` codecs + `AuthService`**: the §7 messages/RPCs.
5. **Risk aggregation + `RiskBookSnapshot`** publish (beside the aggregation hub pattern).
6. **GUI**: the three workspaces (§6) + contract/transport + tests.

Each phase gated (`just t1` per crate; `just t2` at land). GUI verified live.

## 9. Open items for the next review
- ~~**Rule scope**~~ **RESOLVED (2026-07-28):** ONE firm-wide graph — a trade enters at a
  single entry node and the rules direct its risk to different **desk portfolios** (leaf
  books). `RiskRoutingGraph.scope` is fixed to firm-wide; `RoutingScope` per-desk is dropped.
  Every leaf `Book` resolves to a desk-owned portfolio (its `RiskBookDef.desk_id`, directly
  or via its parent in the tree), so routing = "which desk's portfolio does this trade's risk
  belong in", decided centrally.
- **Re-routing existing positions:** rules apply to NEW fills. A manual "move position to
  book" action (and whether a rule change re-buckets open risk) is a follow-up.
- **Limits semantics:** hard (reject fill) vs soft (warn) per book, and how ancestor
  roll-up limits compose with leaf limits.
