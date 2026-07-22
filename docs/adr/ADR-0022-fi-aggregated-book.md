# ADR-0022 — FI Aggregated Book (admin-defined multi-LP price consolidation)

**Status:** Accepted (decisions A, C, E by operator 2026-07-22; B, D as recommended defaults)
**Context doc:** `docs/FI-AGGREGATED-BOOK-REQUIREMENTS.md`
**Supersedes / relates:** ADR-0018 (FI leaf), ADR-0020 (cross-asset pricing/risk contract)

## Context

FI prices for one instrument arrive from multiple liquidity providers on separate
feeds. We need an operator-defined **Aggregated Book**: a named set of LPs whose
per-instrument top-of-book is consolidated into one composite price (best bid/offer +
size + depth + confidence), managed from the Administration surface, and usable for
display, quoting, and booking.

The consolidation engine **already exists and is unwired** — `celnet-aggregation`
(`ConsolidatedBook`, `VenueFeed`, staleness decay `2^{-Δt/τ}`, MAD divergence gating).
This feature primarily *wires that engine in* behind a new admin entity.

## Decisions

- **A (Member = inbound liquidity connection, transport-agnostic).** An LP is any
  **inbound** connection into the aggregation service — **FIX or any other API
  adapter** — modelled behind the existing `celnet-aggregation::VenueFeed` seam
  (ingest side receives quotes/pricing; output side pushes the composite into the
  book). The first adapter backs onto the FIX RFS ingest (`FixedIncomeStream` →
  `services/fix.rs` → `RatesPositionStore`); a non-FIX API adapter slots in behind the
  same trait with no model change. Members are referenced as **connection ids**, not a
  FIX-specific type. *Rejected:* modelling members as the outbound `celnet-rfq` RFQ
  panel (that stays for on-demand multi-dealer RFQ, a different flow).

- **B (Admin surface & naming).** A **5th Administration tab, "Aggregation"**, with
  entity **`AggregatedBook`** — deliberately distinct from the existing netting
  `BookDef` (an accounting partition) to avoid conflation. Mirrors the
  `RegistryPanels.tsx` list+form CRUD pattern.

- **C (Ownership = global).** An aggregated book is **global**: any authenticated user
  may view its composite and (subject to their own FI action capabilities) quote/book
  off it. Defining/editing is **admin-only**. No desk ownership on the book.

- **D (Composite instrument key).** The composite is keyed on the **canonical server
  `instrument_id`** (`InstrumentDef`), with a bridge mapping member quotes → the
  engine's `Instrument{underlying,tenor}`. Keeps one instrument identity across
  reference data, GUI, and the composite.

- **E (Scope = full).** MVP delivers **display + quote + book** off the composite, not
  display-only. Auto-quote and deal booking run against the aggregated price, reusing
  the rates auto-quote + `RatesPositionStore` booking seam.

## Consequences

- `celnet-server` gains a dependency on `celnet-aggregation` (correct one-way
  direction; the aggregation crate stays edge-free / no I/O).
- New persisted `AggregatedBookDef` in `IdentityStore` (`identity.json`), additive
  `#[serde(default)]`, validated at load and every admin write (member connection ids
  and any explicit instrument ids must resolve), mirroring `EntityDef`/`BookDef`.
- New admin RPCs `Create/Update/Delete/ListAggregatedBook` following the `FixAdminEdge`
  pipeline (proto → `handle_unary` + gRPC → dual WS codec + differential test →
  `IdentityStore` → GUI).
- A new `VenueFeed` adapter over the FIX RFS ingest; the engine is instantiated
  per enabled book and publishes composites on a bounded, offloaded path (hot core
  stays alloc/lock/log-free).
- Composite correctness is validated against an independent reference (synthetic
  ground-truth LP ladders), never merely asserted.

## Open (non-blocking) follow-ups

- A non-FIX API `VenueFeed` adapter (proves transport-agnosticism) — later.
- Depth beyond top-of-book — later; top-of-book first.
