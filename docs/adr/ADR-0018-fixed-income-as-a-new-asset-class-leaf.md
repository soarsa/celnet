# ADR-0018: Fixed income (cash bonds + credit) enters as a new asset-class leaf

> **Renumbered 2026-07-01:** was ADR-0012; the number collided with ADR-0012 (unified gBSM kernel, the canonical/heavily-referenced decision). Moved to the next free number. Prior references to "ADR-0012" meaning fixed income now read ADR-0018.

- **Status:** Accepted & Implemented (2026-07-01; celnet-bond and rates leaves landed on main).
- **Relates to:** ADR-0008 (multi-asset carry / asset-class routing), ADR-0010 (FI rates
  onto the carry seam), ADR-0007 (one unversioned contract), ADR-0020 (central contract).
- **Context doc:** `docs/FI-PRICING-ENGINE-DESIGN.md`.

## Context

We want a full cash-bond / credit market-making capability: bond analytics (DCF, YTM,
DV01, duration, convexity), credit spreads (CR01), tiered auto-quoting, and an auto-hedge
loop. The question is **structural**: does fixed income price through celnet's existing
cross-asset pricing core, or as a parallel rates/credit pricing service?

celnet's pricer already branches on **asset class** before product dispatch (ADR-0008:
`celnet-server::pricer::price_instrument` routes FX/metal vs equity/commodity/crypto, then
dispatches via the `ProductEngine` registry). The reference-data registry already holds bond
*definitions*; `celnet-rates` already builds the benchmark curve; the risk cube already
carries DV01 (ADR-0010); the dealer-quoting slice already runs RFQ/IOI + `Book` + limits +
FIX. The only genuinely missing pieces are the **bond/credit pricing leaves** and two
behaviours layered on existing data (client-tier spreads, auto-hedge).

## Decision

**Fixed income (cash bonds + credit) is a new asset-class leaf on the existing cross-asset
`ProductEngine` router — not a parallel pricing system.**

1. A bond/credit `Instrument` arm is added to the one canonical contract (ADR-0007); the
   asset-class router gains a fixed-income branch beside FX/metal/equity/commodity/crypto.
2. Pricing lives in new pure leaves — **`celnet-bond`** (DCF/YTM/DV01/duration/convexity over
   the existing `celnet-rates::Curve`) and **`celnet-credit`** (spread curve / CR01) — that
   return the same `Priced` shape every other leaf returns.
3. **CR01 is a new dimension of the existing risk cube**, not a credit-only silo; credit
   positions roll up firm-wide alongside rates/FX (extends ADR-0010's projection).
4. Quote construction (inventory skew + client-tier spread) and the fill->inventory->hedge
   loop are **server services over the shipped `Book` / `celnet-limits` / entitlements** — no
   new authz model, no second risk store.
5. Every new RPC gets the standard gRPC + WS-mirror frame; outbound streaming/RFQ rides the
   existing `celnet-fix`.

## Consequences

**Positive**
- One contract, one hot path, one risk cube — consistent greeks and firm-wide roll-up across
  all asset classes (honours ADR-0007/0008/0010).
- Maximal reuse: curve, RFQ workflow, FIX, streaming, Book, limits, entitlements are unchanged.
- Adding a bond product is one `ProductEngine` registry entry, mirroring the cross-asset
  pattern already proven for equity/commodity/crypto.

**Negative / costs**
- The `Instrument`/`Priced` contract grows a fixed-income arm; the WS mirror, SDK, and
  GUI/Excel all gain bond/credit views (the standard ripple, paid once).
- The risk cube must accommodate a CR01 dimension without regressing the rates/FX projections.
- Bond *deal-capture* remains blocked on the venue order/exec-report model
  (`FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md`) — analytics proceed first; this ADR is about
  pricing structure, not capture.

**Neutral**
- Latency: bond/credit math is cheap (us per instrument) and rides the existing zero-alloc
  core; no new infra (explicitly NOT adopting JVM off-heap/Disruptor ceremony — see design §7).

## Alternatives considered

- **Parallel FI pricing service.** Rejected: duplicates the contract, the risk store, and the
  hot path; produces a second greeks dialect and breaks firm-wide roll-up. Contradicts
  ADR-0007/0008.
- **Bolt bond pricing onto `celnet-rates` directly.** Rejected as the *contract* home: rates
  is a numerics substrate (the curve), not the asset-class router. `celnet-bond` *consumes*
  `celnet-rates` but the routing/contract decision stays in the pricer (this ADR). The
  analytics crate split keeps the dependency one-way.

## Supporting verified claims

To be authored against the graph on promotion (not yet verified): (a) `price_instrument`
routes on asset class before product dispatch; (b) the risk-cube projection is asset-keyed and
extensible; (c) `celnet-rates::Curve` is a pure consumable substrate. Author via
`knowledge_put` with `kind: spec:satisfies` against the committed acceptance corpus when the
FI pricing milestone starts.
