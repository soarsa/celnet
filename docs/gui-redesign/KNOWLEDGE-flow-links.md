# Integrated Market-Making Loop — code truth (real vs gap)

From the lodestar flow-links capture. This is the HONEST integration state — the redesign presents the
loop as the product/API target but must tag each link **LIVE** vs **GAP** (never fake a wired link).

## The ONE genuinely-integrated loop (LIVE, code-verified)
The **streaming LP-maker** path is real end-to-end:
`Session::handle_subscribe` (`services/stream.rs:1147`) → `attribution::resolve` (`services/attribution.rs:55`:
`quoted_by = maker_auto_pricer()`, `held_by = client seat`) → tick echo `make_snapshot` (stream.rs:490) →
`handle_execute` fill (stream.rs:1370) → `record_booked_position` (stream.rs:897) →
`PositionStore::book_from_attribution` (`services/risk/store.rs:450`). This is the book `RiskService` aggregates.
- **One `SpreadModel`** (`spread.rs`) prices streaming + RFQ + FIX quotes alike. **LIVE**.
- **One versioned `SurfaceBook`** (`surface_book.rs`, AtomicU64) visible to pricing/streaming/FIX simultaneously; a mark pins across all. **LIVE**.
- **Entitlements/desk-scope** thread read-side: `ResolvedCaller.desk_scope` (`services/access.rs:318`) → desk-notify distribution → risk → limits → client → `gui/src/data/riskView.ts`. **LIVE (read-side only)**.

## LP-MAKER vs LP-TAKER vs SALES-TRADER (do NOT compose into one book today)
| Persona | Entry | Books via | Unified book? |
|---|---|---|---|
| **LP-maker** (Celnet streaming) | `handle_subscribe`/`handle_execute` | `PositionStore` (FX) | — this IS the book |
| **LP-taker** (hedge via external LPs) | `MultiDealerEngine::request` / `QuoteEdge::request_multi_dealer_quote`; real `FixLpAdapter` is TEST-ONLY, prod uses synthetic in-process dealers | **nowhere** — `accept_quote` writes an ephemeral `Execution`, never a position store | ❌ GAP |
| **Sales-trader** (inbound client RFQ) | `RfqDeskEdge::submit/respond/accept_desk_quote` (`services/desk/mod.rs`) | `RatesPositionStore` (rates) | ❌ separate silo |

## Integrated vs disconnected — the redesign map
**LIVE:** streaming maker chain · one SpreadModel · one versioned SurfaceBook · read-side desk-scope thread · observability (one-directional sink).
**GAP (design shows the link as a TARGET + surfaces the wiring work — many overlap the D-xva/feed cargo lanes):**
1. **FEEDS → PRICING** — `celnet-integration` blend/divergence has zero runtime callers; live ticks = synthetic `PairProducer::drive` random walk; surface marks manual (not auto-calibrated from feed).
2. **Inventory/book-risk → pricing skew** — no exposure input in `SpreadModel` or the pricing signature.
3. **LP-taking hedge fills → book** — `accept_quote` never writes a position store.
4. **Unified book** — `PositionStore` (FX) vs `RatesPositionStore` (rates) are disjoint types, no unifying view/trait.
5. **Risk/XVA/limits** are pull-on-read, not push; `celnet-xva::compute_xva` has zero non-test callers (scaffold; matches the D-xva deferral).
6. **Fanout** — two impls (`celnet-fanout::BroadcastRing` vs `celnet-server::pricefanout::PriceFanout`) with no edges between; the contribution narrative uses **PriceFanout**.
7. **Entitlements** stop at the server access boundary — pricing/streaming/RFQ compute is entitlement-agnostic; RFQ/desk write-path auth is capability-based, not desk-scoped.

## Design implication
- Present the loop as the **product vision** (and the API-first target); render each link with an honest
  **LIVE / TARGET** state — consistent with the platform's own LIVE/DEFERRED honesty. The redesign's
  cross-asset "one net book" + "feed→pricing" + "risk→skew" + XVA are exactly the high-value links to
  make visible AND to drive toward wiring (coordinate with the D-xva / feed-activation cargo lanes).
- Use `PriceFanout` (not `BroadcastRing`) for the contribution story.

## Upstream flag (lodestar)
Generic-name CALLS-edge resolution (`resolve`/`ctx`/`dispatch`) is unreliable in this graph — ~50
false-positive cross-language callers while MISSING the true prod call site (`stream.rs:1147`); verify
short/common identifiers with `search_code` regex. Candidate `soarsa/lodestar` issue ([[lodestar-no-workaround]]).
