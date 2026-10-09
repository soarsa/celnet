# Curves, Instrument-Driven Bootstrap & Reference Data — Review + Gap Analysis

**Status:** analysis only (no code change). 2026-06-29.
**Sibling of:** [`docs/FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md`](FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md).

**Scope.** Four things the user asked for:

- **A.** The Curves page — can a trader define custom tenors and *broken dates*
  (explicit `YYYY-MM-DD` pillar dates), not just standard tenor tokens?
- **B.** Build curves *by instruments* + an instrument registry — define a set of
  calibrating instruments and bootstrap a curve from them.
- **C.** A static / reference-data repository — enumerate the instrument-definition
  fields a reference-data store needs per product family (the core research ask).
- **D.** Section-by-section comparison to the provided cash-bond *"SYSTEM DESIGN
  DOCUMENT: FIXED INCOME PRICING & HEDGING TOOL v1.0"* (a distributed low-latency
  multi-dealer D2C platform).
- **E.** Recommendations & sequencing.

> **Naming guardrail (CLAUDE.md §8).** Proposed identifiers are purpose-named and
> vendor-neutral. Third-party platforms/venues (MarketAxess, CME, TRACE, CDX,
> Kafka, …) appear only as integration-target references in prose, never in
> proposed API names.

---

## 0. Executive summary — the five biggest gaps

1. **The curve input layer is tenor-restricted to *whole years*, even though the
   engine curve is fully date/fraction-general.** The wire `OisPillar.tenor_years`
   is a `uint32` (whole years), and the GUI `CurveWorkspace` lets a trader edit
   *par rates* of a fixed pillar ladder but **cannot add/remove pillars, set custom
   tenors, or enter broken dates.** The engine `Curve` (`crates/celnet-rates/src/curve.rs`)
   already keys on continuous year-fraction `Time` and accepts *any* strictly
   increasing pillar times — so broken/odd dates are an **input/wire/GUI plumbing
   gap, not a numerical-core gap.**
2. **Curve building is instrument-driven but single-family (OIS only).** The
   bootstrap (`bootstrap_ois`) builds the curve one calibrating OIS quote per
   pillar, and separate builders exist for a STIR-futures strip and turns — but
   there is **no unified multi-instrument bootstrap** that mixes deposits + FRAs +
   futures + swaps + OIS, **no deposit instrument at all**, and the wire `CurveSet`
   carries only `ois_pillars`.
3. **There is no instrument-definition / static-reference-data registry.**
   Instruments are specified **inline at pricing time** (`OisInstrument { tenor_years,
   fixed_rate, notional, side }` reconstructs its own schedule server-side). There
   is **no security-id → definition mapping** for any family. (Confirmed for bonds —
   matches the bond gap-analysis.)
4. **No credit-spread cash-bond *pricing* workflow.** A cash-bond *analytics* engine
   exists (`crates/celnet-rates/src/bond.rs`: YTM, Z-/G-spread, asset-swap spread
   off a curve + an *observed* price), but there is **no credit-spread matrix, no
   TRACE/CDX-style feed, no benchmark-yield + credit-spread price derivation, no
   accrued/settlement (spot-starting only), and the bond arm is not on the wire.**
   This is the central capability the provided design doc is built around.
5. **No automated hedging loop and no internal crossing engine.** celnet computes
   DV01 / key-rate ladders (FRA / OIS risk) and has STIR-futures convexity math,
   but there is **no DV01→futures hedge-count automation with order routing** and
   **no A2A mid-market internal cross.**

### Top three recommendations

1. **Generalise the curve *pillar* to a `{ date | tenor }` discriminated input**
   end-to-end (engine already supports it; change `OisPillar`/`CurveSet`, the
   GUI builder, and the offline pricer mirror). Unlocks both custom tenors and
   broken dates with no new numerics.
2. **Introduce a vendor-neutral reference-data repository** (a registry keyed by an
   internal instrument id with external-id cross-refs, persisted + admin-managed,
   modelled on the existing identity/Entity/Book store) — it underpins *both*
   instrument-driven curve building (B) *and* cash-bond pricing/capture (the bond
   gap-analysis).
