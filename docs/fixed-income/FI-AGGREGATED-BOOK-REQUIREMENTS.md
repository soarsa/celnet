# FI Aggregated Book — Requirements

**Status:** IMPLEMENTED in `celnet-aggregation` & `celnet-server` · **Owner:** (operator) · **Updated:** 2026-07-22
**Depends on / touches:** `celnet-aggregation`, `celnet-server` (config + services + ws),
`gui/` Administration workspace, `celnet-proto`, `identity.json` store.

> Draft to seed the design. **Decisions A, C, E are now RESOLVED (§12)**; B and D
> keep their recommended defaults pending a final nod. Recommendations are marked
> **[R]**; open forks **[?]**.

## Resolved this round
- **LP = an inbound *liquidity connection* (transport-agnostic).** An LP is any
  inbound connection into the aggregation service — **FIX or any other API adapter**
  — that receives quotes / pricing requests on its ingest ("left") side and **pushes
  prices into the aggregated book** on its ("right") side. Not FIX-only.
- **Full scope MVP.** The composite is not just displayed — it must be **quotable and
  bookable** (auto-quote and deal-booking run off the aggregated price).
- **Global books.** An aggregated book is **global**; any authenticated user may view
  its composite. Defining/editing remains **admin-only**. (No desk ownership.)

---

## 1. Motivation

Fixed-income prices for a given instrument arrive from **multiple liquidity
providers (LPs)** on separate feeds. Traders and downstream pricing need **one
consolidated view per instrument** — the best two-way across all contributing LPs,
with size and depth — rather than N disjoint single-LP lines. We want an operator
to **define a named "Aggregated Book"**, **assign a set of LPs** to it, and have the
platform continuously publish a **composite price per FI instrument** aggregated
across those LPs. The book is defined and managed from the **Administration** tab.

## 2. Glossary

| Term | Meaning |
|------|---------|
| **Liquidity Provider (LP)** | An **inbound liquidity connection** (FIX *or any other API adapter*) that celnet consumes prices from: its ingest side receives quotes / pricing, its output side pushes prices into an aggregated book. |
| **Aggregated Book** | A named, admin-defined set of LPs whose per-instrument top-of-book is consolidated into one composite price. |
| **Composite price** | Per instrument: best bid / best offer (+ sizes), composite mid, bid/offer depth, a confidence score, and the contributing LP breakdown. |
| **Contributor** | An LP currently supplying a fresh, non-excluded quote to a given instrument's composite. |

## 3. What already exists (reuse — do not rebuild)

- **Consolidation engine — `celnet-aggregation`** (built, tested, **but unwired**):
  - `ConsolidatedBook { best_bid, best_offer, best_bid_size, best_offer_size, composite_mid, bid_depth, offer_depth, confidence, contributions, gating_undecidable }` — `crates/celnet-aggregation/src/consolidate.rs`. Already implements best-bid/offer across venues, time-weighted **staleness decay** (`2^{-Δt/τ}`), and **MAD-based divergent-source exclusion**.
  - `VenueFeed` trait (`venue()` + `top_of_book(instrument, now_nanos)`) — `crates/celnet-aggregation/src/feed.rs`. The "one venue → poll top-of-book" seam.
  - `VenueId`, `VenueQuote { venue, instrument, bid, offer, bid_size, offer_size, ts, quality }`, `Instrument { underlying, tenor }` — `crates/celnet-aggregation/src/instrument.rs`. Only concrete feed today is `SimVenue` (synthetic).
- **Admin CRUD template** — `RegistryPanels.tsx` (`BooksPanel`/`EntitiesPanel`, list + inline create/edit/delete via a `run()` wrapper) over `BookDef`/`EntityDef`/`DeskDef` in `crates/celnet-server/src/config/identity.rs`, persisted in `identity.json`.
- **Admin RPC pipeline template** — `FixAdminEdge` (`crates/celnet-server/src/services/fix_admin.rs`): proto message → `handle_unary` arm (`ws/mod.rs`) + gRPC → dual WS codec (hand + generated, differential test) → registry store → GUI. Entitlement via `authorize_caller` / admin gate.
- **LP plumbing (two existing shapes)**:
  - **Inbound streaming** — `FixConnectionDef { kind: FixedIncomeStream | FixedIncomeQuote, desk, sender/target_comp_id, … }` in `fix_connections.rs`, CRUD'd in the GUI FIX admin, persisted `fix-connections.json`. RFS (`FixedIncomeStream`) delivers **continuous** per-instrument top-of-book.
  - **Outbound RFQ** — `celnet-rfq` `MultiDealerEngine` / `QuoteSource` / `FixLpAdapter` — **per-request** fan-out to a dealer panel, ranked. Not a standing stream.

