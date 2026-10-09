# Celnet Fixed-Income / Rates — Full Capability Review (lodestar-cited)

> **NOTE (2026-07-02) — the GUI redesign this doc fed has since LANDED (`fe-fi-migration`,
> `33d9a0a` / `fd18594`).** GUI workspace names below are point-in-time and now **superseded**: the
> FX-vs-FI domain-tab split was collapsed into **one class-parametric rail**, the standalone
> `RatesWorkspace` was **deleted** (rates OIS now prices through the shared `TicketWorkspace` via
> `gui/src/products/ois.tsx`), and `CurveWorkspace` / `RatesRiskWorkspace` / `RatesBookWorkspace`
> became **lenses** of `MarketDataWorkspace` / `RiskWorkspace` / `BookWorkspace`. The engine/wire
> facts remain accurate; for the shipped IA see [`../fixed-income/FI-STATUS.md`](../fixed-income/FI-STATUS.md) slice F.

Project: `github.com-soarsa-celnet` · index healthy (24,602 nodes / 75,644 edges, `status:ready`).
Read-only review. All paths absolute under `/Users/adrian/code/celnet/`.

---

## 0. Bottom line

`celnet-rates` is a **functionally rich, engine-complete** linear-rates crate (curves, FRA, IRS,
STIR/bond futures, cash bonds, full PV01/DV01/key-rate risk) with **four live RatesService RPCs**
end-to-end across **all five clients** (gRPC, WS/GUI, Excel, Rust SDK, FIX). But the **wire contract
exposes exactly one instrument arm — `OisInstrument`** (`RatesInstrument.oneof.ois = 1`,
`crates/celnet-proto/proto/celnet.proto:3961-3967`). FRA/IRS/STIR-future/bond-future/cash-bond are
real, tested engines in `celnet-rates` with **no proto arm, no RatesInstrument variant, no client
reachability** — confirmed by reading the oneof directly (only one field). The prior capture that
"only OIS is exposed on the proto" is **correct today**, not stale.

Separately, the **rates position store (`RatesPositionStore`) and the GUI Book workspace are now
LIVE** — `docs/fixed-income/FI-STATUS.md` (dated 2026-06-27) says these are still outstanding; the
code has moved past that doc (see §5, doc-drift).

Architecturally, rates risk and rates booking are a **complete parallel stack**, not a projection of
the options risk cube: `celnet-risk-cube::DimensionId` (crates/celnet-risk-cube/src/dimension.rs:32-50)
has no `Rates` variant — its `Underlying` axis is FX/metals/equity/commodity/crypto only. Rates uses
its own `celnet-risk-fleet::rates` reducer and its own `RatesPositionStore`, wired through a second,
disjoint `RiskService` RPC pair (`AggregateRatesRisk`/`BookRatesPosition`) rather than through
`RiskService.AggregateRisk`/`PositionStore`. `celnet-server::pricer::price_instrument`
(crates/celnet-server/src/pricer.rs:495-595) — the FX/cross-asset dispatch entry point — contains
**zero references to `RatesInstrument`**; rates prices through an entirely separate function,
`rates_pricing::price_rates` (crates/celnet-server/src/rates_pricing.rs:212-234). This is exactly the
gap ADR-0010 exists to close (§6).

---

## 1. Curve construction — `bootstrap.rs`, `curve.rs`

**Status: LIVE (engine) + LIVE (wire, single-curve only)**

- `Curve` (`crates/celnet-rates/src/curve.rs:86-94`) — immutable, `Arc`-backed discount/forward
  snapshot. Constructors: `from_zero_rates` (147), `from_log_linear_dfs` (107),
  `from_monotone_convex_dfs`/`_zero_rates` (129, 161), `from_nodes`/`parse_df_pillars`/
  `zero_rate_pillars` (166-220).
