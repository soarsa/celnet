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
  **OPEN.**
- **[W1] types/carry-model-generalize** — `VanillaInputs`/`MarketContext` bake GK two-rate.
  Replace with `{spot,vol,discount_rate,Carry}`; engine reads `(r,b)`; FX = `b=r−r_for`.
  Oracle: GK price == generalized `(r,b)` to 1e-12 on the golden tables. Effort XL. **OPEN.**
- **[W1] types/sensitivities-generalize** — `Greeks.rho_dom/rho_for` is FX-only; carry-rho for
  equity/crypto would be a silent lie. Replace with carry-tagged `RateSensitivities`. Oracle:
  FX `discount_rho==rho_dom`/`carry_rho==rho_for` bit-identical on every vanilla golden.
  Effort L. **OPEN.**
- **[W1] proto/instrument-underlying-redesign** — `Instrument`/`MarketContext`/`OrgKey` have no
  asset-class discriminator; FX hardwired. Generalize the contract in place (Underlying +
  CarryModel + RateSensitivities + product validity matrix; refactor BasketLeg). Oracle:
  convert round-trip byte-identical for FX; equity instrument priced e2e vs QuantLib. Parity:
  all 5. Effort XL. **OPEN.**
- **[W1] plugin-api/generalize-pricingmodel** — `PricingModel::price(OptionType,&VanillaInputs)`
  cannot express a non-FX model — the headline differentiator is FX-shaped. Parameterize over
  the new vocabulary; update WIT + wasmi (ptr,len) ABI. Oracle: 4 existing plugin gates still
  green for FX + a NEW gate registering an equity-dividend model reconciled to QuantLib.
  Effort XL. **OPEN.**
- **[W1] surface/asset-class-neutral-core** — `celnet-surface` is FX delta-space (RR/BF). Split
  into neutral surface core (arb-free interp, butterfly/calendar) + FX-smile leaf + strike/
  moneyness leaves. Oracle: re-mark every FX surface byte-identical; equity SVI from a
  strike-vol grid. Effort L. **OPEN.**
- **[W1] arch/sequence-interface-crates-once** — The 4 above all touch the frozen Layer-0
  crates; build them in ONE coordinated wave (FX byte-identical) THEN fan out leaves, else lane
  thrash. Oracle: full FX golden/parity byte-identical after the core wave. Effort M. **OPEN.**

## P0/P2 — FX completeness & breadth

- **[W2] linear/fx-forward-swap-ndf** — Forwards/swaps/NDF are not first-class priced products
  (only `OptionInputs.forward()` + a market observable). New `celnet-linear` crate. Oracle:
  QuantLib FxForward PV + closed-form DF to 1e-12; NDF (F−K)·notional·df hand-derived. Parity:
  all 5 book+price the same forward bit-identically. Live NDF fixing VALUES = ENV. Effort L.
  **OPEN.**
- **[W2] conventions/metals-breadth** — Only XAU/XAG vs USD; add XPT/XPD + metal crosses
  (XAUEUR/XAUJPY/XAGEUR), loco-London T+2 + lease-rate. Oracle: published LBMA practice +
  independent rata-die/holiday walk + QuantLib GK with lease=foreign-rate to 1e-12. Effort M.
  **OPEN.**
- **[W2] conventions/pair-universe-superset** — 19 pairs vs SynOption's 75; grow registry +
  calendars to a >75 superset (honest `has_calendar_support=false` for unmodelled lunisolar
  onshore NDF). Oracle: per-pair resolved convention == EMTA/ISDA table; spot date ==
  independent Hinnant rata-die over the (pair,date) cross product. Live fixing VALUES = ENV.
  Effort L. **OPEN.**

## P1 — Asset classes, structured products, workflow

- **[W3] crypto/digital-asset-class** — Zero crypto support. `celnet-crypto-vanilla` +
  `Underlying::DigitalAssetPair` + 24×7 calendar + inverse/coin-settled payoff + funding carry
  + crypto surface leaf. Oracle: GK-with-funding for linear-settled; independent closed-form +
  code-disjoint MC for inverse-settled; published Deribit specs for conventions. Parity: ≥1
  crypto vanilla all 5. Live vols/fixings/exchange conn = ENV. Effort XL. **OPEN.**
- **[W4] exotics/pivot** — Pivot is the one SynOption structured type Celnet lacks.
  `celnet-exotics/src/pivot.rs` MC, reusing the Sobol/bridge + std-error stack. Oracle:
  code-disjoint splitmix64 reimplementation within reported MC stderr; degenerate pivot →
  reduces to plain TARF. Effort M. **OPEN.**
- **[W4] proto/new-payoff-shapes** — Add `PerpetualOption` / `ListedFutureOption` arms (only
  genuinely new shapes). Oracle: QuantLib future-option engine; cross-asset 2-leg basket vs
  independent Cholesky GBM MC (worst≤single≤best sandwich). Parity: all 5. Effort L. **OPEN.**
