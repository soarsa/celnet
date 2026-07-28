---
name: session-pivoted-tiering-shipped
description: "FI Tiering page is now SESSION-pivoted (roster of FIX sessions → the pricing group applied to each); per-book tiering fully REMOVED (groups-only). Branch feature/session-pivoted-tiering aded914, all gates green."
metadata: 
  node_type: memory
  type: project
  originSessionId: b69ceb7d-66d6-4bc7-ac17-96e24adc4293
---

2026-07-28: The FI **Tiering** page was book-pivoted (edit an aggregated book's `TieringConfig`, applied once to the whole composite for every subscriber). Per user direction it is now **session-pivoted** and per-book tiering is **removed entirely** — tiering lives ONLY in per-session pricing groups. Supersedes the per-book tiering in [[fi-agg-book-rfq-gap-and-tiering-backlog]].

**Decisions (user-approved):** groups-only (remove book tiering UI + server); assign + deep-link to edit (not inline editor); page stays under **Fixed Income** (`quote_respond·fixed_income`); do both (GUI + server removal) in one change.

**GUI** — `gui/src/workspaces/TieringWorkspace.tsx` rewritten: left roster = FIX sessions from `listFixConnections()`; each resolved to its group among ENABLED `listPricingGroups()` by `memberConnectionIds.includes(session.id)` (exact) → else `memberDesks.includes(session.desk)` (desk-default) → else none — MIRRORS the server `PricingGroupResolver`. Detail pane: resolved group + how-matched + a READ-ONLY tiering summary read from the group's `espPipeline`/`rfqPipeline` (the `Tiering` feature's strategy/params/guardrails) + the full feature-chip waterfall. **Assign** = a group `<select>` that calls the ADMIN `updatePricingGroup(id, spec)` to move `session.id` between groups' `memberConnectionIds` (gated on `auth.isAdmin`; non-admin sees read-only). Deep-link = `app.setWorkspace("pricinggroups")`. Book-tiering wire seam removed from `contract.ts`/`wsTransport.ts`/`transport.ts`/`wsCodec.ts`/`mockSource.ts` — **kept `FeatureSpec.tiering`** (the group feature path) and the `TieringConfig`/`TieringEditor` types.

**Server** — removed the dead book path: proto `UpdateBookTiering{Request,Response}` + AuthService rpc + `AggregatedBook{Spec,Desc}.tiering`; both WS codecs (`ws/codec.rs` hand + `ws/generated_codec.rs` descriptor-driven `WireBuilder`/`WireAdapter` impls + `codec_overrides.rs`) + the `ws/mod.rs` dispatch arm; `AuthEdge::update_book_tiering`; `AggregatedBookDef.tiering` + `update_aggregated_book_tiering`; `aggregation.rs::apply_tiering` + its 7 book-only numerical tests + the quote.rs `flat_book_hub` tiering. **Kept** `celnet_tiering::TieringConfig`/strategy math + the pricing-group path (`services/stream.rs`/`quote.rs` FeaturePipeline apply). The two group-path aggregation tests were UPDATED (no-group path now publishes the RAW composite 99.50/99.60, not a book-tiered line). `tiering_to_wire`/`tiering_from_wire` are SHARED (feature path) — kept; `agg_tiering_spec()` test helper is SHARED with pricing-group tests — kept.

**Gotcha for future removals of this shape:** `tiering` appears in THREE places — `AggregatedBook*` (book, remove) vs `FeatureSpec.tiering` (group feature, KEEP) vs the shared `TieringConfig`/converters (KEEP). Classify each site before deleting. `generated_codec.rs` is descriptor-driven from `celnet-proto` `wire_contract` (`build.rs`) but has hand-committed per-message decode/encode fns + `WireBuilder`/`WireAdapter` impls that must be hand-removed too; the `ws_codec_differential` harness proves both codecs stay byte-identical.

**Gates (this machine; nextest ABSENT here — use `cargo test`):** `cargo test -p celnet-server -p celnet-proto -p celnet-tiering` → 510 lib + all integration + differential green; `cargo clippy -p celnet-server -p celnet-proto --all-targets -- -D warnings` clean; `npm --prefix gui run build` + `npm --prefix gui test` (1291 vitest) green. Committed `aded914` on `feature/session-pivoted-tiering`.
