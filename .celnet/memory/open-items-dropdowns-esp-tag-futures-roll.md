---
name: open-items-dropdowns-esp-tag-futures-roll
description: "Three carried-over build items (instrument dropdowns, ESP/RFS tag mislabel, futures front-month auto-roll) with the exact file:line evidence already gathered, so a fresh session starts at implementation not investigation."
metadata: 
  node_type: memory
  type: project
  originSessionId: 8d80fdd4-c0ec-4f58-9709-ea1816fd9391
  modified: 2026-08-13T18:12:15.663Z
---

Three items agreed on 2026-08-13 but NOT started — the session hit ~$540 and was
cut to preserve budget. All the investigation is done; each entry below is at the
"write the code" stage. Ordered by user-stated value.

## 1. Instrument dropdowns instead of free-text (the user's headline ask)

**Problem, in the user's words:** "when we have hedge vehicles we should have
dropdowns of all the instruments so the user does not have to type them in. Or we
should have an easy suggestion to auto populate values based on trading
information." Explicitly extended to "all wizards that shows portfolios etc."

Today `Hedge vehicles` (Hedging → Vehicles) makes a trader TYPE a contract id like
`ZFU26` and a DV01-per-unit by hand. Screenshot showed `0 rows`. The guided-setup
wizard has the same shape for portfolios/desks.

**Why this is now easy and wasn't before:** the data exists and is enumerable.
The running server reports **337 tradeable** instruments, and lp-sim quotes
**173** of them. The LP Panel already proves *which* have live liquidity, so the
picker can be filtered to instruments that can actually FILL rather than merely
exist — the difference between a hedge that executes and one that backstops.

**Shape to build:**
- A picker sourced from reference data, grouped (cash bond / Treasury future /
  SOFR STIR / OIS point), filtered to instruments with a live composite.
- DV01-per-unit auto-populated from the contract's own terms rather than typed —
  it is derivable (`TreasuryFutureSpec::dv01_per_contract`, and
  `StirContractTerms::basis_point_value` for the STIR strip).
- Same treatment for the wizard's portfolio/desk dropdowns.

**Files:** `gui/src/workspaces/hedging/HedgeVehicleRegistry.tsx`,
`gui/src/workspaces/hedging/ExitActionEditor.tsx`, the guided-setup wizard, and
`gui/src/lib/hedgeVehicle.ts`. Server side already exposes reference data
(`ListInstruments`) — the ESP sim leg uses it.

## 2. ESP/RFS tag mislabel

**Symptom:** the quotes blotter badges `1y OIS` rows **ESP**. The user: "you can't
RFS and have ESP."

**Cause found:** `EspOrRfq` is proto3 with `ESP_OR_RFQ_ESP = 0` as the **zero
default**, so any writer that leaves the field unset reads as ESP. The genuine RFQ
paths DO stamp it explicitly — `crates/celnet-server/src/services/quote.rs:2579`
and `crates/celnet-server/src/services/aggregation.rs:1229` both set
`EspOrRfq::Rfq` — which is why only *some* rows are wrong.

**What is left:** find the writer that never sets it and stamp the real channel.
I deliberately did not guess.

**Related, and NOT a code change:** `PricingSourceMode::ProductSplit` (enum value
2) is already fully implemented at
`crates/celnet-server/src/services/fix.rs:1311` — `if is_bond { composite } else
{ curve }`, i.e. exactly the user's "quotes onto internal swap curves and quotes
onto the aggregated book". It is a **setting on the pricing group** in
Administration → Pricing Groups. Do NOT change the platform default
(`CompositeFirstCurveFallback`) — its doc comment correctly warns that would
regress book-fed bond streams.

## 3. Futures front-month auto-roll

`celnet_refdata::front_contract(symbol, as_of)` exists in
`crates/celnet-refdata/src/futures.rs` and is exported from `lib.rs`, but is
**wired nowhere** (verified by repo-wide search: only its own definition and the
re-export).

Consequence: `HedgeVehicle::Future { contract_id }`
(`crates/celnet-hedge-routing/src/vehicle.rs`) names a FIXED delivery month, so a
policy set to `ZFU26` keeps pointing at Sep-26 after the September roll — at
which point the contract has stopped trading. `front_contract`'s own doc states
the intent: *"a hedge policy names a product, not a delivery month … that answer
changes four times a year at the quarterly roll."* The design is there; the
connection is not.

**Shape:** let a vehicle name the PRODUCT (`ZF`) and resolve to the front contract
at use time. Touches the vehicle enum, the registry resolver, and the GUI picker —
so it composes naturally with item 1.

## Context a fresh session needs

- **5-year Treasury future = `ZF`** (legacy `FV`); ids `ZFU26` / `ZFZ26`. Quoted
  live by lp-sim at `108.6328 / 108.6484` on the quarter-of-a-32nd grid.
- **`./run_dev.sh`** is the single dev entry point — see
  [[run-dev-single-entry-point]].
- Hedge sizing already works: `units = position_DV01 / vehicle_DV01_per_contract`,
  rounded to whole contracts with the residual reported, and the `Dv01Basis` label
  carried to the screen so a proxy ratio is never mistaken for an exact one.
- Also open, lower priority: **European futures** (Bund/Gilt) need a EUR/GBP cash
  securities-master first — refdata's committed universe is US Treasuries only
  (267 records), and anchoring them without one would fabricate a price handle.
- The server warns `TRADEABLE BUT UNQUOTABLE` for `acme-5y-corp` and
  `ust-2y-note`: advertised as tradeable, quoted by no LP, so a hedge on either
  backstops to the composite.