## 4. The gap (what this requirement adds)

1. A persisted, admin-definable **Aggregated Book entity** (name, members, scope, params) — nothing like it exists.
2. **Wiring `celnet-aggregation` into the server** — today it is unconsumed dead code.
3. A **real streaming `VenueFeed`** backed by live LP prices (today only `SimVenue`).
4. **One FI-instrument key** for the composite — reconcile `celnet-aggregation::Instrument{underlying,tenor}` with server `InstrumentDef{instrument_id, external_ids}`.
5. **Admin GUI** to define/manage the book and its LP membership.
6. **Publication** of the composite per-instrument price to clients.

## 5. Functional requirements

**FR-1 — Define an Aggregated Book.** An admin can create a book with: a unique
`name`; a set of **member LPs**; an **instrument scope**; and **aggregation
parameters** (§7). Create/rename/edit-members/delete, all admin-only.

**FR-2 — Manage LP membership.** Add/remove LPs to/from a book. An LP may belong
to more than one book. Removing an LP takes effect on the live composite promptly.

**FR-3 — Instrument scope.** Either **all** instruments any member quotes, or an
**explicit instrument set** (from the reference-data registry). **[R]** default to
"all instruments members quote", with an optional explicit allow-list.

**FR-4 — Continuous composite.** For each in-scope instrument, the book publishes a
live `ConsolidatedBook` (best bid/offer + sizes, mid, depth, confidence,
contributor breakdown), recomputed as member quotes tick, with staleness decay and
divergent-LP exclusion applied.

**FR-5 — Consume the composite (display + quote + book).** The composite per-instrument
price is (a) **streamed to clients for display** (FI Streaming workspace), labelled
"aggregated" with book name + contributor count; (b) **quotable** — the desk can
auto-quote off the composite; and (c) **bookable** — deals execute/book against the
composite price. All three are in the MVP (decision E).

**FR-6 — Observability of contribution.** For any instrument, an admin/trader can
see which LPs are contributing, which are excluded (stale / divergent) and why, and
the confidence — so a bad LP is diagnosable.

**FR-7 — Entitlement.** Defining/editing a book is **admin-only**. A book is
**global**: any authenticated user may view its composite and (subject to their own
FI action capabilities) quote/book off it. No desk ownership on the book itself.

**FR-8 — Lifecycle robustness.** LP disconnect ⇒ that LP drops from the composite,
confidence falls, book keeps publishing from the rest. Zero contributors ⇒ the
instrument publishes "no price / undecidable", never a stale or fabricated one.

## 6. Proposed data model (for review)

```text
AggregatedBookDef {
  id: String,                     // minted slug, stable key (cf. mint_desk_id)
  name: String,                   // unique display name (global; not desk-owned)
  member_connection_ids: Vec<String>, // inbound liquidity connections (FIX or API) — decision A
  instrument_scope: Scope,        // AllMembersQuote | Explicit(Vec<instrument_id>)
  params: AggregationParams,      // see §7
  enabled: bool,
}
```

An **inbound liquidity connection** is transport-agnostic: today a FIX session
(`FixConnectionDef`, `FixedIncomeStream`/`FixedIncomeQuote`), tomorrow another API
adapter behind the same `VenueFeed` seam. `member_connection_ids` therefore reference
a connection registry entry, not a FIX-specific id — so a non-FIX LP slots in without
a model change. Books are **global** (no `desk_id`).

Persisted alongside `desks`/`books`/`instruments` in `IdentityStore` (`identity.json`),
as an **additive, serde-default** field so existing `identity.json` loads unchanged
(same discipline as `BookDef`). Naming is deliberately **`AggregatedBook`**, distinct
from the existing netting **`BookDef`** (an accounting partition) to avoid conflation.

## 7. Aggregation parameters (map onto the engine)