- Two interpolation schemes selectable at construction (`Interpolation` enum,
  `curve.rs:69-74`): **log-linear-on-log-DF** (shipping default, piecewise-flat instantaneous
  forward — `ln_df_log_linear` 252-258) and **monotone-convex-on-forwards**
  (`ln_df_monotone_convex` 267-283, `monotone_convex_forward` 296-314; Hagan-West-style
  piecewise-quadratic region construction, provenance in prose only per GUIDE.md §8).
  Accessors: `discount_factor` (318), `zero_rate` (330), `instantaneous_forward` (343),
  `forward_rate_continuous`/`forward_rate_simple` (356, 367).
- `turns.rs::with_turns`/`turn_discount_factor` — turn-of-year / meeting-date forward-jump overlay,
  construction-only (hot query path untouched); `no_turns_reproduces_the_base_curve` (161-171)
  verifies degeneracy.
- Bootstrap: `bootstrap_ois` (`bootstrap.rs:75-111`) — **sequential, acyclic** bootstrap of a
  self-discounting USD-SOFR curve from dated `OisQuote` par rates (16-21); `BootstrapError` (25-36).
  A second strip bootstrapper exists for the futures leg: `bootstrap_futures_strip`
  (`futures_strip.rs:115-158`) with `implied_forward_rate`, feeding `StirFuturesQuote`.
- **What feeds the bootstrap today:** only OIS par-rate pillars
  (`OisQuote{schedule, par_rate}`). FRA/futures/swap quotes are **not yet wired into a joint
  multi-instrument bootstrap** — `celnet-rates::bootstrap` takes only `&[OisQuote]`; FRA/futures
  strip bootstrapping exists as a **separate** function (`bootstrap_futures_strip`), not merged
  into one curve build. This matches FI-STATUS.md's note that "wiring [STIR convexity] into the
  short-end bootstrap build is the remaining integration step."
