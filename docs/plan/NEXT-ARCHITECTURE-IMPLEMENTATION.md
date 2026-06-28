# Master implementation plan — finish the optimal-architecture program (lossless resume)

> **Purpose.** This is the **single self-contained entry point** for fully implementing the
> remaining optimal-architecture program via dynamic workflows. It is written so a **fresh
> context** (no memory of the session that produced it) can execute everything without
> re-deriving. Authored 2026-06-27, on `main @ 443ec9b`. Source of truth for scope:
> `docs/ARCHITECTURE-DETERMINATION.md` §3 (11 prioritized refactors), `docs/SECURITY-AUTHZ-FINDING.md`
> (§2/§3 follow-up), `docs/plan/CARRY-SEAM-TO-EDGE.md` (the #1 target, fully scoped).

## 0. Read-first (hard constraints — every workflow agent must honour)
- **CLAUDE.md guardrails are law.** No mocks/placeholders/`todo!()`; only 100% complete impls
  (narrow scope, never fake depth). **Every change passes the gates** before "done": `just check`
  (fmt, clippy `-D`, nextest, deny); numerical code validated against an **independent** oracle
  (QuantLib/published/code-disjoint re-derivation), never "asserted plausible".
- **Tiered gates** (`docs/PARALLEL-SESSIONS.md` §4.2): T0 `cargo check -p` per edit; **T1** one
  multi-`-p` invocation per lane-batch (accumulate edits, never gate per-fix); **T2** once per
  push milestone = full check + **both live GUI/Excel e2e under `CELNET_ACCESS_MODE=enforce`**.
- **lodestar-first** for discovery (`search_graph`/`trace_path`/`get_code_snippet`/`query_graph`);
  the graph auto-indexes. **Caution: do NOT re-ground stale claims via `knowledge_put`** — it
  duplicates (claim-key includes node_content_hash; lodestar **#27**). If claims go stale after a
  merge, leave them; the SessionStart sync + a fresh clone rebuild cleanly from the committed mirror.
- **API-first parity:** every capability lives in the one **unversioned** contract; GUI/SDK/CLI/Excel
  + docs consume it and evolve in lockstep (no `schema_version`; additive proto oneofs default to FX).
- **FX byte-identity is the no-regression gate** for any carry/pricing change
  (`fx_carry_inputs_byte_identical`, `forward_and_df_match_core_carry_inputs_byte_for_byte`); extend
  it to new paths. **No `match carry {…}` in payoff code** (ADR-0008 review-blocker).

## 1. Operational lessons (do not relearn the hard way)
- **Single M4 → serialize heavy cargo.** Only **toolchain-disjoint** lanes parallelize (one cargo +
  N node/TS). Two concurrent heavy cargo builds contend and time out. `celnet-gpu` tests need
  `--test-threads 1`. nextest can wedge — prefer `cargo test` for the heavy server suite.
- **`isolation:worktree` agent lanes base on `main`, NOT the checked-out feature branch.** For
  branch-dependent (cross-cut) work, do it on the branch in the main worktree (sequential, to avoid
  git-index races), or `git worktree add <path> <new-branch-off-the-feature-branch>`. A misbased
  worktree's **disjoint** changes still graft cleanly: `git -C <wt> diff HEAD -- <subtree> | git apply`.
- **A new wire control-verb must be added to BOTH the decoder AND the router classification** — the
  Permissive unit harness cannot see a routing omission; **only the live e2e under Enforce can**.
  Gate any gui/excel/wire change with the live e2e under `enforce`, never defer it.
- **Live-validate the production posture (`Enforce`).** A Permissive demo/test edge masks deny-by-default
  defects. Propagate a server change to every client + gate in the SAME change.

## 2. Status — what is DONE vs REMAINING
**DONE (landed on `main`):**
- Determination finding **#2 (access gate), stream/WS half** — caller-authz cross-cut on the gRPC
  stream + WS mirror + SDK/CLI/GUI/Excel authenticate-first. Merged `8e0ee48`, live-validated under
  Enforce (GUI 165/165, Excel 122/122). See `docs/SECURITY-AUTHZ-FINDING.md`.
- Finding **#7 (docs drift), the INTERFACES.md part** — 40-crate count, `celnet-rates` listed DEFERRED,
  `risk-normalize` cross-asset deps corrected. (Federation-built/line-63 prose may still need a pass.)
- Knowledge base current (0 stale, 1627 claims, committed mirror).
- **Item K (gui-unit guard fix)** — DONE, landed `7f381b0` (pushed). The density JS-free guard
  now exempts `*.stories.tsx` (density/tokens stories legitimately demo the axis); gui-unit **721/721**.
  Test-only delta, zero runtime surface → live-e2e mandate N/A (gate = the plan's `gui-unit 721/721`).

**REMAINING — the work this plan implements (ordered by leverage, determination §3):**

| # | Item | Spec / anchors | Gate |
|---|------|----------------|------|
| A | **Carry seam → streamed edge** (highest leverage) | `docs/plan/CARRY-SEAM-TO-EDGE.md`; anchors `pricefanout::pair_seed`, `proto helpers MarketContext.fx`, `core::carry::CarryInputs::discount_df` | FX `Update` byte-identical + cross-asset conformance ×5 clients; no `match carry` in stream pricing; **T2** at P3 |
| ~~B~~ | ~~**Auth §2/§3 follow-up**~~ **DONE** (spec `docs/plan/B-AUTH-QUOTE-RISK.md`) | §2 quote gating + accept-binding; §3 desk-identity bridge + risk narrowing | ✅ all 4 QuoteService RPCs gated + accept bound to authenticated requester; desk slug→`DeskId` bridge (`DeskDef.books`→interned, boot-populated) narrows non-admin trader to `Rule::on(Desk,d)∩body` (admin/no-session byte-identical); 5 clients; adversarially-verified **SHIP**; **T2 green** (full `just check` + live GUI **165/165** + Excel **122/122** under Enforce). Also fixed 2 pre-existing latent gate gaps the full T2 surfaced: bench stream-auth + a false xva CVA-monotonicity proptest invariant; added a `numerics-serial` nextest group. |
| C | **`price_instrument` god-fn → `ProductEngine` registry** | anchor `celnet-server::pricer::price_instrument` (1207 LOC, cx 154) | per-family impl over the carry seam; `price_instrument` < 100 lines (decode→lookup→dispatch); pricing **byte-identical** before/after (parity corpus) |
| D | **Four dormant crates** — wire or tag deferred | anchors `plugin-host::ModelRegistry::register_native`, `celnet-xva`, `celnet-replog`, `celnet-rates`; `server::pricer` | wire `plugin-host` into the server registry (make "built" true) + xva/replog to a server path + `rates` behind `forward()/discount()`; OR tag each deferred in INTERFACES.md with the named integration edge |
| E | **Decouple `risk-cube` from `exotics`/`gpu` via injected `RepriceFn`** | anchor `risk-cube` deps; aggregation boundary `entitlements::EntitlementFilter::entitled_cube` | risk-cube depends on a `RepriceFn` trait, not the PDE/MC/wgpu subtree; rebuild blast-radius shrinks; roll-ups byte-identical |
| F | **One shared carry→sensitivity mapper in `celnet-core`** | anchor `core::carry::CarryInputs::discount_df`; collapse per-exotic `match carry => RateSensitivities` | a third `Carry` arm = one edit; sensitivities byte-identical |
| G | **Generate WS codec from proto** (retire ~253 hand drift-guards) | anchor `ws::codec::product_from_json` | generated == hand-rolled wire bytes (golden-vector conformance stays green) |
| H | **Unify error taxonomy** | `From<ErrorClass> for tonic::Status` + WS derive; anchor `access.rs` Status sites | one mapping; no behavioural change to status codes (tests pin) |
| I | **Docs-to-reality reconciliation tail** (#7 remainder) + **Celer ingress ADR** (#10) + **risk facts → exotics/cross-asset** | `manage_adr`; anchors `risk-fleet::fan_out_aggregate`, `risk::federate::fan_out` | INTERFACES/ARCH docs match built reality; ADR recorded; exotic/cross-asset positions in roll-ups |
| J | **Restrictive-principal client e2e under Enforce** (#11) | anchor `entitlements::Principal::grant_all` | ≥1 client drives a desk-scoped (non-grant-all) principal end-to-end; denied-outside-scope asserted |
| ~~K~~ | ~~**Pre-existing gui-unit fix**~~ **DONE `7f381b0`** | `gui/test/density.test.tsx` JS-free guard now exempts `*.stories.tsx` | ✅ gui-unit **721/721** |
| L | **(structural hygiene)** drop `client→server` / `surface→crypto-vanilla` dev-dep back-edges via `celnet-testkit`; cargo-deny ban-dep to mechanically gate the `proto`-only-`types` waist + the `match carry` ban | anchors per §3 #L / §2 | deny passes; no back-edges |

## 3. Execution strategy (how the post-clear session drives this)
**Do NOT attempt all items in one workflow.** Each is a gated milestone; the M4 serializes heavy
cargo. Run **one workflow per item (or tight cluster), in priority order A→…→L**, staying in the
loop between them — read each result, land + gate, then launch the next. Suggested order respecting
dependencies: **B and K first** (small, independent, fast wins; B is security) → **A** (highest
leverage, unlocks cross-asset streaming; do before EdgeFrame) → **F** (the shared mapper A relies on)
→ **C** (god-fn → registry) → **E, G, D, H, L** (layering/hygiene, largely independent) → **I, J**
(docs/ADR + the restrictive-principal e2e). Re-evaluate after each — A or C may reshape later items.

**Per-item workflow shape (the proven pattern):**
1. **Scout inline first** (lodestar `search_graph`/`trace_path`/`get_code_snippet`) to turn the spec's
   anchors into the exact edit sites + blast radius (`detect_changes`). Don't fan out before you know the work-list.
2. **Implement** on a feature branch (`arch/<item>`), in the **main worktree** if it's a cross-cut
   (server+clients+proto), or via worktree lanes ONLY for toolchain-disjoint pieces. Authoring agents:
   `celnet-quant` for math/pricing, `celnet-gui` for gui/excel + **live e2e**, `general-purpose`(opus)
   for server/wire. Cite the independent oracle for any numeric.
3. **Adversarially verify** (`celnet-verifier`, read-only): hidden mocks, contract drift, numeric
   validity, "is it actually done". Majority-refute kills a claim.
4. **Gate**: T1 on the changed crates; for any gui/excel/wire item, **T2 with both live e2e under
   `enforce`** (the non-negotiable). Re-run flaky-looking failures once; triage deterministically.
5. **Land**: merge `--no-ff` to `main`, push (origin `github.com/soarsa/celnet` ONLY). Update this
   plan's status table, the ledger (`docs/IMPLEMENTATION-LEDGER.md`, newest first), and the CLAUDE.md
   resume anchor (one line, replace in place).

**Workflow scripting notes (Workflow tool):** scripts are plain JS; `meta` must be a pure literal;
default to `pipeline()`; use `parallel()` only for a true barrier; `celnet-gpu` agents/tests need
`--test-threads 1`; pass file-references not pasted code across seams. Author each item's script
inline, let it persist to the session dir, iterate via `{scriptPath}`.

## 4. Lossless-resume protocol
State lives on disk, so a `/clear` is lossless:
- **This file** = the master plan + live status table (§2). Update the table as items land.
- `docs/ARCHITECTURE-DETERMINATION.md` = the evidence + the 11 findings with anchors.
- `docs/SECURITY-AUTHZ-FINDING.md` (B), `docs/plan/CARRY-SEAM-TO-EDGE.md` (A) = the detailed specs.
- `docs/IMPLEMENTATION-LEDGER.md` (newest-first history) + the CLAUDE.md one-line resume anchor.
- lodestar graph (structural) + committed knowledge mirror (`.lodestar/knowledge/`, the verified "why").
"**Done**" for the whole program = every row in §2's REMAINING table landed on `main`, each T2-green
(live e2e under Enforce where it touches gui/excel/wire), docs reconciled, with no `match carry` in
payoff/stream code and the `proto`-only-`types` waist mechanically gated.
