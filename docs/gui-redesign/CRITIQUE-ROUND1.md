# Shell 00 — UX / Intuitivity critique (round 1), verdict "Nearly"

## Fixes (ranked)
1. **Contribution live-vs-draft state (HIGHEST)** — add per-instrument badge `LIVE ▸ Tier 1–2` vs `DRAFT · unpublished` on the contributed-price panel; mirror the surface dirty-state.
2. **Collapse the 4-verb CTA thicket** — one primary `Contribute ▸ Tier 1 ▾` that flips to `Update live / Pull` when live; CUT "Add to structure" from this row; demote "Send to RFQ desk"; fold "Publish to stream" into primary.
3. **Feed Manage mis-grouped** — it's INBOUND market data; make a `MARKET DATA` group, leave DISTRIBUTE outbound-only.
4. **Always-visible contribution kill switch** — make the right-rail `CONTRIBUTION · live` pill the master live/pull toggle.
5. **License model** — dimmed chips read "not my desk" not "buy this"; add upsell; unify gating grammar (chips-dim vs rail text-tag); CUT status-bar "Licensed:" duplicate.
6. **Redundant echo** — CUT status-bar scope/perspective/license; reserve status bar for ops/throughput only.
7. **Strip verbose copy** — CUT surface paragraph; CUT "· class-appropriate"; shorten "Open surface · mark ▸ publish" → "Open surface".
8. **Skew direction/lean** — show which way leaned, e.g. `leaned ▸ offer −0.04v to reduce +Δ inventory`, not just magnitude.
9. **CUT NEW badges + arbitrary Unicode rail glyphs** — noise; use meaningful icons or drop icons.
10. **Disambiguate "Publish"** — reserve for surface (`Publish surface`); stream = `Go live / Contribute`.
11. **Ambiguous chips → controls or cut** — `RISK-BASED SPREAD` → `Risk-based ▾ / Fixed` selector or cut.
12. **Stale feed color** — grey → AMBER (grey reads as off/disabled).

## KEEP
bid/offer (not buy/sell) framing · spread breakdown (mid→½-spread→skew-why) · Desk▸Book scope + book-scoped right rail · command-palette top bar · vol-surface signature (version/vs-mkt/arb-gated/no-unmarked-edits) · scope-aware right rail · model-first hero · always-visible ops p50/p99/p99.9 · progressive density.

## Verdict
Nearly. Integrated ✓, market-making ✓. Short of intuitive/clean on: (a) no live-vs-draft price state, (b) 4-verb CTA thicket, (c) verbosity + NEW badges + redundant echo + Feed mis-grouped. Highest-impact = fix #1 (explicit singular contribution live-state + one Contribute/Pull control).
