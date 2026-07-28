# FI Pricing Groups — Design Proposal

> **SHIPPED + SUPERSEDES per-book tiering (2026-07-28).** Pricing groups are live, and tiering is
> now **group-only**: the per-book tiering path this doc references as its baseline
> (`AggregatedBookDef.tiering`, `apply_tiering`, the `UpdateBookTiering` RPC, and the book-pivoted
> Tiering page) has been **removed**. A book always publishes the RAW consolidated composite;
> outbound tiering is applied ONLY per session via a group's `FeaturePipeline` (the `Tiering`
> feature). The FI **Tiering** page is now session-pivoted (a roster of FIX sessions → the pricing
> group applied to each). Historical file:line references and the "falls back to book-level tiering"
> notes below are retained for design lineage but no longer describe the code.
>
> Original status: design proposal for review (2026-07-27). Extends the client-tiering dimension of
> `FI-TIERING-RESEARCH.md` §5 (strategy #6, per-client tier) into a concrete model.

## 1. Concept

A **pricing group** is a named, trader-defined grouping (an arbitrary code name — "GROUP-A",
"TIER1-EU", …) that binds a set of **connected clients / FIX sessions** to their own **tiering
strategies**, so different clients receive different markups off the same raw liquidity:

- Client on **GROUP-A** → tiering strategy N (e.g. Flat +Nbp).
- Client on **GROUP-B** → tiering strategy B.

A group carries **separate strategy assignments for the two outbound pricing modes**:
- **ESP** — executable streaming prices (continuous RFS/aggregated-book stream).
- **RFS/RFQ** — request-for-quote pricing + order/quote execution.

And every **execution/deal records full pricing provenance** — the raw price, the markup applied,
the group, and the strategy — so post-trade analytics can reconstruct exactly what a client
executed against.

This is the standard dealer **client-tiering** model (research doc dimension B): the pricing
group supplies the per-client base margin/strategy; it composes with the per-book methodology
tiering and the guardrails.

## 2. Current state (what pricing groups build on)

- **Client identity is desk-only today.** A FIX session resolves to a registered `FixConnectionDef`
  (`config/fix_connections.rs:67-90`: `id`, `sender/target_comp_id`, `kind` = Options /
  FixedIncomeQuote / FixedIncomeStream, and **`desk`**) → `FixContext::with_desk_routing`
  (`services/fix.rs:364-374`). GUI/API callers resolve `session_token → UserDef` (`config/identity.rs:128-163`,
  carries `desk_ids`). **There is no per-connection/per-user pricing identity beyond desk.**
- **Tiering is per-book, applied once, identical for all subscribers.** `TieringConfig` on
  `AggregatedBookDef.tiering` (`identity.rs:456`); `apply_tiering` (`services/aggregation.rs:659-724`)
  tiers the composite once per book tick; `handle_aggregated_book_subscribe` (`services/stream.rs:1555-1611`)
  hands `hub.snapshot(book_id)` out verbatim; RFQ reuses the same tiered composite
  (`aggregation.rs::resolve_rfq_composite:468-524`, `quote.rs::book_composite`). **No per-client hook.**
- **Executions carry no pricing provenance.** `Execution` (`quote.rs:1610-1625`) = `execution_id`,
  `quote_id`, `side`, `traded_premium`, `instrument`, `epoch_nanos`, `attribution` — the single final
  price only. `Deal` (`celnet-client/src/desk.rs:261-288`) likewise only `price`. `AttributionRecord`
  says *who* quoted, not *what markup*. The raw/pre-tiering price is overwritten in place by
  `apply_tiering` and never retained.
- **`celnet-tiering` is client-agnostic and reusable unchanged** — `TieringStrategy`/`TieringConfig`/
  `pipeline::quote` operate on a `QuoteCtx`, decoupled from "book" or "client". Pricing groups are a
  pure `celnet-server` + wire + GUI extension.

## 3. The model — `PricingGroupDef`

A new persisted entity, sibling to `AggregatedBookDef` / `FixConnectionDef` / `UserDef` in
`config/identity.rs`, on the same admin JSON chassis + validation discipline:

```
PricingGroupDef {
  id: String,                       // stable slug (registry key)
  name: String,                     // the trader's code name ("GROUP-A")
  description: String,
  // Membership — a group serves MANY FIX sessions/users (1-to-many); each member resolves to
  // exactly ONE group (deterministic pricing). Desk is a default-group fallback tier.
  member_connection_ids: Vec<String>,   // inbound FIX sessions (FixConnectionDef.id)
  member_user_ids: Vec<String>,         // GUI/API principals (UserDef.id)
  member_desks: Vec<String>,            // desk-level default membership (fallback)
  // Per-mode feature PIPELINES (ordered feature lists composed from the library, §6):
  esp_pipeline: FeaturePipeline,         // ESP / streaming
  rfq_pipeline: FeaturePipeline,         // RFS/RFQ quotes + order pricing
  share_pipeline: bool,                  // true ⇒ RFQ uses esp_pipeline (SAME); false ⇒ separate
  enabled: bool,
}
FeaturePipeline = Vec<FeatureSpec>       // ordered; FeatureSpec = { kind: PricingFeatureKind, params }
```

- **`TieringConfig` is reused verbatim** — the same Flat / Inventory-skew / SCALE_SMOOTH strategies,
  guardrails, and unit the per-book editor already produces. A group is just *another place a
  `TieringConfig` is attached*, one for each pricing mode.
- `None` on a mode ⇒ that mode falls back to the book-level tiering (see §6) — backward compatible.
- Validation mirrors `validate_tiering_config` + membership integrity (known connection/user ids,
  no member in two enabled groups → deterministic resolution).

## 4. Client → pricing-group resolution

At the point we price for a specific client, resolve **caller → pricing group**:
- **FIX session** → `connection id` → the group whose `member_connection_ids` contains it (fallback:
  the desk's default group, then no group).
- **GUI/API caller** → `user id` → the group whose `member_user_ids` contains it.
- Resolution is a cheap map lookup built once from the registry (rebuilt on any group admin write),
  cached on the hub/edge like the book identities map (`aggregation.rs::build_identities`).

A member belongs to **at most one enabled group** (validated) so pricing is deterministic.

## 5. ESP vs RFS/RFQ — the two hooks

The group applies a **different `TieringConfig` per mode**, at the two natural seams the map
identified:

| Mode | Hook (insert per-subscriber tiering here) |
|---|---|
| **ESP / streaming** | `Session::handle_aggregated_book_subscribe` (`stream.rs:1555`) — after `hub.snapshot(book_id)`, before `agg_book_snapshot_msg(...)`. Also `handle_rates_subscribe` (`stream.rs:1476`) for non-aggregated FI RFS. Apply the resolved group's **`esp_tiering`** to *this subscriber's* stream. |
| **RFS/RFQ** | `QuoteEdge::request_quote` (`quote.rs:866-1133`) — at the `book_composite` branch (`~:975`) and the options-engine branch (after `spread.two_way`, `~:1010`). Apply the resolved group's **`rfq_tiering`** before packaging the `Quote`. |

Because each subscriber/quote is priced from a memoised **raw (or book-default-tiered) composite**
plus a cheap per-client `pipeline::quote(&ctx)`, the hot publish loop stays lean — the composite is
consolidated once; the per-client tiering is a pure arithmetic transform per subscriber.

## 6. The outbound pricing pipeline (the canonical flow)

Outbound pricing is a **trader-composable, ordered pipeline of pricing *features*** drawn from a
**feature library**. Each feature is a self-contained code module behind one common interface; the
trader **drag-and-drops** features from the library palette into a pricing group's pipeline and
orders them. The running two-way flows RAW → …the trader's chosen features… → outbound to the FIX
connection.

**Feature library (initial set — extensible; each is a `PricingFeature` code module):**

| Feature | What it does |
|---|---|
| **RAW** | source — the consolidated LP composite (the implicit start of every pipeline) |
| **MID SHIFT** | shift the mid — manual bias / reference-price override (the desk's price construction) |
| **TIERING** | apply margin/markup on the price (celnet-tiering spread strategies: Flat, Scaled-Smoothed-Spread) |
| **AXE** | skew the two-way toward the desk's axe (lean to buy/sell what the desk wants to do) |
| **POSITION** | skew based on net inventory/position (celnet-tiering Inventory-skew) |
| **PANIC / SKEW** | an emergency/risk overlay skew on top |

Canonical example order (the trader composes their own): `RAW → MID SHIFT → TIERING → AXE →
POSITION → PANIC/SKEW → OUTBOUND (ESP stream or RFS/RFQ quote) → FIX connection`.

- **ESP price = RAW + pipeline** and **RFS price = RAW + pipeline** — the SAME feature pipeline, but
  each mode is configured independently on the pricing group (a group's ESP pipeline and RFQ pipeline
  may differ).
- **TIERING is margin only** — literally applying margin on the received market price. **PANIC/SKEW is
  a SEPARATE feature** applied on top of the tier (the `applied_skew` is not part of tiering).
- The features are a **drag-and-drop library** — the trader composes each group's pipeline by
  dragging feature modules from a palette and ordering them; the order IS the pipeline. New feature
  modules added to the library appear in the palette automatically, with no pipeline rework.
- **ESP vs RFS/RFQ**: a group can use the **SAME** pipeline for both modes, or configure **SEPARATE**
  pipelines per mode — the trader's choice per group.
- **`celnet-tiering` generalizes into the feature engine**: its `TieringStrategy` trait becomes the
  `PricingFeature` interface, and its shipped, oracle-tested composition/guardrail pipeline is reused
  to sequence whatever features the trader dropped in. The existing strategies map to features —
  Flat / Scaled-Smoothed-Spread → **TIERING**, Inventory-skew → **POSITION**; **MID SHIFT**, **AXE**,
  and **PANIC/SKEW** are new `PricingFeature` modules added to the library.
- The existing **per-book tiering becomes the default pipeline** (a `RAW → TIERING` pipeline) for a
  client with **no pricing group** (backward compatible); a grouped client's pipeline is entirely its
  group config.

This ordered pipeline replaces the earlier "replace vs stack" question — the composition IS the
pipeline: each feature transforms the running two-way in list order.

## 7. Execution pricing provenance (for analytics)

The analytics requirement — "executions should have a complete view of the raw price used, the
tiering applied, what the user executed against" — needs new fields captured **where tiering is
applied** and threaded through booking:

Add a `PricingProvenance` block, stamped when the per-client two-way is computed (the §5 hooks) and
carried on `Quote` → `Execution` / `Deal`:

The provenance is a **per-feature waterfall** — the two-way after each pipeline stage — so analytics
can attribute the outbound price to each feature, not just "raw + one number":

```
PricingProvenance {
  pricing_group_id: String,   // which group priced it ("" if none → book default)
  mode: EspOrRfq,             // which pipeline (ESP stream vs RFS/RFQ) was used
  raw_bid, raw_mid, raw_offer: f64,          // after RAW               (consolidated LP composite)
  constructed_bid, constructed_offer: f64,   // after TRADER CONSTRUCTION (desk price from mid)
  tiered_bid, tiered_offer: f64,             // after TIERING           (margin applied)
  outbound_bid, outbound_offer: f64,         // after PANIC/SKEW        (= what was sent to FIX)
  applied_margin: f64,        // the TIERING markup actually added (price units)
  applied_skew: f64,          // the PANIC/SKEW overlay actually added (signed, price units)
  features: [FeatureId + params],  // the ordered features that ran + their params (TIERING strategy, skew mode, …)
}
```

- Each stage's two-way is stamped as the feature runs; `applied_margin` = tiered − constructed
  spread, `applied_skew` = outbound − tiered shift. An execution therefore shows RAW → constructed →
  tiered → outbound and exactly which feature moved the price by how much.
- The realized-markout analytics input = executed price vs `raw_mid`, decomposable per feature, per
  group, per client, per mode.

- Captured at `apply_tiering`'s per-line loop (`aggregation.rs:684-719`) / the per-client equivalent
  at the §5 hooks, alongside the tiered `best_bid/best_offer`.
- Threaded: `Quote.pricing_provenance` (today the `Quote` already holds the final `price`; booking
  just replays it — so stamping it on the `Quote` at quote time is sufficient, and `accept_quote`
  copies it onto the `Execution`).
- Persisted on the `Execution`/`Deal` + the journal, so a downstream analytics job can compute
  realized markout = executed price vs raw_mid, per group/strategy/client over time — the input the
  research doc flags for calibrating tiers against real flow.
- **Wire/proto**: additive `PricingProvenance` message on `Quote`/`Execution` (and `Deal`); no
  versioning (one contract).

## 8. Where it all inserts (extension points, all in `celnet-server`)

1. **`PricingGroupDef`** + registry + validation in `config/identity.rs` (+ the caller→group resolver).
2. **Per-client tiering application** at the three hooks (`stream.rs:1555`, `stream.rs:1476`,
   `quote.rs:~975/1010`) via `pipeline::quote` — `celnet-tiering` unchanged.
3. **`PricingProvenance`** capture + threading through `Quote`/`Execution`/`Deal` + journal.
4. **CRUD**: admin `Create/Update/DeletePricingGroup` (mirror the aggregated-book admin CRUD) +, for
   the tiering blocks, a **trader-accessible `UpdatePricingGroupTiering`** gated on
   `quote_respond·fixed_income` (mirror the shipped `UpdateBookTiering` exactly).
5. **GUI — the Pricing Groups builder (Administration).** A dedicated **Administration → Pricing
   Groups** surface with a **drag-and-drop, pipeline-style** editor:
   - **Left rail** — the list of pricing groups (create / **clone** / delete), each showing its
     assigned FIX connections / users / desks and enabled state.
   - **Feature palette** — draggable feature chips (MID SHIFT · TIERING · AXE · POSITION · PANIC/SKEW):
     the library the trader drags from.
   - **Pipeline canvas** (the centrepiece) — a left-to-right flow of feature **cards**:
     `RAW ▸ [ the trader's dragged features, reorderable ] ▸ OUTBOUND`. Drag from the palette to
     insert; drag a card to reorder; click a card to expand its **inline config** (each feature's
     params — TIERING reuses the shipped `TieringEditor`). Every group can hold the **same features
     with different configs** → unique per client.
   - **ESP / RFQ toggle** on the canvas — one pipeline shared, or switch to edit the separate RFQ
     pipeline (`share_pipeline`).
   - **Membership** — assign FIX connections / users / desks to the group (many-to-one).
   - **Live price preview** — a sample RAW two-way fed through the pipeline, rendering the two-way
     **after each feature card** (the provenance waterfall as a live, visual "cool pipeline" view), so
     the trader sees exactly what each feature does to the price before saving.
   Built with the app's design system + a drag-and-drop canvas; reuses `TieringEditor` +
   `celnet-tiering` types. Separately, surface `PricingProvenance` on the deal blotter / an
   executions-analytics view.

## 9. Suggested build order

1. `PricingGroupDef` + registry + validation + caller→group resolver (server-only; unit-tested).
2. Apply group tiering at the ESP hook (aggregated-book subscribe) — the highest-value path — then
   the RFQ hook, then the rates-RFS hook. Layering per §6(B).
3. `PricingProvenance` capture + thread onto `Quote`/`Execution`/`Deal` + journal (analytics
   foundation).
4. CRUD (admin + trader tiering) + proto/WS.
5. GUI: Pricing Groups management + provenance on the blotter/analytics.

Each step oracle-gated (e.g. two clients in different groups subscribing the same book receive
different two-ways off the same raw composite; an execution's provenance reconstructs raw_mid +
markup exactly).

## 10. Open questions to confirm before building

1. **TRADER CONSTRUCTION feature** — define its inputs/behaviour: a manual mid shift/bias, a
   reference-price override, a fixed/where's-my-axe adjustment, or a skew-from-position? (Its slot in
   the pipeline is fixed — after RAW, before TIERING — but what it *does* needs specifying.) Also
   confirm whether PANIC/SKEW is a manual desk toggle, an automatic inventory/risk trigger, or both.
2. **Membership granularity** — group by FIX **connection id** + **user id** (proposed), or also by
   **desk** (a desk-level default group)? Desk is the only grouping that exists today, so a
   desk-default group is a natural fallback tier.
3. **One group per member** (proposed, for deterministic pricing) vs. priority-ordered overlap.
4. **ESP vs RFQ independence** — always two separate configs (proposed), or allow "same as ESP" for
   RFQ to reduce config effort?
5. **Provenance scope** — stamp on `Quote` only (cheap; booking replays) vs. also re-derive at
   execution for audit. Proposed: stamp at quote/stream time, copy to execution.

---

Cross-ref: `FI-TIERING-RESEARCH.md` (the strategies + composition theory), `FI-PRICING-ENGINE-DESIGN.md`
(the quote-construction seam). Reuses the shipped `celnet-tiering` engine and the `UpdateBookTiering`
trader-CRUD pattern.
