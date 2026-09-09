---
name: hedge-orders-denominated-in-dv01
description: Futures hedges were rejected NOT_A_WHOLE_LOT because the order carried DV01, not contracts — why the books pinned at 100%. Fixed across three commits; verify on the live wire, not the plan.
metadata:
  type: project
---

**2026-08-20.** "If we are hedging, why are the books 100% used?" — because no futures hedge
ever filled. The externalised size is denominated in the **budget metric (DV01)** and was
going onto the wire as `OrderQty(38)`:

```
plan:  14 contracts of ZTU26  (34.2707 DV01 each)
wire:  OrderQty = 484.5648            ← the DV01 figure
```

A listed-futures venue trades whole lots and rejects anything else, so every futures shed
came back `NOT_A_WHOLE_LOT`. `HedgeRatioPlan` had the contract count all along —
`plan_hedge_ratio` rounds to whole lots for exactly this reason — but nothing downstream
read it; `units` was used only to scale the DV01 for the book.

**Fix took THREE passes, and each later hole was only found by reading the live wire after
deploying — never by re-reading the diff:**

1. `7846b54a` — `ExternalHedgeRequest` gains `venue_quantity` (venue units) +
   `risk_per_venue_unit` (DV01 per contract) beside `size` (budget metric). The venue is
   asked, and its depth ranked, in venue units; the fill converts back once at the seam.
2. `3bc0a54d` — the helper only used the plan's count when it was NON-ZERO, so a sub-one-lot
   target fell through and put the DV01 back on the wire (`ZFU26 qty=38.3346`) — the very
   case whole-lot rounding exists to prevent. A resolved plan is now always authoritative;
   zero units ⇒ nothing sent, whole target stays an honest residual.
3. the blotter still LOGGED `effective_external`, so a 1-contract plan read
   "Requested 38.3347" and looked unfixed. `record_street_order` now takes the venue
   quantity + factor.

4. `4509e8a5` — whole CONTRACTS were STILL refused (`UBU26 qty=2`): the venue denominates
   order quantity in **FACE**. `contract_lot_size` IS the contract's face value and its
   quoted clips are `contracts × face` — deliberate, so one denomination spans the whole
   aggregated book. The wire now gets `units × face` and `risk_per_venue_unit` divides by
   the same factor, leaving the risk conversion exact. New
   `celnet_refdata::contract_face_value` (matched on product prefix, so any delivery month
   resolves). **Three units were conflated on this path: DV01 → contracts → face.**

**The tell at each stage:** the venue's answer changed `NOT_A_WHOLE_LOT` → `NO_MARKET` →
`filled`. **VERIFIED END TO END:** `SO-4 ZTU26 qty=1,800,000` (9 contracts × 200,000 face)
filled complete; HDG-5 removed **308.4 DV01**; WASH BOOK fell **499.9 → 6.8** (100% → 1%)
and RATES USD **4,997 → 4,472**. The books drain.

**Two more findings from the same screens:**

- **The bucket tiles rounded themselves to 100%.** `rates-usd` at 4,997/5,000 and
  `wash-book` at 499.9/500 both `toFixed(0)`'d to "100%" under a header reading "no bucket
  at limit" (the badge tests `>= 1` exactly). `displayPercent` now caps an under-limit book
  at 99, so **"100%" means AT the limit**; a real breach still reports its true size.
- **wash-book will never be hedged as configured.** Its RISK LIMIT (`RiskBookRisk.limits`,
  dv01 500) and its HEDGE THRESHOLD (`HedgeIntent.threshold`, 1,000,000) are two separately
  configured numbers. The board shows it full; the hedge engine sees 0.05% and correctly
  does nothing. Aligning them is a config change — arguably the engine should hedge to the
  book's actual limit, which is a design question.

Reject reasons were on the wire the whole time and only reachable by HOVERING; they now
print under the outcome chip (same lesson as `9d43370a`).

Related: [[hedge-rate-cap-lifetime-deadlock]], [[bond-hedge-books-no-offsetting-leg]].
