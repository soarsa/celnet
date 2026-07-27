---
name: fi-pricing-groups-shipped
description: FI Pricing Groups COMPLETE + live UAT 3000141 — drag-and-drop feature-pipeline builder (Administration) binding FIX sessions to per-client pricing; PricingFeature library (MidShift/Tiering/Axe/Position/PanicSkew) + per-client ESP/RFQ apply + CRUD + execution provenance waterfall.
metadata: 
  node_type: memory
  type: project
  originSessionId: cf0956fd-d116-49a0-8307-d104e532a50c
---

## COMPLETE + live on UAT `3000141` (2026-07-27). Design: `docs/FI-PRICING-GROUPS-DESIGN.md`.
Outbound pricing = a trader-composable, ordered pipeline of pricing FEATURES drawn from a library;
pricing groups bind FIX sessions/users/desks (1-to-many; desk default) to per-mode (ESP/RFQ, shared
or separate) feature pipelines. Built engine → apply → CRUD → GUI → provenance, all gated.

- **P1 `f122fdb`** — `celnet-tiering` gains `PricingFeature` (MidShift/Tiering{TieringConfig}/Axe/
  Position/PanicSkew) + `FeaturePipeline.run(raw,ctx)->PricedResult{raw,after,outbound,applied_margin,
  applied_skew}` (per-feature waterfall). ADDITIVE — shipped TieringStrategy/TieringConfig unchanged
  (Tiering feature byte-identical to shipped). [[fi-agg-book-rfq-gap-and-tiering-backlog]].
- **P2a `227a4ff`** — `PricingGroupDef` on IdentityStore (members: connection_ids/user_ids/desks;
  esp_pipeline/rfq_pipeline; share_pipeline; enabled) + `PricingGroupResolver` (conn/user/desk maps,
  cached on AggregationHub, rebuilt on reconcile) + apply per-client at the ESP hook
  (`stream.rs handle_aggregated_book_subscribe`/`drive_agg_tick` over `PublishedBook.raw_snapshot`) and
  RFQ hook (`quote.rs book_composite_for_caller`). No group ⇒ book-default (byte-identical).
- **P2b `07fc99f`** — proto/WS + CRUD: FeatureKind/AxeSide/EspOrRfq enums, FeatureSpecDesc/
  FeaturePipelineDesc/PricingGroupDesc; AuthService RPCs ListPricingGroups (auth read),
  Create/Update/DeletePricingGroup (admin), **UpdatePricingGroupPipeline (trader,
  quote_respond·fixed_income)**. Byte-identical hand+generated codecs.
- **P3 `b240c7b`** — drag-and-drop builder under **Administration → Pricing Groups**
  (`gui/src/workspaces/PricingGroupsWorkspace.tsx`, `PricingFeatureCard.tsx`, `lib/pricingGroups.ts`):
  feature palette, native HTML5 DnD (no new deps, keyboard-accessible), RAW▸features▸OUTBOUND canvas,
  inline per-feature config (TIERING reuses TieringEditor), ESP/RFQ+share toggle, membership, LIVE
  client-side preview waterfall.
- **P2c `0509e48`** — `PricingProvenance` (pricing_group_id, mode, raw/constructed/tiered/outbound
  waterfall, applied_margin, applied_skew, features[]) derived from PricedResult, stamped on Quote at
  request_quote → copied onto Execution at accept_quote → Deal; additive proto/WS. No-group ⇒ None
  byte-identical. Analytics: markout = executed vs raw_mid, decomposable per feature/group/client.
- **`3000141`** — fixed a stale `celnet-lp-sim/tests/book_resolver.rs` (missing `AggregatedBookDesc.tiering`,
  latent since the tiering work — we gate `--lib` so the heavy integration binaries weren't compiled).

## Gotchas
- Gate `--lib` (the integration-test binaries ENOSPC the box); the workspace `cargo check --all-targets`
  can surface stale integration tests — sweep them periodically.
- Deferred seams (not built): the rates-RFS hook (`stream.rs:1476`, FIX-connection callers) is not yet
  group-priced; per-client SCALE_SMOOTH EWMA state uses the indicative fallback. Provenance on the
  rates dealer-quoting Deal path is honestly None (not group-priced).
- UAT deploy remedy (undersized box) → [[fi-agg-book-rfq-gap-and-tiering-backlog]].