- **Wire (`CurveSet`, `celnet.proto:3927-3937`):** `{currency, reference_date, repeated OisPillar}`
  — **exactly one self-discounting OIS curve**, no separate discount-vs-projection curve container
  (despite `FI-ARCHITECTURE.md` §1 describing a `CurveSet` as "discount + per-tenor projection
  curves"). `CurveSet.currency` is carried but **non-USD is rejected** until the multi-currency arm
  ships (proto comment, line 3928-3930) — confirmed single-currency USD-SOFR P0.

## 2. Instruments — `fra.rs`, `vanilla_swap.rs`, `futures.rs`/`futures_strip.rs`, `bond.rs`, `ois.rs`

All **LIVE in the engine** (`celnet-rates`), tested against structural identities and QuantLib-pinned
day-count fixtures; **only `ois.rs`'s `OisInstrument` reaches the wire.**

| Instrument | Symbol(s) | Inputs | Conventions | Wire status |
|---|---|---|---|---|
| **OIS** | `ois.rs::{OisSchedule, FixedPeriod, ois_pv, ois_par_rate, ois_annuity}` (21-148) | spot-starting schedule, fixed rate, notional | modified-following US calendar, ACT/360 accrual, ACT/365F discount time (`schedule.rs::usd_sofr_ois_schedule`, 45) | **LIVE** — `RatesInstrument.ois` (proto 3965), server `rates_pricing::price_rates` (212-234), FIX `dialect_rates` (OIS-only — `SecurityType=OIS`, `decode_rates_rfq` 222-260), WS mirror, Rust SDK, GUI `RatesWorkspace`, Excel `=CELNET.RATES(...)`. |
| **FRA** | `fra.rs::{Fra, fra_pv, fra_par_rate, fra_pv01, fra_risk}` (46-408) | `[fixing,maturity]`, contractual accrual `tau`, fixed rate, notional | curve coords ACT/365F from ref date; accrual on caller-supplied `AccrualBasis` (ACT/360 or 30/360 Bond Basis) | **NO-WIRE** — no `fra` field in `RatesInstrument` oneof. Identity-verified vs OIS single-period swaplet (`fra_pv_equals_single_period_swaplet`). |
| **Vanilla IRS** | `vanilla_swap.rs::{VanillaSwap, SwapLeg, swap_pv, swap_par_rate, swap_risk}` (159-608) | independent fixed/float legs, `PaymentFrequency` (Annual/Semi/Quarterly) per leg, explicit per-period float projection | leg accrual on caller `DayCount` (ACT/365F, ACT/360 only — **30/360 fixed-leg basis deferred**, needs a coordinated `celnet_types::DayCount` addition per the doc-comment at `vanilla_swap.rs:33-38`) | **NO-WIRE** — no `vanilla_swap` field in oneof despite FI-ARCHITECTURE.md §2 table listing it as a planned additive arm. |
| **STIR future** | `futures.rs::{StirFuture, stir_futures_rate, stir_futures_price, convexity_adjustment}` (47-158) | curve forward + one-factor Gaussian convexity (`½σ²T₁T₂`, σ caller-supplied), `100·(1−rate)` price | — | **NO-WIRE**. |
| **Bond future** | `futures.rs::{Deliverable, conversion_factor, gross_basis, implied_repo_rate, cheapest_to_deliver}` (131-191) | notional-yield conversion factor, invoice price, CTD by max implied repo | delivery-window accrued + stochastic convexity **deferred** | **NO-WIRE**. |
| **Cash bond** | `bond.rs::{CashBond, bond_pv, yield_to_maturity, z_spread, g_spread, asset_swap_spread}` (52-275) | fixed-coupon bond reusing the swap coupon schedule; yield/Z-spread via the shared Brent solver | mid-period accrued (clean vs dirty) **deferred**; callable OAS **deferred** (option-free ⇒ OAS≡Z) | **NO-WIRE**. |

Day-count layer: `daycount.rs::AccrualBasis` (25-32) has **3 variants** (`Act360`, `Act365Fixed`,
`Thirty360BondBasis`) — a superset of the shared `celnet_types::DayCount`
(`crates/celnet-types/src/lib.rs:732-737`), which itself has **only 2 variants**
(`Act365Fixed`, `Act360` — no 30/360, no ACT/ACT-ISDA/ICMA). This means the full ISDA day-count set
described in `docs/fixed-income/FI-CONVENTIONS.md` §1 (`Thirty360`, `ActActIsda`, `ActActIcma` for
govt bonds) is a **spec, not yet implemented** on the shared seam. Similarly, `FI-CONVENTIONS.md` §3
specifies a full `RfrObservation` axis (`Lookback`/`ObservationShift`/`Lockout`/`PaymentDelay`,
geometric daily-compounding) and a `RatesConvention` per-`(currency,index,tenor)` schema — **neither
exists in code**: `celnet-rates/src/lib.rs` (1-66) declares no `rfr` or `conventions` module (only
`bond, bootstrap, curve, daycount, fra, futures, futures_strip, ois, risk, schedule, solver, turns,
vanilla_swap` — flatter than the nested `curve/build/conventions/product/rfr/pricing/risk` layout
`FI-ARCHITECTURE.md` §1 originally sketched). The OIS engine treats the floating leg as a par/annuity
identity, not an explicit daily-RFR-compounding accrual. **Gap for the report requester: the
convention-schema breadth described in the FI docs is materially ahead of what `celnet-rates` ships.**

## 3. Pricing/risk — `risk.rs`, `RatesService` RPCs, `RatesPositionStore`

**Status: LIVE**, but risk is a **silo**, not a cube projection.

- `risk.rs::{pv01, OisRisk, ois_risk}` (26-123) — analytic PV01 (fixed-leg annuity), DV01 (central
  1bp parallel quote bump + re-bootstrap), key-rate ladder (per-pillar central bump). Each product
  module repeats the same pattern locally: `fra_risk` (fra.rs 199-233), `swap_risk`
  (vanilla_swap.rs 293-330) — no shared risk-cube bump-and-revalue engine is reused inside
  `celnet-rates` itself (each does its own re-bootstrap loop).
- **`RatesService` (all 4 RPCs LIVE on `celnet.proto:3035-3064`):**
  - `PriceRates` (3000) → `rates_pricing::price_rates` (`crates/celnet-server/src/rates_pricing.rs:212-234`) — thin `PricingService` handler mapping wire `CurveSet`/`OisInstrument` → `bootstrap_ois` → `ois_risk`, applies client `side` sign.
  - `AggregateRatesRisk` (3056) → `crates/celnet-server/src/services/rates_risk/aggregate.rs` (`curve` 172-199, `position` 201-222) → `celnet-risk-fleet::rates::{firm_aggregate_rates, RatesFirmRollup, firm_book}` (`crates/celnet-risk-fleet/src/rates.rs:283-608`) — **additive-only** fan-in (PV/PV01/DV01/key-rate ladder sum over a netting set; par rate is curve-derived, never aggregated), bit-for-bit vs single-node per FI-STATUS.md.
  - `BookRatesPosition` / `ListRatesPositions` (3061, 3064) → `RatesPositionStore`
    (`crates/celnet-server/src/services/rates_book.rs:33-120`) — `book` (64-93, id=0 ⇒ fresh, else
    upsert), `snapshot`/`len`/`is_empty`, `entitlement_prunes_by_book_cell` (233-261). Wired into
    `RiskEdge` via `rates_store`/`with_rates_store`/`book_rates_position_impl`
    (`crates/celnet-server/src/services/risk/mod.rs:237-681`). **This directly supersedes
    `FI-STATUS.md`'s "no firm rates position store yet" note** — the proto's own comment block at
    `celnet.proto:4122-4130` says the store "now exists server-side," while the older comment 12
    lines above it (`4064-4066`, on `AggregateRatesRiskRequest`) still says "there is no firm rates
    position store yet" — **a stale in-proto comment**, worth a doc fix.
- **Risk cube isolation:** `celnet-risk-cube::DimensionId` (`crates/celnet-risk-cube/src/dimension.rs:32-50`)
  — `Trader | Book | Desk | Underlying | Location | Entity` — the `Underlying` axis's doc-comment
  explicitly enumerates "FX pairs, metals, equities, commodities and digital assets" with **no
  rates/curve axis**. `pricer::price_instrument` (pricer.rs:495-595, 62 callers) never mentions
  `RatesInstrument`. Rates risk therefore rolls up through an **entirely separate** additive-only
  federation path (`celnet-risk-fleet::rates`), not the general `celnet-risk-cube` bump/non-additive
  machinery FX/equity/commodity/crypto share.

## 4. Five-client + GUI workspace status (as of this review, HEAD)

| Client | Rates surface | Status |
|---|---|---|
| gRPC `PricingService`/`RiskService` | 4 RPCs (§3) | LIVE, OIS-only |
| WS/GUI transport | `wsTransport.priceRates/aggregateRatesRisk/bookRatesPosition/listRatesPositions` (`gui/src/data/wsTransport.ts:921-1216`) + codec (`gui/src/data/wsCodec.ts:837-2149`) | LIVE |
| Rust SDK | `celnet-client::rates` (`crates/celnet-client/src/rates.rs`, `UsdSofrCurve`/`Ois` builders, `curve_builds_wire_curve_set` 197-205) | LIVE |
| Excel | `=CELNET.RATES(...)` (`excel/src/functions/shaping.ts::shapeRatesCurve` 3917-3923, `excel/src/contract/wsCodec.ts::ratesCurveSetToWire` 1182-1195) | LIVE |
| FIX | `celnet-fix::dialect_rates` (`crates/celnet-fix/src/dialect_rates.rs`, `RatesSide`/`decode_rates_rfq`/`two_way_rates`, 54-438) | LIVE, OIS-only (`SecurityType=OIS`) |
| federation | `celnet-risk-fleet::rates::RatesFleetReducer` | LIVE, additive fan-in |
| GUI **Curve** workspace | `gui/src/workspaces/CurveWorkspace.tsx` (108-343) — DF/zero/forward inspection | LIVE |
| GUI **Ticket** (Rates pricing) | `gui/src/workspaces/RatesWorkspace.tsx` (71-304) | LIVE |
| GUI **Risk** | `gui/src/workspaces/RatesRiskWorkspace.tsx` (205-483) — editable OIS book → `AggregateRatesRisk` → per-ccy PV/PV01/DV01/key-rate ladder | LIVE |
| GUI **Book** | `gui/src/workspaces/RatesBookWorkspace.tsx` (65-391) — `BookTicket`, `defaultTicket`, `directionLabel` | **LIVE** — contradicts `FI-STATUS.md`'s "Book blotter awaits persisted rates position store" (§5). |

All 4 of the `FI-ARCHITECTURE.md` §4.2 FI workspaces are now built (Curve/Ticket/Risk/Book), all
riding the top-level `AssetDomain` tab layer (Options | Fixed Income) over the existing data-driven
`RAIL` registry per that doc's design.

## 5. Doc-drift found during this review

`docs/fixed-income/FI-STATUS.md` (dated 2026-06-27) is **stale relative to HEAD** in two places:
1. It lists the rates position store and the GUI **Book** workspace as outstanding/remaining; both
   are live in code (`RatesPositionStore`, `RatesBookWorkspace.tsx`).
2. `crates/celnet-proto/proto/celnet.proto` itself carries a contradiction: the comment on
   `AggregateRatesRiskRequest` (line ~4066) still says "there is no firm rates position store yet,"
   while the section header 56 lines below it (line ~4126) says the store "now exists server-side."
   Both cannot be current — recommend cleaning the stale line the next time the proto file is touched.

`FI-ARCHITECTURE.md`'s planned nested module layout (`curve/`, `build/`, `conventions/`, `product/`,
`rfr/`, `pricing/`, `risk/`) was **not** what shipped — `celnet-rates` is 13 flat top-level modules
(§2). Not a defect, but the architecture doc's module tree should not be read as current structure.

