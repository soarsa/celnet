# Fixed Income — implementation status & outstanding features

**Status:** LANDED on `main` · **Updated:** 2026-07-02
**Scope tracked:** the locked P0 (USD-only, linear rates + cash, no vol/credit — see
[`OPEN-QUESTIONS.md`](./OPEN-QUESTIONS.md) D3–D12) plus the cross-asset/UI items.

This is the single source of truth for *what is built vs outstanding*. Each outstanding item is a
**gated slice** (compiles + `clippy -D warnings` + tests + rustfmt; numeric items validated against
QuantLib or a closed-form/structural identity per [`FI-VERIFICATION-CONTRACT.md`](./FI-VERIFICATION-CONTRACT.md)).

---

## ✅ Delivered (gated + pushed to `origin`)

The `celnet-rates` crate — the disjoint-leaf numeric core (depends only on the shared seams):

| Slice | Commit | What | Tests |
|---|---|---|---|
| 1 | `089e63d` | `Curve` — immutable Arc-backed discount/forward snapshot; **log-linear-on-log-DF** interpolation (Q10 default); DF / zero / instantaneous-forward / forward-rate accessors. | 12 |
| 2 | `8190eaf` | `solver::brent_root` — derivative-free bracketing root-finder for calibration. | +6 |
| 3 | `2f12098` | OIS pricing identities (self-discounting par/annuity/PV) + **sequential SOFR bootstrap**. | +9 |
| 4 | `00e4668` | **USD-SOFR date/schedule layer** — `usd_sofr_ois_schedule` (modified-following US calendar, ACT/360 accrual, ACT/365F discount-time); reuses `celnet-calendar`. | +6 |
| 5 | `93a9fd3` | OIS **risk** — `pv01` (analytic annuity), `ois_risk` → DV01 (parallel quote bump) + **key-rate ladder** (per-instrument Jacobian). | +4 |

**Net: a working USD-SOFR rates engine** — bootstrap a curve from dated OIS quotes → price any OIS →
PV / PV01 / DV01 / key-rate ladder. `celnet-rates`: **37/37 tests**, clippy `-D` clean.

Integration phase — the engine wired through the wire contract, server, and FIX edge:

| Slice | Commit | What | Tests |
|---|---|---|---|
| C | `f4b82d7` | **Proto rates arms** — additive `CurveSet` / `OisPillar` / `OisInstrument` / `RatesInstrument` oneof / `RatesPricingResult` / `RatesPrice{Request,Response}` on the single unversioned `celnet.proto` (reuses `Side`+`BrokenDate`, no renumber). | 62 |
| D | `5757a16` | **Server `PriceRates` rpc** — `rates_pricing` maps the wire `CurveSet`/`OisInstrument` → engine, prices via `ois_risk`, applies the client `side` sign; thin `PricingService` handler. Server path byte-identical to a direct engine call; par-swap PV≈0; payer == −receiver. | +12 |
| E1 | `ea9e29d` | **FIX FI dialect** (`celnet-fix/dialect_rates`) — OIS `QuoteRequest` encode/decode, `RatesSide` (pay/receive/two-way), `SubscriptionRequest` (RFQ / RFS subscribe / unsubscribe), `TAG_TENOR_YEARS`. | +9 |
| E2 | `2c49ab5` | **Live FIX routing** — `price_request` branches on `SecurityType=OIS` to a par-rate two-way line reusing the SAME keyed-MAC token / Quote / last-look / fill path. Real loopback FIX 4.4: OIS RFQ → Quote @ engine par+spread (1e-12) → pay-fixed lift → fill. | +1 e2e |

**Net: "the FIX API supports fixed income" is true end-to-end** — an external FIX counterparty RFQs an
OIS, gets a two-way rate market, and lifts to a fill, all on the live acceptor. gRPC `PriceRates`
prices the same arm. Single unversioned contract; no placeholder arms (only OIS ships, additive).

> **P0 market note:** the FIX edge prices against a documented **static USD-SOFR par-OIS ladder**
> (`rates_pricing::default_usd_sofr_curve_set`) — a *real* calibrating market, not a stub — pending a
> live SOFR feed (Q6, test-environment data-provider access, deferred). When the feed lands it
> replaces the table; nothing else changes.

