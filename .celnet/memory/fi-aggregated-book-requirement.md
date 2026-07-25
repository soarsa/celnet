---
name: fi-aggregated-book-requirement
description: New FI Aggregated Book requirement — admin-defined book compositing price-per-instrument across N liquidity connections; reuses the unwired celnet-aggregation engine.
metadata: 
  node_type: memory
  type: project
  originSessionId: fc815517-2b4d-4d65-96c9-8795d695a3f6
---

New requirement (2026-07-22): a Fixed-Income **Aggregated Book** — an admin-defined,
**global** entity that composites **price per instrument** across a set of member
**liquidity connections**, defined/managed in the Administration tab. Requirements
draft: `docs/FI-AGGREGATED-BOOK-REQUIREMENTS.md`.

**Key finding:** the compositor already exists but is 100% **dead/unwired** —
`celnet-aggregation` (`ConsolidatedBook`: best-bid/offer + sizes + depth + confidence,
`2^{-Δt/τ}` staleness decay, MAD outlier gating; `VenueFeed` trait; only `SimVenue`
concrete feed). This feature is mostly *plumbing that engine in*, not new math.

**Resolved decisions (operator):**
- **LP = an inbound liquidity connection, transport-agnostic** (FIX *or any API adapter*)
  behind the `VenueFeed` seam. "Left side ingests quotes/pricing, right side pushes the
  composite into the book." First adapter = FIX RFS (`FixedIncomeStream`). NOT the
  outbound `celnet-rfq` RFQ panel.
- **Full-scope MVP: display + quote + book** off the composite (not display-only).
- **Global books** — any authenticated user views; admin-only to define; no desk ownership.

**Still defaulted (need final nod):** B = 5th Admin tab "Aggregation" + entity name
`AggregatedBook` (distinct from netting `BookDef`); D = key composite on server
`instrument_id` (`InstrumentDef`) vs engine `Instrument{underlying,tenor}`.

**Build order:** P1 `AggregatedBookDef` in `IdentityStore` + admin CRUD (mirror
`FixAdminEdge` + `RegistryPanels.tsx`) → P2 wire engine + FIX-RFS `VenueFeed` + publish
composite → P3 auto-quote + book off it. Reuse: `RegistryPanels.tsx`, `identity.rs`
`BookDef`/`DeskDef`, `fix_admin.rs` proto→handle_unary→dual-codec→registry→GUI pipeline.
Related: [[fi-desk-routing-and-asset-separation]] [[fi-rfs-streaming-and-booking]].
