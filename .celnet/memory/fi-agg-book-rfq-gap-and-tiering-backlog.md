---
name: fi-agg-book-rfq-gap-and-tiering-backlog
description: "Tiering vertical DONE + live UAT (fde21ae) — engine (Flat/InventorySkew/SCALE_SMOOTH) + per-book config + composite apply + FI-tab config screens + per-strategy doc links. ONLY remaining — Phase 2b — wire inbound RFQ to price against the admin book, not the synthetic-demo panel."
metadata: 
  node_type: memory
  type: project
  originSessionId: cf0956fd-d116-49a0-8307-d104e532a50c
---

## DONE + live on UAT `fde21ae` (2026-07-25) — the FI outbound tiering vertical
Design/math: `docs/FI-TIERING-RESEARCH.md`. Built as: engine → server integration → GUI, all gated.
- **`celnet-tiering`** crate (`297781c`): pure `TieringStrategy` + `FlatMarkup`, `InventorySkew`,
  `ScaledSmoothedSpread` (SCALE_SMOOTH — EWMA-smoothed spread from the PDF); `quote()` pipeline;
  guardrails (anti-cross via `offer−bid=2h` skew-invariance); `SpreadUnit` + DV01 conversion.
- **Server** (`67e4bee` + `fde21ae`): `TieringConfig` persisted on `AggregatedBookDef` (admin CRUD +
  validate); applied on the composite publish path (`services/aggregation.rs::apply_tiering`) — mid
  from raw composite, inventory via `InventorySource` seam, DV01 from `celnet_bond` for YieldBps,
  SCALE_SMOOTH EWMA state kept per-book/instrument in `BookState.smoothed_spread` (strategy stays
  pure). No-config path byte-identical. Additive proto/WS (`TieringStrategyKind` + descs), byte-
  identical hand+generated codecs.
- **GUI** (`5dd9cbe` + `fde21ae`): per-book `TieringEditor` under Fixed Income (admin-only Manage
  mode on the Agg Book workspace); all 3 strategies + guardrails + validation; per-strategy "?" doc
  links → `docs/FI-TIERING-RESEARCH.md` §9. Also fixed a pre-existing prod bug (step/min mismatch
  silently blocked ALL agg-book submits).

## ONLY REMAINING — Phase 2b (fresh session)
Wire inbound **RFQ/`QuoteService`** (`crates/celnet-server/src/services/quote.rs`) to price against the
admin-defined book's `member_connection_ids`/composite, NOT the synthetic-demo panel
(`LpPanelConfig{synthetic_lps}` from env `CELNET_DEMO_LPS`) — the VERIFIED gap. Admin book definition +
streaming + member-LP ingest already ✅; the tiering engine now also tiers the composite it prices off.

## UAT box gotcha (deploy)
The UAT box `/dev/sda1` is 9.7G, ~7G permanently OS (celnet user has NO sudo → can't reclaim it), so it
sits near-full and the deploy's `npm ci` / binary-copy hits ENOSPC at 99%. Remedy: `cd deploy && ansible
uat -m shell -a '<reclaim>'` — delete old release dirs EXCEPT `readlink -f /opt/celnet/current`, plus
`/opt/celnet/build/gui/node_modules` (npm ci rebuilds) + `~/.npm` + `~/.cache` — then re-run `release`
(Rust `target/` is preserved for incremental; the post-publish cache-reclaim only runs on SUCCESS, which
is why failed deploys pile up). The box really needs a bigger disk/swap. See
[[deploy-ssh-drop-on-silent-build]], [[fi-bond-terms-and-govvie-feed-shipped]], [[fi-aggregated-book-shipped]].