3. **Add a multi-instrument curve-build request** (`value date` + a list of
   `(instrument-definition-ref, market quote)`) that the bootstrap calibrates
   against, building on the existing per-family builders.

---

## A. Curves page — tenors & broken dates

### A.1 Current state (GUI)

`gui/src/workspaces/CurveWorkspace.tsx`:

- The builder state is `EditablePillar { tenorYears: number; parRatePct: number }`,
  seeded from `DEFAULT_USD_SOFR_CURVE.pillars` (`gui/src/data/ratesPricing.ts`,
  pillars at 1/2/3/5/7/10/15/20/30y).
- The trader can **edit the par rate** of each pillar (a `<input type="number">`)
  and pick an **inspect horizon**, but the pillar **set is fixed**: there is no
  add-pillar / remove-pillar control, no tenor entry, and **no date field**. The
  reference date is fixed to `DEFAULT_USD_SOFR_CURVE.referenceDate`
  (`{2026,6,25}`) and displayed read-only.
- The curve is assembled as a `RatesCurveSet { currency, referenceDate, pillars:
  [{ tenorYears, parRate }] }` and bootstrapped in-browser by
  `bootstrapCurveFromSet` → `bootstrapOis` (the offline mirror of the server).

So today a trader can **perturb** the standard ladder but **cannot define custom
tenors and cannot inject broken/explicit dates.** Tenors are even more restricted
than standard tokens (1M/3M/5Y): they are **whole-year integers**.

### A.2 Current state (wire + engine)

- **Wire** (`crates/celnet-proto/proto/celnet.proto`): `OisPillar { uint32
  tenor_years; double par_rate }` and `CurveSet { currency; BrokenDate
  reference_date; repeated OisPillar ois_pillars }`. The pillar is a **whole-year
  tenor**; there is no broken-date pillar variant. (Note: a `BrokenDate` message
  and a date-valued FX-options `Tenor` already exist in the proto — proto:585 — so
  the date primitive is in the vocabulary, just not on the curve pillar.)
- **Schedule layer** (`crates/celnet-rates/src/schedule.rs`): `usd_sofr_ois_schedule`
  / `usd_ois_schedule_with_basis` already map real calendar `Date`s →
  year-fraction `Time` via `year_fraction(Act365Fixed, reference, date)` with
  modified-following US-calendar rolls. **An explicit pillar date would map to a
  year fraction with the existing primitives.**
- **Engine curve** (`crates/celnet-rates/src/curve.rs`): `Curve` stores pillars as
  `(Time, ln DF)` keyed on **continuous year-fraction time**, requires only that
  times be **strictly increasing** (`parse_df_pillars`), and supports log-linear
  and monotone-convex interpolation. It is **completely agnostic to whether a
  pillar came from a round tenor or a broken date.** The doc comment is explicit:
  *"the date→time mapping (day-count + calendar) is supplied by a later slice and
  is intentionally not part of this numeric core."*

**Conclusion:** the numerical core *already* supports arbitrary pillar dates. The
restriction is entirely in the **input contract (whole-year `tenor_years`)** and
the **GUI (fixed ladder, no date field).**

### A.3 Gap & what each layer needs

Support a pillar that is **either** a standard tenor **or** an explicit civil date:

- **Wire (`OisPillar`):** replace the bare `tenor_years` with a discriminated
  pillar key — a `oneof { uint32 tenor_years; BrokenDate pillar_date; Tenor tenor }`
  (reusing the existing `BrokenDate`/`Tenor` messages) carrying the same `par_rate`.
  One clean contract (rule 9), additive field numbers.
- **Server resolve:** when a `pillar_date` is supplied, the schedule builder uses
  that date as the period-end directly (roll-adjusted) instead of `add_months(start,
  12·i)`; the curve-time coordinate is the existing `year_fraction(Act365Fixed,
  reference, date)`. Strictly-increasing-date validation replaces the
  strictly-increasing-tenor check.
