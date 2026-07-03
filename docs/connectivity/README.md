# CelNet connectivity — vendor intelligence & target catalog

This directory holds the **vendor-connectivity intelligence** for the CelNet
connectivity extension (`crates/celnet-connectivity`). It is a *map of the FX
connectivity universe* and a **demand-driven backlog** — **not** a set of built
adapters. Design + critique: [`../CELNET-CONNECTIVITY-INTEGRATION.md`](../CELNET-CONNECTIVITY-INTEGRATION.md).

## What's here

- **`vendor-adapters.csv`** — the target catalog: 240+ `(vendor × product ×
  connection-type)` tuples across ~113 vendors and 4 priority tiers, with FIX
  spec-version provenance. Imported from the `soarsa/celnet-connectivity` review.
- **`VENDOR-ADAPTERS-INVENTORY.md`** — the per-vendor inventory (122 vendor
  folders, tiered), including the source's own honest count of what was actually
  built.

## Honest status (read before trusting the `status` column)

The CSV's `status` column reflects the **source repo's** own scaffold state — a
value like `CONFORMING` there meant *"a loopback `StubCounterparty` round-trip
test passed"*, **not** vendor certification and **not** a CelNet build. The
source was ~98.5% machine-generated scaffold (3 of 201 adapter modules real).

**In CelNet, exactly three adapters are real** — seeded as data in
`crates/celnet-connectivity/src/adapters.rs` with their real embedded FIX 4.4
dictionaries:

| code | counterparty | role |
|---|---|---|
| `bloomberg_fxgo_price` | Bloomberg FXGO | PRICE |
| `bloomberg_fxgo_order` | Bloomberg FXGO | ORDER |
| `rabofx_esp_order` | Rabobank FX | ORDER |

Every other row is a **target**, prioritised by tier. We do **not** brute-force
the catalogue; venues are built when a desk actually asks for one (an
investment-bank FX-options platform needs a handful of LP venues + the vol feed,
not 200 execution venues).

## Adding a venue (the workflow)

1. Obtain the vendor's FIX spec PDF.
2. Draft the QuickFIX dictionary: `tools/connectivity/spec-extract/` turns the
   PDF into a starting-point `FIX44_<VENDOR>.xml` (a human review pass follows —
   see that tool's README for what it does and does not do).
3. Add the dictionary under `crates/celnet-connectivity/specs/` and a
   `VendorAdapterSpec` entry in `src/adapters.rs` (vendor identity is **data**,
   never a Rust identifier — guardrail #8).
4. The certification harness (`src/cert.rs`) + the session driver (`src/driver.rs`)
   then exercise and certify the new venue with no per-adapter code.

## The higher-priority gap

The most important FX-options connectivity input — a **vol-surface (ATM/25Δ&10Δ
RR/BF) feed** — is **absent** from this catalogue (it is all spot/fwd/NDF FIX).
That gap is tracked separately as the `conn-vol-feed` lane (an FMD-shaped adapter
on `celnet-integration`'s `MarketDataSource` seam), and is arguably ahead of any
LP FIX venue in value.