---

## 6. (a) FI curve-first quant selection tree vs FX surface-first tree

**FX (surface-first):** pair → `SurfaceWorkspace` reads/marks the calibrated **smile** (`GetSmile`/
`MarkSurface`, `SurfaceService`, `celnet.proto:3069-3076`) on the **delta axis** → a ticket
(`TicketWorkspace`) resolves strike/vol off that one live smile → `price_instrument` picks a
`ProductEngine` by product family (vanilla/exotic/strategy) → the 13/14-Greek set. The **one shared
market object is the vol surface**; every FX product for a pair reads the *same* smile. Convention
resolution (day-count/vol-time/spot-lag) is a **flat, single-point** lookup per `(pair, tenor)`
(`docs/CONVENTIONS.md`).

**FI (curve-first):** currency/index → `CurveWorkspace` builds/inspects the **term structure**
(bootstrap from dated par-instrument pillars, choose interpolation scheme, apply turn/meeting jumps)
→ a rates ticket (`RatesWorkspace`) selects an **instrument family** (OIS today; FRA/IRS/futures/bond
once wired) whose **schedule** (not a strike/delta) is the object being priced → conventions are
resolved **per leg, per instrument, per accrual period** — day-count, business-day roll, RFR
observation method, spot lag (`FI-CONVENTIONS.md` §5's `RatesConvention` schema, keyed on
`(currency, index, tenor)`, is deliberately per-leg, never a flat pair-level default) → risk is not
one Greek vector but a **key-rate (Jacobian) ladder** against every calibrating pillar
(`risk.rs::ois_risk`, `fra_risk`, `swap_risk`).