- **[W4] workflow/multi-dealer-rfq** — Celnet is single-dealer; SynOption's actual moat is the
  multi-bank RFQ venue. `MultiDealerQuote` flow fanning to N quote sources (internal + FIX LP
  adapters), ranking best bid/offer, audited panel. Oracle: in-repo loopback ≥3 synthetic LP
  responders (best-price selection, tie-break, timeout/last-look; `lp_count`/`lp_won`
  consistency). Parity: GUI/CLI/SDK see identical ranked panel. Live LP conn + MAS-RMO = ENV.
  Effort XL. **OPEN.**

## P1/P2 — Cross-asset risk & further leaves

- **[W5] risk/cross-asset-canonical-leaf** — `CanonicalLeaf.pair:CcyPair`/`inputs:VanillaInputs`
  block a mixed FX+equity+rates firm book. Generalize to a cross-asset risk fact (generic
  factor set + asset-class tag + underlying ref); keep the additive/non-additive algebra.
  Oracle: existing FX firm_aggregate==single-node 1e-12 stays green; mixed-asset roll-up nets
  per factor. Effort L. **OPEN.**
- **[W5] risk/frtb-cross-asset-buckets** — FRTB-SA is FX-bucket-specific. Extend to GIRR/equity/
  commodity/CSR buckets as gated parity rows. Oracle: longhand independent recomputation;
  re-derive constants from BCBS text (the 0.75ρ circular-oracle lesson). Effort M. **OPEN.**
- **[W5] equity/equity-vanilla-leaf** — `celnet-equity-vanilla` (dividend + repo carry).
  Oracle: QuantLib AnalyticEuropeanEngine with dividend yield to 1e-12. Parity: all 5. Effort M.
  **OPEN.**
- **[W5] commodity/commodity-vanilla-leaf** — `celnet-commodity-vanilla` (Black-76, convenience
  yield, futures-settled). Oracle: Black-76 closed form + QuantLib commodity golden table.
  Parity: all 5. Effort M. **OPEN.**
- **[W6+] rates/rates-vanilla-leaf** — Normal/Bachelier vol + curve-bucketed rho. Designed seam
  only unless a wave is funded; honestly deferred, not faked. Oracle: QuantLib
  BachelierSwaptionEngine. Effort XL. **OPEN (deferred).**

## P1/P2 — Client redesign

- **[W1/W3+] clients/sdk-builders** — No instrument builders; callers hand-build deep enum
  literals; FX-delta-only StrikeSpec. Typed builders per family over `Underlying` + generalized
  StrikeSpec + per-class examples. Oracle: builder output prices == analytics crate per class;
  SDK==CLI==Excel==GUI for ≥1 product/class. Effort L. **OPEN.**
- **[W1/W3+] clients/excel-polymorphic** — 27 one-per-product fns; BARRIER 12 positional args;
  no composition. Polymorphic `CELNET.PRICE/QUOTE/SURFACE/RISK(underlying,product,terms-range)`
  + `CELNET.INSTRUMENT(…)` spec token. Retire per-product table per #10 at parity. Oracle:
  headless suite asserts polymorphic fn == SDK == server per class. Effort L. **OPEN.**
- **[W1/W3+] clients/gui-structuring-and-universe** — No structuring workspace (Ticket
  hardcodes per-product forms); universe is FX-pair-only. Asset-class-aware UniverseNavigator +
  composable contract-derived leg-builder Structuring workspace + surface-family switch. Oracle:
  Playwright e2e per class (navigate→structure→price→stream→risk) + axe; GUI price==server.
  Effort XL. **OPEN.**
- **[W0+] clients/cross-client-parity-matrix-gate** — api-first parity asserted piecemeal, not
  as one matrix gate over (asset-class × product × client). Executable matrix gate seeded with
  FX (green now), one row per new (class,product) before "done". Oracle: the golden-vector
  corpus. Effort M. **IN-PROGRESS** (the executable gate now exists: golden-vector corpus +
  per-client conformance + the `verification-coverage` lint, all seeded green for the 18 FX
  families; remaining = add a row per new (asset-class, product) as W1+ lands).

## P2/P3 — Rigor & depth

- **[W6] rigor/mutation-fuzz-coverage-floor** — 100% mutation-kill/deepest fuzz only on
  `celnet-vanilla`. Per-crate floor (≥90% kill-rate, fuzz target) across exotics/surface/
  risk-cube/xva/MC crates, wired as CI lanes. Oracle: cargo-mutants kill-rate + llvm-cov
  threshold + fuzz survives N h zero crashes. Effort L. **OPEN.**
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
  Effort S. **OPEN** (gated on operator review of the HTML — see the capabilities lane).
- Cross-host wire p99 / inter-DC SLO (loopback proves compute+framing+quorum only).
- Live LP-panel connectivity + regulated-venue / MAS-RMO status (multi-dealer RFQ workflow).
- Live crypto/metal NDF fixing VALUES (only fixing identity + convention are in-repo).
- Live JVM Celer estate lifecycle.
