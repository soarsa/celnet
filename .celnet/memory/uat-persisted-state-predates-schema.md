---
name: uat-persisted-state-predates-schema
description: "UAT's identity.json outlives code changes — persisted config keeps RETIRED ids and misses NEWLY-ADDED fields, so a feature looks broken while the code is correct. Bit twice on 2026-08-14/15 (LP-SIM-0N member ids; empty sub_asset_type on 343 instruments)."
metadata: 
  node_type: memory
  type: project
  originSessionId: 832024c0-a3e6-45ac-b329-511c2dfee9f6
  modified: 2026-08-15T16:43:24.803Z
---

`/home/celnet/identity.json` on UAT persists CONFIG (users, desks, books,
`aggregated_books`, `instruments`, pricing/hedge graphs) and is **not** rewritten by a
deploy or a restart. So it silently drifts from the code in two directions, and both
present as "the feature is broken" when the code is fine:

1. **Retired identifiers survive.** The LP panel roster was renamed from `LP-SIM-01..0N`
   to the named OTC counterparties (`marketaccess-sim`, `traderweb-sim`, `citigroup-sim`,
   `jpm-sim`; futures split to `cme-sim`). The `ust` book still named the old ids, so
   `celnet-lp-sim`'s resolver — "a book whose `member_connection_ids` name none of our
   impersonated members is ignored" (`crates/celnet-lp-sim/src/books.rs`) — stood up
   **0 streams**. Fix is a data edit (Administration → Admin → Aggregation → Edit), not a
   deploy. Verify on the box: `tail /opt/celnet/shared/logs/lp-sim.out.log` should read
   `poll plan: N stream(s) over 4 member(s)`, not `0 stream(s) over 0 member(s)`.
   It arrived via the 2026-08-14 reset, which restored `aggregated_books` **verbatim from a
   pre-rename backup** and called it "known-good plumbing".

2. **Newly-added fields stay empty.** `94ba3fad` added `sub_asset_type` + `region` to
   `InstrumentDefDesc`. The 343 instruments already persisted carry `sub_asset_type: ""`
   and the server does **not** backfill from `celnet_refdata` on load — so anything that
   groups by the taxonomy (the Aggregation asset-type tabs) sees one `Unclassified` bucket.

**Check before debugging code:**
`ssh celnet@34.26.86.222 'python3 -c "import json;d=json.load(open(\"/home/celnet/identity.json\"));print([k for k in d])"'`
then inspect the relevant collection for stale ids / empty new fields.

**Why it matters:** twice now the visible symptom pointed at code that was already correct.
A recorded diagnosis ("known-good plumbing") was itself the bug. Related:
[[uat-reset-dv01-hedging-acceptance]], [[uat-login-identity-store-and-reset]],
[[run-dev-single-entry-point]].