**Why FI needs its own Curve workbench, not the FX Vol-Surface:**
1. **The market object is different in kind.** A vol surface is a 2D (tenor×delta) smile calibrated
   once per pair; a `Curve` (`curve.rs:86-94`) is a bootstrapped discount/forward *function of time*,
   sequentially built from *par instruments*, not marked from broker vol quotes. Curve construction
   choices (interpolation scheme, turn overlays) have no FX analogue and need their own controls
   (`CurveWorkspace.tsx::EditablePillar`, `HorizonMetric`).
2. **The instrument selection axis is schedule/leg-shape, not strike/delta.** An FX ticket picks a
   strike or delta; a rates ticket picks a *schedule* — tenor, payment frequency per leg, accrual
   basis per leg (`vanilla_swap.rs::PaymentFrequency`/`SwapLeg`) — an entirely different input
   grammar the FX `TicketWorkspace`'s strike/delta/vol fields cannot express.
3. **Risk is a ladder against the calibrating instruments, not a Greek vector against spot/vol.**
   `key_rate_ladder` buckets by *calibrating pillar tenor* (`KeyRateDv01`, `celnet.proto:4089-4094`)
   — a fundamentally curve-relative sensitivity with no FX-Greeks equivalent; the FX risk cube's
   `Underlying` dimension (§3) has no notion of "pillar."
