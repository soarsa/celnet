# Parallel Claude Sessions — service-mesh coordination

> How N independent Claude sessions deliver the `docs/MASTER-EVOLUTION-PROGRAM.md` scope
> **faster, conflict-free, collaborating** through git + this live board. Read this in full
> before claiming work. The governing rule from CLAUDE.md holds: **disjoint crate/file
> ownership ⇒ no merge conflicts; interface/seam crates are coordinator-owned and frozen.**

## 1. Roles

- **Coordinator** (exactly one session — currently the session running W1): owns the
  interface/seam crates `celnet-types`, `celnet-core`, `celnet-proto`, `celnet-plugin-api`;
  the **root `Cargo.toml` registry**; **all** `celnet.proto` edits; and **every merge to
  `main`** + the full-workspace re-gate. Freezes the contract; unblocks lanes; integrates.
- **Workers** (any number): each owns ONE disjoint **lane** (a set of member crates / file
  regions) in its OWN git worktree on its OWN branch. A worker never touches a seam crate, the
  proto, the root manifest, or another lane's files. It builds its lane to green, pushes its
  branch, and hands off to the coordinator to merge.

## 2. The 3 serialization points (coordinator-gated — never two open at once)

From `docs/DOWNSTREAM-EXECUTION-MAP.md`:
1. **`celnet.proto`** — only the coordinator edits it; product-arm/field numbers are reserved
   per wave; one open proto edit at a time across all sessions.
2. **shared multi-product files** (`celnet-exotics`, the 5 clients' shared codecs) — a lane that
   must touch these marks `NEEDS-COORDINATOR` and stops at that boundary.
3. **5-client surfacing** (SDK/CLI/Excel/GUI shared files + server routing) — the last stage of
   each wave; serialized across waves by the coordinator. The W0 conformance corpus is the gate.

## 3. Worker protocol (follow exactly)

1. **Read**: `CLAUDE.md` → this board → `docs/MASTER-EVOLUTION-PROGRAM.md` →
   `docs/DOWNSTREAM-EXECUTION-MAP.md` → your lane's plan doc (`docs/W*-PLAN.md` /
   `docs/GW-FOUNDATION-PLAN.md`) → `docs/VERIFICATION-CONTRACT.md`.
2. **Claim**: pick an `OPEN` (unblocked) lane below (or the one the operator named). Edit its
   board row → `status: CLAIMED`, `owner: <your-session-tag>`, `branch: lane/<id>`. Commit
   **only this file**, `git push`. If the push is rejected, `git pull --rebase` and re-pick
   (someone claimed it first). The board is the lock.
3. **Isolate**: `git worktree add ../celnet-<id> -b lane/<id> origin/main` (off latest `main`).
   Work ONLY your lane's crates/files. (GUI lanes: `ln -s <main>/gui/node_modules
   ../celnet-<id>/gui/node_modules` to skip reinstall; do NOT `npm install` in the worktree.)
4. **Build to the lane gate** — SOTA, **zero workarounds** (no `#[ignore]`/`#[allow]`-dodge/
   `as any`/`@ts-ignore`/skipped tests/lowered tolerance/mock-as-real). Validate every numeric
   against an **independent** oracle (QuantLib/closed-form/published/code-disjoint MC). The
   **3 hard lessons**: (a) verify the literal `All gates passed.` line yourself (never a wrapper
   exit code); (b) clippy the **parity TEST target** (`clippy -p celnet-parity --test <name> -D
   warnings`), not just the product crate; (c) **re-derive published constants** vs the source,
   not just re-run the gate (the FRTB 0.75ρ circular-oracle lesson).
5. **Hand off**: commit to your branch, `git push -u origin lane/<id>`. Set the board row →
   `status: READY-FOR-MERGE`, record the branch + one-line gate evidence (test counts + the
   literal gate line). **Stop.** Do NOT merge to `main` or touch `main` yourself.
6. **Coordinator** merges your branch, runs the full `just check` (+ 5-client conformance + GUI/
   Excel suites), and sets the row → `DONE` (or returns it with notes). Re-index codebase-memory
   after structural change.

