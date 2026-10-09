# FI Deal Capture — Bond-Platform Gap Analysis

**Status:** analysis only (no code change). 2026-06-29.
**Question answered:** does the fixed-income deal blotter capture all the correct
information to receive a bond trading platform's (MarketAxess) **order objects**
and **execution-report (deal) objects**?

**Short answer:** No. The current capture models the **rates / swaps
dealer-quoting** flow (RFQ → quote → lift on OIS / swaps / FRAs). It has no
cash-bond instrument identity, no bond price/yield/spread economics, no
accrued/settlement money, and no order-lifecycle or regulatory fields. To ingest
a bond venue's order + execution reports we need a bond-capable capture model.

> **Naming guardrail.** Per `GUIDE.md` §8, no vendor/competitor names appear in
> proposed identifiers; "MarketAxess" is referenced here only as the integration
> target (analysis context). Proposed types are purpose-named.

---

## 1. Scope note — rates derivatives vs cash bonds

The shipped FI desk (`25293f8`) trades **rates derivatives**: the `Deal.instrument`
is a `RatesInstrument` (OIS / swap / FRA) and `Deal.price` is a single all-in
fixed rate. A bond venue like MarketAxess trades **cash bonds** (govvies / credit),
whose economics (clean/dirty price, yield, spread-to-benchmark, accrued, net money,
settlement date) and identity (ISIN/CUSIP, coupon, maturity, issuer) have **no
representation** in the current model. So this is not a "missing columns" tweak —
it is a new instrument family + a superset deal/order model.

---

## 2. Current state (what we capture)

### `Deal` (proto) — 14 fields
`deal_id`, `request_id`, `kind` (RFQ/IOI), `counterparty`, `desk`,
`instrument` (`RatesInstrument`), `curve_set`, `side`, `notional`, `price`
(all-in level), `executed_at_nanos`, `trader`, `position_id`, `correlation_id`.

### Blotter columns (`DealsBlotterWorkspace.tsx`) — 10
Time · Counterparty · Desk · Instrument · Ccy · Notional · Price · Side · Trader · Deal-id.

---

## 3. Reference field set

Drawn from the **FIX 4.4 `ExecutionReport` (35=8)** and **`NewOrderSingle` (35=D)**
fixed-income usage plus standard cash-bond trade capture. This is the *generic*
reference; the authoritative list is MarketAxess's own order + execution-report
objects, which must be reconciled against §5 once supplied (see §6).