4. ADR-0010 makes this explicit and rejects folding them together: "Keep separate — the valuation
   paradigm... Linear rates (cashflow PV over the curve — deterministic, no vol) vs rates options
   (swaptions/caps — forward *produced by* the curve, priced with vol under the annuity measure).
   They share the curve substrate and risk cube, **not the payoff engine**."

## 7. (b) Integrating FI into the single front-end's shared surfaces

Per ADR-0010's three-layer convergence plan (§8 below) and the live code state (§3-4), the correct
integration shape for a unified front-end is:

- **pricing/model → price rates instruments.** Long-term (ADR-0010 item 3): dispatch
  `RatesInstrument` through the *same* `ProductEngine` registry `price_instrument` already uses
  (`engines::dispatch_live`, `pricer.rs:585-595`), so a rates ticket is just another product family
  in the one pricing surface instead of the disjoint `rates_pricing::price_rates` function. Today
  (interim/correct-now state) it is a **parallel** RPC (`PriceRates`) on the *same* `PricingService`
  — already unified at the **service** level, not yet at the **dispatch-function** level.
- **contribute → stream rates.** `StreamService.StreamSession` is the one multiplexed subscribe
  channel FX already uses; rates pricing/risk currently has no streaming RFQ/RFS surface
  (`FI-STATUS.md` §F: "the §C/mockup RFQ · IOI · RFS surfaces await their own server contracts" —
  confirmed **DEFERRED**, no live symbols found for a rates RFS stream beyond the FIX
  `SubscriptionRequest` dialect scaffold in `dialect_rates.rs:91`).
- **risk → unify rates risk into the cross-asset cube.** This is the single biggest structural gap
  (§3): add a `Rates`/`Curve` variant to `celnet-risk-cube::DimensionId`
  (`dimension.rs:32-50`) and make `RateSensitivities` (ADR-0010's proposed type) a curve-bucketed
  member of the *same* additive fact the FX/equity/commodity/crypto legs populate, so
  `AggregateRatesRisk` becomes a **typed projection** of `RiskService.AggregateRisk`'s rollup
  instead of a second `AggregateRatesRisk` RPC over a second `celnet-risk-fleet::rates` reducer.
- **books → unify `RatesPositionStore` into the one blotter.** `RatesPositionStore`
  (`rates_book.rs:33-120`) and the options `PositionStore` (`services/risk/store.rs`) are two
  in-memory stores today, each with its own booking RPC pair (`BookRatesPosition`/
  `BookPosition`) and its own GUI workspace (`RatesBookWorkspace.tsx` vs `BookWorkspace`). A single
  blotter needs one position type with an asset-class tag, one store, one `ListPositions`/`Book`
  RPC pair — the rates store's entitlement-pruning shape (`entitlement_prunes_by_book_cell`,
  `rates_book.rs:233-261`) is already structurally close to the options store's
  (`book_from_attribution`, `services/risk/store.rs:450-484`), which makes this a tractable merge,
  not a rewrite.
- **FI-specific new surfaces that stay FI-specific (correctly so):**
  - **Curve construction/bootstrap workbench** (`CurveWorkspace.tsx`, live) — has no FX analogue;
    keep as the FI-domain-tab's dedicated workspace, per §6.
  - **Rates-instrument ticket** (`RatesWorkspace.tsx`, live) — keep as a distinct ticket *shape*
    (schedule/leg-based, not strike/delta-based) even after pricing dispatch unifies, exactly as
    ADR-0010 keeps the *payoff engine* separate while unifying the *substrate*.

## 8. (c) ADR-0010 — "Carry = degenerate curve" (from `manage_adr`/direct read)

