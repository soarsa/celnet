# Celnet front-end redesign — knowledge capture

Sources: lodestar graph `github.com-soarsa-celnet` (24,609 nodes / 75,708 edges, `status:ready`), verified-knowledge claims, direct Read of gui/excel TS source. All citations are `qualified_name` + `file:line`.

---

## Part A — Asset-class → quant-detail selection trees

### A.0 The root split: `Underlying` (celnet-types) and the two carry regimes

`Underlying` (`crates/celnet-types/src/lib.rs:410-421`) is the top of every selection tree:

```
Fx(CcyPair) | Metal(MetalPair) | Equity(EquityRef) | Commodity(CommodityRef) | DigitalAsset(CryptoPair)
```

GUI mirrors this 1:1 as `AssetClass = "FX" | "METAL" | "EQUITY" | "COMMODITY" | "CRYPTO"` (`gui/src/products/types.ts:22`), and `underlierAssetClass()` (`gui/src/lib/assetUniverse.ts:45-58`) maps `Underlying.kind` → `AssetClass` for the UI. This is the mandatory first drill-down step for the redesign: **asset class must be selected before underlier**, because it gates everything downstream (product menu, model menu, convention set).

The economics split into exactly two `Carry` regimes (`crates/celnet-types/src/lib.rs:919-934`):
- `FxRates { r_dom, r_for }` — FX and Metal (two-currency carry).
- `CostOfCarry { r, b }` — Equity, Commodity, Crypto (single discount rate + net carry `b`; the "carry seam" the cross-asset leaves were built on).

This is *the* structural reason FX/Metal get the full convention/surface apparatus below and Equity/Commodity/Crypto don't (yet) — they ride a deliberately narrower generalized-BSM/Black-76/funding-carry seam instead.

### A.1 Product/structure selection — what's offered per class

The full product enumeration lives in `Product` (`crates/celnet-client/src/vocab.rs:1697-2090`, 24 arms): `Vanilla, Strategy, SingleBarrier, DoubleBarrier, Digital, Touch, VarianceSwap, VolatilitySwap, AsianOption, ForwardStart, Cliquet, Quanto, Tarf, Pivot, Accumulator, Lookback, WindowBarrier, American, Basket, FxForward, FxSwap, Ndf, PerpetualOption, ListedFutureOption`.

The GUI's live capability kernel gates which of these a selected asset class may build — **this is the exact matrix the redesign's structure-picker must drive off**, `gui/src/products/capability.ts`:

```
ALL_PRODUCT_KINDS (24, capability.ts:30-55)        — everything above, "vanilla".."listedFutureOption"
CROSS_ASSET_PRICEABLE (capability.ts:64-68)        — ["vanilla", "perpetualOption", "listedFutureOption"]
PRICEABLE: Record<AssetClass, Set<ProductKind>> (capability.ts:75-81)
  FX:        ALL_PRODUCT_KINDS   (full exotics book)
  METAL:     ALL_PRODUCT_KINDS   (full exotics book — same as FX)
  EQUITY:    CROSS_ASSET_PRICEABLE   (vanilla / perpetual / listed-future-option only)
  COMMODITY: CROSS_ASSET_PRICEABLE
  CRYPTO:    CROSS_ASSET_PRICEABLE
```

`galleryCardStates()` (capability.ts, `in_degree:1`) turns this into per-card `priceable`/`dimmed(reason)` UI state — the redesign's product gallery should reuse this function directly rather than re-deriving eligibility. `ProductGroup` (`gui/src/products/types.ts:24-31`) gives the existing gallery taxonomy: *Vanilla & strategies, Linear (forwards & swaps), Barriers & digitals, Volatility, Path-dependent, Structured, Cross-asset*.

**FX/Metal only** (barriers, digitals, touches, Asian, TARF/Pivot/Accumulator, cliquet, lookback, American, basket, window-barrier, quanto, variance/vol swaps, FX forward/swap/NDF) — the full exotics book, priced by closed-form + ADI-PDE + Monte-Carlo engines in `celnet-exotics`/`celnet-vanilla`/`celnet-engine`.
**Equity/Commodity/Crypto**: vanilla, `PerpetualOption` (`Product::PerpetualOption`, vocab.rs ~2044-2058, closed-form no-expiry American), `ListedFutureOption` (vocab.rs ~2060-2090, needs `future_symbol` + `future_expiry_years >= expiry_years` + `Margining` convention) — these two arms were purpose-built "asset-class-agnostic" so cross-asset gets *some* exotic-like reach without the FX-only barrier/path-dependent machinery.