### 3a. Order object (inbound `NewOrderSingle` / RFQ)
| Concept | FIX tag | Have? |
|---|---|---|
| Client order id | ClOrdID(11) | ❌ |
| RFQ / quote-request id | QuoteReqID(131) | ⚠️ `request_id` (internal id, not the venue's) |
| Side (buy/sell) | Side(54) | ✅ `side` (desk-perspective — needs counterparty perspective too) |
| Security id (ISIN/CUSIP) | SecurityID(48)+Source(22) | ❌ |
| Symbol / ticker | Symbol(55) | ❌ |
| Order qty (par amount) | OrderQty(38) | ⚠️ `notional` (rates notional, not bond par) |
| Order type (limit/mkt) | OrdType(40) | ❌ |
| Limit price / yield / spread | Price(44)/Yield(236)/Spread(218) | ❌ |
| Time in force | TimeInForce(59) | ❌ |
| Settlement date | SettlDate(64) | ❌ |
| Account | Account(1) | ❌ |
| Currency | Currency(15) | ⚠️ implied by instrument |
| Counterparty / parties | NoPartyIDs(453) | ⚠️ `counterparty` (string only) |

### 3b. Execution-report (deal) object
| Concept | FIX tag | Have? |
|---|---|---|
| Order id | OrderID(37) | ❌ |
| Exec id | ExecID(17) | ⚠️ `deal_id` (internal) |
| Exec type / order status | ExecType(150)/OrdStatus(39) | ❌ (only terminal "accepted" persisted) |
| Last / cum / leaves qty | LastQty(32)/CumQty(14)/LeavesQty(151) | ❌ (no partial fills) |
| **Security id (ISIN/CUSIP/SEDOL)** | SecurityID(48)+Source(22) | ❌ |
| Issuer / coupon / maturity / desc | Issuer(106)/CouponRate(223)/MaturityDate(541)/SecurityDesc(107) | ❌ |
| **Clean price** | Price(44)/LastPx(31) | ⚠️ one `price` field, type-ambiguous |
| **Dirty price** | (derived) | ❌ |
| **Yield** | Yield(236)+YieldType(235) | ❌ |
| **Spread to benchmark** | Spread(218)+BenchmarkCurveName(221)/benchmark id | ❌ |
| **Accrued interest** | AccruedInterestAmt(159)/Rate(158) | ❌ |
| Gross trade amount (principal) | GrossTradeAmt(381) | ❌ |
| **Net money / settlement amount** | NetMoney(118) | ❌ |
| Avg px | AvgPx(6) | ❌ |
| Trade date | TradeDate(75) | ⚠️ derivable from `executed_at` |
| Transact time | TransactTime(60) | ✅ `executed_at_nanos` |
| **Settlement date** | SettlDate(64) | ❌ |
| Settlement currency | SettlCurrency(120) | ❌ |
| Trading capacity (principal/agent) | OrderCapacity(528) | ❌ |
| Counterparty LEI | PartyID+PartyRole(LEI) | ❌ |
| Executing / sales party | NoPartyIDs(453) | ⚠️ `trader`, `desk` only |
| Venue / market id (MIC) | LastMkt(30)/MarketSegmentID | ❌ |
| Regulatory (MiFID II / TRACE) | various | ❌ |

**Tally:** of ~30 standard execution-report concepts, we fully capture ~2
(`transact time`, `side`), partially ~5 (id, qty, price, counterparty, trade
date), and are **missing ~23** — almost the entire cash-bond surface.

---

## 4. Proposed vendor-neutral schema (for review — not yet built)

A new instrument family + a superset deal that carries bond economics. All
purpose-named (no vendor identifiers); one current contract (no versioning).

### 4a. `BondInstrument` (new)
```
BondInstrument {
  security_id        // the identifier value (e.g. an ISIN string)
  security_id_scheme // enum: Isin | Cusip | Sedol | Figi
  ticker             // optional display ticker
  issuer
  coupon_rate        // annual %, 0 for zeros/FRNs (+ a coupon_type enum)
  maturity_date      // ISO date
  currency
  description        // human label
}
```

### 4b. `BondDeal` (or extend `Deal` with a oneof instrument + bond economics)
```
BondDeal {
  // identity / lifecycle
  deal_id, order_id, exec_id, request_id (venue RFQ id)
  exec_type, order_status            // enums (New|PartialFill|Fill|Done|Cancelled…)
  // who
  counterparty, counterparty_lei, account, desk, trader, sales
  trading_capacity                   // enum: Principal | Agent | RisklessPrincipal
  venue_mic                          // market identifier
  // what
  instrument: BondInstrument
  side                               // counterparty perspective + desk perspective
  // quantity
  par_amount                         // OrderQty
  last_qty, cum_qty, leaves_qty      // partial-fill support
  // price (capture ALL conventions; do not collapse to one number)
  price_clean, price_dirty
  yield, yield_convention            // enum
  spread_bps, benchmark_security     // spread-to-benchmark
  // money
  accrued_interest
  principal                          // gross_trade_amt
  net_money                          // settlement amount
  settlement_currency
  // dates
  trade_date, settlement_date, transact_time
  // ours
  position_id, correlation_id
}
```

### 4c. `BondOrder` (inbound) — mirror of §3a with the same identity/economics block.

### 4d. Blotter columns (proposed)
Time · Status · Side · Security (ticker + ISIN) · Coupon/Maturity · Par ·
Clean · Yield · Spread · Accrued · Net money · Settle date · Counterparty ·
Capacity · Trader · Venue · Deal-id — with the rates-swap columns retained for
the existing derivatives deals (the blotter shows both families, column set keyed
on instrument family).

---

## 5. What "verify against MarketAxess" requires

This doc compares to the *generic* FIX/bond standard. The authoritative check the
request asks for needs MarketAxess's own:
1. **Order object** — a sample inbound order (their FIX `NewOrderSingle`/RFQ
   dialect, or API JSON), with every tag they populate.
2. **Execution-report (deal) object** — a sample fill, same.

With those, the §3 tables become a definitive field-by-field reconciliation and
§4 is finalized to a superset of *their* objects (not just the generic standard).

---

## 6. Open questions (block a definitive build)
- **Asset scope:** are we adding **cash-bond trading** as a product, or only
  ingesting bond deals for capture/booking/risk? (Changes whether `BondOrder` and
  a pricing path are needed, or just `BondDeal` capture.)
- **MarketAxess dialect:** FIX (which version/dialect) or REST/API? Drop-copy vs
  order-routing? (Affects the ingress mapper — cf. `celnet-fix` + ADR-0011 estate ingress.)
- **Identifier scheme(s):** ISIN-only, or CUSIP/SEDOL/FIGI too?
- **Settlement / regulatory depth:** which regimes (MiFID II transaction reporting,
  TRACE)? Determines the regulatory field block.
- **Price convention authority:** do we store all of clean/dirty/yield/spread and
  derive, or store what the venue sends + one canonical? (Recommend: store what
  they send, never collapse.)

## 7. Recommendation
Treat this as a **new bond capture model**, not a blotter tweak. Sequence:
(1) obtain the two MarketAxess objects (§5); (2) finalize §3 into a reconciliation;
(3) build `BondInstrument` + `BondDeal`(+`BondOrder`) as a oneof alongside the
rates deal; (4) extend the blotter to a family-keyed column set; (5) wire the
ingress in `celnet-fix` (per ADR-0011) mapping their objects → `BondDeal`. Each
step gated + validated against a replayed sample of their real objects.