`manage_adr(mode=get)` returns no standalone ADR node (the write surface requires a `deliverable`
slug and none has been attached yet); the durable record is the committed doc,
`docs/adr/ADR-0010-converge-fi-rates-onto-carry-seam.md`. **Status: Proposed / Accepted as a design
direction (2026-06-28), NOT yet implemented** — the flat-carry path remains authoritative until the
curve trait lands. It extends ADR-0008 (`carry-seam` deliverable) and honours ADR-0007 (one
unversioned contract).

**Core insight:** `celnet-types::Carry` (`FxRates{r_dom,r_for}` | `CostOfCarry{r,b}`, consumed via
`celnet-core::carry::CarryInputs::{discount_df,forward}`) is **flat** — single-rate, single-expiry.
`celnet-rates::Curve` is a full bootstrapped term structure. ADR-0010's thesis: **`Carry` is a
degenerate flat term-structure; `Curve` is the general case** — `Carry::FxRates{r_dom,r_for}` is
already two flat discount curves producing a forward, i.e. FX carry is a **2-curve special case of
multi-curve**. This is why convergence is principled rather than forced.

**Three-layer decision:**
1. **Term-structure substrate (unify).** Hoist a `DiscountCurve`/`TermStructure` trait into
   `celnet-core` implemented by *both* `Carry` (flat) and `celnet-rates::Curve` (general); generalize
   `CarryInputs` from a flat rate to a curve-backed context; `CurveSet` becomes the multi-curve
   container with FX two-rate carry as its 2-curve special case.
2. **Risk (unify into ONE cube).** `RateSensitivities` carries a curve-bucketed key-rate-DV01 ladder
   and rates get a real dimension in the firm-wide risk cube; `AggregateRatesRisk` becomes a typed
   projection, not a silo (directly what §3/§7 above found missing today).
3. **Contract/clients (one Instrument oneof).** Dispatch rates via `price_instrument`'s
   `ProductEngine` registry so all five clients consume rates through the one contract; the dedicated
   rates GUI workspaces become views over the one surface, not a parallel app.

**Deliberately kept separate:** the *payoff engine*. Linear rates (deterministic cashflow PV) vs
rates options (swaptions/caps — forward produced by the curve, priced with normal/Bachelier + SABR
vol under the annuity measure) share the curve substrate and risk cube but not the pricer — a
distinct `celnet-rates` crate remains correct (multi-curve bootstrap, schedules/day-count/calendars,
curve risk have no FX-option analogue).

**Invariant (no-regression gate):** FX byte-identity — `Carry::FxRates → CarryInputs::discount_df/
forward → VanillaInputs` must stay bit-identical (`to_bits` equality) through the curve-trait
generalization; the flat curve is the single-point degenerate of a term structure and must not move
the FX two-rate discount/forward by one ULP.

**Verified-knowledge claims recorded against the ADR** (lifecycle **draft** — promotion to `active`
awaits cross-family Stage-2 review *and* implementation; `knowledge_get` on `Curve`/`price_instrument`
returned 0 active claims at time of review, consistent with draft status):
- `cl_f412933641ecc72d` (decision) — Carry degenerate/Curve general; converge onto one
  `DiscountCurve` trait. Anchors: `Carry`, `Curve`, `Carry::discount_df`.
- `cl_a2a865886c8bfe90` (decision) — rates risk unifies into the one cube via curve-bucketed
  `RateSensitivities`; `AggregateRatesRisk` becomes a projection. Anchors: `RateSensitivities`,
  `AggregateRatesRisk`, `celnet-rates::risk::ladder`, `CurveSet`, `RatesInstrument`.
- `cl_c701d7d260f82523` (invariant) — FX byte-identity must survive the generalization. Anchors:
  `CarryInputs::discount_df`, `CarryInputs::forward`, `Carry`.

**What "term-structure-unified core" means for a single coherent FX+FI UX:** today the GUI has *two*
market-object abstractions a trader must context-switch between — a vol Surface (FX) and a Curve
(FI) — feeding *two* ticket shapes, *two* risk RPCs, *two* books. Once the `DiscountCurve` trait
lands, the **market-state layer** becomes one abstraction (`Carry` is literally a `Curve` with one
node) even though the **product/payoff layer** and the **UX ticket shapes** correctly stay distinct
per §6/§7 — the convergence is at the substrate and the risk cube, never at the trader-facing
ticket or the vol-vs-schedule input grammar.

