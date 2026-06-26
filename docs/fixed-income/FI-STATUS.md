# Fixed Income — implementation status & outstanding features

**Branch:** `feature/fixedincome` · **Updated:** 2026-06-25
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

**Five-client status:** FIX ✅ (E2), WS edge ✅ (F1), Rust SDK ✅ (F2). **Remaining: GUI** (F3 —
`Options | Fixed-Income` asset tabs + a rates pricing workspace, on the F1 WS transport), **Excel**
(F4 — rates worksheet functions), **federation** (F5 — rates fan-out). The GUI is the largest piece
and, per the web rules, wants visual-regression + a11y verification — best done in a focused session.

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
- **Monotone-convex-on-forwards** interpolation (the smooth-view scheme; log-linear-DF is shipped).
- **Turn-of-year / central-bank-meeting** forward jumps.
- **Deterministic STIR convexity** placeholder wired into the short-end build.

### C. Wire contract (`celnet-proto`, additive — single contract, guardrail #9)
- New arms on the one `celnet.proto`: **`CurveSet`**, **`RatesInstrument`** oneof
  (`fra` · `ois` · `vanilla_swap` · `tenor_basis_swap` · `cross_ccy_basis_swap` · `stir_future` ·
  `bond_future`), **`PricingResult { pv, par_rate, pv01, dv01, key_rate_ladder }`** — next free
  field numbers, no renumber.
- **Golden vectors + parity rows** per arm (`tools/check-verification-coverage.mjs` parses the oneof).

### D. Server (`celnet-server`, `celnet-risk-cube`)
- Pricing service consumes `celnet-rates` for the rates arms.
- **Rates risk folds into the existing server-owned `RiskService`** rollup — clients never loop-sum
  (FI-ARCHITECTURE §3); FRTB GIRR cross-checked vs ORE.

### E. FIX fixed-income dialect (`celnet-fix`) — **"FIX API supports FI"**
- NEW `crates/celnet-fix/src/dialect_rates.rs` mirroring `dialect_fx.rs`: decode inbound FI
  **RFQ**, **RFS-stream subscribe**, and **order** messages → route to the rates engine → quote / fill.
- Gated by a loopback FIX initiator (mirror `tests/fix_acceptor.rs`).

### F. Five-client parity (slice 9)
- **GUI** — the D2 **Options | Fixed-Income asset-class tab layer** (above the rail) + the FI
  workspace set (Curve · Ticket · RFQ · IOI · RFS · Blotter), per FI-ARCHITECTURE §4 and the
  [`mockups/`](./mockups/). *Real-GUI implementation pending the proto arms it renders.*
- **Excel** add-in — `CELNET.*` rates functions (curve DF, swap PV, par, PV01/DV01, key-rate).
- **Rust SDK** (`celnet-client`) — typed builders for the rates instrument vocab.
- **FIX** — the dialect in (E).
- **Federation** — rates pricing/risk fans out across shards.

---

## 🚫 Deferred (locked, **not** half-built — see OPEN-QUESTIONS Q3/Q7)
- **Rates vol** (`celnet-rates-vol`: swaptions/caps, normal/shifted-SABR cube, 1F Gaussian) — Q7.
- **Credit** (`celnet-credit`: single-name + index CDS) — Q3 (low appetite).
- Inflation (deflation-floor ILB), full multi-CSA / CTD-collateral, callable/putable OAS.
- Multi-currency generalisation + cross-currency basis (post-USD, hard currencies first — Q1).

---

## UI changes — explicit status
- **Administration tab:** ✅ done (`6c978cf`).
- **FI asset-class tabs + FI workspace set:** ⏳ outstanding (slice F) — designed in
  [`mockups/`](./mockups/) and FI-ARCHITECTURE §4. Now **unblocked**: the `celnet-proto` arms (C)
  and the server `PriceRates` rpc (D) exist, so the workspaces render live contract data, not
  placeholders. Remaining F work: WS mirror for `price_rates` (codec + dispatch), the GUI
  `Options | Fixed-Income` asset-class tabs + a rates pricing workspace (curve → OIS → PV / par /
  DV01 / key-rate ladder), then Excel rates functions + SDK builders + federation fan-out.

---

## Build order for the outstanding phase
**C (proto arms) ✅ → D (server consumes `celnet-rates`) ✅ → E (FIX dialect) ✅ → F (WS mirror ·
GUI asset-tabs + rates workspace · Excel · SDK · federation) ⏳**, with A/B product breadth
(FRA / IRS / futures / cash-bond RV) landing into `celnet-rates` in parallel (disjoint leaf).
C/D/E are committed and gated; F (the five-client surface) is the remaining front-end-led phase.