### A.2 FX (+ Metal) — full quant-detail tree

`FX/Metal underlier → product (any of 24) → model → smile/surface → conventions → calibration → greeks`

**Models selectable** — `SmileModel` (`crates/celnet-types/src/lib.rs:627-640`, mirrored in `crates/celnet-surface/src/surface.rs:35-47`), routed by the pure selector `build_model_smile()` (`crates/celnet-surface/src/calibrate.rs:182-208`, verified claim `cl_f47d4594e9b175b2`: *"the single pure selector that reaches all FIVE smile families through one contract"*):

| `SmileModel` arm | Method | Calibrator | Params (backing struct) |
|---|---|---|---|
| `MarketHedge` (default) | vanna-volga, broker-pillar interpolation | `build_smile()` | market-hedge pillars |
| `StochasticVol` | SABR | `fit_sabr()` | `StochasticVolParams{alpha,beta,rho,nu,forward,t}` (`crates/celnet-surface/src/stochvol.rs:48-61`) |
| `Parametric` | SVI (single slice) | `fit_svi()` | total-variance slice |
| `ParametricSurface` | SSVI (surface-level, closed-form arb-free) | `fit_ssvi()` | surface family |
| `ExtendedSurface` | eSSVI (+ maturity-dependent correlation) | `fit_essvi()` | extended surface family |

This is the "5-smile surface" — model choice is a first-class per-request parameter (`decode_smile_model`/`wire_smile_model`, `crates/celnet-server/src/services/surface.rs:184-226`; GUI round-trip `smileModelToWire`, `gui/src/data/wsCodec.ts:1661`).

Separately, `celnet-heston` (`heston_c4` etc., `crates/celnet-heston/src/lib.rs`) is **not** a `SmileModel` arm — it backs the closed-form fair-variance/fair-volatility replication for `VarianceSwap`/`VolatilitySwap` (`fair_volatility`, `crates/celnet-exotics/src/vol_swap.rs:154-185`), independently oracle-checked against a published Heston reference (`heston_transforms_match_published_reference`, `crates/celnet-golden/tests/heston_grid.rs`). The redesign should not offer Heston in the same model picker as SABR/SVI/SSVI/eSSVI — it's a different selection axis (variance-swap replication engine, not a smile fit).

**Conventions** — `ConventionRecord` (`crates/celnet-conventions/src/record.rs:26-51`), resolved per `(pair, tenor)`:
- `delta: DeltaConvention` — `SpotUnadjusted | ForwardUnadjusted | SpotPremiumAdjusted | ForwardPremiumAdjusted` (`celnet-types/src/lib.rs:655-664`)
- `atm: AtmConvention` (ATM-forward vs delta-neutral straddle)
- `premium_style: PremiumStyle` — `DomesticPips | PercentForeign | PercentDomestic | ForeignPips` (`lib.rs:677-686`)
- `cut: Cut` — `NewYork1000 | Tokyo1500` (`lib.rs:723-728`)
- `day_count_vol` / `day_count_accrual_for` / `day_count_accrual_dom`: `DayCount` — `Act365Fixed | Act360` (`lib.rs:732-737`)
- `settlement: Settlement` — `Deliverable | NonDeliverable` (`lib.rs:741-746`)

`InstrumentClass` (`crates/celnet-conventions/src/registry.rs:96-106`) splits `Fiat` (two-currency FX) vs `PreciousMetal` (metal-as-base, lease rate modelled as the foreign rate) — Metal reuses the *entire* FX convention/surface stack via this one enum, which is why Metal shares `PRICEABLE`'s full product set with FX.

**Calibration inputs**: `MarketContext{spot, carry: Carry::FxRates{r_dom,r_for}, t, conventions}` (`crates/celnet-surface/src/quotes.rs:155-164`) + `MarketQuotes` broker pillars (ATM/RR/BF by tenor) feed `build_model_smile`.

**Greeks set**: the full 14-Greek strip (`Greeks` in `gui/src/data/contract.ts`, exercised in `gui/src/components/GreeksStrip.stories.tsx:15-30`): `price, deltaSpot, deltaForward, gamma, vega, theta, rhoDom, rhoFor, vanna, volga, charm, speed, zomma, color`. For FX/Metal, `rhoDom`/`rhoFor` are the literal domestic/foreign discount-curve rhos (GreeksStrip.stories.tsx:113-116, "FX / Metal — domestic rate ρd and foreign rate ρf").

### A.3 Fixed Income / Rates — curve-first tree, narrower wire surface than the math crate

`Curve/pillar selection → instrument type → convention → pricing`. Structurally different from FX: **the user selects a curve first** (bootstrap inputs), not a smile.