Slice F — five-client parity (in progress):

| Sub-slice | Commit | What | Tests |
|---|---|---|---|
| F1 — WS mirror | `61a23b6` | `price_rates` → `rates_price_response` on the WebSocket edge (codec + dispatch) so browser/GUI clients reach the rates path; gRPC and WS share one impl. | +1 codec |
| F2 — Rust SDK | `32d5504` | `celnet-client::rates` — `UsdSofrCurve` / `Ois` fluent builders + `Client::price_rates` returning side-signed `RatesPriced`. | +4 |

**Five-client status:** all delivered — FIX ✅ (E2), WS edge ✅ (F1), Rust SDK ✅ (F2), GUI ✅
(the class-parametric front-end, `fe-fi-migration` — FI as lenses of the shared workspaces, **not**
an asset-tab peer), Excel ✅ (`=CELNET.RATES` + `=CELNET.CURVE`, `fe-unified-book`), federation ✅
(F5 rates fan-out). Deeper FI depth on each surface is TARGET, gated on the backend wire-lanes (see
*Remaining* below).

GUI (existing):

| Commit | What |
|---|---|
| `6c978cf` | **Administration rail section** — Connections + Admin grouped into an admin-gated Administration group, out of the main trading rail (closes the prior admin-visibility gap). 721/721 GUI tests. |

---

## ⏳ Outstanding (the integration phase — each a gated slice)

### A. More rates products (`celnet-rates`)
- **FRA** ✅ (`89b0588`) — single forward-fixing off the curve; PV ≡ one-period OIS swaplet, par
  zeroes PV, analytic PV01, central-difference DV01 + key-rate ladder.
- **Vanilla IRS conventions** ✅ (`0d9f55b`) — two-leg fixed-vs-float swap with per-leg payment
  frequency (annual/semi/quarterly), independent schedules, explicit per-period float projection
  (projection-curve seam), PV/par/PV01/DV01/key-rate. Verified by structural identities (par PV=0,
  leg decomposition, exact PV01 finite-diff, ladder sums to DV01, explicit float == telescoping
  DF(0)−DF(T), annual case == `ois_par_rate`). **30/360 fixed-leg basis deferred** — needs a
  coordinated `celnet_types::DayCount` addition (proto / engine handoff / GUI+Excel enum mirrors);
  the builder takes a supported `DayCount` (ACT/365F, ACT/360) until then.
- **STIR & bond futures** ✅ (`36b853d`) — STIR: curve forward + deterministic one-factor Gaussian
  convexity (`½σ²T₁T₂`, σ a caller input per Q11), `100·(1−rate)` price. Bond: conversion factor
  (price at notional yield), gross basis, implied repo, CTD by max implied repo. Verified by
  identities (zero-vol == forward, convexity ↑ in σ, CF == 1 on idealised par schedule, gross basis
  vanishes at converted price, CTD selects max implied repo). Delivery-window accrued + stochastic
  convexity deferred.
- **Cash-bond analytics** ✅ (`eb24f0b`) — fixed-coupon bond reusing the swap coupon schedule:
  PV on curve, periodic **yield-to-maturity**, **Z-spread** (cc spread over curve zeros),
  **G-spread** (yield over same-cashflow curve yield), par-par **ASW**; yield/Z via the shared
  Brent solver. Verified by identities (yield recovers its pricing rate, price↔yield round-trip,
  monotone price/yield, Z/G/ASW vanish at curve-fair price with correct cheap/rich signs).
  **Mid-period accrued (clean vs dirty)** deferred to the settlement-date layer; **callable OAS**
  deferred (slice is option-free, where OAS ≡ Z).