- **GUI (`CurveWorkspace`):** make the pillar list editable (add/remove), and each
  pillar a `{ kind: "tenor" | "date"; tenorYears? ; date? }` with date parsing +
  validation (valid civil date, strictly after the reference date, strictly
  increasing). The offline mirror (`ratesPricing.ts`) already has full civil-date
  arithmetic (`dayNumber`/`addMonths`/roll rules), so the date→year-fraction map is
  a thin addition, keeping offline ≡ live byte-identity.
- **Day count:** value-date → explicit-date year fraction must use the same basis
  as the schedule (ACT/365F curve axis; ACT/360 accrual) so a broken-date pillar
  reprices consistently.

This is a contract + GUI change with **zero new numerical machinery** in the curve.

---

## B. Build curves by instruments + an instrument registry

### B.1 Current state — the bootstrap *is* instrument-driven (single family)

- `crates/celnet-rates/src/bootstrap.rs` — `bootstrap_ois(quotes: &[OisQuote])`:
  each pillar **is** a calibrating instrument. `OisQuote { schedule: OisSchedule,
  par_rate: Rate }` is solved (1-D Brent root-find in the pillar zero rate) so the
  OIS reprices to its quoted par rate. This is a genuine *build-by-instrument*
  bootstrap (short→long, acyclic single-curve).
- Other per-family builders/pricers already exist in `celnet-rates`:
  - `futures_strip.rs` — `bootstrap` a short-end curve from a contiguous strip of
    convexity-adjusted **STIR futures** (`StirFuturesQuote`).
  - `fra.rs` — **FRA** pricing + curve risk (one-period swaplet).
  - `vanilla_swap.rs` — **vanilla fixed-vs-float IRS** with sub-annual leg
    frequencies and per-leg day counts.
  - `turns.rs` — year-end/turn jumps overlaid on a curve.
  - `ois.rs` — OIS schedule + pricing.

### B.2 Gaps

1. **No unified multi-instrument bootstrap.** Each family builds (or prices off) a
   curve independently. There is no single calibration that ingests a *mixed* set —
   deposits (short end) + futures/FRAs (middle) + swaps/OIS (long end) — and solves
   one consistent curve. The standard market build (cash deposits → futures/FRA
   strip → par swaps) is not assembled.
2. **No deposit / cash instrument** at all (the conventional front-pillar).
3. **No instrument-set curve-build request on the wire.** `CurveSet` carries only
   `ois_pillars` (whole-year OIS par rates). There is no way to say "build the curve
   from *these* instruments with *these* quotes."
4. **No instrument *definition* indirection** — every calibrating instrument is
   re-specified inline (a tenor + a quote); there is no reference to a stored
   instrument definition carrying its conventions.

### B.3 Proposed shape

- **An instrument-definition type per family** (deposit / FRA / STIR future /
  vanilla IRS / OIS), each carrying its full convention block (see §C). These live
  in the reference-data repository (§C), referenced by an internal id.
- **A curve-build request** = `{ value_date, [ (instrument_definition_ref,
  market_quote) ] }`. The server resolves each ref → definition → schedule, then
  runs a generalised bootstrap that orders the instruments by maturity and solves
  one pillar per instrument (extending `bootstrap_ois` to a family-dispatching
  residual; deposits and futures contribute closed-form pillars, FRAs/swaps/OIS
  contribute root-solved pillars). This subsumes the current OIS-only path.
- **A curve-set definition** mapping a named curve (e.g. `USD-SOFR`) → its ordered
  list of calibrating instrument refs, so the GUI builds and perturbs *by
  instrument* rather than by a hard-coded ladder.

---

## C. Static / reference-data repository — instrument-definition fields (RESEARCH)

### C.1 Current state

**There is no instrument-definition / static-data registry of any kind.**
Instruments are fully inline at pricing time. The only persisted registry in the
system is the **identity / Entity / Book / User store**
(`crates/celnet-server/src/config/identity.rs`) — an in-process, JSON-persisted,
admin-managed registry of *principals/desks/books*, not instruments. That store is
the **architectural pattern to copy** for a reference-data repo (keyed map,
load-on-start, admin-mutated, deny-by-default), but it holds no security static.

Confirmed: **no bond reference data** (ISIN/coupon/maturity/issuer) anywhere — as
the bond gap-analysis states.