**Curve bootstrap instruments** (`celnet-rates` crate — richly built): OIS deposits (`bootstrap_ois`, `crates/celnet-rates/src/bootstrap.rs:75-111`, `OisQuote`), FRAs (`Fra`, `fra_pv`/`fra_par_rate`/`fra_risk`, `crates/celnet-rates/src/fra.rs`), vanilla fixed/float swaps (`crates/celnet-rates/src/vanilla_swap.rs`), STIR futures + convexity-adjusted futures strips (`bootstrap_futures_strip`, `StirFuturesQuote`, `crates/celnet-rates/src/futures_strip.rs:31-158`, with an explicit convexity-debias test `convexity_debiases_the_forward_downward:211-231`), and cash bonds with z-spread/g-spread/asset-swap-spread (`crates/celnet-rates/src/bond.rs`).

**Day-count**: `thirty_360_bond_basis_dispatches_to_calendar` (`crates/celnet-rates/src/daycount.rs:83-97`) — bond-specific 30/360 conventions distinct from the FX `DayCount` enum, dispatched through `celnet-calendar`.

**Entitlements gate**: `AssetClass::FixedIncome` exists in the capability kernel (`crates/celnet-entitlements/src/capability.rs:119-124`, alongside `FxOptions`) — so FI is a first-class entitled asset class — but **the wire/server contract currently exposes only the OIS arm**: `RatesInstrument` is a oneof whose JSON codec accepts *only* `ois` (`rates_instrument_from_json`, `crates/celnet-server/src/ws/codec.rs:1275-1285`, docstring: *"exactly the `ois` arm in the P0 contract"*) and `price_rates()` (`crates/celnet-server/src/rates_pricing.rs:212-234`) matches only `rates_instrument::Instrument::Ois`. **This is a real UX-affecting gap the redesign must design around**: the FI selection tree should surface OIS end-to-end today, and either hide or explicitly badge FRA/swap/bond/STIR-strip as "priced in the engine, not yet wired" rather than imply parity with FX depth.

**Models/smile**: none — rates pricing here is curve-discounting (deposit/OIS/annuity math), not a vol-surface product; no `SmileModel` selection applies to the FI tree as currently scoped.

### A.4 Crypto — linear vs inverse settlement, funding-driven carry

`DigitalAsset(CryptoPair){base, quote}` (`crates/celnet-types/src/lib.rs:374-379`, e.g. BTC/USD, BTC/USDT) → product (vanilla / perpetual / listed-future-option only, §A.1) → **settlement style** → carry.