### B. Curve completeness (`celnet-rates`)
- **Monotone-convex-on-forwards** interpolation (the smooth-view scheme) ✅ (`aaa9bb6`) — a second
  scheme on `Curve` selected at construction (`Interpolation` enum + `from_monotone_convex_dfs`/
  `_zero_rates`), alongside the shipped log-linear default. A piecewise-quadratic instantaneous
  forward (Hagan-West region construction; provenance in prose only per §8) that reproduces every
  pillar DF exactly, is continuous across pillars, and is monotonicity/convexity-preserving (no
  overshoot on monotone discrete forwards). Knot forwards precomputed once at build; the query path
  stays allocation-free (one quadratic on the bracketing segment via a scheme dispatch). Verified by
  identities only (no external oracle): exact pillar reproduction, forward continuity at pillars,
  `forward == −d lnDF/dt` by central difference (the stored integral is the exact antiderivative of
  the forward), monotone-forward preservation, single-segment coincidence with log-linear, negative
  rates. **Lane B core complete.**
- **Turn-of-year / central-bank-meeting** forward jumps ✅ (`ae79a5c`) — `turns::with_turns`
  overlays localized forward spikes by re-sampling the base curve at pillars + jump boundaries and
  applying `exp(−size·overlap)`, then rebuilding a `Curve`. **Construction-only — hot query path
  untouched.** Verified by identities (no-turns == base, DFs before unchanged, DFs after scaled by
  `exp(−size·width)`, in-window forward raised by exactly `size`, inverted turn lowers it, multiple
  turns compose, degenerate/out-of-range reject).
- **Deterministic STIR convexity** ✅ delivered in lane-A futures (`36b853d`, `convexity_adjustment`);
  wiring it into the short-end bootstrap build is the remaining integration step.

### C. Wire contract (`celnet-proto`, additive — single contract, guardrail #9)
- New arms on the one `celnet.proto`: **`CurveSet`**, **`RatesInstrument`** oneof
  (`fra` · `ois` · `vanilla_swap` · `tenor_basis_swap` · `cross_ccy_basis_swap` · `stir_future` ·
  `bond_future`), **`PricingResult { pv, par_rate, pv01, dv01, key_rate_ladder }`** — next free
  field numbers, no renumber.
- **Golden vectors + parity rows** per arm (`tools/check-verification-coverage.mjs` parses the oneof).

### D. Server (`celnet-server`, `celnet-risk-cube`)
- Pricing service consumes `celnet-rates` for the rates arms.
- **Rates risk folds into the existing server-owned `RiskService`** rollup — clients never loop-sum
  (FI-ARCHITECTURE §3). **FRTB GIRR** (delta SBM, `celnet-risk-cube::fi::girr_delta_sbm`) is
  implemented + tested but is **not yet server-wired to any client** (tracked: `be-rates-wire-arms`,
  `be-girr-vega`/`be-girr-curvature`) and is validated by **SBM structural identities, NOT an ORE
  cross-check** — no ORE integration exists in the codebase (FI numerics use structural/analytic
  identities per `FI-VERIFICATION-CONTRACT.md`).

### E. FIX fixed-income dialect (`celnet-fix`) — **"FIX API supports FI"**
- NEW `crates/celnet-fix/src/dialect_rates.rs` mirroring `dialect_fx.rs`: decode inbound FI
  **RFQ**, **RFS-stream subscribe**, and **order** messages → route to the rates engine → quote / fill.
- Gated by a loopback FIX initiator (mirror `tests/fix_acceptor.rs`).

### F. Five-client parity (slice 9)
- **GUI** — ✅ FI is now integrated as **one class-parametric front-end**, not a peer domain
  (`fe-fi-migration`, merge `33d9a0a`; capstone `fd18594`). The FX-vs-FI **domain-tab split is
  retired**: `gui/src/app/Shell.tsx` renders a single class-parametric rail whose asset class is
  chosen by **scope + license** (the `RAIL` + `railState`/`configuredLicense` predicate — verified
  via lodestar). Each shared workspace opens at its default lens and the trader picks the asset
  class *inside* via the lens bar + active scope:
  - **Pricing/Ticket** — `TicketWorkspace.tsx`; a rates OIS is a fixed-income product family
    (`gui/src/products/ois.tsx`) priced over `price_rates`. The standalone `RatesWorkspace.tsx` is
    **deleted**.
  - **Market Data** — `MarketDataWorkspace.tsx` with the FX vol **surface** (`SurfaceWorkspace`) and
    the FI rates **curve** (`CurveWorkspace`) as two lenses of one workspace.
  - **Risk** — `RiskWorkspace.tsx` with FX scenario + FI rate risk (`RatesRiskWorkspace` → the
    federated `AggregateRatesRisk` WS-mirror `bad2625`) as two lenses.
  - **Book** — `BookWorkspace.tsx` with Positions & Booking / Aggregate-Risk / Deals
    (`RatesBookWorkspace` + `DealsBlotterWorkspace`) as three view lenses.
  The §C/mockup RFQ · IOI · RFS FI surfaces await their own server contracts (see *Remaining* below).
