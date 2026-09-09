# ADR-0020: One central cross-asset pricing/risk contract (`Priceable` / `MarketResolver` / `RiskMeasure`)

- **Status:** Accepted & Implemented (Phase A1, A2, and Phase B landed on main).
  This ADR records the authoritative decision and central contract specification.
- **Relates to:** ADR-0007 (one unversioned contract), ADR-0008 (multi-asset carry / asset-class
  router + `ProductEngine` registry), ADR-0010 (converge FI rates onto the shared `DiscountCurve`
  seam), ADR-0016 (hot-core curve-handle embargo).
- **Supersedes:** the prior two-paradigm split — a separate FI/rates pricing+risk path distinct
  from the options pricing+risk path.
- **Context doc:** `docs/ARCHITECTURE.md`.
- **Home of the contract:** `crates/celnet-core/src/contract.rs` (the traits) +
  `crates/celnet-server/src/pricer/contract.rs` (the option leaves + resolvers).

## Context

Celnet must price and risk **every asset class** — FX options first, then equity / commodity /
crypto (linear + inverse) / perpetual / listed-future, and fixed income — on one hot path, one
risk cube, and one wire contract (ADR-0007/0008). The open structural question was whether
options and FI keep **two paradigms** (a nonlinear option pricer + Greeks, and a separate linear
rates PV + ladder), each with its own pricing entry, market plumbing, and risk type.

Two paradigms fork the platform: two "priced" shapes, two risk dialects, two market-resolution
paths, and a hot core that would have to understand both. That contradicts ADR-0007 (one
contract) and breaks firm-wide risk roll-up.

The market plumbing is the second axis. Curve/surface resolution (which `DiscountCurve` to
discount on, the foreign leg, the resolved spot/vol, the trade conventions) is a **request-tier**
concern that must NOT leak into the zero-alloc hot core — ADR-0016 embargoes any curve handle in
`MarketState`.

## Decision

**There is ONE central pricing/risk contract that every asset class conforms to.**

1. **`Priceable`** (`celnet-core::contract::Priceable`) is the single leaf seam: `price` returns
   present value + the priced result; `risk` returns the unified tagged **`RiskMeasure`**. A leaf
   declares its own associated `Market`/`Ctx`/`Priced`/`Error` types — celnet-core names none of
   the leaves, staying acyclic.
2. **`RiskMeasure`** is an **additive tag set**, not a fork: `OptionGreeks(CarryGreeks)` for the
   nonlinear options/cross-asset Greek strip, `RateLadder(RateLadder)` for the linear FI PV +
   parallel ladder. A new asset class adds a variant; it never forks the risk type. The VaR/FRTB
   layer re-derives Greeks under scenarios; the linear layer sums bit-exactly.
3. **`MarketResolver` + `ResolvedMarket`** are **request-tier only**. `MarketResolver::resolve`
   turns a per-request input into a **borrowed** `ResolvedMarket` — the discount (numeraire)
   `DiscountCurve` handle, an optional foreign-leg curve, resolved scalar spot/vol, and the
   borrowed `ConventionSet` — behind the one shared `DiscountCurve` seam (ADR-0010). Resolution is
   paid once per request/batch.
4. **The hot core stays flat.** The zero-alloc hot pricing core keeps a flat `MarketState`
   (`spot, r_dom, r_for, t, conventions, smile`) and holds **no** curve handle, `ResolvedMarket`,
   or `Arc<Curve>` — the ADR-0016 embargo, pinned by a `size_of` + `'static` guard in
   `celnet-engine/src/rt.rs`. `MarketResolver` is not a hot-core consumer.
5. **Options-first, then FI.** Phase A1 conforms the FX vanilla leaf (`VanillaEngine`,
   `PluginModelEngine`) via `FxSurfaceResolver`; Phase A2 conforms the cross-asset leaves
   (`EquityVanillaEngine`, `CommodityVanillaEngine`, `CryptoLinearEngine`, `CryptoInverseEngine`,
   `CrossAssetPerpetualEngine`, `CrossAssetListedFutureEngine`) via `CrossAssetCarryResolver`.
   Fixed income conforms in **Phase B** (ADR-0018), reporting `RateLadder`.
