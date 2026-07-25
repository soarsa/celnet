---
name: fi-agg-book-rfq-gap-and-tiering-backlog
description: "VERIFIED gap — admin agg-book streams off its real member LPs but inbound RFQ prices against a synthetic-demo panel (CELNET_DEMO_LPS), NOT the book. Fresh-session backlog — wire RFQ→book + build celnet-tiering + FI-tab tiering UI."
metadata: 
  node_type: memory
  type: project
  originSessionId: cf0956fd-d116-49a0-8307-d104e532a50c
---

Verified 2026-07-24 (UAT on `bc7e9a6`). The FI aggregated-book chain is 3/4 wired; the 4th
link is the real gap the operator flagged ("define book → stream → subscribe LPs → handle
inbound requests to price"):

- **Admin-only definition** ✅ — `create/update/delete_aggregated_book` gated to admins
  (test `aggregated_book_admin_gate_denies_non_admin`, `services/auth.rs:3679`).
- **Streams a composite** ✅ — `AggregationHub` + `AggregatedBookSubscribe` on the stream edge
  (`services/stream.rs`); the GUI Agg Book view reads it.
- **Ingests its member LPs** ✅ — `services/aggregation.rs::ingest` routes each `LpQuote` into
  every enabled book listing that LP in `member_connection_ids`.
- **Inbound RFQ against the book** ❌ **GAP** — `services/quote.rs` (`QuoteService`, the
  RFQ/multi-dealer path) has ZERO refs to the aggregated book. Its panel is
  `LpPanelConfig { synthetic_lps: u32 }` sourced from env `CELNET_DEMO_LPS` — synthetic demo
  dealers, NOT the book's real member LPs/composite. So streaming and RFQ pricing use two
  disconnected aggregation concepts.

**Fresh-session backlog (same subsystem — do together; this one is deferred, ~$174 spent):**
1. Wire RFQ/`QuoteService` to price against the admin-defined book's `member_connection_ids`/
   composite instead of the demo panel.
2. Build the new `celnet-tiering` crate (Flat markup + Inventory-skew FIRST, then vol/size/
   toxicity/client-tier) on the composite seam — full design in `docs/FI-TIERING-RESEARCH.md`.
3. FI-tab **Tiering** admin UI (currently absent — only the research doc exists, no code/UI).

Both (1) and (2) share the aggregation composite seam (`services/aggregation.rs` publish path).
See [[fi-aggregated-book-shipped]] and [[fi-bond-terms-and-govvie-feed-shipped]]. Session also
shipped: agg-book bond-terms, lp-sim full govvie universe, New-Instrument portal modal,
new-version-refresh modal (all live on `bc7e9a6`).