- **Excel** add-in — ✅ FI parity is live and **registered** (`fe-unified-book`, merge `86ecb46` /
  `bb36ad8`): `=CELNET.RATES(...)` prices an OIS over the live `price_rates` RPC (PV / par / PV01 /
  DV01 + key-rate DV01 ladder) and `=CELNET.CURVE(...)` bootstraps a discount curve over `build_curve`
  (per-pillar time / DF / cc-zero). Both are custom-function entry points in
  `excel/src/functions/functions.ts` on the same WS codec (`excel/src/contract/wsCodec.ts`) — verified
  via lodestar. **Now LIVE + registered (2026-07-03): `=CELNET.RATESRISK` (AggregateRatesRisk),
  `=CELNET.RATESBOOK` (ListRatesPositions), `=CELNET.INSTRUMENTS` (reference-data), `=CELNET.BOND`,
  `=CELNET.IRS`, `=CELNET.FRA` (all via `price_rates`), and `=CELNET.XVA` (CVA/DVA/FVA via `price_xva`)
  — 9 FI/XVA functions total.** Remaining `=CELNET.*` FI fns (FI stream/RFQ/mark-bond) are TARGET,
  gated on the backend wire-lanes below.
- **Rust SDK** (`celnet-client`) — ✅ rates instrument vocab + `Client::price_rates`/`price_irs`/
  `price_fra`/`price_bond` (`sdk-fi-price-parity`, landed).
- **FIX** — the dialect in (E).
- **Federation** — ✅ cross-shard rates risk fan-out + bit-exact rollup in `celnet-risk-fleet`
  (`af8ad25`: `RatesFleetReducer::fan_in_additive` == single-node, proptest-pinned bit-for-bit;
  additive PV/PV01/DV01 + key-rate ladder by tenor; `PartitionKey::currency` for `(entity, ccy)` HRW
  sharding) **and** the server-owned RPC that drives it: `RiskService.AggregateRatesRisk`
  (`5bb8099`) — positions → `price_rates` → `partition_rates_facts` → `fan_in_additive` → per-ccy
  rollup, deny-by-default entitlement, endpoint == `firm_aggregate_rates` bit-for-bit (additive proto
  change). Now reachable from the GUI over a **WS-mirror** (`bad2625`: codec + dispatch arm + GUI
  transport/in-app rollup) and surfaced in the **Rates Risk** workspace (`034065a`). Positions inline;
  a persisted execution-fed rates position store is the remaining follow-up (it also unblocks the GUI
  **Book** workspace).

---

## 🚫 Deferred (locked, **not** half-built — see OPEN-QUESTIONS Q3/Q7)
- **Rates vol** (`celnet-rates-vol`: swaptions/caps, normal/shifted-SABR cube, 1F Gaussian) — Q7.
- **Credit** (`celnet-credit`: single-name + index CDS) — Q3 (low appetite).
- Inflation (deflation-floor ILB), full multi-CSA / CTD-collateral, callable/putable OAS.
- Multi-currency generalisation + cross-currency basis (post-USD, hard currencies first — Q1).

---

## UI changes — explicit status
- **Administration:** ✅ done (`6c978cf`) — an admin-gated rail group.
- **Class-parametric single rail (FX + FI on ONE rail):** ✅ **LANDED** (`fe-fi-migration`,
  `33d9a0a` / capstone `fd18594`). The earlier top-level FX-vs-FI domain-tab split is **retired**;
  asset class is chosen by scope + license, and FI capability is reached as **lenses** of the shared
  workspaces (see slice F above). "FI integrated, not a peer."