If your lane needs a seam/proto change mid-stream: set `status: NEEDS-COORDINATOR` with the
exact ask, push the board, and stop at that boundary — do not edit the contract yourself.

## 4. Hard rules (the mandate — all sessions)

One **unversioned** contract (no `schema_version`, no N/N−1); **FX byte-identical** through any
generalization (`to_bits`); delete legacy (#10); vendor-/person-neutral purpose-named
identifiers (#8); zero-alloc hot core stays alloc/lock/log-free (#11); push **only** to
`origin` = github.com/soarsa/celnet (#1) — workers push **branches**, never to `main`.

## 5. Live lane board

> Status: `OPEN` (claimable now) · `BLOCKED:<dep>` (opens when the dep lands) · `CLAIMED` ·
> `READY-FOR-MERGE` · `DONE`. **Claim by editing your row, commit-only-this-file, push.**

| Lane | Owns (disjoint crates / files) | Depends on | Gate | Status | Owner / branch |
|------|-------------------------------|-----------|------|--------|----------------|
| **W6-RIGOR-INFRA** | `celnet-journal`, `celnet-replog`, `celnet-fanout`, `celnet-router` (+ their `fuzz/` + `.config/mutants-*.toml`) — fully disjoint from W1 | — | per-crate mutation ≥90% kill + a fuzz target/decoder + `check-crate` green | **OPEN** | — |
| **W2-A-LINEAR** | NEW `celnet-linear` (forward/swap/NDF) + its parity/golden rows | W1 contract | QuantLib FxForward + closed-form DF + structural; conformance row | IN-PROGRESS | session-B (W2 integrator) / main |
| **W2-B-BREADTH** | `celnet-conventions`, `celnet-calendar` (>75 pairs + XPT/XPD + metal crosses) | W1 `Underlying::Metal` | EMTA/ISDA/LBMA tables + independent rata-die walk | IN-PROGRESS | session-B (W2 integrator) / main |
| **W3-CRYPTO** | NEW `celnet-crypto-vanilla` (+ crypto surface leaf — deferred) | W1 contract | GK-funding + independent inverse closed-form + code-disjoint MC; Deribit specs | **DONE (pricing leaf)** | coordinator / `lane/w3-crypto` |
| **W4-A-PIVOT** | `celnet-exotics/src/pivot.rs` + new payoff arms | W1 + coordinator proto | code-disjoint MC oracle + degenerate→TARF limit | OPEN | — |
| **W4-B-RFQ** | NEW `celnet-rfq` (multi-dealer aggregation) | W1 + coordinator proto | ≥3 synthetic LP loopback; best-price/tie-break/last-look | **DONE (engine)** | coordinator / `lane/w4-b-rfq` |
| **W5-A-XRISK** | `celnet-risk-normalize`, `celnet-risk-cube` (cross-asset fact + FRTB buckets) | W1 + W5-B leaves | longhand recomputation; FX firm_aggregate==single-node 1e-12 stays green | OPEN | — |
| **W5-B-LEAVES** | NEW `celnet-equity-vanilla`, `celnet-commodity-vanilla` | W1 contract | QuantLib AnalyticEuropean (div) + Black-76 golden | **DONE** | coordinator / `lane/w5-b-leaves` |
| **GW2-STRUCTURING** | `gui/src/products/*` (Ticket→ProductSpec registry) + tests | GW0/GW1 merge | vitest per-ProductSpec round-trip + Playwright e2e + axe | **DONE** | coordinator / `lane/gw2-structuring` |

## 6. Coordinator state (updated by the coordinator each milestone)

- **▶ DONE (2026-06-08): W4-B-RFQ engine landed (coordinator lane `lane/w4-b-rfq`).** NEW disjoint crate
  **`celnet-rfq`** — a `MultiDealerEngine` fanning one `QuoteRequest` to N `QuoteSource`s concurrently
  (`join_all` + per-source `tokio::time::timeout`), ranking best-bid (max) / best-offer (min), deterministic
  tie-break (earlier `epoch_nanos` → smallest `lp_id`), timeout-drop (excluded from `lp_count`) + last-look
  promotion (winner past `valid_until_nanos` rejected, next-best promoted), with the `lp_won ⊆ responders`
  consistency invariant. A **real `FixLpAdapter`** runs a genuine celnet-fix 4.4 Logon→QuoteRequest→Quote
  session over an ephemeral 127.0.0.1 loopback socket (priced by the golden-gated `celnet-vanilla`), proven
  non-vacuous by a closed-port negative control. Build → adversarial-verify (ACCEPT on all six axes:
  oracle-independent injected-ladder ground truth, real-loopback-not-mock, ranking/tie-break/last-look
  re-derived, no workarounds, honest boundary, re-gate). **No proto / no root `Cargo.toml`** (deps are
  registered; auto-join). **Gate:** `workspace-deps` OK · `check-crate celnet-rfq` 20/20. **Deferred
  (batched in `POST-W2-INTEGRATION-MANIFEST.md`):** `QuoteService.RequestMultiDealerQuote` +
  `MultiDealerQuote`/`DealerQuote` proto + `AcceptQuote(quote_id, lp_id)` + server `InternalPricerSource`
  wiring + 5-client ranked-panel surfacing. **Honest boundary:** live LP-panel WAN connectivity +
  regulated-venue/MAS-RMO status are ENV (`CELNET_LP_PANEL`, default synthetic in-repo panel).
- **▶ MESH STATUS (2026-06-08, for session-B): see `docs/POST-W2-INTEGRATION-MANIFEST.md`.** The
  `celnet.proto` window is the single critical path; it now holds W2's `fx_forward/swap/ndf` + `metal`.
  **All deferred wire integration for the five landed/in-flight new crates (`celnet-{equity,commodity,
  crypto}-vanilla`, `celnet-rfq`) is reserved in ONE batch there** — Underlying arms `equity=4 /
  commodity=5 / digital_asset=6`, `Instrument.settlement_style=29` (crypto inverse), and
  `QuoteService.RequestMultiDealerQuote` + `MultiDealerQuote`/`DealerQuote` (RFQ) — so session-B lands them
  in its current window (or just reserves the numbers); no per-lane proto re-contention. The coordinator
  runs the remaining disjoint NON-proto lanes (W4-B finishing → W6 → W5-A) concurrently in isolated
  worktrees; none blocks the window.

- **▶ DONE (2026-06-08): W3-CRYPTO pricing leaf landed (coordinator lane `lane/w3-crypto`).** NEW disjoint
  crate **`celnet-crypto-vanilla`** on the W1 carry seam (ADR-0008 — no `match carry`): the LINEAR path is
  generalized-BSM with funding carry (`b = r − funding`); the INVERSE/coin-margined path is the genuinely
  new `1/S_T` payoff with the exact share/coin-measure closed form
  `V_coin = φ·df·[Φ(φd2) − (K/F)·e^{σ²t}·Φ(φd3)]`, `d3 = d1 − 2σ√t` (the `e^{σ²t}`/`d3` drift correction —
  the naive `V_lin/S_0` is materially WRONG). Built by a build→DUAL-adversarial-verify workflow; the
  inverse crux (the FRTB-0.75ρ-class trap) is gated THREE independent ways — code-disjoint splitmix64 MC
  (the disagree-capable guard) + a literal-`1/S_T` midpoint quadrature + a SIGNED convexity sandwich
  (call `coin·S0 < V_lin`, put `coin·S0 > V_lin`). The workflow **caught + fixed two plan errors**: the
  plan's unconditional `V_inverse·S_0 > V_linear` sandwich is wrong for calls (the correct law is signed by
  `Cov(1/S_T, payoff)`), and Gauss-Hermite under-resolves the `1/S_T` fat tail at crypto vols. A mutation to
  the naive rescale fails all three oracles, proving non-circularity. Deribit specs (European, coin
  settlement, 08:00 UTC cut, index fixing, tick) hand-pinned + cited; live fixing VALUES are ENV. **No root
  `Cargo.toml` edit** (auto-join + registered `celnet-core`/`celnet-types` + dev-dep on the golden FX
  `celnet-vanilla`; `just workspace-deps` OK) ⇒ ZERO overlap with the parallel session's W2. **Gate:**
  `workspace-deps` OK · `check-crate celnet-crypto-vanilla` 18/18. **Deferred to the coordinator (serialized
  after the W2 proto window frees):** the `Underlying::DigitalAsset`/`CryptoPair` wire arm + `CryptoPricer`
  `CarryPricer` impl + the strike/log-moneyness crypto surface leaf (in `celnet-surface`) + proto +
  `celnet-golden`/`celnet-parity` rows + 5-client surfacing.
- **▶ DONE (2026-06-08): W5-B-LEAVES landed (coordinator lane `lane/w5-b-leaves`).** Two NEW disjoint leaf
  crates on the W1 carry seam, asset-class-agnostic (ADR-0008 — NO `match carry`/`match underlying`):
  **`celnet-equity-vanilla`** (generalized-BSM via `Carry::CostOfCarry{r, b=r−q}`, full Greeks,
  `RateSensitivities::Carry.carry_rho` = the dividend-rho) and **`celnet-commodity-vanilla`** (Black-76 on a
  future, the `b=0` degenerate, `carry_rho` = convenience-rho). Built by a 2-worker dynamic workflow +
  adversarial verify; each gated against an INDEPENDENT, circular-oracle-free set (external Python-`erf`-pinned
  Hull / Haug references + model-free put-call parity + the `q=0`/`b=0` limit + central-FD Greeks — which
  caught + fixed real bugs: an equity `discount_rho` partial, a commodity wrong-sign theta + over-negated
  charm/color). **No root `Cargo.toml` edit** — the crates auto-join `members=["crates/*"]` and use only the
  registered `celnet-core`/`celnet-types` (`just workspace-deps` OK), so this lane is purely additive new
  dirs with ZERO overlap with the parallel session's W2 (proto / `celnet-linear` / conventions / server /
  5 clients). **Gate:** `just workspace-deps` OK · `check-crate celnet-equity-vanilla` 12/12 ·
  `check-crate celnet-commodity-vanilla` 10/10. **Deferred to the coordinator (serialized after the W2 proto
  window frees):** the additive `Underlying::Equity`/`Commodity` wire arms + proto + `celnet-golden` vectors +
  `celnet-parity` rows + `celnet-risk-normalize` asset-class-leaf selection (W5-A) + 5-client surfacing.
- **▶ DONE (2026-06-08): GW2-STRUCTURING landed (coordinator lane `lane/gw2-structuring`).** The 3606-line
  `TicketWorkspace` monolith is decomposed into a `gui/src/products/*` **ProductSpec registry** (21 families:
  vanilla + 4 strategies + 15 legless exotics), each `toInstrument` byte-identical to the legacy
  `buildInstrument` (proven by the registry round-trip gate). New structuring UI: a grouped/searchable
  **StructureGallery** (replaces the flat `<select>`; listbox/option/group a11y), a payoff-at-expiry
  **PayoffChart** (honest "—" for path-dependent families), and a **NetStructureStrip** (honest "—" on
  missing leg values). The ticket shell is now 759 lines (down from 3607); the monolith's per-family
  inline code is **deleted** (zero legacy, #10). Built via three dynamic workflows (registry seam →
  15-family fan-out → phase-2 components) + a rewire agent, coordinator-integrated + re-gated. **Gate:**
  `tsc` (app + test) clean, **vitest 50 files / 517 tests** pass, `tsc -b && vite build` green. **Deferred
  (honest):** the live Playwright e2e + axe boots a real Rust `demo_edge` — a cold cross-crate build that
  would share the parallel session's active `target/` (collision); it RFQs the DEFAULT structure so it is
  interaction-compatible with the gallery, and the gallery a11y is asserted structurally in vitest. Run it
  on a warm Rust build. Adding a product is now a registry entry — the multi-asset GUI (GW6) arrives as data.
- **▶ LIVE (2026-06-08): W2 is OWNED BY THE PARALLEL SESSION; the coordinator session has stepped OFF W2 to
  avoid duplication.** The parallel session pushed `w2-contract-freeze` (Underlying::Metal + types, FX
  byte-identical) → already on `main` (`325cfab`), and has verified engine branches on origin:
  `crosscheck/w2-a-linear-verified` (the celnet-linear forward/swap/NDF leaf) and
  `crosscheck/w2-b-breadth-verified` (>75-pair + XPT/XPD + metal-cross conventions/calendar). The
  coordinator's competing W2 attempt has been **stopped and discarded** (tree clean at `325cfab`).
  **W2 owner = parallel session.** Reserved: product-oneof arms **fx_forward=26 / fx_swap=27 / ndf=28**,
  Underlying **metal=3**. The proto window is the parallel session's until W2 fully lands.
  - **W2 integration still OPEN** (not in any branch yet): wire the celnet-linear products onto the proto
    product oneof (arms 26-28) + server pricer routing + golden vectors {fx_forward,fx_swap,ndf} +
    `verification-coverage` → 21/21 + 5-client surfacing. The parallel session should complete these (or
    set `NEEDS-COORDINATOR`); the coordinator merges + full-re-gates each `READY-FOR-MERGE` branch on a
    clean tree and is available to do the cross-cutting wire-up on request.
  - **Coordinator session is now on `GW2` (GUI structuring)** — the only lane with ZERO Rust/proto/root-
    Cargo overlap with the parallel session's W2 (gui/ only). Disjoint, parallel-safe.
  - **SHARED-FILE SERIALIZATION (both sessions):** `crates/celnet-proto/proto/celnet.proto` AND the root
    `Cargo.toml` registry are coordinator-serialized — new-crate registrations + proto edits route through
    the coordinator (or one-at-a-time with reserved field/line regions). Before any push to `main`,
    `git pull --rebase` (both sessions write `main`; rebase to integrate).

- **2026-06-08 — ▶ W1 LANDED + `gw-foundation` MERGED (commits `ba0fc03` W1 core, `6143409` GUI merge;
  pushed). ALL fan-out lanes are now `OPEN`.** The multi-asset contract is frozen on `main`:
  `Underlying`/`Carry`/`CarryModel`/`RateSensitivities` generalized in place, FX byte-identical
  (`just check` "All gates passed.", 1343 tests; conformance 120/120; GUI 425; Excel e2e 81). The
  carry-producing-market seam (ADR-0008) is load-bearing on the server price path — a non-FX carry is
  refused with a typed error, no silent fallback. GW0/GW1 GUI foundation merged (design system +
  accessible DataGrid + three-axis scope nav; 7 redundant pair affordances deleted).
  **Workers: claim any lane above — build it in a worktree off `origin/main` (which now carries the
  generalized contract), push your branch, mark READY-FOR-MERGE.** The 3 serialization points apply:
  one open `celnet.proto` edit at a time across all running lanes (coordinate the proto-touching lanes
  W2/W4); 5-client surfacing serializes per wave. Recommended first crate-disjoint set:
  **W2-A-LINEAR ∥ W3-CRYPTO ∥ W5-B-LEAVES**. The `celnet-surface` split is a follow-up — surface stays
  FX delta-space until a non-FX surface leaf is needed (W3 carries its own crypto surface leaf).
- Plans on `main`: `docs/W2-LINEAR-PLAN.md`, `W3-CRYPTO-PLAN.md`, `W4-STRUCTURED-RFQ-PLAN.md`,
  `W5-CROSSASSET-RISK-PLAN.md`, `GW-FOUNDATION-PLAN.md`, `DOWNSTREAM-EXECUTION-MAP.md`,
  `adr/ADR-0008-multi-asset-carry-architecture.md`.
