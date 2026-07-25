---
name: fi-bond-terms-and-govvie-feed-shipped
description: Agg Book tiles now show bond static terms (issuer/coupon/freq/day-count/maturity) joined client-side from the registry; LP-SIM streams the FULL government_universe (US + UK/EUR govvies); download-taxonomy docstring overclaim corrected. Live UAT 67cfdc2.
metadata: 
  node_type: memory
  type: project
  originSessionId: cf0956fd-d116-49a0-8307-d104e532a50c
---

Shipped 2026-07-24 (origin/main `67cfdc2`, deployed UAT `67cfdc2-20260724T124123Z`,
server RUNNING). Three review follow-ups on the celnet-refdata static-data slice:

1. **GUI bond terms on the FI Aggregated Book** (commit `50bcc7e`). The composite wire
   message carries only `displayName`/`isin`/`cusip`, so the tile now joins the full
   `InstrumentDef` client-side (via `listInstruments`, keyed by instrument_id → ISIN →
   CUSIP) and renders issuer/coupon%/frequency/day-count/maturity as an accessible
   `<dl>`. New `gui/src/lib/bondTerms.ts` seam + `gui/test/bondTerms.test.ts` (12 cases).
   `mockSource.ts` seeds the curated bonds so `?mock` shows it offline. NO proto/wire change.

2. **LP-SIM prices the full government universe** (commit `67cfdc2`). Server seeds the whole
   `celnet_refdata::government_universe()` (US + UK gilts + EUR govvies) but LP-SIM only
   priced US Treasuries → non-US names showed with no price. Added `TreasuryBond::from_gov_spec`
   + `load_curated_universe`/`load_government_universe` building real `celnet_bond` schedules
   for the curated govvies. **Treasury feed (wire ids/CUSIP/prices) is byte-unchanged.**

3. **Doc fix**: celnet-refdata `lib.rs` + `government_bond_defs` claimed the FIX/RFS
   security-list download "can filter by region/sub-asset-type at download time" — NO such
   filter is wired (list answered verbatim). Docs corrected; `region`/`sub_asset_type` stay on
   `GovBondSpec` for future filtering. See [[fi-aggregated-book-shipped]].

Gotcha this session: `GITHUB_TOKEN` env var is stale/invalid and shadows the valid keyring
token — git/gh fail until you `env -u GITHUB_TOKEN <git cmd>`. Also the M4 root volume filled
(APFS) during a cold cargo build after `target/` purge; a parallel cargo + npm agent will OOM
the disk — keep `target/` pruned and watch `df`. Deploy: `./deploy/celnet-deploy.sh -t uat
release` (non-interactive; `full` needs interactive sudo and is only for first-time setup).