| Param | Meaning | Default **[R]** |
|-------|---------|-----------------|
| `staleness_tau_ms` | Decay half-life τ for `2^{-Δt/τ}` | tune vs live feeds; validate |
| `max_quote_age_ms` | Hard cut — older quotes never contribute | e.g. 5× τ |
| `divergence_gating` | MAD-based exclusion on/off | on |
| `min_contributors` | Below this ⇒ low confidence / `gating_undecidable` | 1 (publish-with-warning) or 2 |
| `depth_levels` | How many levels to consolidate | top-of-book first; depth later |

These bind directly to the existing `consolidate.rs` knobs — no new math. Values must
be **validated against live/reference feeds**, not merely asserted (CLAUDE.md rule 5).

## 8. Architecture & wiring

- **Dependency:** `celnet-server` gains a dependency on `celnet-aggregation` (pure,
  `#![forbid(unsafe_code)]`, no I/O). This is the **correct one-way direction** — the
  aggregation crate must never depend on the server (preserve its edge-free property).
- **Ingest ↔ publish shape:** the service has a **left (ingest) side** — inbound
  liquidity connections receive quotes / pricing per instrument — and a **right
  (publish) side** — the consolidated composite is pushed into the aggregated book
  and out to consumers. The `VenueFeed` trait is exactly this ingest seam.
- **Feed adapter (new), transport-agnostic:** a `VenueFeed` impl per inbound
  connection. **[R]** the first adapter backs onto the existing FIX RFS ingest
  (`FixedIncomeStream` → `services/fix.rs` rates dialect → `RatesPositionStore`); the
  same trait admits a **non-FIX API adapter** later with no engine/model change
  (decision A). Each member connection contributes its latest per-instrument
  top-of-book.
- **Publish for display + quote + book:** the composite feeds three consumers — the
  FI streaming display, the auto-quoter (quote off the composite), and the booking
  path (book deals against it). The quoting/booking wiring reuses the existing rates
  auto-quote + `RatesPositionStore` booking seam, sourced from the composite rather
  than a single LP line (decision E).
- **Instrument key (new bridge):** key the composite on the server's canonical
  `instrument_id` (`InstrumentDef`), mapping member quotes → `instrument_id`. The
  engine's `Instrument{underlying,tenor}` becomes an internal detail or is adapted.
  **[?]** confirm the key (decision D).
- **Runtime:** the server maintains, per enabled book, a consolidator fed by its
  members; recomputes on tick; publishes composites over the WS mirror + gRPC on a
  bounded, offloaded telemetry/stream path (hot core stays alloc/lock/log-free —
  CLAUDE.md rule 11).
- **Admin RPCs:** `Create/Update/Delete/ListAggregatedBook` following the
  `FixAdminEdge` pattern (proto → `handle_unary` + gRPC → dual codec + differential
  test → `IdentityStore` → GUI).

## 9. Administration GUI

- A place in the **Administration** workspace to define the book, manage its LP
  membership (multi-select of available LPs), set scope + params, enable/disable —
  mirroring the `RegistryPanels.tsx` list+form pattern.
- **[?]** A **5th `AdminTab`** (proposed name **"Aggregation"** — *not* "Books", which
  is the netting tab) **[R]**, vs. its own top-level workspace (both precedents exist:
  `EntitiesPanel`/`BooksPanel` are admin tabs; `ReferenceDataWorkspace` is top-level).
- A **live/read** view of a book's composite (per-instrument best two-way +
  contributor breakdown + confidence) — likely surfaced in the **FI Streaming**
  workspace, labelled as aggregated.

## 10. Non-functional requirements

- **Scale:** many instruments × many LPs × many books (IB-sized). O(members) per
  instrument recompute; structures chosen for many-instrument fan-in (CLAUDE.md §6).
- **Latency/throughput:** composite updates within the FI streaming budget
  (`docs/ARCHITECTURE.md` §1.2); hot path zero-alloc, telemetry offloaded.
- **Observability:** structured logs + metrics + HdrHistogram for compositor
  recompute latency, per-LP contribution/exclusion counts, per-book confidence.
- **Correctness:** composite validated against an independent reference (hand /
  QuantLib-style / synthetic ground-truth LP ladders), never merely plausible.
- **Contract:** one clean current contract, no versioning (rule 9); vendor-neutral,
  purpose-named identifiers (rule 8).

## 11. Edge cases

