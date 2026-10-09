# Celnet — Master Evolution Program: Exceed SynOption (multi-asset, no-legacy, fully verified)

> Chief-Architect synthesis of five read-only research/critique lenses (SynOption/market
> scope, architecture, contract/API, clients, verification). This is the single governing
> program. It supersedes the asset-class-expansion sections of `COMPLETION-PROGRAM.md`,
> `LEADERSHIP-PROGRAM.md`, `POST-COMPLETION-AUDIT.md`, and
> `CAPABILITIES-REVITALISATION-PLAN.md` for the multi-asset goal (those remain valid for
> their FX-options scope; cross-link, do not duplicate — guardrail #10). The single live
> backlog is `docs/WORLD-CLASS-BACKLOG.md`.

## 0. Governing goal & guardrails

Evolve Celnet from an FX-options platform that already EXCEEDS SynOption on FX-option
product/analytics breadth into a **cross-asset derivatives platform** that exceeds
SynOption's *asset-class* coverage (FX + digital-asset/crypto via Synchro/OrBit, metals,
the TARF/Pivot/Accumulator structured set, the 75-pair panel) — **fully integrated, no
legacy, SOTA, composable, with redesigned/evolved APIs + Excel + GUI + SDK + CLI, FULLY
VERIFIED end-to-end**.