### C.2 Standards used as the "what's required" reference

- **FIX 4.4 `Instrument` block** (component of `NewOrderSingle`/`ExecutionReport`/
  `SecurityDefinition`): `SecurityID(48)`+`SecurityIDSource(22)`, `Symbol(55)`,
  `Issuer(106)`, `CouponRate(223)`, `MaturityDate(541)`, `CouponPaymentDate(224)`,
  `IssueDate(225)`, `Factor(228)`, `RedemptionDate(240)`, `SecurityDesc(107)`,
  `Currency(15)`, `CFICode(461)`, `Product(460)`.
- **FpML** product taxonomy (interest-rate-derivative & bond schemas):
  `calculationPeriodDates`, `paymentDates`, `resetDates`, `floatingRateIndex`,
  `dayCountFraction`, `businessDayConvention`, `businessCenters`, `rollConvention`.
- **ISO 4914 (CFI / classification)** and **ISO 6166 (ISIN)** / CUSIP / SEDOL /
  FIGI for identifiers.
- **Common QuantLib instrument inputs** (the open golden oracle): schedule
  (effective/termination/tenor/calendar/convention/rule/EOM), day counters, index
  (`Sofr`, `USDLibor`-style term), fixed/float leg conventions, settlement days.

### C.3 Field tables (Field · Type · Required? · Notes)

#### C.3.1 Common header (every instrument family)

| Field | Type | Required? | Notes |
|---|---|---|---|
| `instrument_id` | internal id (newtype) | ✅ | The registry key; vendor-neutral. |
| `external_ids` | map(scheme → value) | ⚠️ | ISIN/CUSIP/SEDOL/FIGI/RIC cross-refs; ≥1 recommended. |
| `family` | enum (Deposit/Fra/StirFuture/Irs/Ois/Bond/FxOption…) | ✅ | Selects the convention block. |
| `currency` | ISO 4217 | ✅ | Pricing/settlement currency. |
| `description` | string | ⚠️ | Human label for blotters/GUI. |
| `cfi_code` | string (ISO 4914) | optional | Classification. |

#### C.3.2 Rates — Deposit (cash)

| Field | Type | Required? | Notes |
|---|---|---|---|
| `index`/`reference` | enum/string | ✅ | The o/n or term rate (e.g. SOFR). |
| `tenor` | tenor or maturity date | ✅ | e.g. ON/TN/1W/1M, or explicit date (broken). |
| `day_count` | enum | ✅ | ACT/360 typical for USD money market. |
| `business_day_convention` | enum | ✅ | Modified-following typical. |
| `calendar(s)` | calendar id(s) | ✅ | US settlement calendar etc. |
| `spot_lag` (settlement) | days | ✅ | T+2 (or T+0 for SOFR-style). |
| `quote` | rate (decimal) | ✅ (at build) | The deposit rate (a market quote, not static). |

#### C.3.3 Rates — FRA

| Field | Type | Required? | Notes |
|---|---|---|---|
| `float_index` | enum/string | ✅ | Projected index. |
| `start`/`fixing` + `end`/`maturity` | tenor or dates | ✅ | The `[T1,T2]` window (broken dates allowed). |
| `accrual_day_count` | enum | ✅ | `tau` basis (ACT/360). |
| `business_day_convention`, `calendar(s)` | enum / ids | ✅ | Rolls. |
| `spot_lag` | days | ✅ | Settlement offset. |
| `quote` | rate | ✅ (at build) | Contractual/market FRA rate. |

(Engine support already present: `Fra::from_dates` takes `reference, fixing_date,
maturity_date, accrual_basis`.)

#### C.3.4 Rates — STIR future

| Field | Type | Required? | Notes |
|---|---|---|---|
| `contract`/`symbol` | string | ✅ | The listed contract code. |
| `reference_window` `[T1,T2]` | dates/tenor | ✅ | Fixing window. |
| `price`→`futures_rate` | `(100−price)/100` | ✅ (at build) | Quote. |
| `convexity_vol` (σ) | f64 | ⚠️ | Debias adjustment; 0 ⇒ no adjustment. |
| `day_count`, `calendar` | enum / id | ✅ | |
| `contract_size`/`tick` | money | optional | For hedge-count math (§D.6). |