**`SettlementStyle`** (`crates/celnet-crypto-vanilla/src/settlement.rs:26-32`, mirrored generically in `celnet-types/src/lib.rs:759-769` where it's marked "meaningful only for a `DigitalAsset` underlying"):
- `Linear` — USD(T)-margined, generalized-BSM (`crates/celnet-crypto-vanilla/src/linear.rs`: `price`/`greeks`/`LinearInputs`).
- `InverseCoin` — coin-margined, `1/S_T` convexity transform, "the classic Deribit inverse contract" (`crates/celnet-crypto-vanilla/src/inverse.rs`: `InverseInputs`, `InverseGreeks`, `cost_of_carry`, `convexity_sandwich_vs_linear_is_signed:523-551`).

**Carry**: `CostOfCarry{r, b}` funded by `funding_carry`/`funding_assembles_net_carry` (`crates/celnet-crypto-vanilla/src/funding.rs:39-65`) — the funding rate (perpetual-swap funding) *is* the carry input, tested against the FX-rate carry identity for consistency (`funding_maps_to_fx_foreign_rate_within_1e12`, `crates/celnet-crypto-vanilla/src/linear.rs:168-192`).

**Greeks**: same 14-Greek strip, but `rhoFor` is relabelled "funding" (GreeksStrip.stories.tsx:76-77, `CRYPTO_GREEKS`) — the UI must swap the rho label per class, not the schema.

**No smile-model selector** for crypto today (single generalized-BSM / inverse-BSM leaf, no SABR/SVI/SSVI/eSSVI calibration wired to crypto surfaces) — the redesign's model picker should simply not render for this class rather than show disabled options.

### A.5 Equity — dividend-yield carry

`Equity(EquityRef){symbol, currency}` → vanilla / perpetual / listed-future-option → `EquityInputs` (`crates/celnet-equity-vanilla/src/lib.rs`): `dividend_paying()` (85-87), generalized-BSM that collapses to standard BSM at zero dividend (`no_dividend_limit_is_standard_bsm:285-302`). Routed server-side via `equity_vanilla_routes_to_equity_leaf` (`crates/celnet-server/src/pricer.rs:3554-3574`). Greeks: `rhoFor` relabelled "dividend yield" (GreeksStrip.stories.tsx:41, `EQUITY_GREEKS`). No smile-model selector (single leaf, no surface calibration).

### A.6 Commodity — futures-style vs equity-style margining

`Commodity(CommodityRef){symbol, currency}` → vanilla / perpetual / listed-future-option → `CommodityInputs` (`crates/celnet-commodity-vanilla/src/lib.rs`, Black-76-style: `price`/`greeks`/`forward`). **Extra selection axis unique to this class + listed-future-option**: `Margining` (`crates/celnet-commodity-vanilla/src/lib.rs`, also `crates/celnet-client/src/vocab.rs`) — `futures_style` (undiscounted, daily-margined; `futures_style_is_undiscounted_and_rate_invariant:847-871`, `listed_future_option_futures_style_has_honest_zero_discount_rho`, `crates/celnet-server/src/pricer.rs:4085-4122`) vs `equity_style` (upfront-premium, discounted; `equity_style_is_discounted_futures_style_bitwise`, `crates/celnet-parity/tests/listed_future.rs:192-222`) — proven bitwise-equal at zero rate (`futures_style_equals_equity_style_at_zero_rate_bitwise:899-931`). Greeks: `rhoFor` relabelled "net carry" (GreeksStrip.stories.tsx:59, `COMMODITY_GREEKS`). No smile-model selector.

### A.7 Where the selection UX must adapt per class — summary table

| Axis | FX | Metal | FI/Rates | Equity | Commodity | Crypto |
|---|---|---|---|---|---|---|
| Underlier identity | `CcyPair` | `MetalPair` | curve/pillar set | `EquityRef{symbol,ccy}` | `CommodityRef{symbol,ccy}` | `CryptoPair{base,quote}` |
| Product menu | 24 (full) | 24 (full) | OIS only (wire); FRA/swap/bond/STIR built in-engine, not wired | 3 (cross-asset) | 3 (cross-asset) | 3 (cross-asset) |
| Smile/model picker | 5 arms (MarketHedge/SABR/SVI/SSVI/eSSVI) | 5 arms (same) | none (curve discounting) | none | none | none |
| Extra axis | delta/premium/cut conventions | same + `PreciousMetal` lease-rate leg | day-count (bond 30/360 vs money-market) | dividend yield | **Margining** (futures- vs equity-style) | **SettlementStyle** (linear vs inverse-coin) |
| Carry model | `FxRates{r_dom,r_for}` | `FxRates` | curve discount factors | `CostOfCarry{r,b}` | `CostOfCarry{r,b}` | `CostOfCarry{r,b}` funded by `funding_carry` |
| Greeks rho labels | dom/for rate | dom/for rate | n/a | rate + dividend | rate + net-carry | rate + funding |

**Redesign implication**: a single generic "asset class → underlier → product → model → conventions → greeks" wizard works for all six, but steps 4 (model) and the "extra axis" row must be *conditionally rendered* per class — not just disabled — since three of six classes have no model-selection step at all, and each of FI/Equity/Commodity/Crypto has a distinct bespoke axis (day-count family / dividend / margining / settlement-style) that doesn't generalize to the others. `capability.ts`'s `PRICEABLE`/`galleryCardStates`/`priceability` functions already encode the product-menu half of this; there is **no equivalent typed matrix yet for the model-picker or the bespoke-axis step** — that would need to be built for the redesign (a natural `crates/celnet-entitlements`-adjacent or `gui/src/products/capability.ts`-adjacent extension).

---

## Part B — Personas → capabilities

*(from a parallel read-only lodestar research pass over `crates/celnet-entitlements`, `celnet-server`, `celnet-rfq`, `celnet-risk-*`, `celnet-limits`, `celnet-journal`/`celnet-replog`, and the GUI)*


### B.0 The capability kernel — what's actually built vs. the aspirational role table

The **Action × AssetClass, scoped by Desk** kernel described in `docs/plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md` §3.1 (status header: *"REQUIREMENT (not yet implemented)"*, authored 2026-06-28) **is now built** in `crates/celnet-entitlements/src/capability.rs`:

- `Action` (47-72 lines, 9 arms): `View | Price | QuoteRespond | RfqRespond | IoiRespond | Stream | Execute | Book | Simulate | Administer`.
- `AssetClass` (119-124): `FxOptions | FixedIncome` (Rates is part of FixedIncome — confirmed §A.3).
- `Capability{action: Action, asset: AssetClass}` (`Capability.new`, `capability.rs`), and `CapabilitySet{grants: BTreeSet<Capability>, denies: BTreeSet<Capability>, grant_all: bool}` with **deny-wins** semantics: `allows()` (`capability.rs:226-231`) returns `false` if `denies.contains(cap)`, else `grant_all || grants.contains(cap)` — exactly the requirement doc's "Deny wins" rule (§3.3), verified by `deny_wins_over_grant`/`deny_wins_over_grant_all` tests.

**But the *role* layer is thinner than the requirement doc's seed table.** `docs/plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md` §3.3 proposed five seeded bundles (`Administrator`, `FX Options Trader`, `FI/Rates Trader`, `Sales`, `Risk/Read-only`). What's actually implemented (`crates/celnet-server/src/config/identity.rs:53-58`) is a **binary** `Role { Admin, Trader }` — matching the requirement doc's own §1 complaint ("Role is binary... today"). On top of it:
- `default_trader_bundle()` (`crates/celnet-server/src/config/identity.rs:95-106`) grants **every** `Action` except `Administer`, across **both** `AssetClass` values, to every `Trader` — i.e. today's baseline Trader is undifferentiated across FX/FI and across Price/Quote/RFQ/IOI/Stream/Execute/Book.
- `Admin` gets `grant_all: true` (immutable — `useRoleCapabilityEditor.ts:94`, "the Admin role cannot be narrowed").
- Per-user **narrowing** exists: `set_role_capabilities` RPC (`crates/celnet-server/src/services/auth.rs:1668-1722`, `set_role_capabilities_narrows_trader_base_persists_and_revokes`) + `per_user_overlay_widens_and_narrows_role_bundle` (`crates/celnet-server/src/services/sessions.rs:451-474`) let an admin grant/deny individual `(Action, AssetClass)` cells per user, edited live through `gui/src/hooks/useRoleCapabilityEditor.ts:82-166` and `gui/src/lib/capabilityMatrix.ts` (`can()`, `capsFor()`, `overlayFromCapabilities()`).
- **Desk scope** is real and separate: `DeskScope{All, Desk(String), Deskless}` (`crates/celnet-server/src/services/access.rs:230-240`), resolved per session (`ResolvedCaller.desk_scope`, `access.rs:318-327`), with `desk_scope_trader_sees_only_their_desk`/`desk_scope_admin_sees_all` tests (`access.rs:837-848`, `815-822`) and desk-filtered fan-out (`publish_filters_by_desk_scope`, `crates/celnet-server/src/services/desk/notify.rs:209-222`).

**Implication for the redesign:** the 5 requested personas are **not** pre-existing named roles anywhere in the system today. They must be realized as **capability-overlay presets** — a curated `(grant, deny)` set of `(Action, AssetClass)` cells layered on the `Trader` base (or `Admin` for firm-wide roles) via the existing `set_role_capabilities`/per-user-overlay mechanism — not as new server-side role enum variants (guardrail #9: no parallel role system; `celnet-entitlements` is already the one contract). The GUI's `Administration` workspace (`useRoleCapabilityEditor`, `capabilityMatrix.ts`) is the natural place to ship these as named **presets** over the same grid, and the redesign's persona pickers below assume exactly that realization.

### B.1 The five personas

#### QUANT
**Primary jobs**: build/calibrate/validate pricing and calibration models; extend the analytics surface via the SDK; is the "who defines what's selectable in §A" role. Not primarily a live-market seat.
**Grounding**: `docs/ARCHITECTURE.md:152` names `celnet-cli` explicitly as the **"operator/quant CLI: price, surface, exotic, convention, risk, stream"** — the Quant's primary tool is the CLI + `celnet-golden` (frozen QuantLib 1.42.1 reference tables, the independent oracle every numerical change is validated against per guardrail #5) rather than the live trading GUI. Model authorship rides `celnet-plugin-api`/`celnet-plugin-host`: `ModelRegistry.register_native`/`register_wasm` (`crates/celnet-plugin-host/src/registry.rs:56-66`) register Tier-0 native and Tier-2 wasmi-sandboxed models under one registry (`native_and_wasm_twins_agree_through_one_registry`, `crates/celnet-plugin-host/tests/sandbox.rs:643-689`); `docs/EXPERIENCE-ARCHITECTURE.md` §5 confirms *"the model list is not hard-coded UI: it enumerates the registered pricing/calibration models behind `celnet-plugin-api`... A user-extensible analytic added via the SDK shows up in the same selector with the same provenance line."*
**Capability shape**: `View + Price` on both `FxOptions` and `FixedIncome` (needs to price/inspect everything to validate), **no** `Execute`/`Book`/`QuoteRespond`/`RfqRespond`/`IoiRespond` (never deals), `DeskScope::All` (cross-desk model consistency, not desk-owned risk) — i.e. `price_without_execute` is literally a named test fixture in `capability.rs` (`price_without_execute`), the exact shape a Quant grant needs.
**Default perspective**: not one of the 5 shared-loop workspaces at all in the trading sense — a **model/surface inspector** built from the *Surface* workspace's inspector strip (model selector VV/SABR/SVI/SSVI/eSSVI + per-mark provenance, `docs/EXPERIENCE-ARCHITECTURE.md` §5/§6 Surface) in **read/calibrate-only** mode, plus a golden/oracle diff panel (celnet-golden comparison) and the plugin/model-registry admin view. No blotter, no Stream tiles, no Book P&L.

#### TRADER / market-maker
**Primary jobs**: stream two-way prices, manage desk risk, hit/lift on the RFS blotter, mark the surface day-to-day (`docs/SURFACE-WORKFLOW.md` §3 frames the "optimal marking workflow" as a **desk/book trader** activity, not a Quant one — Publish is a trader action that deposits an immutable `MarkedSurface` under a fresh `surface_version`, `docs/EXPERIENCE-ARCHITECTURE.md` §6 Surface).
**Grounding**: `StreamService.StreamSession` subscribe/execute (`Capability(Stream,·)`/`Capability(Execute,·)` per the requirement doc §4), `QuoteService.AcceptQuote` (`Capability(Execute, FxOptions)`), `RiskService.BookRatesPosition` (`Capability(Book, FixedIncome)`). Today's `default_trader_bundle()` already grants a Trader `View,Price,QuoteRespond,RfqRespond,IoiRespond,Stream,Execute,Book` on **both** asset classes — an FX-only or FI-only trader preset (the requirement doc's `FX Options Trader`/`FI Rates Trader` rows) is a **narrowing overlay** (deny the other `AssetClass`'s action set), not new server logic.
**Capability shape**: full action set (`View,Price,QuoteRespond,Stream,Execute,Book`, optionally `RfqRespond/IoiRespond` if the desk also takes RFQ flow) on **one** `AssetClass` (`FxOptions` *or* `FixedIncome`, per the requirement doc's split), `DeskScope::Desk(self)`.
**Default perspective**: the full 5-workspace shared loop, trading-configured — **Stream** as the resting state (multiplexed RFS blotter, flash-on-change, click-a-side — `docs/EXPERIENCE-ARCHITECTURE.md` §6 Stream) with **Ticket** (pricer, full 14-Greek strip, last-look ring) and **Risk** (drilled to the trader's own book: bucketed vega tenor×delta, cross-gamma, theta-roll, limits overlay) one click away via the shared-selection drill (§4 Book↔Risk linkage), **Surface** for marking, **Book** scoped to `DeskScope::Desk(self)`.

#### SALES-TRADER
**Primary jobs**: client-facing relay — receive/respond to counterparty **RFQ**s and **IOI**s, show prices, never book the desk's own directional risk. The requirement doc's `Sales` seeded role is the closest named analogue: `View,RfqRespond,IoiRespond` on **both** asset classes, `DeskScope::Desk(self)` (§3.3) — notably **no** `Execute`/`Stream`/`QuoteRespond` in that seed, i.e. sales relays but the desk trader deals.
**Grounding**: the RFQ/IOI relay path is `RfqDeskService` — `RfqDeskEdge.submit_desk_request` (`crates/celnet-server/src/services/desk/mod.rs:306-392`) and `.respond_desk_request` (394-472), gated per the requirement doc §4 as `Capability(RfqRespond|IoiRespond, class)` keyed by request `kind`. The counterparty side of this rides FIX: `conformance_ioi_maps_to_desk_ioi` (`crates/celnet-fix/tests/desk_gateway.rs:251-286`) confirms IOI is a first-class desk-gateway concept distinct from a tradeable quote. `AttributionRecord` (referenced in `docs/EXPERIENCE-ARCHITECTURE.md` §6 Stream, "book / owner (human seat OR auto-pricer)") is how a sales-relayed line is distinguished from an auto-priced one on the shared blotter.
**Capability shape**: `View,RfqRespond,IoiRespond` on both `FxOptions`+`FixedIncome`, `DeskScope::Desk(self)`; **no** `Execute`/`Book` — sales never commits the desk's balance sheet directly, consistent with the requirement doc's separation-of-duties example (§3.1: *"pricing and dealing are distinct capabilities"*).
**Default perspective**: an **RFQ/IOI inbox-first** perspective — Stream workspace filtered/grouped to `RfqRespond`/`IoiRespond` rows only (not the full streaming blotter a market-maker watches), with the price-show/last-look Ticket surface for constructing a response, and no Risk/Book workspace by default (no `View` gap since `View` is granted, but nothing to book) — the redesign should default sales-trader to the narrowest of the five perspectives.

#### RISK MANAGER
**Primary jobs**: cross-desk risk oversight — aggregated Greek roll-ups, limit-utilization monitoring, breach escalation, information-barrier-respecting drill-down. Explicitly **read-only on actions**: the requirement doc's `Risk / Read-only` seed is `View` on both asset classes, `DeskScope::All`, **no action capabilities** — i.e. the *only* persona of the five entitled to see across every desk but forbidden from acting on any of it (pure separation of duties, `docs/RISK-HIERARCHY.md` §4 "Separation of duties — risk/product-control principals have read scopes the front office does not").
**Grounding**: the risk surface is the **cube**, not a single tree (`docs/RISK-HIERARCHY.md` §2.1: `Trader → Book → Desk → CcyPair → Location → Entity → Firm`, independent dimensions, an OLAP-style group-by/reduce over an immutable `RiskFact` table). It's built and served end-to-end (`docs/EXPERIENCE-ARCHITECTURE.md` §3: `celnet-risk-cube`, `celnet-risk-normalize`, `celnet-limits`, `celnet-entitlements` gated green, served as `RiskService.{ListPositions,AggregateRisk,DrillRisk,LimitStatus}`). Entitlement pruning happens **server-side, pre-reduction** (RISK-HIERARCHY §2.6/§4, EXPERIENCE-ARCHITECTURE §3: *"a parent total can never leak the magnitude of an invisible subtree"*) — the architectural reason a Risk Manager's `DeskScope::All` `View`-only grant is safe to hand out broadly. `docs/RISK-HIERARCHY.md` §5 gives the limits taxonomy (Greek, bucketed-vega, concentration, VaR/ES, scenario/stress, stop-loss) cascading board→entity→desk→book→trader, soft (early-warning) vs hard (blocking), pre-trade **and** post-trade with escalation — this is the Risk Manager's core workflow, not a trader's.
**Capability shape**: `View` only, both asset classes, `DeskScope::All`.
**Default perspective**: **Risk** and **Book** workspaces as primary, configured firm-wide (`ScopeContext.path` defaulting to the "Firm · all desks · all books" root chip, `docs/EXPERIENCE-ARCHITECTURE.md` §3), with the toolbar's breadcrumb-driven group-by picker as the main navigation (drill firm→entity→desk→trader→book→pair per RISK-HIERARCHY §2.1/§6) rather than a fixed pair/desk pin. Panel content per RISK-HIERARCHY §6: a **hierarchical risk explorer** (virtualized tree grid, server-side row model, O(log n) streaming-sort), **heatmaps** (vega-by-tenor×delta, signed cross-pair correlation, perceptually-uniform diverging colormap), the **limit dashboard** (utilization/RAG/soft-breach/escalation), **scenario/stress** (spot×vol grids, tornado charts, live what-if), and a **P&L-explain waterfall**. No Ticket/Stream (no pricing or dealing action to take), no Surface (marking is a trader/quant act, not a risk act — though Risk Manager can *view* the currently-official surface via provenance, not edit it).

#### DESK HEAD
**Primary jobs**: the FRTB-regulatory head-trader concept — owns the desk's aggregate risk/limits/P&L and the trading-account-to-desk assignment. `docs/RISK-HIERARCHY.md` §2.1 point 3 cites the regulatory definition directly: *"an unambiguously defined group of traders or trading accounts,"* with **one (max two) head trader**, each account assigned to a single desk (BCBS MAR12/FRTB). `docs/TRADING-UNIVERSE-SCALE.md:198` gives a concrete real-world analogue ("FX Options G10 Correlation Trading Desk Head").
**Not a seeded role in the requirement doc** — this is the report's own inference: Desk Head sits **between** the plain `Trader` bundle and firm-wide `Administrator`/`Risk-Read-only`. It needs the full trader action set (so the head can still deal/quote personally) **plus** the desk-scoped analogue of the Risk Manager's aggregate view (limit *setting*, not just monitoring — RISK-HIERARCHY §5.2 "limits are set at any node," desk being one), which the plain Trader bundle does not carry today (`default_trader_bundle()` has no `Administer`, and desk limit-setting isn't in the 9-action `Action` enum as a separate verb — it would ride `Administer` scoped to the desk, or needs a dedicated `Action::SetLimit`-class addition; **this is a real gap** the redesign/entitlements team should flag, not paper over).
**Capability shape (proposed overlay)**: full `default_trader_bundle()` action set on the desk's `AssetClass`, `DeskScope::Desk(self)`, **plus** `View` at `DeskScope::All` is *not* appropriate (that's Risk Manager's firm-wide read) — instead the Desk Head's `View`/`Book` reach should extend to every book **under their own desk** (already the natural meaning of `DeskScope::Desk(id)`, since `desk_filter_intersects_scope`, `crates/celnet-server/src/services/desk/mod.rs:1002-1017`, and `list_positions_shows_a_desk_trader_only_its_desk`, `crates/celnet-server/tests/desk_session_boundary.rs:403-427`, already aggregate at the desk, not the individual trader, level).
**Default perspective**: **Book** workspace as the home view, scoped to `DeskScope::Desk(self)` (desk-wide aggregate: per-trader/per-book/per-pair breakdown, aggregate vega ladder, drill to Risk for any slice — `docs/EXPERIENCE-ARCHITECTURE.md` §4/§6 Book), with the **Risk** workspace's limit dashboard *scoped to the desk* (not firm-wide) so the Desk Head sees their own desk's soft/hard breaches and can action escalations (RISK-HIERARCHY §5.3), plus a lighter Stream/Ticket presence than a pure Trader (still capable of dealing, but the default landing view is the desk roll-up, not a single-pair blotter).

### B.2 Cross-persona summary table

| Persona | Actions (from `Action`, capability.rs:47-72) | AssetClass | DeskScope | Home workspace(s) |
|---|---|---|---|---|
| Quant | View, Price | FxOptions + FixedIncome | All | Surface (model/calibrate, read/validate-only) + golden-oracle diff + plugin/model-registry admin |
| Trader / MM | View, Price, QuoteRespond, Stream, Execute, Book (+RfqRespond/IoiRespond if desk takes RFQ) | one of FxOptions / FixedIncome | Desk(self) | Stream (resting) ⇄ Ticket ⇄ Risk ⇄ Surface (marking) ⇄ Book, all desk-scoped |
| Sales-Trader | View, RfqRespond, IoiRespond | FxOptions + FixedIncome | Desk(self) | RFQ/IOI inbox (filtered Stream) + Ticket (price-show) |
| Risk Manager | View only | FxOptions + FixedIncome | All | Risk + Book, firm-wide breadcrumb/group-by, no Ticket/Stream/Surface |
| Desk Head | full trader bundle + desk-level limit administration (gap — see B.1) | desk's AssetClass | Desk(self), reaching every book under the desk | Book (desk aggregate, home) + Risk (desk-scoped limit dashboard) + Stream/Ticket (lighter) |

---

## Appendix — open gaps surfaced by this pass (for the redesign team, not fixed here)

1. **No model-picker / bespoke-axis capability matrix** (§A.7) — `gui/src/products/capability.ts` only encodes the product-menu axis; the model-selector and per-class bespoke-axis (day-count family / dividend / margining / settlement-style) steps have no equivalent typed matrix yet.
2. **FI/Rates wire surface is OIS-only** (§A.3) — `celnet-rates`'s FRA/swap/bond/STIR-strip math is built and tested but not reachable from the `RatesInstrument` wire oneof (`rates_instrument_from_json`, `ws/codec.rs:1275-1285`, explicitly "P0 contract").
3. **`celnet-entitlements::AssetClass` has no Equity/Commodity/Crypto arm** (§A.0/§B.0) — only `FxOptions`/`FixedIncome` exist; the cross-asset leaves (equity/commodity/crypto-vanilla) currently ride the GUI-only `capability.ts` gate, not a server-enforced entitlement, for asset-class-level authorization.
4. **No named persona role bundles exist server-side** (§B.0) — only binary `Role{Admin,Trader}` + per-user capability overlay. The 5 personas must be shipped as curated overlay *presets* in the redesign's Administration UI, not as new server role variants.
5. **Desk Head has no dedicated `Action` arm** for desk-level limit administration (§B.1) — the 9-action `Action` enum (View/Price/QuoteRespond/RfqRespond/IoiRespond/Stream/Execute/Book/Simulate/Administer) has no "set limit at my desk" verb short of full `Administer`; needs a scoped decision before the Desk Head perspective can be entitlement-enforced (today it can only be UI-conventioned, not server-gated).
