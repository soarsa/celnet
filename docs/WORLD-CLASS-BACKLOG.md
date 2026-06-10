# Celnet — World-Class Backlog (single source of backlog truth)

> THE single live backlog for the multi-asset evolution (see
> `docs/MASTER-EVOLUTION-PROGRAM.md`). Supersedes the asset-class-expansion backlogs in
> COMPLETION-PROGRAM / LEADERSHIP-PROGRAM / POST-COMPLETION-AUDIT / CAPABILITIES-REVITALISATION
> (cross-link, do not duplicate — guardrail #10). Maintained by the §6 convergence loop.
>
> **Dedup rule:** before adding, search by `dedup-key`; on match append evidence, never
> duplicate. **Genuine-gap calibration:** real + in-repo-buildable + bar-raising + NOT
> deploy-bound + NOT padding. **DRY:** round adds zero genuine gaps; STOP at 2 consecutive dry.

Status legend: OPEN / IN-PROGRESS / DONE / ENV (deploy-bound, not a gap). Each item:
`{wave, dedup-key, title, oracle, client-parity, effort, priority, status}`.

## Convergence ledger
- Round 0 (seed): initial gap list below, distilled from the five lenses. Dry-round counter: 0/2.
- **Round 1 — RECONCILIATION (2026-06-10, session-B):** the backlog had drifted far behind `main`.
  Closed with evidence: ALL W1 core items (`ba0fc03`/`6143409`), W2 linear+metals+universe
  (`513c47c`/`02ce7a5`/`17873e0`/`80d1596`), W3 crypto leaf+wire (`32250bc`/`1484fb0`), W4 pivot
  (`83596cd`), W4 RFQ engine+proto+server (`6b00940`/`1484fb0`), W5 risk fact+FRTB+leaves
  (`100eb7a`/`7aad04b`/`48a61bc`/`2c61193`/`1484fb0`), the parity-matrix gate (verification-coverage
  21 arms + 3 cross-asset families, proto-driven), sdk-builders. **Narrowed to the genuinely-open
  remainder:** `clients/rfq-panel-surfacing` (split from multi-dealer-rfq), `surface/crypto-leaf`
  (split from W3), `clients/excel-polymorphic` (PRICE exists; per-product table not retired),
  `clients/gui-universe` (structuring DONE via GW2; universe still FX-only), W6 rigor floor
  (4 infra crates done; analytics crates remain), proto/new-payoff-shapes, exotics second-gen,
  rates leaf (deferred). **IN-PROGRESS:** ADR-0008 Waves B/C/D+S (session-A, `lane/adr0008-tail`).
  Round added 0 brand-new gaps (narrowings only) → dry-round counter: 1/2.
- **W0 foundation CLOSED (2026-06-08):** all four W0 items DONE + the cross-client parity-matrix
  gate IN-PROGRESS (FX seed green). Executable verification floor now exists (golden-vector
  corpus + 5-client conformance + Excel real-edge + coverage lint). Next: **W1 multi-asset CORE**
  (generalize the FX-only Layer-0 seams: `Underlying`/`Carry`/`Sensitivities` + proto + plugin-api,
  FX byte-identical) — the single highest-risk wave; build as ONE coordinated wave per §2 sequencing.
- **W1 multi-asset CORE CLOSED (2026-06-08; commits `ba0fc03`/`6143409`):** the P0 W1 items
  `arch/underlying-abstraction`, `types/carry-model-generalize`, `types/sensitivities-generalize`,
  `proto/instrument-underlying-redesign`, `plugin-api/generalize-pricingmodel`,
  `arch/sequence-interface-crates-once` are **DONE** — generalized in place, FX byte-identical
  (`just check` "All gates passed.", 1343 tests; equity-dividend plugin model vs independent GBS oracle
  1e-12). `surface/asset-class-neutral-core` is **DEFERRED** (not needed until a non-FX surface leaf
  lands — W3 carries its own crypto surface leaf). GW0/GW1 GUI foundation merged. **▶ The fan-out lanes
  (W2/W3/W4/W5 + GW2) are now OPEN** — see `docs/PARALLEL-SESSIONS.md`. Dry-round counter: 0/2 (the
  convergence loop runs after the asset-class waves land). Next recommended parallel set:
  W2-A-LINEAR ∥ W3-CRYPTO ∥ W5-B-LEAVES.

## P0 — Foundation & core (must precede asset-class fan-out)

- **[W0] verify/cross-client-golden-vector** — Executable cross-client golden-vector oracle.
  server==SDK==CLI==Excel==GUI is a doc claim today, not a gate. Build
  `crates/celnet-golden/vectors/*.json` (engine-generated, each value cross-checked by its
  parity oracle) + a per-client conformance harness booting the real edge.
  Oracle: per-product parity re-derives each vector. Parity: all 5. Effort L. **DONE**
  (commit pending; 84 vectors / all 18 families in `crates/celnet-golden/vectors/*.json` +
  generator `src/bin/gen_vectors.rs` + `tests/vectors_selfcheck.rs`; conformance:
  `celnet-client/tests/conformance.rs` (SDK, 18 families) + `celnet-cli/tests/conformance.rs`
  (9 local-compute) + `gui/test/conformance.test.ts` (in-process pricer, 16) + Excel real-edge
  e2e. No circular oracle — verifier-confirmed. Surfaced + fixed two real defects: one-touch
  at-hit vs at-expiry, lookback Brownian-bridge extremum.)
- **[W0] verify/excel-real-edge** — Excel never dials the real edge (only in-process
  `FakeSocket`); its wire conformance is unproven e2e. Add an Excel real-edge suite mirroring
  `gui/e2e/`. Oracle: shared golden vectors. Parity: Excel==GUI==SDK. Effort M. **DONE**
  (commit pending; `excel/e2e/` boots a real `demo_edge` over a real `ws` socket; 81 tests
  green across 17 Excel-exposed families — closes the FakeSocket-only gap).
- **[W0] verify/verification-contract-doc** — No written/enforced per-asset-class verification
  contract; the FX recipe is tribal. Write `docs/VERIFICATION-CONTRACT.md` (independent +
  model-disjoint oracle, golden table, cross-client vector, e2e, perf, fuzz, honesty stmt) +
  a CI lint: new proto product arm ⇒ matching parity row + golden vector. Oracle: the lint.
  Effort M. **DONE** (commit pending; `docs/VERIFICATION-CONTRACT.md` + the
  `tools/check-verification-coverage.mjs` lint wired into `just check` — parses the proto's
  18 product arms and asserts each has BOTH a golden vector AND a celnet-parity row; now 18/18.
  Added two new independent-oracle parity rows to reach 18/18: `american.rs`, `strategy.rs`.)
- **[W0] arch/workspace-dep-registry** — `celnet-xva`/`-heston`/`-qmc` bypass
  `[workspace.dependencies]` (11 internal path-deps). Register them + switch to
  `.workspace = true` + a lint banning internal path-deps outside the registry. Do BEFORE the
  crate explosion. Oracle: `grep '{ path = "../celnet-'` returns only registry; `just check`
  green. Effort S. **DONE** (commit `7c40aaf`; 5 crates registered, 11 path-deps migrated,
  `just workspace-deps` lint wired into `just check`).
- **[W1] arch/underlying-abstraction** — `Ccy([u8;3])`/`CcyPair` cannot name crypto/equity/
  listed; keystone for every non-FX class. `celnet-types::Underlying` enum + `Symbol` newtype.
  Oracle: `to_bits` byte-identity for the FX (rename-only) projection. Parity: all 5. Effort L.
  **DONE** (W1 `ba0fc03`; extended with Equity/Commodity/DigitalAsset arms in `1484fb0`).
- **[W1] types/carry-model-generalize** — `VanillaInputs`/`MarketContext` bake GK two-rate.
  Replace with `{spot,vol,discount_rate,Carry}`; engine reads `(r,b)`; FX = `b=r−r_for`.
  Oracle: GK price == generalized `(r,b)` to 1e-12 on the golden tables. Effort XL. **DONE**
  (W1 `ba0fc03` — Carry/CarryModel seam; exotics analytic core followed in ADR-0008 Wave A
  `0fbadab`; MC/PDE/composite + MarketContext = the ADR-0008 B/C/D/S tail, IN-PROGRESS below).
- **[W1] types/sensitivities-generalize** — `Greeks.rho_dom/rho_for` is FX-only; carry-rho for
  equity/crypto would be a silent lie. Replace with carry-tagged `RateSensitivities`. Oracle:
  FX `discount_rho==rho_dom`/`carry_rho==rho_for` bit-identical on every vanilla golden.
  Effort L. **DONE** (W1 `ba0fc03` — `RateSensitivities`; cross-asset carry-rho live in `1484fb0`).
- **[W1] proto/instrument-underlying-redesign** — `Instrument`/`MarketContext`/`OrgKey` have no
  asset-class discriminator; FX hardwired. Generalize the contract in place (Underlying +
  CarryModel + RateSensitivities + product validity matrix; refactor BasketLeg). Oracle:
  convert round-trip byte-identical for FX; equity instrument priced e2e vs QuantLib. Parity:
  all 5. Effort XL. **DONE** (W1 `ba0fc03` + the xasset proto window/integration `47f5ad6`/`1484fb0`
  — equity/commodity/crypto priced e2e through all 5 clients vs independent oracles).
- **[W1] plugin-api/generalize-pricingmodel** — `PricingModel::price(OptionType,&VanillaInputs)`
  cannot express a non-FX model — the headline differentiator is FX-shaped. Parameterize over
  the new vocabulary; update WIT + wasmi (ptr,len) ABI. Oracle: 4 existing plugin gates still
  green for FX + a NEW gate registering an equity-dividend model reconciled to QuantLib.
  Effort XL. **DONE** (W1 `ba0fc03` — equity-dividend plugin vs independent GBS oracle 1e-12).
- **[W1] surface/asset-class-neutral-core** — `celnet-surface` is FX delta-space (RR/BF). Split
  into neutral surface core (arb-free interp, butterfly/calendar) + FX-smile leaf + strike/
  moneyness leaves. Oracle: re-mark every FX surface byte-identical; equity SVI from a
  strike-vol grid. Effort L. **OPEN.**
- **[W1] arch/sequence-interface-crates-once** — The 4 above all touch the frozen Layer-0
  crates; build them in ONE coordinated wave (FX byte-identical) THEN fan out leaves, else lane
  thrash. Oracle: full FX golden/parity byte-identical after the core wave. Effort M. **DONE**
  (W1 built as one coordinated wave; fan-out lanes followed conflict-free per the board).

## P0/P2 — FX completeness & breadth

- **[W2] linear/fx-forward-swap-ndf** — Forwards/swaps/NDF are not first-class priced products
  (only `OptionInputs.forward()` + a market observable). New `celnet-linear` crate. Oracle:
  QuantLib FxForward PV + closed-form DF to 1e-12; NDF (F−K)·notional·df hand-derived. Parity:
  all 5 book+price the same forward bit-identically. Live NDF fixing VALUES = ENV. Effort L.
  **DONE** (`513c47c` engine; `17873e0` server+SDK+CLI+Excel+golden+parity; `80d1596` GUI —
  verification-coverage 21/21).
- **[W2] conventions/metals-breadth** — Only XAU/XAG vs USD; add XPT/XPD + metal crosses
  (XAUEUR/XAUJPY/XAGEUR), loco-London T+2 + lease-rate. Oracle: published LBMA practice +
  independent rata-die/holiday walk + QuantLib GK with lease=foreign-rate to 1e-12. Effort M.
  **DONE** (`02ce7a5` — XPT/XPD + metal crosses + lease leg).
- **[W2] conventions/pair-universe-superset** — 19 pairs vs SynOption's 75; grow registry +
  calendars to a >75 superset (honest `has_calendar_support=false` for unmodelled lunisolar
  onshore NDF). Oracle: per-pair resolved convention == EMTA/ISDA table; spot date ==
  independent Hinnant rata-die over the (pair,date) cross product. Live fixing VALUES = ENV.
  Effort L. **DONE** (`02ce7a5` — >75-pair universe).

## P1 — Asset classes, structured products, workflow

- **[W3] crypto/digital-asset-class** — Zero crypto support. `celnet-crypto-vanilla` +
  `Underlying::DigitalAssetPair` + 24×7 calendar + inverse/coin-settled payoff + funding carry
  + crypto surface leaf. Oracle: GK-with-funding for linear-settled; independent closed-form +
  code-disjoint MC for inverse-settled; published Deribit specs for conventions. Parity: ≥1
  crypto vanilla all 5. Live vols/fixings/exchange conn = ENV. Effort XL. **DONE** (`32250bc` leaf
  — linear + inverse 1/S_T, triple-gated; `1484fb0` wire `digital_asset` + `settlement_style` +
  all-5-client surfacing + golden/parity. The crypto SURFACE leaf split out → `surface/crypto-leaf`
  OPEN below).
- **[W4] exotics/pivot** — Pivot is the one SynOption structured type Celnet lacks.
  `celnet-exotics/src/pivot.rs` MC, reusing the Sobol/bridge + std-error stack. Oracle:
  code-disjoint splitmix64 reimplementation within reported MC stderr; degenerate pivot →
  reduces to plain TARF. Effort M. **DONE** (`83596cd` — pivot TRA + degenerate→TARF `to_bits`).
- **[W4] proto/new-payoff-shapes** — Add `PerpetualOption` / `ListedFutureOption` arms (only
  genuinely new shapes). Oracle: QuantLib future-option engine; cross-asset 2-leg basket vs
  independent Cholesky GBM MC (worst≤single≤best sandwich). Parity: all 5. Effort L. **OPEN.**
- **[W4] workflow/multi-dealer-rfq** — Celnet is single-dealer; SynOption's actual moat is the
  multi-bank RFQ venue. `MultiDealerQuote` flow fanning to N quote sources (internal + FIX LP
  adapters), ranking best bid/offer, audited panel. Oracle: in-repo loopback ≥3 synthetic LP
  responders (best-price selection, tie-break, timeout/last-look; `lp_count`/`lp_won`
  consistency). Parity: GUI/CLI/SDK see identical ranked panel. Live LP conn + MAS-RMO = ENV.
  Effort XL. **DONE (engine+proto+server)** (`6b00940` MultiDealerEngine + loopback FIX;
  `1484fb0` `RequestMultiDealerQuote` RPC + `AcceptQuote(quote_id,lp_id)` wired to the live
  pricer). **The client ranked-panel surfacing split out → `clients/rfq-panel-surfacing` OPEN
  below** (no client renders the panel yet — verified by grep 2026-06-10).

## P1/P2 — Cross-asset risk & further leaves

- **[W5] risk/cross-asset-canonical-leaf** — `CanonicalLeaf.pair:CcyPair`/`inputs:VanillaInputs`
  block a mixed FX+equity+rates firm book. Generalize to a cross-asset risk fact (generic
  factor set + asset-class tag + underlying ref); keep the additive/non-additive algebra.
  Oracle: existing FX firm_aggregate==single-node 1e-12 stays green; mixed-asset roll-up nets
  per factor. Effort L. **DONE** (`100eb7a`/`7aad04b` W5-A + the 6-crate downstream migration
  `7a6f9be`).
- **[W5] risk/frtb-cross-asset-buckets** — FRTB-SA is FX-bucket-specific. Extend to GIRR/equity/
  commodity/CSR buckets as gated parity rows. Oracle: longhand independent recomputation;
  re-derive constants from BCBS text (the 0.75ρ circular-oracle lesson). Effort M. **DONE**
  (`7aad04b` — FRTB buckets re-derived from MAR21).
- **[W5] equity/equity-vanilla-leaf** — `celnet-equity-vanilla` (dividend + repo carry).
  Oracle: QuantLib AnalyticEuropeanEngine with dividend yield to 1e-12. Parity: all 5. Effort M.
  **DONE** (`48a61bc` leaf; `1484fb0` wire + all-5-client surfacing + golden/parity).
- **[W5] commodity/commodity-vanilla-leaf** — `celnet-commodity-vanilla` (Black-76, convenience
  yield, futures-settled). Oracle: Black-76 closed form + QuantLib commodity golden table.
  Parity: all 5. Effort M. **DONE** (`2c61193` leaf; `1484fb0` wire + surfacing + golden/parity).
- **[W6+] rates/rates-vanilla-leaf** — Normal/Bachelier vol + curve-bucketed rho. Designed seam
  only unless a wave is funded; honestly deferred, not faked. Oracle: QuantLib
  BachelierSwaptionEngine. Effort XL. **OPEN (deferred).**

## P1/P2 — Client redesign

- **[W1/W3+] clients/sdk-builders** — No instrument builders; callers hand-build deep enum
  literals; FX-delta-only StrikeSpec. Typed builders per family over `Underlying` + generalized
  StrikeSpec + per-class examples. Oracle: builder output prices == analytics crate per class;
  SDK==CLI==Excel==GUI for ≥1 product/class. Effort L. **DONE** (family builders landed across
  W0–W2; `1484fb0` adds `vanilla_on`/`equity_vanilla`/`commodity_vanilla`/`crypto_vanilla` over
  `Underlying` + `examples/price_cross_asset.rs`; conformance SDK==server==oracle per class).
- **[W1/W3+] clients/excel-polymorphic** — 27 one-per-product fns; BARRIER 12 positional args;
  no composition. Polymorphic `CELNET.PRICE/QUOTE/SURFACE/RISK(underlying,product,terms-range)`
  + `CELNET.INSTRUMENT(…)` spec token. Retire per-product table per #10 at parity. Oracle:
  headless suite asserts polymorphic fn == SDK == server per class. Effort L. **OPEN**
  (narrowed 2026-06-10: `CELNET.PRICE`/`RFQ`/`SURFACE`/`RISK` exist; the remaining gap is the
  `CELNET.INSTRUMENT` spec token + retiring the ~20 per-product fns per #10 at parity).
- **[W1/W3+] clients/gui-structuring-and-universe** — No structuring workspace (Ticket
  hardcodes per-product forms); universe is FX-pair-only. Asset-class-aware UniverseNavigator +
  composable contract-derived leg-builder Structuring workspace + surface-family switch. Oracle:
  Playwright e2e per class (navigate→structure→price→stream→risk) + axe; GUI price==server.
  Effort XL. **IN-PROGRESS** (structuring workspace DONE via GW2 — ProductSpec registry +
  leg-ladder + gallery, `80d1596` adds the linear group, `1484fb0` the cross-asset spec;
  **remaining = the asset-class-aware universe/navigator + surface-family switch — CLAIMED
  session-B 2026-06-10**, gui/-only lane, dedup-key `clients/gui-universe`).
- **[W0+] clients/cross-client-parity-matrix-gate** — api-first parity asserted piecemeal, not
  as one matrix gate over (asset-class × product × client). Executable matrix gate seeded with
  FX (green now), one row per new (class,product) before "done". Oracle: the golden-vector
  corpus. Effort M. **DONE** (`1484fb0` — the `verification-coverage` lint is now proto-driven
  over BOTH the 21 product-oneof arms AND every non-FX `Underlying.ref` arm (3 cross-asset
  families), failing if any lacks a golden vector + parity row; a new (class,product) cannot
  ship un-gated).

## Round-1 narrowings & live lanes (2026-06-10)

- **[W4+] clients/rfq-panel-surfacing** — Split from `workflow/multi-dealer-rfq` (engine+proto+
  server DONE). No client renders the ranked panel: SDK has no `request_multi_dealer_quote`,
  CLI no subcommand, GUI/Excel no panel view; `AcceptQuote.lp_id` unreachable from clients.
  Surface the identical ranked panel in SDK/CLI/GUI(+Excel) with last-look countdown + book-by-
  `(quote_id,lp_id)`. Oracle: loopback ≥3-LP panel — every client shows the same ranking
  bit-identically; booking the winner == server execution record. Effort M. **OPEN**
  (sequencing: touches `celnet-client`/`celnet-cli` — schedule AFTER the ADR-0008 Wave S
  blast radius clears to avoid colliding on those crates).
- **[W3+] surface/crypto-leaf** — Split from `crypto/digital-asset-class`. The crypto SURFACE
  (strike/log-moneyness quoting, no RR/BF delta-space) needs the `surface/asset-class-neutral-core`
  split first; both deferred together. Oracle: re-mark every FX surface byte-identical; a crypto
  SVI fit from a strike-vol grid round-trips. Effort L. **OPEN** (after ADR-0008 Wave S, which
  refactors `celnet-surface`'s MarketContext — do not overlap).
- **[ADR-0008] exotics-surface/carry-seam-tail** — Waves B (MC drift-step) / C (PDE/ADI/American
  + rho re-tag) / D (composites: var/vol-swap, LSV, quanto carry-shift, multiasset) / S
  (`MarketContext::new` → `Carry`, ~60 sites/12 crates). FX byte-identical at every step; spec
  `docs/plan/ADR0008-EXOTICS-SURFACE-REMEDIATION.md`. **IN-PROGRESS** (session-A,
  `lane/adr0008-tail`, claimed `3398776`).

## P2/P3 — Rigor & depth

- **[W6] rigor/mutation-fuzz-coverage-floor** — 100% mutation-kill/deepest fuzz only on
  `celnet-vanilla`. Per-crate floor (≥90% kill-rate, fuzz target) across exotics/surface/
  risk-cube/xva/MC crates, wired as CI lanes. Oracle: cargo-mutants kill-rate + llvm-cov
  threshold + fuzz survives N h zero crashes. Effort L. **IN-PROGRESS** (W6 `7cbb997`…`7d3df75`:
  journal/replog/fanout/router at ZERO-survivor mutation + loom seqlock + sync-word + fuzz
  targets; remaining = the analytics crates floor: exotics/surface/risk-cube/xva/MC — sequence
  AFTER the ADR-0008 tail so the refactored code is what gets mutation-hardened).
- **[W6] exotics/second-gen-depth** — Out-catalogue the deep platforms: KIKO, double-touch
  family, corridor variance, fader/range-accrual, power/quanto-power, on the existing PDE/LSV/MC
  engines. Oracle: PDE≈MC≈analytic; QuantLib for KIKO/double-barrier; corridor variance vs
  independent strip replication; fader vs code-disjoint MC within stderr. Effort XL. **OPEN
  (P3, behind asset-class superset).**

## Deliverables (operator-facing artifacts)

- **[DOCS] deliverable/capabilities-pdf** — Produce the **perfectly-styled professional PDF**
  from `docs/celnet-capabilities.html` (the comprehensive capabilities document — full FX +
  multi-asset + SOTA coverage, all ~30 visual assets base64-embedded). Approach: Playwright
  print-to-PDF off the document's `@media print` stylesheet (A4 or US-Letter, page breaks per
  section, `figure { break-inside: avoid }`, embedded webfonts, the Celer-branded cover,
  no clipped figures or horizontal overflow). Run as a dynamic workflow AFTER: (a) the
  visual-asset set is complete + rendered + uncut-checked, (b) all assets base64-embedded into a
  self-contained HTML, (c) **operator review** of the HTML. Oracle: visual proof — every section
  + every visual present, no clipping, professional print layout; the committed
  `tools/check-figures-uncut.mjs` + `check-html-responsive.mjs` green on the source.
  Effort S. **DONE** (`tools/render-capabilities-pdf.mjs` + `just capabilities-pdf` →
  `docs/celnet-capabilities.pdf`, 3.2 MiB, A4, per-section page breaks, all 30 visuals embedded).
- Cross-host wire p99 / inter-DC SLO (loopback proves compute+framing+quorum only).
- Live LP-panel connectivity + regulated-venue / MAS-RMO status (multi-dealer RFQ workflow).
- Live crypto/metal NDF fixing VALUES (only fixing identity + convention are in-repo).
- Live JVM Celer estate lifecycle.