(Engine support present: `StirFuturesQuote`, `convexity_adjustment`.)

#### C.3.5 Rates — Vanilla IRS / OIS

| Field | Type | Required? | Notes |
|---|---|---|---|
| `effective`/`maturity` or `tenor` | dates or tenor | ✅ | Broken dates allowed. |
| `fixed_leg.frequency` | enum (A/SA/Q) | ✅ | `PaymentFrequency` exists. |
| `fixed_leg.day_count` | enum | ✅ | Market USD fixed is 30/360 — **not yet in `celnet_types::DayCount`** (ACT/365F + ACT/360 only); 30/360 selectable via `AccrualBasis` in schedules but not the shared `DayCount` enum (see `vanilla_swap.rs` "not in this slice"). |
| `float_leg.index` | enum/string | ✅ | SOFR / term SOFR. |
| `float_leg.frequency` | enum | ✅ | |
| `float_leg.day_count` | enum | ✅ | ACT/360 typical. |
| `business_day_convention` | enum | ✅ | Modified-following. |
| `calendar(s)` | ids | ✅ | |
| `roll_convention` | enum (IMM/EOM/DOM) | ⚠️ | EOM handling. |
| `spot_lag` | days | ✅ | |
| `notional` conventions | money | ⚠️ | Constant/amortising. |
| `fixed_rate`/`quote` | rate | ✅ (at build) | Par quote. |
| `discount_curve` / `projection_curve` refs | curve ids | ⚠️ | Single-curve today; dual-curve is the documented seam (`vanilla_swap.rs`). |

#### C.3.6 Cash bonds (forward-looking — per the bond gap-analysis)

| Field | Type | Required? | Notes |
|---|---|---|---|
| `security_id` + `scheme` | value + enum (Isin/Cusip/Sedol/Figi) | ✅ | Identity. |
| `ticker` | string | optional | Display. |
| `issuer` (+ LEI) | string | ✅ | |
| `coupon_rate` | decimal | ✅ | 0 for zeros/FRNs. |
| `coupon_type` | enum (Fixed/Frn/Zero) | ✅ | FRN adds index+margin. |
| `coupon_frequency` | enum | ✅ | A/SA/Q. |
| `day_count` | enum | ✅ | ACT/ACT, 30/360, ACT/365… |
| `issue_date`, `dated_date`, `first_coupon_date`, `maturity_date` | dates | ✅ (maturity), ⚠️ others | First-coupon drives the short/long stub. |
| `currency` | ISO 4217 | ✅ | |
| `redemption` / `face` | decimal | ✅ | Par redemption. |
| `ex_div` / `settlement` convention | rule | ✅ | T+1/T+2; ex-div window. |
| `benchmark_security` ref | id | ⚠️ | For spread-to-benchmark. |
| `sector` / `rating` | enum/string | optional | RV / credit bucketing. |
| `min_piece` / `increment` | money | optional | Tradeable size. |
| `calendar(s)` | ids | ✅ | |

(Engine: `bond.rs` `CashBond` covers fixed-coupon schedule + redemption + YTM/Z/G/ASW
but is **spot-starting** — no accrued/settlement, no FRN, no identity.)

#### C.3.7 FX options (already priced — for completeness)

| Field | Type | Required? | Notes |
|---|---|---|---|
| `pair` | ccy pair | ✅ | Already in `Instrument`. |
| `premium_currency` + `unit` | enum | ✅ | Already in FIX dialect (`dialect_fx.rs`). |
| `delivery`/`settlement` convention | enum | ✅ | settlement_style on `Instrument`. |
| `cut` | enum (NY/TOK/…) | ✅ | Expiry cut. |
| `day_count` | enum | ⚠️ | Expiry-time basis. |
| `delta`/`premium` conventions | enum | ✅ | Smile/quoting conventions. |

### C.4 Recommended `celnet` reference-data repository

A purpose-named registry (e.g. an *instrument reference store*), keyed by an
**internal instrument id** with an `external_ids` cross-ref map, holding a
per-family definition (the convention blocks above). Design points:

- **Pattern:** mirror the existing identity/Entity/Book store
  (`config/identity.rs`) — an in-process keyed map, loaded on start, **admin-managed**
  (an admin service + GUI surface), **deny-by-default** access, persisted as
  git-committed config / a durable file (consistent with celnet's persistence
  model; see §D.4).
- **Resolution seam:** pricing and curve-building **resolve a ref → definition**
  instead of carrying inline conventions. The OIS path becomes a thin special case
  (definition = "spot-starting USD-SOFR OIS of tenor N").
- **Relation to the just-built Entity/Book registry:** same storage/admin chassis,
  a sibling registry; deals/positions reference instrument ids the same way they
  reference book/desk ids.
- **Relation to the bond gap-analysis:** the `BondInstrument` proposed there is one
  family in this registry; capture (`BondDeal`) references an instrument id, so
  capture and pricing share one definition.

---

## D. Comparison to the provided cash-bond System Design Document

The provided doc describes a **distributed, low-latency, multi-dealer D2C** bond
platform (Java/SQL/Kafka-flavoured). celnet's actual design (per
`docs/ARCHITECTURE.md`, ADRs 0007–0011) is an **in-process, zero-alloc hot-core,
gRPC/WebSocket(+FIX) edge, deny-by-default, git-committed-config** options/rates
platform. The divergences below are mostly **deliberate**; the *capabilities* worth
adopting are called out separately from the *mechanisms* that conflict.

### D.1 RFQ ingestion over FIX 4.4 (tags 35/55/54/38, <5ms)

- **celnet today:** a complete hand-rolled FIX 4.4 engine exists
  (`crates/celnet-fix/`): SOH framing, FIXT/4.4 session FSM, **acceptor + initiator**
  roles, an **FX-options dialect** (`dialect_fx.rs`) and a **rates dialect**
  (`dialect_rates.rs`), plus gRPC and WebSocket edges. Estate ingress is governed by
  **ADR-0011** (`celnet-fix` as the in-repo CelNet-estate ingress).
- **Aligned:** FIX 4.4 transport, RFQ→Quote/ExecutionReport flow, acceptor venue
  role, sub-engine latency targets (ARCHITECTURE §1.2 p50/p99 budgets).
- **Missing — a cash-bond FIX dialect.** There is no bond mapping:
  `SecurityID(48)`/`Source(22)`, `Issuer(106)`, `CouponRate(223)`,
  `MaturityDate(541)`, `Yield(236)`, `Spread(218)`, `AccruedInterestAmt(159)`,
  `SettlDate(64)`, `NetMoney(118)` — exactly the ~23 missing concepts from the bond
  gap-analysis §3. Verdict: **aligned on transport, missing-and-wanted on the bond
  dialect.**

### D.2 Valuation core: YTM = benchmark yield + credit spread; price = Σ discounted coupons (worked 2Y 5% corp → 99.069)

- **celnet today:** the discounting building blocks exist. `bond.rs` (`CashBond`)
  computes **price = Σ discounted coupons** (curve PV), **YTM** (from an observed
  price), **Z-spread**, **G-spread**, **asset-swap spread**. The curve is the
  bootstrapped SOFR discount curve.
- **What's missing (the central gap):** the provided doc *derives a price* from
  **benchmark yield + a credit spread** pulled from a **credit-spread matrix**
  (their TRACE/CDX-style feed). celnet has **no credit-spread matrix, no
  TRACE/CDX-style feed, and no benchmark-yield + credit-spread → price workflow.**
  Additionally `bond.rs` is **spot-starting only** (no accrued interest, no
  clean/dirty, no settlement date) and the **bond arm is not on the wire**
  (`RatesInstrument` oneof = `ois` only; no `BondInstrument` proto). So the
  *valuation math is partly present* (discounting + YTM + spreads), but the
  *credit-spread-driven cash-bond pricer the doc centres on does not exist as a
  product path.* Verdict: **missing-and-wanted — a new bond pricing engine/feed +
  accrued/settlement layer + wire arm.** (Their worked example — 2Y 5% corp →
  99.069 from benchmark + spread — is exactly the workflow celnet lacks.)