6. **The re-seat is byte-identical.** `price_vanilla_via_contract` and
   `price_cross_asset_via_contract` produce a `Priced` `to_bits`-equal to the direct
   `price_instrument` path; the pre-dispatch guard/refusal cascade is preserved verbatim. The
   contract is a structural re-seat, not a numeric change.

## Consequences

**Positive**
- One contract, one hot path, one risk type across all asset classes — consistent Greeks and
  firm-wide roll-up (honours ADR-0007/0008/0010).
- Request-tier resolution keeps the hot core flat and alloc-free (ADR-0016); curve cost is paid
  once per request, never per tick.
- Adding an asset class is a `Priceable` leaf + a `RiskMeasure` variant + a resolver — mirroring
  the cross-asset pattern already proven in A2.

**Negative / costs**
- The `RiskMeasure` enum and the resolver seam grow as asset classes land; the WS mirror / SDK /
  GUI / Excel gain the new views (the standard ripple, paid once).
- FI must land its `RateLadder` leaf under this contract (Phase B) rather than a bespoke path.

**Neutral**
- Latency: the borrowed-handle `ResolvedMarket` is a per-request lending pattern; the hot loop
  never dereferences a curve.

## Open questions

- **R-a / R-b risk-cube fork (OPEN).** Whether `risk` aggregates at the leaf (R-a: each leaf
  emits its full `RiskMeasure`) or delegates to a dedicated risk-cube consumer that recomputes
  factor decomposition (R-b: leaves emit raw sensitivities, the cube buckets them) is not yet
  decided. ADR-0008 §Sensitivities currently places FRTB bucketing at the risk layer; the central
  contract is compatible with both and does not pre-empt the choice.

## Alternatives considered

- **Two paradigms (options pricer + separate FI pricer).** Rejected: duplicates the contract, the
  risk type, the market plumbing, and the hot path; produces two Greeks dialects and breaks
  firm-wide roll-up. Contradicts ADR-0007/0008. This ADR supersedes it.
- **Push resolution into the hot core** (hold a `ResolvedMarket`/curve handle in `MarketState`).
  Rejected by ADR-0016: it re-introduces allocation and borrowed state into the zero-alloc core.
  Resolution stays request-tier behind `MarketResolver`.

## Supporting verified claims (lodestar knowledge layer)

Authored against the graph on the landing commit (`ef50fa8`); anchors resolved:

- `cl_6198daf557bb6f88` — **invariant:pure, active** — the A1/A2 re-seat is byte-identical
  (`price_vanilla_via_contract` + `price_cross_asset_via_contract` `to_bits`-equal to
  `price_instrument`; guard cascade verbatim).
- `cl_9e024d524a86b12f` — **invariant:pure, active** — hot-core curve-handle embargo (ADR-0016),
  anchored to `celnet-engine::rt::hot_core_embargoes_request_tier_curve_handles`.
- `cl_1beb3bbef8133e37` — **decision, draft** — the one central contract (`Priceable` +
  `RiskMeasure` additive tags), coordinator-attested pending soarsa/lodestar#52.
- `cl_ebaf7665d9c1f63a` — **decision, draft** — market resolution is request-tier only
  (`MarketResolver` + `ResolvedMarket`), coordinator-attested pending soarsa/lodestar#52.

> Governance note: `manage_adr` cannot yet attach a `Decision -GOVERNS-> Deliverable` edge for
> this ADR — no committed acceptance target maps to the central contract, and the existing targets
> produce zero `Deliverable` graph nodes (soarsa/lodestar#52 / #19). Until those land, ADR-0020 is
> **coordinator-attested** in `docs/adr/ATTESTATION.md`.
