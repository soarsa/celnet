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
- **Round 5 — DRY (2026-06-11, session-B; 7-lens + adversarial synthesis, workflow-driven over the FULL RC cut `a0817d6` = R1–R8):**
  the release-decision round. **`genuineNew: []`, `rejected: []` — ZERO genuinely-new findings.** Four lenses
  (sota-scope / numerical-correctness / performance / api-ux) returned EMPTY — math core, priced-arm
  completeness, hot-path budgets, API ergonomics all clean at the cut. The Round-4 client-entitlement P1 did
  NOT re-surface (clients assert grant-all; verified vs an Enforce edge). **No new P0/P1 in the freshly-landed
  R7/R8 surface-leaf / mutation code; nothing ships a wrong price.** The 4 non-empty findings all ABSORBED by
  existing tracked items (by semantics): (1) **R8 crypto strike-axis surface leaf is built + 256-case
  parity-gated but NOT wired to proto/server/clients** (no `quote_basis`/`StrikeQuoteSet`; gui CRYPTO:[]) →
  the OPEN `surface/crypto-leaf` item's unbuilt *surfacing* half; **NOT a defect** (crypto vanilla pricing
  works via the FX-style surface) — a capability gap that RC cut-criterion #2 explicitly routes to the
  operator ("R7+R8 DONE or re-scoped"). (2) cross-asset leaf crates lack a mutation floor → the IN-PROGRESS
  `rigor/mutation-fuzz-coverage-floor` broadened (P3; leaves are oracle-verified TODAY). (3)+(4)
  ARCHITECTURE/API-CLIENTS/RELEASE-RC/CONVENTIONS stale FX-only counts (34-vs-39 crates, 18/23-vs-24 arms,
  pair universe missing XPT/XPD) → the same guardrail-#10 stale-docs class already filed thrice
  (capabilities-fx-only-stale + interfaces-registry-lags-contract + client-parity-matrix-stale); P2 doc
  accuracy, no wrong price. **R9 cut criterion (convergence carries zero NEW P0/P1) = SATISFIED.** Dry-counter
  1/2 (a 2nd consecutive dry round = full-program convergence; NOT required for the RC cut, whose bar is
  zero-P0/P1). Gate: `GATE: a0817d6 T2 16/16` (web typecheck+unit + e2e under `enforce`).
  **OPERATOR DECISION (2026-06-11): `releaseReady` = TRUE; cut `1.0-RC` = `a0817d6`.** R8-surfacing
  re-scoped (criterion #2) to the first post-RC fast-follow — new tracked OPEN item
  `surface/crypto-strike-axis-surfacing` (proto `quote_basis`/`StrikeQuoteSet` oneof on
  `MarkSurfaceRequest` + server strike-slice ingestion → `strike_surface` + SDK/GUI/Excel
  mark-by-strike + an independent (k,w) parity row; session-A's `surface/crypto-leaf` lane,
  on capacity). Post-RC backlog also carries the 4 Round-4/5 docs-sync + mutation-floor-broadening
  + raft-flake items (none P0/P1).
- **Round 4 — NOT DRY (2026-06-11, session-B; 7-lens + adversarial synthesis, workflow-driven over the RC batch-D landing):**
  The two Round-3 P1 blockers did NOT re-surface (correctly fixed-at-root, not re-counted); sota-scope /
  numerical-correctness / completeness lenses all returned EMPTY (the math core + product completeness are
  genuinely covered at Round 4). **1 genuine P1 — now FIXED (`87c6f77`):**
  `entitlements/client-default-omits-principal-stale-grant-all-docs` — the server deny-by-default (Enforce)
  landing was not propagated to the clients: all four (SDK/CLI/GUI/Excel) OMITTED the principal by default and
  documented "omit ⇒ grant-all", so the headline risk workflow was denied against a production edge, masked
  only because demo_edge + the harnesses ran Permissive. FIX (operator-chosen): every client now asserts an
  EXPLICIT grant-all by default (audited; the boundary still denies a genuinely-absent principal); harnesses +
  gui/excel e2e demo_edge run Enforce to unmask; regression guards added. **3 P2 + 1 P3 → post-RC backlog:**
  `bench/core-throughput-floor-ungated` (P2 — §1.2 ≥1M/core sustained measured but no gating floor; only the 3
  latency percentiles gate), `rigor/fix-frame-decode-fuzz-target-not-wired-into-ci` (P2 — fuzz/Cargo.toml
  declares 9 targets, ci.yml runs 6; FrameReader's only adversarial coverage unexecuted + false README "every
  target" claim), `docs/capabilities-fx-only-stale-vs-24-arm-multiasset` (P2 — CELNET-CAPABILITIES.md still
  "FX-only 18-product" across ~11 captions), `rigor/plugin-host-wasm-decode-no-fuzz-or-honesty-stmt` (P3 —
  Module::new on untrusted Wasm with no fuzz target + no VERIFICATION-CONTRACT clause-(f) statement; thin
  residual behind audited wasmi). The registry-doc finding ABSORBED by the open `interfaces-registry-lags-contract`.
  **Also fixed in the P1 landing (pre-existing defects uncovered while verifying):** excel
  `instrumentPolymorphic.test` threw for the cross-asset families (born polymorphic, no retired per-product
  function) — scoped the legacy-parity check to LEGACY_FAMILIES; and the **GATE GAP** that hid it — gui/excel
  typecheck + unit vitest were not in T2 — closed (added to the t2 recipe). Gate evidence: `GATE: 87c6f77
  T2 16/16` (web typecheck+unit + e2e under `CELNET_ACCESS_MODE=enforce`). **Test-hygiene item filed**
  (`test-hygiene/raft-snapshot-convergence-deadline-flakes-under-t2-load`, P3): `celnet-parity
  raft_snapshot::snapshot_plus_tail_equals_full_log_single_node_replay` flaked once on its 30s `wait_until`
  convergence deadline under T2 parallel-test CPU contention (passed clean on resume, same code) — the
  load-flake class in the `cargo-gate-environment-pitfalls` memory; harden the deadline / reduce in-test
  contention so a landing gate can't be blocked by a transient. Dry-counter RESET 0/2.
- **Round 3 — NOT DRY (2026-06-11, session-B; 7-lens + security-deep + batch-D adversarial verify):**
  batch-D = LAND (pivot/entitlements/VV all confirmed). **2 genuine P1 blockers** (releaseReady=False):
  (1) `cross-asset-ws-pair-precedence` — ws/codec.rs:203 gives `pair` precedence over `underlying`,
  so equity/commodity/crypto are WS-unreachable and INVERSE_COIN misprices 4 orders of magnitude;
  **batch-D D2 FROZE the bug** (cross_asset_ws.rs:388 pins the wrong value; gui/excel declared
  not-exposed) instead of fixing the routing — workaround, not coverage (guardrail #2). FIX: let
  `underlying` win (or reject ambiguity); flip the pin test to assert CORRECT pricing; un-freeze the
  3 families in gui/excel WS corpora. (2) `dos/fix-frame-accumulation-unbounded` — celnet-fix
  acceptor has no message-size cap (the WS edge got one; FIX didn't). 8 P2/P3 → post-RC. Dry-counter
  RESET 0/2. Also: entitlements audit TEST capture bug (production emits correctly per the verifier;
  the test's subscriber capture is broken — separate fix in flight).
- **Round 2 — NOT DRY (2026-06-10, session-B; 7-lens + adversarial synthesis, workflow-driven):**
  22 genuine findings survived the judge (1 absorbed, 1 rejected). Headline **P0:
  `exotics/one-touch-at-hit-pairing-flip-circular-oracle`** — the at-hit one-touch closed form
  pairs (H/S)^{mu+lam} with the WRONG CDF argument (~28% high; 2.08x on far barriers); the golden
  oracle is a structural copy of the same flip, so corpus+selfcheck+5-client conformance jointly
  certified a wrong streamed price (the FRTB-0.75rho archetype recurring DESPITE the W0
  'no circular oracle — verifier-confirmed' claim; judge verified 3 independent ways: published-form
  pairing, exact numeric reproduction of the frozen vectors under the flip, T->inf limit + the
  model-free R*P_hit bound from the engine's own correct at-expiry branch). Full findings below
  (Round-2 section). **Dry-counter RESET: 0/2.**
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
  Effort XL. **DONE** (`6b00940` engine; `1484fb0` RPC; panel surfacing across all 4 clients
  2026-06-10 — see `clients/rfq-panel-surfacing` below).

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
  headless suite asserts polymorphic fn == SDK == server per class. Effort L. **DONE**
  (2026-06-10 session-B: `CELNET.INSTRUMENT` token + polymorphic PRICE/GREEKS/RFQ/SUBSCRIBE;
  18 per-product fns retired at byte-identical wire parity (all 20 golden families + 22
  optional-param cases + 4 cross-asset shapers; net −1,859 lines); metal-metal underliers
  rejected with a typed error (honesty); vitest 309 + LIVE real-edge e2e 91/91).
- **[W1/W3+] clients/gui-structuring-and-universe** — No structuring workspace (Ticket
  hardcodes per-product forms); universe is FX-pair-only. Asset-class-aware UniverseNavigator +
  composable contract-derived leg-builder Structuring workspace + surface-family switch. Oracle:
  Playwright e2e per class (navigate→structure→price→stream→risk) + axe; GUI price==server.
  Effort XL. **DONE** (structuring via GW2 + `80d1596`/`1484fb0`; the asset-class-aware
  universe + surface-family switch landed `18ef00f` — class rail in the scope leaf, seeded
  metal/equity/commodity/crypto universes (metal-vs-fiat only — honesty-verified vs the
  conventions registry), ticket pre-targeting via the test-gated `crossAssetInputsFor`
  inverse, honest per-class surface empty state; vitest 54 files / 564 tests, FX untouched).
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
  bit-identically; booking the winner == server execution record. Effort M. **DONE**
  (2026-06-10 session-B: server LpPanelConfig synthetic-LP panel + pinning + book-any-row +
  WS mirror; SDK RankedPanel handle (in-test law re-derivation, non-circular); CLI panel mode;
  GUI panel + LastLookRing (axe-clean); Excel panel spill. Adversarial verdict: law==spec,
  booking==pinned row, cross-client bit-identical. cargo 30/30 targets; GUI e2e 14/14;
  Excel e2e 93/93).
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


## Round-2 findings (2026-06-10 — 7-lens adversarially-judged; ALL verified with file:line evidence)

- **[P0/M] `exotics/one-touch-at-hit-pairing-flip-circular-oracle crates/celnet-exotics+crates/celnet-golden`** — At-hit one-touch closed form is wrong (~28% high; 2x on far barriers) and its golden oracle is a circular copy of the same flipped formula.
  Evidence: crates/celnet-exotics/src/touch.rs:136-170; crates/celnet-golden/src/oracle.rs:295-348; crates/celnet-golden/vectors/touch.json (touch-1/touch-3); the FRTB-0.75rho archetype recurring. Oracle: VERIFIED DECISIVELY by the judge, three independent ways. (1) Pairing check: crates/celnet-exotics/src/touch.rs:142-144 pairs (H/S)^{mu+lam} with Phi(base + drift_sign*lam*vsqt); the published discounted-first-passage form (Reiner-Rubinstein/Haug; Shreve first-passage) pairs (mu+lam) with Phi(base -… **OPEN.**
- **[P1/M] `pivot-wire-surfacing celnet-proto/celnet-server/clients`** — Pivot TRA is engine-only — no proto arm, no golden vector, no parity row, unreachable from any client.
  Evidence: crates/celnet-exotics/src/pivot.rs (engine exists); crates/celnet-proto/proto/celnet.proto; docs/COMPETITIVE-ANALYSIS.md:86 + catalogue table row. Oracle: VERIFIED: 'pivot' in celnet.proto appears only as the Accumulator strike field (proto:1073-1085); no Pivot product arm (arms run …25, 26-28 linear, 29 settlement_style, 30/31 in-flight); zero pivot hits in celnet-server/celnet-client/excel/gui beyond the accumulator field; no vectors/pivot*.json in … **OPEN.**
- **[P1/M] `cross-asset-client-priced-vectors-ws-e2e celnet-server/ws + excel/e2e + gui/e2e + celnet-golden`** — Cross-asset option families: golden vectors never priced by any client, server-WS cross-asset decode path has zero test coverage, and the Excel exclusion rationale is factually stale.
  Evidence: gui/test/conformance.test.ts:178-196; excel/e2e/corpus.ts:88-115; crates/celnet-golden/vectors/equity_option.json; crates/celnet-server/src/ws/codec.rs:221-228. Oracle: VERIFIED: gui/test/conformance.test.ts FAMILIES_NOT_EXPOSED_BY_GUI includes equity_option/commodity_option/crypto_option (and fx_forward/fx_swap/ndf); excel/e2e/corpus.ts:88-110 excludes them from the WS corpus path with the rationale that vectors carry generalized carry 'the FX-two-rate WS price pa… **OPEN.**
- **[P1/M] `gui-ticket-fx-vanilla-strategy-leg-builder-solve gui/src/products/strategy.tsx`** — GUI ticket cannot structure a custom-strike FX vanilla or edit strategy legs, and the visible caption falsely claims 'Solve inline'.
  Evidence: gui/src/products/strategy.tsx:18-130; gui/src/workspaces/TicketWorkspace.tsx:866; crates/celnet-proto/proto/celnet.proto:808-827; docs/GUI-DESIGN.md §4.1. Oracle: VERIFIED: gui/src/products/strategy.tsx doc comment states legs are 'template-fixed … There is no per-leg editing'; the lone VANILLA is vanillaInstrument(pair, tenorYears, "CALL", 0.25, …) — hardcoded 25-delta call, no strike entry, no put; leg strikes render the literal placeholder 'K —'; keywords … **OPEN.**
- **[P1/S] `bench/iai-instruction-gate-no-limits crates/celnet-bench/benches/iai_instructions.rs`** — The iai-callgrind 'instruction-count regression gate' is structurally unable to fail — no RegressionConfig/limits exist.
  Evidence: crates/celnet-bench/benches/iai_instructions.rs (no config anywhere); docs/ARCHITECTURE.md §1.2 footnote; .github/workflows/ci.yml iai-instructions job. Oracle: VERIFIED: grep for RegressionConfig/soft_limit/hard_limit/LibraryBenchmarkConfig in crates/celnet-bench/benches/iai_instructions.rs returns nothing — the lane prints a comparison and always exits 0, while docs/ARCHITECTURE.md §1.2 claims 'iai-callgrind for deterministic instruction-count regression … **OPEN.**
- **[P1/S] `interfaces-registry-lags-contract docs/INTERFACES.md + client-parity-matrix-stale docs/CLIENT-PARITY-MATRIX.md (merged: one docs-registry-sync remediation)`** — Contract registry docs lag the committed contract: INTERFACES.md's 'No proto symbol is present-but-unregistered' is false (arms 26-29 + Underlying/RFQ symbols unregistered) and CLIENT-PARITY-MATRIX.md documents 15 retired Excel per-product functions as live.
  Evidence: docs/INTERFACES.md:636-692; crates/celnet-proto/proto/celnet.proto:1266-1334; docs/CLIENT-PARITY-MATRIX.md:58-72; excel/src/functions/functions.ts. Oracle: VERIFIED: rg for fx_forward/fx_swap/ndf/settlement_style/perpetual/listed_future in docs/INTERFACES.md returns ZERO hits while celnet.proto has arms 26/27/28, field 29 (and in-flight 30/31); the registry table stops at basket=25 and :689-692 still asserts 'No proto symbol is present-but-unregistered… **OPEN.**
- **[P2/M] `exotics/qmc-pathwise-wiring celnet-exotics`** — Scrambled-Sobol/Brownian-bridge QMC is not wired into the TARF, accumulator, lookback, quanto, cliquet/Asian-MC and pivot pricers (still plain Philox CounterRng).
  Evidence: crates/celnet-exotics/src/{tarf,accumulator,lookback,quanto,pivot}.rs imports; docs/ANALYTICS-SPEC.md §5.4 + implemented-status note; docs/WORLD-CLASS-BACKLOG.md:139-141. Oracle: VERIFIED: tarf.rs:57, accumulator.rs:46, lookback.rs:44, quanto.rs:46 and pivot.rs:65 all import crate::rng::CounterRng; only american.rs and multiasset.rs use celnet_qmc. docs/ANALYTICS-SPEC.md's implemented-status note states 'Wiring celnet-qmc into celnet-exotics' path-dependent pricers … is the … **OPEN.**
- **[P2/L] `lsv/market-calibration-frontend celnet-exotics+celnet-heston`** — LSV booking model has no market-calibration front end: no Heston-backbone NLS calibration and no mixing-weight (eta) tuning to touch/DNT quotes.
  Evidence: crates/celnet-heston/src/lib.rs; crates/celnet-exotics/src/{lsv,particle,stochvol}.rs; docs/ANALYTICS-SPEC.md §4/§5.2/§5.3; docs/COMPETITIVE-ANALYSIS.md kACE claim. Oracle: VERIFIED: rg 'mixing' across celnet-exotics/src and celnet-surface/src = zero hits; rg 'calibrat' in celnet-heston/src/lib.rs hits doc comments only (Cui et al. citation for the CF, no calibrator). ANALYTICS-SPEC §5.2 names 'calibrate by nonlinear least squares with analytic gradients', §5.3 names t… **OPEN.**
- **[P2/M] `surface/event-weighted-clock celnet-surface+celnet-calendar`** — Event-weighted business-time clock is a trait seam only — the sole BusinessClock impl in the workspace is the identity CalendarClock.
  Evidence: crates/celnet-surface/src/termstructure.rs:24-50; docs/ANALYTICS-SPEC.md:173 (§3.6). Oracle: VERIFIED: repo-wide rg 'impl BusinessClock' = exactly one hit, crates/celnet-surface/src/termstructure.rs:48 (CalendarClock, tau(t)=t). ANALYTICS-SPEC §3.6 specifies term-structure interpolation 'with weekend/holiday and scheduled-event weighting (central-bank meetings, fixings)' as the market stand… **OPEN.**
- **[P2/L] `risk/pnl-attribution celnet-risk-cube`** — P&L attribution (Greeks-based P&L explain) is claimed in the competitive positioning but exists nowhere in the platform.
  Evidence: docs/COMPETITIVE-ANALYSIS.md:154-158; crates/celnet-risk-cube/src/ (scenario_grid/nonadditive/frtb only); ws/codec.rs AttributionRecord = identity only. Oracle: VERIFIED: docs/COMPETITIVE-ANALYSIS.md:156 claims Celnet 'collapses FMD-data + kACE-analytics + spreadsheet glue into one product with full Greeks, scenario/what-if, P&L attribution and IPV'; repo-wide grep for attribution/pnl-explain finds only the trade-identity AttributionRecord (who-traded, ws/c… **OPEN.**
- **[P2/M] `exotics/vanna-volga-overlay-magnitude-unvalidated crates/celnet-exotics/src/market_hedge_overlay.rs`** — Vanna-volga exotic smile overlay has no quantitative magnitude oracle — only flat-smile/sign/scaling tests; the spec-mandated VV-vs-LSV cross-validation is unimplemented.
  Evidence: crates/celnet-exotics/src/market_hedge_overlay.rs:286-427; docs/ANALYTICS-SPEC.md:179; celnet-server grep empty. Oracle: VERIFIED: market_hedge_overlay.rs has a small in-module suite (flat_smile_zero_cost + sign/scaling pattern, 8 test hits); ANALYTICS-SPEC §4 mandates 'Validate prices across methods (VV vs LSV-PDE vs LSV-MC)'; rg for market_hedge_overlay/hedge_smile in celnet-server/src = empty (not wired to the wire… **OPEN.**
- **[P2/M] `bench/exotic-vv-indicative-budget-gate crates/celnet-bench`** — §1.2 first-generation-exotic (VV indicative) p99 ≤ 50µs budget has no bench arm and no gate; its zero-alloc claim is untested.
  Evidence: crates/celnet-bench/Cargo.toml; crates/celnet-bench/src/bin/bench_gate.rs; docs/ARCHITECTURE.md:62. Oracle: VERIFIED: rg 'celnet-exotics' in crates/celnet-bench/Cargo.toml = empty — the bench crate cannot even reference the exotic pricers; bench_gate arms cover vanilla/surface/wire/fleet only. docs/ARCHITECTURE.md §1.2 commits the 50µs row and its footnote claims 'All latency NFRs are measured, not assert… **OPEN.**
- **[P2/M] `entitlements-trust-boundary-audit crates/celnet-entitlements + crates/celnet-server/src/services/risk`** — Entitlements trust boundary inverted (client-asserted principal; omitted ⇒ grant-all) and the documented per-decision audit is unimplemented.
  Evidence: crates/celnet-server/src/services/risk/convert.rs:79-107; crates/celnet-entitlements/src/lib.rs:48-57; docs/RISK-HIERARCHY.md §4. Oracle: VERIFIED: convert.rs:81-93 — an absent wire principal yields Principal::grant_all() (documented as 'the show-all-now posture'); any client can self-assert grants or omit the field to see the firm book. rg for observability/audit/tracing across services/risk/*.rs finds only a doc-comment word in fede… **OPEN.**
- **[P2/S] `fix-decoder-fuzz-target crates/celnet-fix + fuzz/`** — celnet-fix violates verification-contract clause (f): no fuzz target for the only byte parser fed external-counterparty bytes.
  Evidence: fuzz/Cargo.toml; docs/VERIFICATION-CONTRACT.md:186-190, 279; crates/celnet-fix/src/framing.rs. Oracle: VERIFIED: rg 'fix' in fuzz/Cargo.toml = empty (replog/journal/proto targets only); docs/VERIFICATION-CONTRACT.md:187 mandates a 'Fuzz target for any crate that decodes untrusted bytes (wire / journal / …)' and the checklist line 279 ties it to done-ness. The FIX framing/typed-decode/session layers r… **OPEN.**
- **[P2/S] `lint-per-client-manifest-axis tools/check-verification-coverage.mjs`** — Verification-coverage lint enforces only vector+parity; the per-client covered/not-exposed manifests are uncoordinated and 3 of 5 clients silently skip new families — the matrix's client axis is unenforced.
  Evidence: tools/check-verification-coverage.mjs:21-45; excel/e2e/corpus.ts; crates/celnet-cli/tests/conformance.rs:30-46; gui/test/conformance.test.ts. Oracle: VERIFIED: the lint's own header documents exactly two checks per arm — golden vector file + curated parity-row map (tools/check-verification-coverage.mjs:28-34); no client-manifest parsing anywhere. GUI alone asserts corpus completeness; Excel's EXCEL_FAMILIES/FAMILIES_NOT_EXPOSED and the CLI's CLI_… **OPEN.**
- **[P2/M] `gui-per-family-real-edge-ws-conformance gui/e2e + gui/src/data/wsCodec.ts`** — GUI per-family WS wire conformance gap: only 6 live workflow e2e tests exist; every non-vanilla GUI-bookable family is gated solely by GUI-self round-trip codec tests (drift-blind vs the server codec).
  Evidence: gui/e2e/workflows.e2e.ts (6 tests); gui/test/wsCodec.test.ts; excel/e2e/conformance.e2e.ts as the in-repo template. Oracle: VERIFIED: gui/e2e/workflows.e2e.ts contains exactly 6 tests (ticket-vanilla, RFQ panel, surface, stream, risk drill, status ribbon) — no per-family pricing rows; gui wire tests are fromWire(toWire(x)) self-inverses which by construction cannot detect cross-implementation drift; Excel by contrast has… **OPEN.**
- **[P2/M] `excel-instrument-strategy-family excel/src/functions/instrumentSpec.ts`** — Excel CELNET.INSTRUMENT cannot express the STRATEGY family (risk reversal / straddle / strangle / seagull) — the most-traded OTC FX structures.
  Evidence: excel/src/functions/instrumentSpec.ts; excel/e2e/corpus.ts:105-110; crates/celnet-golden/vectors/strategy.json. Oracle: VERIFIED: rg 'STRATEGY' in excel/src/functions/instrumentSpec.ts = empty; excel/e2e/corpus.ts documents the exclusion ('a multi-leg structure — built leg-by-leg in the GUI ticket / SDK, not as one Excel cell') — a rationale contradicted in the same grammar by BASKET's repeated matrix-row legs; the s… **OPEN.**
- **[P2/S] `cli-exotic-strike-silent-default crates/celnet-cli/src/cli.rs`** — CLI exotic strike grammar: strike-requiring families silently price a degenerate K=0 contract; american alone demands a separate per-subcommand --strike.
  Evidence: crates/celnet-cli/src/cli.rs:392-394, 969-1011, 1257; excel/src/functions/shaping.ts strike validation contrast. Oracle: VERIFIED: cli.rs ExoticArgs --strike has default_value_t = 0.0; dispatch builds inputs via a.market.to_market().inputs(a.strike) and passes it unvalidated into Digital and SingleBarrier specs (DNT/window-barrier/tarf/accumulator validate their OWN fields; only american validates strike at :1257 via … **OPEN.**
- **[P2/S] `cli-stream-rfq-tenor-expiry-drift crates/celnet-cli/src/cli.rs`** — CLI stream/rfq take --tenor and --expiry-years as independently-defaulted flags: `--tenor 3M` silently streams a 1Y-priced quote labelled 3M.
  Evidence: crates/celnet-cli/src/cli.rs:172-214; excel/src/functions/shaping.ts; gui/src/workspaces/TicketWorkspace.tsx TENOR_CHOICES. Oracle: VERIFIED: cli.rs StreamArgs and RfqArgs both declare --tenor default "1Y" and --expiry-years default 1.0 ('authoritative for pricing') with no coupling; the CLI tenor parser yields identity only. Excel derives expiryYears from the tenor and the GUI pairs label+years in TENOR_CHOICES — the CLI is the… **OPEN.**
- **[P3/S] `cli-mc-families-corpus-gate crates/celnet-cli`** — CLI MC-family argv seam un-gated against the corpus, and the conformance header's claimed scope contradicts its own CLI_FAMILIES constant.
  Evidence: crates/celnet-cli/tests/conformance.rs:12-19 vs :33-46; crates/celnet-cli/src/cli.rs:495-658. Oracle: VERIFIED: the header (conformance.rs:12-19) claims scope including 'asian / forward-start / quanto / cliquet / tarf / accumulator / lookback / american' and 'basket' plus 'Every family covered here is asserted reachable through the CLI', while CLI_FAMILIES is 12 families excluding asian/cliquet/tarf… **OPEN.**
- **[P3/S] `ws-edge-resource-caps crates/celnet-server/src/ws`** — WS edge accepts connections with default tungstenite limits (64 MiB messages) — no explicit frame/message caps on the streaming edge.
  Evidence: crates/celnet-server/src/ws/mod.rs:225. Oracle: VERIFIED: ws/mod.rs:225 calls tokio_tungstenite::accept_async(tcp) with no WebSocketConfig (defaults 64 MiB message / 16 MiB frame); no payload guard in the module. Contract frames are ~KB JSON. A clear deviation from the repo's otherwise-strict bounded-resource discipline (replog MAX_FRAME_LEN, bou… **OPEN.**
- **[P3/M] `proto/strategy-per-leg-expiry celnet-proto+celnet-server`** — Multi-leg strategies are single-expiry: proto Leg carries no per-leg tenor, so calendar/diagonal spreads cannot be booked as one structure.
  Evidence: crates/celnet-proto/proto/celnet.proto:806-827 (read this round); docs/COMPETITIVE-ANALYSIS.md OVML row. Oracle: VERIFIED: celnet.proto Leg = {option_type, strike, side, ratio} with no expiry field; Strategy legs share the enclosing Instrument expiry. A 1M-vs-3M calendar/diagonal — routinely-traded FX vol structures that OVML/venues book as one net-premium ticket — is unexpressible on the wire. Survived the ad… **OPEN.**

## (2026-06-28) Permissions-program tail + integration backlog

Surfaced while delivering the action-capability permissions program (slices 1–5,
merged to `main` in `cc2e3b9`; see the per-project memory `permissions-capability-track`
and `docs/plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md`). Each is a self-contained,
independently-landable feature. Format matches this file: dedup-key, evidence, acceptance.

- **[P1/M] `authz-quote-accept-gating celnet-proto+celnet-server+gui+excel`** — FX
  click-to-trade (`QuoteService.AcceptQuote`) is **not** capability-gated: the `QuoteAccept`
  wire message carries no `session_token`/`principal`, so `Execute·FxOptions` cannot be
  enforced server-side — unlike the FI accept path (`AcceptDeskQuote`), which is gated. This
  is the FX half of permissions slice 2.
  Evidence: `crates/celnet-proto/proto/celnet.proto` (QuoteAccept message lacks the auth
  fields the FI accept path carries); `crates/celnet-server/src/services/quote.rs` (no
  `authorize_caller(Capability(Execute, FxOptions))` on accept); adjacent reachability item
  `AcceptQuote.lp_id` elsewhere in this file. Acceptance: add `session_token` (+ optional
  `principal`) to QuoteAccept → regen → GUI/Excel/client inject the token (as every other
  unary RPC already does) → server gates `Capability(Execute, FxOptions)` requiring an
  authenticated session (finding-#3 guard: a body principal must not self-grant) → e2e under
  Enforce: an un-`Execute` trader's accept is denied; gui affordance already disabled (slice
  5b gates the button on `execute·fx_options`). **DONE** (`db824c3`, 2026-06-29): `QuoteAccept`
  already carried `session_token`/`principal`; the gap was the handler gating only `ReadAny`.
  `accept_quote` now gates `Capability(Execute, FxOptions)`; a grant-all body principal
  (admitted for read-side RequestQuote) is denied for AcceptQuote under Enforce. The FX server
  half is closed; an Excel sign-in (see `excel-signin-affordance-gating`) is the remaining
  client-UX item.
- **[P1/M] `authz-stream-session-gating celnet-server/src/services/stream.rs`** — the
  streaming session (`StreamSession` subscribe / execute frames) is not capability-gated
  (`Stream` / `Execute`). Hot path; gate at the WS stream driver without alloc/lock/log in the
  pinned core. **Coordinate** — `stream.rs` is touched by multiple sessions.
  Evidence: `crates/celnet-server/src/services/stream.rs` (subscribe/execute frame handlers);
  `crates/celnet-server/src/ws/mod.rs` stream-control dispatch. Acceptance: subscribe gates
  `Capability(Stream, <asset>)`, click-to-trade-over-stream gates `Capability(Execute,
  <asset>)`, both requiring an authenticated session; e2e under Enforce; the decoder/router
  lockstep test still green. **OPEN.**
- **[P2/M] `excel-signin-affordance-gating excel/`** — the Excel add-in cannot client-side-gate
  affordances on the caller's capabilities the way the GUI does (slice 5b), because it has **no
  interactive login**: it presents a pre-minted `sessionToken` via `ConnectionOptions` and never
  receives `LoginResponse.capabilities`. Server still enforces every gated RPC, so this is a UX
  parity gap, not a security hole.
  Evidence: `excel/src/transport/connection.ts` (token via `ConnectionOptions`, no login round-
  trip); `excel/src/taskpane/capability.ts` is the product price-matrix, a different concept.
  Acceptance: an Excel sign-in flow that performs `Login`, captures `capabilities`, and
  disables/explains ribbon + task-pane affordances by action×asset (Excel's idiom for
  "not permitted"); parity with the GUI's disable-+-tooltip discipline. **DONE** (`d15f122`,
  2026-06-29): `excel/src/contract/access.ts` (can()/denial-title byte-identical to the GUI +
  ENTRY_POINTS map), `transport/session.ts` (UserSession + bearer install), `connection.ts`
  login()/logout(), task-pane sign-in card, `functions.ts` cell gating (#CELNET_DENIED!).
  Posture: anonymous permissive, signed-in narrows, expired denies. 474 excel tests (+25).
- **[P2/M] `role-bundle-editing celnet-proto+celnet-server+gui`** — make the per-ROLE capability
  bundles (Admin ⇒ grant-all, Trader ⇒ all-but-`administer`, hardcoded in
  `sessions.rs::capabilities`/`TRADER_ACTIONS`) admin-editable + persisted, so an admin can change
  the baseline for ALL holders of a role at once (today only the per-user overlay is editable).
  **SPECCED/DEFERRED** (2026-06-29): lowest-value of the program — the per-user overlay
  (`caa7186`/`d1c440c`/`daeb5fd`) already lets an admin set any capability on any user, so this is
  a bulk-convenience layer, not new power. Design: a persisted role-bundle store (Role → Vec<Capability>,
  default = today's hardcoded values) snapshotted onto `AuthenticatedUser` at login exactly like the
  per-user overlay (`from_user`); `capabilities()` uses the snapshot as the base instead of the
  hardcoded match; admin-only `Get/SetRoleCapabilities` RPCs (proto + regen + WS codec + client) with
  fail-fast label validation + session-revoke-on-change for all holders; a GUI "Roles" section reusing
  the slice-4 matrix. Acceptance: e2e under Enforce — admin narrows the Trader bundle, every trader's
  effective set narrows on next login. **DONE** (`4072beb`, 2026-06-29): `IdentityStore.role_bundles`
  (serde-default, fail-fast label validation); `AuthenticatedUser.role_caps` snapshotted at login via
  `from_user_with_role_base`; `capabilities()` uses it as the non-admin base (Admin stays grant-all,
  immutable — Admin Set ⇒ `failed_precondition`). `Get/SetRoleCapabilities` RPCs (admin-only; Set
  revokes all holders' sessions) + WS codec/dispatch; GUI `RoleCapabilityEditor` panel. 318 server +
  842 gui tests; live e2e under Enforce (admin removes `book·fixed_income` from Trader → re-logged
  trader's Book-position control disabled).
- **[P3/S] `gui-a11y-book-view-domain-nav gui/e2e`** — `e2e/a11y.e2e.ts` "Book view" times out:
  `gotoWorkspace(page,"book")` clicks the Book rail button **without switching to the
  `fixed-income` domain**, where Book lives (`gui/src/lib/commands.ts`); the test starts in FX
  Options so the click never resolves. Pre-existing harness/domain-nav bug (not a product
  defect; the other 8 a11y views + the slice-4/5 e2e specs pass).
  Evidence: `gui/e2e/a11y.e2e.ts` "Book view"; `gui/e2e/helpers.ts` `gotoWorkspace`;
  `gui/src/lib/commands.ts` (Book ∈ fixed-income domain). Acceptance: `gotoWorkspace` switches
  domain before clicking the rail; the "Book view" a11y case passes. **DONE** (resolved-by-merge,
  2026-06-29): `gui/e2e/helpers.ts` now has a `RAIL_DOMAIN` map (`book`→"Fixed Income") and
  `gotoWorkspace` selects that tab before clicking the rail — the concurrent fixed-income
  session added it. No code change needed; confirm in the batched e2e run.
- **[P2/L] `fi-fix-quoting-wiring`** — wire the FIX transport for FI / dealer-quoting (the live
  counterparty venue is a deploy-tier dependency). Original integration item.
  Evidence: FI dealer-quoting desk shipped (`25293f8`, see memory `fi-dealer-quoting-shipped`)
  but over the native WS contract, not FIX. Acceptance: FIX acceptor/initiator carrying the
  RFQ/IOI/quote/accept lifecycle, validated against a session that replays a recorded FIX
  conversation; no commercial FIX engine (OSS only). **OPEN.**
- **[P2/M] `cli-fix-test-client`** — a CLI FIX test client to exercise the above, runnable
  against local and the UAT edge (`celnet@136.115.32.199`). Original item.
  Evidence: none yet. Acceptance: `celnet-cli` subcommand (or sibling bin) that initiates a FIX
  session and drives quote/accept; conformance test replaying a fixture. **OPEN.**
- **[P3/S] `bench-wire-load-bounded celnet-bench`** — get
  `celnet-bench::wire_load_runs_bounded_and_reports` green (architecture-tail item). **Re-verify
  post-merge** — the concurrent xasset session whose WIP this was landed in `cc2e3b9`; confirm
  whether it is still red before scheduling.
  Evidence: `crates/celnet-bench` (the named test). Acceptance: the bench runs bounded and
  reports p50/p99/p99.9 within the budget; `just`/cargo gate green. **DONE** (verified
  2026-06-29): `cargo test -p celnet-bench wire_load_runs_bounded_and_reports` exits 0 — the
  concurrent xasset session's WIP landed via the merges. No action needed.
- **[P2/M] `rates-daycount-stir-convexity celnet-rates`** — the rates engine needs 30/360
  day-count handling and STIR (futures) convexity adjustment in the short-end bootstrap.
  Independent and self-contained. Original item.
  Evidence: the rates curve bootstrap (FRA slice 1 `89b0588`); `docs/CONVENTIONS.md` day-count
  spec. Acceptance: 30/360 selectable per leg and matched to a QuantLib reference; STIR
  convexity adjustment in the short end validated against published/QuantLib prices (tolerances
  never silently loosened). **DONE** (`175d1e9`, 2026-06-29): `celnet-calendar`
  `thirty_360_bond_basis_*` (ISDA 2006 §4.16(f), QuantLib `Thirty360(BondBasis)` integer-exact
  across 11 edge cases); `celnet-rates` `AccrualBasis` threaded into FRA + OIS schedule;
  `bootstrap_futures_strip` debiases by ½·σ²·T₁·T₂ (matches Hull worked example to 1e-9). 97+65
  tests; kept in quant crates (did not extend wire-facing `celnet_types::DayCount`).

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