### D.3 A2A internal crossing engine (mid-market cross, 20 ms fallback)

- **celnet today:** an **RFQ dealer-quoting desk** exists (`RfqDeskService` — submit/
  respond/accept/list) and a notification stream, but **no automated internal
  crossing** (no mid-market auto-cross, no A2A matching, no timed fallback).
- **Verdict:** **missing.** Fit is reasonable as an optional desk feature, but it is
  a market-structure/matching component, not a pricing one — lower priority than the
  bond pricer + reference data, and it must respect the deny-by-default authz model.

### D.4 ACID SQL persistence (`fixed_income_rfqs`, `deal_risk_positions` tables)

- **celnet today:** **deliberately not a relational design.** State lives in
  **in-process Rust stores** — the identity/Entity/Book/User registry
  (`config/identity.rs`, JSON-persisted keyed maps) and the rates position store /
  deal blotter (in-process), plus **git-committed config**. The pinned hot core is
  log/lock/alloc-free (CLAUDE.md §11); telemetry offloads over a bounded queue.
- **Trade-off:** in-proc gives the latency/throughput the architecture mandates;
  it trades away SQL's transactional durability and ad-hoc query/audit surface. The
  provided doc's ACID tables buy durability/audit at the cost of per-RFQ DB
  round-trips (incompatible with the zero-alloc hot path).
- **Verdict:** **intentionally different.** Adopt the *capability* (durable,
  auditable trade/risk record) **without** the *mechanism* (a relational DB on the
  hot path): an append-only event journal / WAL on the bounded offload queue is the
  celnet-shaped way to get durability + audit. Don't put SQL in the price path.

### D.5 Kafka post-trade topic (`fixed-income.trades.v1`)

- **celnet today:** `NotificationService.StreamNotifications` (dedicated push,
  bounded entitlement-scoped broker) + WebSocket mirror.
- **Verdict:** **aligned in spirit, different transport.** celnet's stream is the
  post-trade fan-out; a Kafka bridge could be an *edge adapter* if an external
  consumer needs it, but the internal transport stays gRPC/WS. No versioned topic
  name (rule 9: one current contract).

### D.6 Automated macro-hedging loop (DV01 → CME-futures hedge count)

- **celnet today:** the **risk inputs exist** — DV01, PV01, and key-rate ladders
  (`fra_risk` / `ois_risk`, `RatesPricingResult.key_rate_ladder`) plus STIR-futures
  convexity math (`futures.rs`/`futures_strip.rs`). There is a `RiskService`
  rates-position book. But there is **no automated hedge loop**: no DV01→futures
  hedge-count computation wired to **order routing/execution**.
- **Verdict:** **partially present (risk), missing (automation/execution).** The
  hedge-count arithmetic (portfolio DV01 ÷ futures DV01) is a small addition on top
  of existing risk; the execution loop is a larger, market-connectivity piece
  (needs the futures contract reference data from §C and an order-routing edge).

### D.7 Summary table

| Provided-doc capability/mechanism | celnet state | Verdict |
|---|---|---|
| FIX 4.4 RFQ ingestion | FIX engine + fx/rates dialects, gRPC/WS | Aligned (transport); **bond dialect missing** |
| Cash-bond YTM/discounting math | `bond.rs` YTM/Z/G/ASW off curve | Partly present |
| **Credit-spread matrix / TRACE-CDX feed / benchmark+spread pricing** | **none** | **Missing-and-wanted (central)** |
| Accrued/settlement (clean/dirty) | spot-starting only | Missing-and-wanted |
| Bond arm on the wire | `RatesInstrument` = OIS only | Missing |
| A2A internal crossing | none (RFQ desk only) | Missing (optional) |
| ACID SQL persistence | in-proc stores + git config | Intentionally different |
| Kafka post-trade | NotificationService/WS | Aligned-in-spirit, diff. transport |
| Automated DV01→futures hedge loop | DV01/ladders + futures math, no automation | Partial (risk) / missing (execution) |
| Reference / static data | **none** | Missing-and-wanted (underpins all) |

