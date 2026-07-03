# ADR-0021 — Uniform architecture for every asset class, on every surface

**Status:** Accepted (operator directive 2026-07-03).
**Deliverable:** multi-asset-core-integration.

## Context

CelNet is a multi-asset platform (FX vanilla, cross-asset equity/commodity/crypto,
exotics, fixed income OIS/IRS/FRA/Bond, credit CDS, inflation). The central-core program
already unified the engine room — one `PricingEngine` (ADR-0020/C1), one risk cube
(C2c), one descriptor-driven codec (G) — but audits kept finding **asset-class-specific
paths** on the surfaces: FI streaming exists only over FIX/RFS (not the WS `PriceFanout`);
market-series observables are FX-only (`handle_series_subscribe` → `unimplemented` for
non-FX); the quote/RFQ risk leaf is FX-vanilla-only (`quote.rs::as_fx()`). Each such
fork is a place where a new asset class needs bespoke wiring instead of dropping into a
shared seam.

## Decision

**Every asset class leverages the SAME optimal architecture on EVERY surface. Never fork
a per-asset-class path — generalize the seam.**

Surfaces in scope, each with one architecture for all classes:
- **Pricing dispatch** — one `PricingEngine`; every product is a `Priceable` leaf.
- **Risk** — one non-additive risk cube; every class rides the joint VaR/ES + SbM path.
- **Streaming price-tick** — one `PriceFanout`; generalize the subscription key from
  `Underlying` to a general key that also covers curves/rate-instruments (rates-stream-ws).
- **Market-series observables** — one observable seam per class (market-series-generalize).
- **RFQ / quote** — one quote path carrying each class's class-correct risk leaf
  (quote-risk-generalize, rates-rfq-ws).
- **Wire contract** — one `Price(oneof Instrument{…})` RPC (unified-price-rpc).
- **Codec** — descriptor-driven, generated for every message (ws-codec-from-proto).
- **Limits, entitlements, SDK, XVA, market-data** — class-parametric, no FX-only branch.

**Legitimate exception:** *routing within* a unified engine by asset-class arm (e.g.
`is_cross_asset` selecting the FX two-rate carry guard vs the cost-of-carry leaf) is not a
fork — it is the correct per-arm dispatch inside one architecture. The rule forbids
*parallel* per-class subsystems, not per-arm branches inside a shared one.

## Consequences

- A new asset class is added by extending shared seams (a oneof arm, a fanout key, a
  registry entry), not by building a parallel path — bounded, uniform, testable blast radius.
- **Conformance is ongoing:** each surface is audited for asset-class-specific guards
  (`as_fx()`, `FX-only`, `unimplemented` non-FX); every divergence becomes a generalization
  task under the `uniform-asset-class-architecture` board epic.
- Byte-identity for existing (options) paths is preserved through every generalization.
