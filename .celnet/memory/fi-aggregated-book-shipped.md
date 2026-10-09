---
name: fi-aggregated-book-shipped
description: "FI Aggregated Book + LP-SIM liquidity feed — full stack built, gated, live on UAT (origin/main 9912e8c). How to run it."
metadata: 
  node_type: memory
  type: project
  originSessionId: fc815517-2b4d-4d65-96c9-8795d695a3f6
---

FI **Aggregated Book** + **LP-SIM** feed shipped end-to-end and LIVE on UAT
(2026-07-23, `origin/main` `9912e8c`; `celnetapp.uat.celnet.uk`). Completes the
requirement in `docs/FI-AGGREGATED-BOOK-REQUIREMENTS.md` + ADR-0022. Layers:

- **Server (D3)**: `LiquidityFeedService.LpFeed(stream LpQuote)` gRPC ingest →
  `AggregationHub` (per-enabled-`AggregatedBookDef` `ConsolidatedBook`, latest-quote
  sink, params→config) → composite published over the bidi `StreamService.StreamSession`
  as `AggregatedBookSnapshot`/`Update` (subscribe via `aggregated_book_subscribe`).
  Files: `celnet-server/src/services/{aggregation,liquidity_feed}.rs`, `ws/*`. Proven by
  `tests/aggregation_ingest_ws.rs`.
- **GUI (D4)**: `Administration → Aggregation` admin tab (book CRUD) + FI **"Agg Book"**
  workspace (live composite grid: bond identity/ISIN/CUSIP + BBO + confidence +
  expandable per-LP contributions + stale flags). `gui/src/workspaces/Aggregat*`.
- **Sim**: `celnet-lp-sim` — book-aware `lp-sim` binary runs 5 LPs (`LP-SIM-01..05`,
  distinct seeded character), authenticates, polls `ListAggregatedBooks`, and prices
  exactly the bonds each enabled book scopes to (create/edit a book ⇒ priced next poll).
  Real US-Treasury universe bundled at `crates/celnet-lp-sim/data/treasury-universe.json`
  (267 bonds, from celnet-treasury-data-fetcher). Broadcast fallback: `--no-book-poll`.

**Run it:** GUI → Aggregation → create book w/ members `LP-SIM-01..05`, scope
all-members-quote, enabled → on host start the daemon (release ships `start-lp-sim.sh`;
`lp-sim --addr http://127.0.0.1:50051 --members 5`, admin seed login) → FI → Agg Book →
select book → composite streams. **Live e2e verified:** 5 LPs × 91 priceable Treasuries
= 455 quotes accepted through the running server into the book.

**Gotcha (Scope serde):** `instrument_scope` is adjacently-tagged — all-members-quote is
`{"mode":"all_members_quote"}` (NO `instrument_ids`); explicit is
`{"mode":"explicit","instrument_ids":[...]}`. **Numeric:** the sim's per-member yield
dispersion is kept ~1.5bp (`skew_step 4e-3`, `yield_dispersion 1.5e-4`) so fresh
competing LPs don't trip the server's absolute divergence gate on long-duration bonds
(the gate is for outliers). Related: [[fi-aggregated-book-requirement]]
[[fi-desk-routing-and-asset-separation]].
