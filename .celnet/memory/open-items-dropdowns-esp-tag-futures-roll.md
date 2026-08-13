---
name: open-items-dropdowns-esp-tag-futures-roll
description: "All three carried-over items (instrument picker, ESP→RFS desk-kind, futures front-month auto-roll) are DONE on local main — including the two prior-session diagnoses that turned out to be wrong."
metadata: 
  node_type: memory
  type: project
  originSessionId: 37108ac7-fabe-4b2a-bb57-ca650635433a
  modified: 2026-08-13T19:02:21.901Z
---

✅ **All three DONE 2026-08-13** on local `main` — gates green (2274 GUI / 647 Excel
tests, both production builds, full Rust suite, fmt + clippy `-D warnings`).
**NOT pushed to `origin/main`, NOT deployed to UAT.**

Two of the three prior-session diagnoses were **wrong**. The corrections below are the
durable part: a hypothesis recorded in memory is a lead, not a finding.

## 1. Instrument dropdowns — the cause was a SILENT CODEC MISDECODE

Not a missing UI. `celnet.proto` has always carried `BondFutureDef bond_future = 12`
in the `InstrumentDef` oneof, and the server seeds the whole listed Treasury complex
with a **derived DV01 per contract**. But `gui/src/data/wsCodec.ts::instrumentDefFromWire`
had no `bond_future` arm and **fell through to `bond` for anything unrecognised** — so
every seeded future decoded as a cash bond with an all-zero `BondDef` (zero coupon,
1970 maturity). The data was on the wire; the client destroyed it. That is why the
trader was typing `ZFU26` and a DV01 by hand.

Fixed: `bond_future` decoded end-to-end; the fallthrough now **throws** on an unknown
family rather than guessing. New `gui/src/lib/instrumentPicker.ts` (pure, 6 tests) +
`gui/src/components/InstrumentPicker.tsx` (WAI-ARIA combobox). Picking a future
auto-populates DV01/unit + whole-lot flag + unit label; **a cash bond pre-fills
nothing** — its DV01 is a function of the live curve, not a static term.

Also deleted: `INSTRUMENT_OPTIONS = ["AGG-OIS","AGG-US10Y","AGG-EURUSD","AGG-UK5Y"]`,
hardcoded in **both** `HedgingWorkspace.tsx` and the setup wizard — four invented ids
that looked like configuration. `ExitActionEditor` needed no change: it already offers
a registered-vehicle `<select>`, correct by design.

## 2. ESP/RFS — the `EspOrRfq` proto3-zero hypothesis was WRONG

The blotter badge is `DeskRequestKind`, whose zero is `UNSPECIFIED` — there was no
default-value bug. The real cause: `DeskRequestKind::Esp` had exactly **one** writer,
`fix.rs:1659/1668`, the `FixedIncomeStream` venue — which decodes a **client-supplied
notional** and streams a two-way priced for that clip. That is request-for-stream by
definition; an ESP is dealer-published and clip-independent. The platform already
called that venue RFS in two places (`RatesIntent::Rfs`, the GUI connection label)
while stamping its rows ESP.

`DESK_REQUEST_KIND_ESP` → `DESK_REQUEST_KIND_RFS`, **tag 3 unchanged** so the encoding
is byte-identical. Pricing-group `esp_pipeline` / `EspOrRfq` **deliberately not
renamed** — that is a separate, coherent taxonomy where RFS is grouped with RFQ.
Latent bug fixed en route: the Excel `deskRequestKind` codec stopped at tag 2 and
could not decode an inbound RFS notification at all.

## 3. Futures front-month auto-roll — wired

New `celnet_refdata::{is_product_symbol, front_contract_id}` (legacy floor symbols
aliased, so `FV` resolves `ZF`) + `rates_book.rs::roll_to_front_month`, applied inside
`resolve_hedge_vehicle`. A registry row may name a **product** (`ZF`) and re-points
itself at each quarterly roll — test-verified across the real cessation boundary
(`ZFU26` → `ZFZ26`). An explicit delivery month is **never** silently re-pointed; a
product whose listed cycle has fully expired resolves to `None`, so the caller falls
back to the self-hedge and logs why rather than routing at an invented code.

The roll lives in the **server**, not `celnet-hedge-routing` — that crate's charter is
"no server, wire, or market-data dependency".

## Incidental / still open

- `gui/test/commands.test.ts` was **already red on `main`** before this work (the LP
  Panel's `liquidity` rail row landed in `43c02a6a` without updating two rail
  assertions). Verified pre-existing by stashing, then fixed.
- Unchanged: European futures need a EUR/GBP cash securities-master first (refdata is
  US-only, 267 records). The picker's `onRawCommit` escape hatch keeps such an
  instrument expressible meanwhile, flagged unknown.
- Live-liquidity marking is **supported but unwired** on the Vehicles screen: the
  picker takes an optional `liquidIds`, and that config screen carries no book context
  to source a composite from without opening a book stream.