- LP disconnect / no quote for an instrument ⇒ excluded; confidence reflects it.
- All members stale ⇒ `gating_undecidable` / no-price; never publish a stale price.
- Single contributor ⇒ low confidence (per `min_contributors`).
- Divergent LP (fat-finger) ⇒ MAD-excluded; visible in the contributor breakdown.
- LP in multiple books ⇒ independent composites, no cross-talk.
- Instrument quoted by a member but out of an explicit scope ⇒ ignored.
- Book deleted / disabled ⇒ its composites stop publishing cleanly.

## 12. Decisions to confirm (ADR-worthy)

**A — What is an "LP" (member)? — ✅ RESOLVED: inbound liquidity connection
(transport-agnostic).** An LP is any inbound connection into the aggregation service —
**FIX or any other API adapter** — that ingests quotes/pricing and pushes prices into
the book, modelled behind the `VenueFeed` seam. First adapter = FIX RFS
(`FixedIncomeStream`); the trait admits non-FIX adapters without a model change.
(Not the outbound RFQ `celnet-rfq` panel — that stays for on-demand multi-dealer RFQ.)

**B — Admin surface & naming. [?]** **[R]** a 5th Administration tab named
**"Aggregation"**, entity **`AggregatedBook`** (distinct from netting `BookDef`).
Alt: its own top-level workspace. *(Default pending final nod.)*

**C — Ownership & view entitlement. — ✅ RESOLVED: global.** A book is global; any
authenticated user may view its composite (and quote/book off it per their own FI
capabilities). Defining/editing is admin-only. No desk ownership on the book.

**D — Composite instrument key. [?]** **[R]** canonical server `instrument_id`
(`InstrumentDef`) with a mapping from member quotes; vs. the engine's
`Instrument{underlying,tenor}`. *(Default pending final nod.)*

**E — Consumption scope (MVP boundary). — ✅ RESOLVED: full.** MVP includes **display
+ quote + book** off the composite (not display-only). Auto-quote and deal booking
run against the aggregated price.

## 13. Phasing (proposed)

Full scope is in the MVP (decision E); phases are build order, not scope cuts.
- **P0 (this spec + ADRs):** confirm B & D; write ADR(s) for A–E.
- **P1:** `AggregatedBookDef` in `IdentityStore` + admin CRUD RPCs + GUI "Aggregation"
  tab (define book, manage member connections, scope, params, enable). No live composite yet.
- **P2:** wire `celnet-aggregation` into the server; first `VenueFeed` adapter over FIX
  RFS members; publish composite per instrument; FI Streaming view + contributor
  breakdown; validation harness vs. reference.
- **P3:** **quote + book off** the composite — auto-quote off the aggregated price and
  book deals against it (reuse the rates auto-quote + `RatesPositionStore` seam).
- **Later (non-blocking):** a non-FIX API `VenueFeed` adapter (proves transport-agnosticism).

## 14. Acceptance criteria (P1–P2)

- An admin creates an Aggregated Book, adds ≥2 streaming LPs, sets scope + params —
  persisted in `identity.json`, survives restart, admin-gated.
- With ≥2 members quoting an instrument, the book publishes a composite whose best
  bid = max(member bids fresh), best offer = min(member offers fresh), with size,
  depth, mid, confidence, and a contributor breakdown — matching an independent
  reference within tolerance.
- Staleness decay and divergent-LP exclusion demonstrably change the composite as a
  member goes stale / prints an outlier.
- LP disconnect drops it from the composite without interrupting publication; zero
  contributors ⇒ no-price, never stale.
- A desk can **auto-quote** off a book's composite, and a deal **books** against the
  composite price into the rates position store (P3).
- `just check` green; composite validated against a reference, not asserted plausible.

## 15. P1 task breakdown (build order)

P1 = the admin entity + CRUD + GUI tab (no engine wiring yet). Each task is
independently gate-able; complete (no placeholders) before moving on.

- **T1.1 — ✅ DONE (2026-07-22, uncommitted).** Data model + store in
  `crates/celnet-server/src/config/identity.rs` (+595 lines): `AggregatedBookDef` /
  `Scope` (adjacently-tagged `{mode, instrument_ids}`) / `AggregationParams`
  (defaults 500/2500/true/1/1); `IdentityStore.aggregated_books` (`#[serde(default)]`);
  `aggregated_book`/`create_`/`update_`/`delete_aggregated_book`, `mint_aggregated_book_id`;
  load + write validation (`validate_aggregated_books`/`check_aggregated_book`: unique
  ci name, no dup members, explicit-scope instrument ids resolve, `min_contributors≥1`,
  `tau>0`; connection-id resolution deferred to T1.3); 10 unit tests. Gates green:
  `cargo check`/`test config::identity` (31 passed)/`fmt --check`/`clippy -D warnings`.
  Original spec ↓