---

## Appendix — key symbols cited (qualified_name → file:line)

- `github.com-soarsa-celnet.crates.celnet-rates.src.curve.Curve` — `crates/celnet-rates/src/curve.rs:86`
- `github.com-soarsa-celnet.crates.celnet-rates.src.bootstrap.bootstrap_ois` — `crates/celnet-rates/src/bootstrap.rs:75`
- `github.com-soarsa-celnet.crates.celnet-rates.src.fra.Fra` / `fra_risk` — `crates/celnet-rates/src/fra.rs:46,199`
- `github.com-soarsa-celnet.crates.celnet-rates.src.vanilla_swap.VanillaSwap` / `swap_risk` — `crates/celnet-rates/src/vanilla_swap.rs:159,293`
- `github.com-soarsa-celnet.crates.celnet-rates.src.futures.StirFuture` / `cheapest_to_deliver` — `crates/celnet-rates/src/futures.rs:67,171`
- `github.com-soarsa-celnet.crates.celnet-rates.src.bond.CashBond` — `crates/celnet-rates/src/bond.rs:52`
- `github.com-soarsa-celnet.crates.celnet-server.src.rates_pricing.price_rates` — `crates/celnet-server/src/rates_pricing.rs:212`
- `github.com-soarsa-celnet.crates.celnet-server.src.services.rates_book.RatesPositionStore` — `crates/celnet-server/src/services/rates_book.rs:33`
- `github.com-soarsa-celnet.crates.celnet-server.src.services.rates_risk.aggregate.curve` — `crates/celnet-server/src/services/rates_risk/aggregate.rs:172`
- `github.com-soarsa-celnet.crates.celnet-risk-fleet.src.rates.firm_aggregate_rates` — `crates/celnet-risk-fleet/src/rates.rs:341`
- `github.com-soarsa-celnet.crates.celnet-risk-cube.src.dimension.DimensionId` — `crates/celnet-risk-cube/src/dimension.rs:32`
- `github.com-soarsa-celnet.crates.celnet-server.src.pricer.price_instrument` — `crates/celnet-server/src/pricer.rs:495`
- `github.com-soarsa-celnet.crates.celnet-fix.src.dialect_rates.decode_rates_rfq` — `crates/celnet-fix/src/dialect_rates.rs:222`
- `github.com-soarsa-celnet.gui.src.workspaces.CurveWorkspace.CurveWorkspace` — `gui/src/workspaces/CurveWorkspace.tsx:108`
- `github.com-soarsa-celnet.gui.src.workspaces.RatesWorkspace.RatesWorkspace` — `gui/src/workspaces/RatesWorkspace.tsx:71`
- `github.com-soarsa-celnet.gui.src.workspaces.RatesRiskWorkspace.RatesRiskWorkspace` — `gui/src/workspaces/RatesRiskWorkspace.tsx:205`
- `github.com-soarsa-celnet.gui.src.workspaces.RatesBookWorkspace.RatesBookWorkspace` — `gui/src/workspaces/RatesBookWorkspace.tsx:65`
- Proto: `RatesInstrument` `crates/celnet-proto/proto/celnet.proto:3961-3967`; `CurveSet:3927-3937`;
  `RatesPosition:4034-4046`; `RiskService`:3035-3064; `AggregateRatesRiskRequest`:4067-4084 (stale
  comment 4064-4066 vs the corrected block at 4122-4130).
- ADR: `docs/adr/ADR-0010-converge-fi-rates-onto-carry-seam.md`; related `docs/adr/ADR-0008-multi-asset-carry-architecture.md`.
- Docs: `docs/fixed-income/FI-ARCHITECTURE.md`, `FI-STATUS.md` (stale, §5), `FI-CONVENTIONS.md`
  (spec ahead of code, §2), `FI-CURVES-SPEC.md`.