- **Excel FI parity:** ✅ **LIVE — 9 functions** — `=CELNET.RATES`/`CURVE`/`RATESRISK`/`RATESBOOK`/
  `INSTRUMENTS`/`BOND`/`IRS`/`FRA` (via `price_rates`/`build_curve`/`aggregate_rates_risk`/
  `list_rates_positions`) + `=CELNET.XVA` (CVA/DVA/FVA via `price_xva`), all registered
  (`fe-unified-book` + follow-ons through 2026-07-03; XVA closes GUI↔Excel client parity).

---

## Remaining — HONEST gap register (LIVE vs TARGET)

The client FI surfaces above are LIVE against **today's** wire contract (`PriceRates` /
`AggregateRatesRisk` / `BuildCurve`). Deepening FI to *first-class on every surface* is gated on
**backend wire-lanes that are backend-owned and currently unclaimed** on the multi-asset board
([`../plan/MULTI-ASSET-CORE-INTEGRATION.md`](../plan/MULTI-ASSET-CORE-INTEGRATION.md) §2–§4):

- **✅ LANDED since this register was written (no longer TARGET):** `fi-wire-instruments`
  (bond/IRS/FRA `price_rates` arms — **now fully wired + client-reachable**, `5b5d375`),
  `fix-bond-dialect` (bonds on FIX/RFQ), `sdk-fi-price-parity` (SDK `price_irs`/`fra`/`bond`),
  `be-combined-tail-risk-rpc` (`RiskService.CombinedTailRisk`), and the client surfaces
  `fi-bond-ticket-gui`, `fi-scenario-gui`, `excel-bond-fn`, `=CELNET.IRS/FRA/XVA`.
- **TARGET (backend, still open):** `rates-stream-ws` (FI on WS StreamService), `rates-rfq-ws` +
  `multi-dealer-rates-rfq`, `curve-surface-query` (`GetCurve`/`MarkCurve`), `be-xva-rates-exposure`,
  `unified-price-rpc` (collapse `Price`/`PriceRates`/`PriceXva` → one `Price(oneof Instrument)`), and
  **`be-rates-wire-arms` — wire the built-but-orphaned FI engines** (FRTB GIRR delta SBM, bond-future
  relative-value CTD/basis/implied-repo, smooth-curve monotone-convex interpolation + turns) which are
  implemented + tested in `celnet-rates`/`celnet-risk-cube` but reach no client today.
- **TARGET (client, waits on the above):** `fi-stream-gui`/`excel-fi-stream`, `fi-rfq-gui`/
  `excel-fi-rfq`, `fi-xva-gui`, `excel-fi-mark-bond`.
- **Contract nuance:** the "one Priceable/MarketResolver/RiskMeasure contract" is honored by **OIS +
  Bond**; **IRS/FRA dispatch to correct engines but route *around* the Priceable seam** (functional
  parity, not abstraction parity) — closing this is `uniform-asset-class-architecture` (ADR-0021).

---

## Build order for the outstanding phase
**C (proto arms) ✅ → D (server consumes `celnet-rates`) ✅ → E (FIX dialect) ✅ → F (WS mirror ✅ ·
Rust SDK ✅ · GUI class-parametric front-end ✅ · Excel `=RATES`+`=CURVE` ✅ · federation rollup ✅)**,
with A/B product breadth (FRA / IRS / futures / cash-bond RV) landed into `celnet-rates` (disjoint
leaf). C/D/E/F are committed, gated, and landed on `main`; the federated
`RiskService.AggregateRatesRisk` (`5bb8099`) + WS-mirror (`bad2625`) drive the Risk lens; the GUI
class-parametric rail (`fe-fi-migration`) and Excel `=CELNET.RATES`/`=CELNET.CURVE` (`fe-unified-book`)
are live. Shipped to UAT via `deploy/celnet-deploy.sh` option 2 (binary release). **Remaining is the
backend-owned multi-asset wire-lanes + their downstream client FI surfaces — see the gap register
above.**