- **T1.1 (spec).** `AggregatedBookDef { id, name,
  member_connection_ids, instrument_scope, params, enabled }` + `AggregationParams` +
  `Scope` in `config/identity.rs` (or a new `config/aggregation.rs`); additive
  `#[serde(default)] pub aggregated_books: Vec<AggregatedBookDef>` on `IdentityStore`;
  `mint_aggregated_book_id`, `aggregated_book(_by_id)`, create/update/delete helpers;
  **load-time + write-time validation** (unique name case-insensitive; member ids
  resolve to a connection; explicit-scope instrument ids resolve). Unit tests (create/
  rename/dup-reject/validation/persistence-round-trip). Gate: `cargo test -p celnet-server`.
- **T1.2/T1.3/T1.4 — ✅ DONE (2026-07-22, uncommitted, main tree).** Wired end-to-end on
  **`AuthService`** (same `IdentityStore`/`persist_and_commit` as entity/book CRUD):
  - **T1.2 proto** (`crates/celnet-proto/proto/celnet.proto`, additive after `DeleteBookResponse`):
    `AggregationScopeMode` enum, `AggregationParamsDesc`, `AggregatedBookSpec`,
    `AggregatedBookDesc`, `List/Create/Update/Delete AggregatedBook{Request,Response}` +
    4 RPCs on `AuthService`. (Scope carried flat: `scope_mode` + `repeated instrument_ids`.)
  - **T1.3 service** (`services/auth.rs`): `list` (authenticated — global, decision C),
    `create/update/delete` (`require_admin`); wire mappers; validation-string→gRPC-status
    mapping. Tests: CRUD round-trip, dup-name→AlreadyExists, non-admin→PermissionDenied
    (but reads), persistence. gRPC methods + 4 `handle_unary` arms (`ws/mod.rs`).
  - **T1.4 dual codec**: hand (`ws/codec.rs`) + generated (`ws/generated_codec.rs`) +
    null-absent override (`ws/codec_overrides.rs`); **7 byte-identical differential tests**.
  - Gates green: `check -p celnet-proto -p celnet-server`, differential 100/100,
    auth tests 31/31, `fmt --check`, `clippy -D warnings`.
  - ⚠️ **Open:** member `connection_id`s are NOT cross-validated against the FIX registry
    (`AuthEdge` holds only `IdentityStore`); store still validates dedup/params/instrument
    scope. Rejecting unknown member ids needs injecting the fix registry into `AuthEdge`
    (or is deferred to P2 wiring) — confirm.
- **T1.5 — GUI "Aggregation" admin tab.** 5th `AdminTab` in `AdminWorkspace.tsx`; an
  `AggregationPanel` mirroring `RegistryPanels.tsx` (list + create/edit/delete via the
  `run()` wrapper); member-connection multi-select; scope + params form; wsCodec +
  types. Gate: `npm run build`.
- **T1.6 — E2E.** A Playwright admin flow: create book, add ≥2 members, edit, delete;
  persistence survives reload. Gate: t2 GUI e2e.

P2 (engine wiring) and P3 (quote/book) follow once P1 is green. Order dependency:
T1.1 → T1.2 → {T1.3, T1.4} → T1.5 → T1.6.

---

### Appendix — key citations
- Engine: `crates/celnet-aggregation/src/{consolidate.rs,feed.rs,instrument.rs,risk.rs}`
- Admin CRUD template: `gui/src/workspaces/RegistryPanels.tsx`; `crates/celnet-server/src/config/identity.rs` (`BookDef`/`DeskDef`)
- Admin RPC pipeline: `crates/celnet-server/src/services/fix_admin.rs`; `ws/mod.rs` `handle_unary`; `ws/codec.rs` + `ws/generated_codec.rs` (+ `tests/ws_codec_differential.rs`)
- LP plumbing: `crates/celnet-server/src/config/fix_connections.rs`; `crates/celnet-server/src/services/fix.rs` (rates dialect); `crates/celnet-rfq/src/{panel.rs,lp_fix.rs}`
- FI instruments: `crates/celnet-server/src/config/reference_data.rs` (`InstrumentDef`); `gui/src/workspaces/ReferenceDataWorkspace.tsx`
