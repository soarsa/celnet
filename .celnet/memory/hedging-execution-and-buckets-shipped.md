---
name: hedging-execution-and-buckets-shipped
description: Hedging upgrade (live execution seam + portfolio/bucket-scoped policies + hedge-desk hedging-only filter) COMPLETE on local main 7d7a32e5; UAT deploy blocked only by box OOM on Vite + background-command kills.
metadata: 
  node_type: memory
  type: project
  originSessionId: bd2a0685-2d52-4bdc-95d4-4e7872001f3e
---

Shipped 2026-08-10 on **local main** (NOT pushed to origin — local-only directed). Two commits:
- `05d8cd23` feat(gui): numeric inputs select-on-focus + k/m/b magnitude shorthand (5m=5,000,000, 1k=1,000, 2b=2,000,000,000). One document-level installer `gui/src/lib/numberInputAutoSelect.ts` wired in `main.tsx` covers all ~180 number inputs (native number inputs reject letters, so k/m/b is intercepted on keydown and pushed via React's value setter).
- `7d7a32e5` feat(hedging): the big one, three parts:
  1. **Live execution seam** — replaced `HedgeConfigDef.advisory_only` with `HedgeExecutionMode {Advisory,LpPanel,Composite,LpPanelThenComposite}` (default LP→Composite) + `composite_spread_bp`. New `crates/celnet-server/src/services/auto_hedge/executor.rs` prices an external shed off the Agg-Book composite mid (spread worsens correct side by net-risk sign), books a REAL offsetting leg so book net actually reduces, stamps real provenance (advisory=false, real hedge_price/mid/signed slippage_bp/lp_won). `NoLpSource` honestly misses → LpPanelThenComposite hits composite backstop (no fabricated fill). Added `ExitAction::ClearRisk` (flatten whole net).
  2. **Portfolio/bucket policies** — `HedgePolicyScope {Firm,Book,Bucket}` + `ScopedHedgeGraph`; `select_hedge_policy_graph` = most-specific (Book → nearest-ancestor Bucket → Firm). Bucket policy's HedgeContext built off subtree roll-up (`subtree_net_dv01`) so "PORTFOLIO notional > n → market order / clear risk" works. Scope-aware wire on the 4 HedgePolicyGraph messages (scope_kind/scope_id).
  3. **Hedge-desk filter** — `HedgeDealsView` shows only `is_external()` hedges (kinds 3,4,5,7) with a "Show internalised" toggle (default off); mock transport no longer emits Warehouse rows (synthesizes live composite hedges). GUI exec-mode selector + composite spread live on Hedging→Monitor; Firm/Book/Bucket scope selector on Hedging→Exit Policy.

Gates GREEN: fmt, clippy -D warnings (fixed 2 new + 3 pre-existing trace.rs lints), cargo test 771 passed/0 failed, GUI build + 45 hedge unit tests. Independently verified (celnet-verifier GO): composite price/slippage sign, net-risk reduction, ClearRisk full-flatten, subtree rollup, Warehouse invariant, wire parity, no mocks.

**DEPLOY STATE:** UAT box has the new **Rust binaries built** (server/lp-sim/fix-sim) from a prior run; deploy repeatedly dies at the **Vite GUI build** (box OOM, swapless 3.8Gi) AND my **background Bash ansible runs get killed by a runtime ceiling** before the ~6-11min remote build finishes. FIX: run the deploy **in-session via `!`** (foreground — that's how the session-start deploy succeeded), not a background task:
`! cd /Users/benjamincuthbert/code/soars/celnet/deploy && env -u GITHUB_TOKEN ansible-playbook deploy.yml --limit uat -e celnet_cargo_build_jobs=1`
It resumes incrementally + finishes Vite → symlink swap → restart. See [[deploy-ssh-drop-on-silent-build]] + [[uat-deploy-recovery-and-hazards]].

FOLLOW-UP FIX `54b077aa` (2026-08-10): below-min-edge fills are shed externally in full but the per-fill provenance was stamped with the graph's book-band action (Warehouse when GREEN) → LIVE HEDGE DESK showed self-contradictory rows (Warehouse (hold) + live COMPOSITE hedge + slippage) and the is_external filter hid these real back-to-backs (a "B2B" client deal had no matching hedge deal). Fix in `rates_book.rs` stamp_internalise: when a fill sheds externally but the graph action is an internal hold, stamp `SubmitMarketOrder{Fixed(external_dv01),Immediate}` via `exit_action_to_wire` so the row is truthful + the filter keeps it. Gates green (721 passed). HEAD now `54b077aa`.

Two OPEN follow-ups (verifier, non-blocking): (1) on a book-band BREACH path the desk record keeps the pre-stamped honest-zero book-level price instead of surfacing the real composite fill price (risk reduction unaffected; `rates_book.rs` stamps real record only `if outcome.provenance.is_none()`); (2) none — the cosmetic wsCodec `0..6` comment was fixed.