---

## E. Recommendations & sequencing

Ordered by leverage; dependencies noted.

1. **(C) Reference-data repository — build first; it underpins B and the bond
   work.** A vendor-neutral instrument reference store keyed by internal id with
   external-id cross-refs, modelled on `config/identity.rs` (keyed map, load-on-start,
   admin-managed, deny-by-default, durable). Start with the rates families and the
   bond family (the latter doubles as the `BondInstrument` from the bond gap-analysis).
2. **(A) Broken-date / custom-tenor curve pillars — cheap, high-visibility, no new
   numerics.** Generalise `OisPillar` to a `{ tenor | date }` discriminated key
   (reuse `BrokenDate`/`Tenor`), thread it through the server schedule builder
   (year-fraction map already exists), the GUI `CurveWorkspace` (editable pillars +
   date field + validation), and the offline mirror (`ratesPricing.ts`, which
   already has civil-date arithmetic). Keep offline ≡ live byte-identity.
3. **(B) Instrument-driven, multi-family curve build.** Add a deposit instrument and
   a generalised bootstrap that ingests a mixed `(instrument_definition_ref, quote)`
   list (deposits/futures/FRAs/swaps/OIS), resolving refs against the §C registry.
   Subsumes the OIS-only `CurveSet`. Depends on (C) and benefits from (A)'s
   date-general pillar.
4. **(D.2) Cash-bond pricing path — the biggest functional gap.** In order:
   (a) add the bond arm to the wire (`RatesInstrument::Bond` / `BondInstrument`,
   resolving a reference-data id); (b) add accrued-interest / settlement-date /
   clean-vs-dirty to `bond.rs`; (c) add a **credit-spread reference source** (a
   spread matrix / curve by sector×rating, the celnet-shaped analogue of the doc's
   TRACE/CDX feed) and a **benchmark-yield + credit-spread → price** workflow.
   Depends on (C) for identity and benchmark refs.
5. **(D.6) DV01→futures hedge-count helper.** Small: portfolio DV01 ÷ futures DV01
   from existing risk + futures reference data (§C). Defer the *execution/order-routing*
   loop (larger connectivity piece) and the **(D.3) A2A crossing engine** as
   optional desk features.
6. **(D.4 durability) Append-only trade/risk journal** on the bounded offload queue
   — the celnet-shaped way to get the doc's audit/durability *capability* without a
   relational DB on the hot path. A **(D.5)** Kafka *edge adapter* only if an
   external estate consumer requires it.

**Do NOT adopt:** SQL/ACID on the price path, Kafka as an internal transport, or
versioned topic/schema names — these conflict with ARCHITECTURE.md (zero-alloc hot
core) and ADR-0007 (one unversioned contract).

---

## Appendix — files cited

- GUI curve page: `gui/src/workspaces/CurveWorkspace.tsx`,
  `gui/src/data/ratesPricing.ts` (offline bootstrap mirror + civil-date arithmetic).
- Engine curve/bootstrap: `crates/celnet-rates/src/curve.rs`,
  `.../bootstrap.rs`, `.../schedule.rs`, `.../fra.rs`, `.../futures_strip.rs`,
  `.../futures.rs`, `.../vanilla_swap.rs`, `.../ois.rs`, `.../turns.rs`,
  `.../bond.rs`, `.../lib.rs`.
- Wire contract: `crates/celnet-proto/proto/celnet.proto`
  (`OisPillar`/`CurveSet`/`OisInstrument`/`RatesInstrument`/`RatesPricingResult`,
  plus existing `BrokenDate`/`Tenor`).
- Registry pattern: `crates/celnet-server/src/config/identity.rs`.
- FIX edge: `crates/celnet-fix/` (`dialect_fx.rs`, `dialect_rates.rs`).
- Grounding docs: `docs/ARCHITECTURE.md`, `docs/ANALYTICS-SPEC.md`,
  `docs/CONVENTIONS.md`, `docs/CELNET-INTEGRATION.md`,
  `docs/adr/ADR-0011-celnet-estate-ingress.md`,
  `docs/FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md`.
</content>
</invoke>