Binding constraints: ONE clean unversioned contract (evolve in place, no `schema_version`,
no N/N−1 — #9); delete superseded code (#10); vendor-/person-neutral purpose-named
identifiers (#8); api-first parity across server/SDK/CLI/Excel/GUI; composable; SOTA methods
with cited references; every new asset class/product validated vs an **independent** oracle
(QuantLib / closed-form / published / code-disjoint MC) + cross-client parity + e2e.

**Honest boundary (NOT gaps, validated at deploy not in-repo):** CUDA/NVIDIA absolute
throughput, cross-host wire p99, live LP-panel/venue connectivity & regulatory (MAS-RMO)
status, live crypto/metal fixing VALUES, live JVM CelNet estate lifecycle. In-repo proves
payoff math + convention identity + routing/quorum arithmetic + relative regression + a
host-local ratio. These are excluded from the convergence dry-round count.

## 1. Target asset-class + product SUPERSET (concrete)

Celnet already exceeds SynOption on FX-option breadth: 18 on-wire product families
(vanilla, strategy, single/double/window barrier, digital, touch, var/vol swap, Asian,
forward-start/cliquet, quanto, TARF, accumulator, lookback, American/Bermudan,
basket/best-of/worst-of), 5 smile families (VV/SABR/SVI/SSVI/eSSVI), LSV + Heston, GPU,
FRTB/XVA, open SDK, single contract, 5 clients. SynOption's true moats are the regulated
multi-dealer venue and the LP panel — **not** product depth.

The superset to BUILD (each tagged: NEW = genuine in-repo gap, IN-CELNET = already present,
ENV = deploy-bound):

**Asset classes**
- FX (deliverable + NDF) — IN-CELNET (19 pairs; expand toward >75 panel — NEW breadth).
- Precious metals XAU/XAG — IN-CELNET; extend to XPT/XPD + metal crosses — NEW breadth.
- Digital-asset / crypto options (BTC/ETH/… + inverse/coin-settled + 24×7 + funding carry)
  — NEW (the single biggest "exceed SynOption" move; matches Synchro/OrBit).
- Equity / index options (dividend + repo carry, strike/moneyness surface) — NEW
  (cross-asset reach toward Bloomberg-MARS positioning; lower priority than crypto).
- Commodity (listed-future) options (Black-76, convenience yield, futures-settled) — NEW
  (lower priority; equity/commodity share the same generic-carry seam).
- Rates options (normal/Bachelier vol, curve-bucketed rho) — NEW, lowest priority
  (designed seam only unless a wave is funded; honestly deferred, not faked).

**Linear / forward products (highest-value, lowest-risk breadth)**
- FX outright forward, FX swap (near+far), NDF — NEW first-class priced products (today only
  derived math on `OptionInputs.forward()` + a market observable; conventions already model
  NDF settlement + FixingSource).

**Structured / exotic additions**
- Pivot (the one SynOption structured type Celnet lacks) — NEW.
- Perpetual / funding-settled crypto structure; option on a listed future — NEW payoff shapes.
- Second-gen exotics depth (KIKO, double-touch family, corridor variance, fader/range-accrual,
  power/quanto-power) — NEW, P3 (depth, behind the asset-class superset).

**Workflow**
- Multi-dealer RFQ aggregation (fan to N quote sources incl. external LP over the existing
  FIX 4.4 engine, rank best bid/offer, audited panel) — NEW algorithm in-repo; the regulated
  venue + live LP connectivity are ENV.

## 2. Evolved no-legacy composable ARCHITECTURE

The 34-crate workspace is correctly one-way acyclic (interface crates first; hot
`celnet-engine` never touches the async edge; GPU/plugin behind traits). The FX coupling is
concentrated in three load-bearing seams that run through *every* layer:

1. `celnet-types::VanillaInputs`/`Greeks` — the Garman-Kohlhagen shape (spot/strike/vol/t +
   `r_dom`/`r_for`, two-rho, spot/forward-delta) baked into the Layer-0 interface crate.
2. `celnet-proto` `Instrument`/`MarketContext`/`CcyPair` — no asset-class discriminator;
   `{spot,vol,r_dom,r_for}` hardwired; all 18 arms FX. `Ccy([u8;3])` cannot name BTC/ETH/USDT.
3. `celnet-plugin-api::PricingModel` — signature literally `price(OptionType, &VanillaInputs)
   -> Greeks`; the headline "user-extensible asset classes" differentiator cannot express a
   non-FX model.

**Evolution (additive at the graph level, generalize-in-place at the leaf level):**

- New Layer-0/1 vocabulary in `celnet-types`/`celnet-core`:
  - `Underlying` enum — `Fx(CcyPair) | Metal(MetalPair) | DigitalAsset(CryptoPair) |
    Equity(EquityRef) | Listed(ContractRef)` carrying asset class + settlement style
    (deliverable / cash@fixing / inverse-coin) + quote orientation. `Symbol` newtype
    (length-validated) for non-3-char symbols; `Ccy` retained for FX legs.
  - `Carry` model — subsumes FX two-rate `(r_dom,r_for)`, equity `(r,q)`, commodity
    cost-of-carry/convenience, crypto funding behind a `forward()`/`df()` interface. Cleanest
    generalization is generalized-BSM `(r, b)` with FX = `r=r_dom, b=r_dom−r_for`.
  - `Sensitivities` — generic risk-factor-named set (delta-vs-underlying, vega-vs-vol-factor,
    discount-rho, carry-rho) with FX two-rho recovered as the FX projection.
- New thin **pricing-core** trait layer (a `celnet-pricing-core` crate, or folded into
  `celnet-core`) defining forward/discount/payoff/sensitivity over the new vocabulary.
- `celnet-vanilla` becomes the **FX (GK) leaf** of that trait (rename to `celnet-fx-vanilla`
  per #10 if it clarifies; FX path byte-identical). Siblings: `celnet-crypto-vanilla`,
  `celnet-equity-vanilla`, `celnet-commodity-vanilla` (rates designed-only unless funded).
- New `celnet-linear` crate: forward / swap / NDF closed-form PV + Greeks.
- `celnet-surface` splits into an asset-class-neutral **surface core** (arb-free interp,
  butterfly/calendar gates) + an FX-smile leaf (delta-axis/RR/BF) + strike/moneyness leaves
  for equity/crypto/commodity. Exotics PDE/MC/QMC engines stay neutral (they operate on
  paths/grids) — parameterize only payoff + carry.
- `celnet-risk-normalize`/`-cube` `CanonicalLeaf` generalizes to a cross-asset risk fact
  (generic factor set + asset-class tag + underlying ref); the additive-merge / non-additive
  re-gather algebra is already factor-generic and stays unchanged. FRTB-SA extends from the
  FX bucket to GIRR/equity/commodity/CSR buckets as additional gated parity rows.
- Workspace hygiene (do FIRST, before the crate explosion): register `celnet-xva`,
  `celnet-heston`, `celnet-qmc` in `[workspace.dependencies]`; switch the 11 internal
  path-deps to `.workspace = true`; add a lint asserting no internal path-dep outside the
  registry. This protects the disjoint-lane property during the multi-asset fan-out.

**Sequencing rule (critical):** the four interface-crate generalizations (types,
pricing-core, proto, plugin-api) land together in ONE coordinated core wave with FX proven
byte-identical, THEN per-asset-class leaves fan out to disjoint lanes. Dribbling contract
edits across lanes would thrash the frozen seams and break disjoint ownership.

## 3. One-unversioned-CONTRACT redesign (celnet.proto)

Keep the 5-service shape and the flat product oneof. Generalize the *vocabulary* in place
(full cutover, no `schema_version`, no parallel old/new fields):

- **`Underlying` message** with `oneof ref { CcyPair fx; CryptoPair crypto; MetalPair metal;
  EquityRef equity; ListedContract listed; }` + `Ccy settlement_ccy`. Replace `CcyPair pair`
  on `Instrument`, `BasketLeg`, all Risk*/Surface*/MarketSeries* messages, and
  `OrgKey.ccy_pair`. FX is the first-class arm → existing flows byte-equivalent after the
  codec maps the old `pair` into `Underlying.fx`.
- **`MarketContext` / `VanillaInputs`**: `{spot, vol, discount_rate, CarryModel carry}` where
  `CarryModel` is `oneof { FxRates fx; ContinuousYield yield; DividendRepo equity; … }`. FX
  maps to `FxRates{r_for}` + `discount_rate=r_dom`; the engine reads `(r,b)`. Delete top-level
  `r_dom`/`r_for`. `RiskPosition.inputs` and scenario `base_market` inherit this for free.
- **`Greeks`**: keep the asset-class-invariant strip flat; replace `rho_dom`/`rho_for` with a
  carry-tagged `RateSensitivities{discount_rho, carry_rho}` (fx⇒rho_for, equity⇒dividend-rho,
  commodity⇒carry-rho). FX `discount_rho==rho_dom`, `carry_rho==rho_for` bit-identical.
- **Product oneof**: keep flat. Most arms (vanilla/barrier/digital/touch/Asian/lookback/
  American/var-vol) become underlying-agnostic once `Underlying`+`CarryModel` generalize.
  Add new arms ONLY for new payoff SHAPES: `FxForward`, `FxSwap`, `Ndf`, `Pivot`,
  `PerpetualOption`, `ListedFutureOption`. Refactor `BasketLeg` to embed `Underlying`+
  `CarryModel` (enables cross-asset rainbow baskets — beyond SynOption). Server enforces a
  **product × underlying validity matrix** (invalid combo ⇒ `INVALID_ARGUMENT`, never silent
  fallback — mirrors the existing PricingModel guard).
- **Surface input**: `MarkSurfaceRequest`/`Smile` gain `oneof quotes { BrokerQuoteSet
  fx_broker; StrikeVolGrid strike_axis; MoneynessVolSlice log_moneyness; }`; `SmilePoint`
  gains `oneof axis { double delta; double strike; double log_moneyness; }`; widen
  `MarketObservable`; `Conventions` becomes optional for non-FX underlyings. FX RR/BF is the
  default arm (no-op for existing surfaces).
- **`VegaPillar`** gains `oneof axis { int32 delta_bp; double strike; }`. `RISK_DIMENSION_
  CCY_PAIR` → `RISK_DIMENSION_UNDERLYING`.

Use the `protobuf` skill for enum-prefix / field-numbering hygiene. Validate every redesign
step with a celnet-proto convert round-trip + `to_bits` byte-identity for the FX projection
(the no-regression gate).

## 4. CLIENT redesign (GUI / Excel / SDK / CLI) — composable + intuitive, api-first

- **SDK**: typed builders per product family over `Underlying` (`Instrument::tarf(underlying)
  .strike(k).target(t)…` with sane MC defaults) replacing hand-built deep enum literals.
  FX-default constructors keep `celnet price EURUSD` source-compatible in spirit; add
  `Underlying::crypto(…)/::equity(…)` + `CarryModel::dividend_yield(…)`. Generalized
  `StrikeSpec` (absolute / moneyness / log-moneyness / fx-delta). One runnable example per
  asset class + risk-aggregation + surface-mark, all gated against a real edge.
- **CLI**: `--underlying`/`--asset-class` flag defaulting to FX-pair parsing (preserves
  `celnet price EURUSD …`, enables `celnet price --crypto BTCUSDT …`).
- **Excel**: retire the one-function-per-product table (27 fns, BARRIER has 12 positional
  args) for a small POLYMORPHIC set over the generalized contract — `CELNET.PRICE(underlying,
  product, terms-range)`, `.QUOTE`, `.SURFACE`, `.RISK` where `terms` is a 2-col key/value
  range — plus `CELNET.INSTRUMENT(…)` returning an opaque spec token consumable by
  PRICE/QUOTE/SUBSCRIBE so structures compose from cells. Keep the convention-transparency
  footer and typed `#CELNET_*` errors. Add a **real-edge** Excel conformance suite (today
  Excel only uses an in-process `FakeSocket` — its wire conformance is UNPROVEN e2e).
- **GUI**: asset-class-aware `UniverseNavigator` (AssetClass → class-native buckets); a new
  **Structuring workspace** replacing hardcoded per-product Ticket forms with a composable
  contract-derived leg-builder (a new product needs zero bespoke form code); a surface-family
  switch (FX RR/BF vs strike/moneyness) bound to the underlying's class.
- **api-first parity gate (NEW, executable):** a language-neutral golden-vector corpus
  (`crates/celnet-golden/vectors/*.json`, generated by the engine, each value independently
  cross-checked by its parity oracle), plus a conformance harness per client that boots the
  REAL edge (the `gui/e2e/demoEdge.ts` pattern) and asserts each surface == the frozen vector
  AND that the case is REACHABLE from every client. `CLIENT-PARITY-MATRIX.md` becomes
  generated FROM the passing harness. Seed with the current FX set (green now); require a row
  per new (asset-class, product) before that capability is "done".

## 5. Dependency-ordered WAVE plan

Each wave: disjoint tracks where possible; independent oracle + green gate; cross-client
parity touch; a built-in critique/review step (adversarial-verify the diff for lowered gates
/ `#[ignore]` / lint-dodge / circular oracles / overclaim, then independently re-gate). See
the structured `waves` array for crates/oracle/parity/deps/priority per wave. Summary:

- **W0 — Verification & hygiene foundation (P0):** golden-vector corpus + 5-client
  conformance harness (FX, green); Excel real-edge suite; `docs/VERIFICATION-CONTRACT.md`
  (per-class mandatory gate set + proto-arm⇄parity-row⇄vector CI lint); workspace-dep
  registry cleanup + path-dep lint. Raises the floor BEFORE expansion.
- **W1 — Multi-asset CORE wave (P0, single coordinated wave):** generalize `celnet-types`
  (Underlying/Carry/Sensitivities), the new pricing-core trait, `celnet-proto` (Underlying/
  CarryModel/RateSensitivities + product validity matrix), `celnet-plugin-api` (generalized
  PricingModel + WIT/wasmi ABI). FX proven byte-identical end-to-end (golden + parity +
  4 plugin gates + 5-client conformance all unchanged) — the no-regression gate. Lodestar
  auto-indexes; run `mcp__lodestar__detect_changes` to confirm scope; reconcile INTERFACES/ARCHITECTURE docs.
- **W2 — FX linear products + pair/metals breadth (P0/P2):** Track A `celnet-linear`
  (forward/swap/NDF, QuantLib + closed-form oracle). Track B pair universe → >75 + XPT/XPD +
  metal crosses (EMTA/ISDA/LBMA tables + independent rata-die oracle).
- **W3 — Crypto asset class (P1, XL):** `celnet-crypto-vanilla` + 24×7 calendar +
  inverse/coin-settled payoff + funding carry; crypto surface leaf. Oracle: GK-with-funding
  for linear-settled, independent closed-form + code-disjoint MC for inverse-settled,
  published Deribit specs for conventions. Cross-client parity for ≥1 crypto vanilla.
- **W4 — Structured + workflow (P1):** Track A `celnet-exotics/pivot.rs` (code-disjoint MC
  oracle + degenerate→TARF limit) + new payoff arms (perpetual/listed-future option). Track B
  multi-dealer RFQ aggregation (≥3 synthetic LP responders loopback oracle; best-price
  selection, tie-break, last-look; live LP = ENV).
- **W5 — Cross-asset risk + equity/commodity leaves (P1/P2):** Track A generalize
  `CanonicalLeaf` to cross-asset fact + FRTB GIRR/equity/commodity/CSR buckets (longhand
  recomputation oracle; re-derive constants from BCBS text — the 0.75ρ circular-oracle
  lesson). Track B `celnet-equity-vanilla` + `celnet-commodity-vanilla` (QuantLib
  AnalyticEuropeanEngine / Black-76 oracle).
- **W6 — Rigor uplift + 2nd-gen exotics depth (P2/P3):** mutation/fuzz/coverage floor across
  exotics/surface/risk/MC crates (≥90% kill-rate, fuzz target per numeric crate); KIKO /
  double-touch family / corridor variance / fader / power options (PDE≈MC≈analytic +
  QuantLib where available).
- **W7 — Convergence rounds (recurring):** run the §6 loop until 2 consecutive dry rounds.

Genuinely deploy-bound items (NVIDIA absolutes, cross-host wire, live estate, regulated
venue) are designed + seamed + ADR'd in-repo and validated at deploy — never block or claim
in-repo (this is the existing honest-boundary discipline, unchanged).

## 6. CONVERGENCE LOOP

`docs/WORLD-CLASS-BACKLOG.md` is the SINGLE backlog truth. A round = SEVEN parallel
read-only critique lenses, each emitting findings `{lens, title, oracle, client-parity
impact, effort, priority, dedup-key}`:

1. **SOTA scope** — vs SynOption/competitors + ≤current academic refs (cite the paper).
2. **Numerical correctness** — independent-oracle audit; hunt circular oracles.
3. **Completeness** — every proto arm reachable from 5 clients + parity-gated.
4. **Performance** — vs ARCHITECTURE §1.2 budgets + the bench gates.
5. **Rigor/security** — mutation kill-rate, fuzz corpus, audit/deny, token/auth paths.
6. **API/UX** — contract ergonomics, GUI/Excel/CLI/SDK intuitiveness, e2e workflows.
7. **Doc accuracy** — every doc claim cites a real path; no stale "deferred" for shipped code.

**DEDUP RULE:** before adding, search by dedup-key (normalized title + crate path); on match,
append evidence to the existing item — never duplicate.

**GENUINE-GAP CALIBRATION:** a finding counts as a gap only if it is (a) real, (b)
in-repo-buildable, (c) bar-raising, (d) NOT environment/deploy-bound, (e) NOT padding/style.

**DRY CRITERION:** a round is *dry* when it adds zero new genuine gaps. **STOP** when TWO
consecutive full rounds are dry. Oracle for the loop itself: the backlog's monotonic shrink +
the dry-round counter.

## 7. Git-push MILESTONES

Branch-first off `main`; push ONLY to the single sanctioned `origin`
(github.com/soarsa/celnet). Push a milestone ONLY when ALL hold:
1. full-workspace `just check` prints the literal `All gates passed.` (verified, NEVER the
   background-wrapper exit code — the recorded contention-bug lesson);
2. the 5-client conformance harness is green against a freshly-booted edge;
3. all client suites pass (Rust nextest + GUI vitest + GUI Playwright real-edge e2e + Excel
   real-edge e2e);
4. the per-class verification contract is complete for any new product;
5. docs reconciled (no stale "deferred" for shipped code) + lodestar auto-indexed (run `mcp__lodestar__detect_changes` to confirm scope).

A milestone = one asset class or one cross-cutting capability fully landed across all 5
clients with all gates green. See the structured `milestones` array.

## 8. Honest scope/effort

This is large: ~4 interface-crate redesigns + ~6 new leaf crates + 5-client redesign +
~10 new parity rows + a new conformance estate, over W0–W7. W1 (the core wave) is the
highest-risk single change (it touches all frozen seams at once) and is gated entirely on
"FX byte-identical". Crypto (W3) and cross-asset risk (W5) are XL. The convergence loop is
open-ended by design but bounded by the 2-dry-round stop. Deploy-bound absolutes are
explicitly out of in-repo scope and are not failures of this program.
