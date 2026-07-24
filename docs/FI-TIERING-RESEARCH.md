# FI Outbound Price Tiering — Research & Implementable Strategy Design

> Status: **research + design proposal** (2026-07-24). No code yet — this doc is the
> deep methodology behind `FI-PRICING-ENGINE-DESIGN.md` §6.3 (quote construction), and the
> basis for a `celnet-tiering` strategy seam on the aggregation composite. Cited from a
> deep-research pass (22 sources, 25 claims adversarially verified, 23 confirmed / 2 refuted).

## 1. Problem

We consolidate multiple LP feeds into a composite best bid/offer (`celnet-aggregation`).
Today we publish that composite **as-is**. We want to construct the price we stream/quote
to **our** clients by **widening around mid** and/or **skewing**, e.g.:

```
LP composite:  bid 99.50 / offer 99.60   (mid 99.55, market spread 0.10)
flat ±25bp:    bid 99.30 / offer 99.80   (mid 99.55, our spread 0.50)
```

The engine must support several **methodologies** (flat, inventory-skew, and more dynamic
strategies), be **per-counterparty tierable**, and be **asset-agnostic** (FI-first, FX later).

## 2. What the research says (the framework)

The dominant academic-and-practitioner framework is the **Avellaneda–Stoikov / Guéant–Lehalle
inventory-control lineage**. The outbound two-way is:

```
bid  = mid − h − s          (h = half-spread ≥ 0,  s = skew, signed)
offer= mid + h − s
```

Two **functionally distinct** knobs (verified, arXiv:1810.04383, 3‑0):

- **Half-spread `h`** balances *fill frequency vs profit per round-trip*. To first order it is
  **constant** in inventory; it grows with **volatility** and **trade size**. Closed form
  (Model B): `h = 1/k + ½·√(γ·z)·σ`, where `k` = liquidity/price-sensitivity, `γ` = risk
  aversion, `z` = trade size, `σ` = vol (covariance term `eᵀΓe`).
- **Skew `s`** manages **inventory risk** and is (to first order) **linear in inventory**:
  `s ≈ √γ · Γ · q` (q = signed position). A dealer **long** inventory skews the whole two-way
  **down** — cheaper offer (lift the sale), less generous bid — to shed risk (verified
  arXiv:1810.04383 / 1907.01225, 3‑0). In our sign convention `bid=mid−h−s, offer=mid+h−s`,
  so `s>0` when long.

Superimposed spread drivers, each verified:

- **Adverse selection / flow toxicity** (Glosten–Milgrom; Glosten–Harris 1988; Easley–O'Hara).
  Widen against informed flow: spread `∝ PIN·(value uncertainty)`, permanent, **linear in trade
  size**, and **economically significant only for large trades** (Glosten–Harris: round-trip
  ≈ $0.075 @1k shares vs $0.31 @10k). Toxicity is measurable in real time as **VPIN** (abs
  buy/sell volume imbalance over ~50 volume buckets). **⚠ Caveat:** VPIN's *predictive* power
  was **refuted (0‑3)** — treat it as a *computable signal to calibrate against realized
  markouts*, not a forward-looking truth.
- **Volatility scaling** (Stoll 1978; Ho–Stoll): half-spread **∝ σ**. `h = h_base·(σ_now/σ_ref)`.
  For bonds use **bond return volatility** as the inventory-risk proxy (Feldhütter, 3‑0).
- **Size tiering** (Bergault–Guéant, "Size matters for OTC MMs", 3‑0): OTC dealers answer
  different prices to different sizes → a **spread ladder by size band** + size-dependent skew.
- **Client tiering** (Barzykin/HSBC–Bergault–Guéant, arXiv:2112.02269, 3‑0): tiers are built by
  **k-means on each client's fitted logistic fill-intensity `(α,β)`**. Each tier is streamed its
  own **multiplicative ladder** `bid=S(1−δ), offer=S(1+δ)` with `δ` tier- & size-specific; the
  hit probability is `Λ(δ)=λ/(1+e^{α+βδ})` (higher `β` = more price-sensitive client). HSBC
  EURUSD → two tiers (α=−0.3,β=5 vs α=−1.9,β=15 bps⁻¹). **⚠ These magnitudes are FX; refit for
  govvies.**
- **Internalization band → the natural guardrail** (Barzykin–Bergault–Guéant, 2112/2106, 3‑0):
  a **"pure flow internalization area"** — an inventory band around zero where the dealer *only
  skews* (no hedge); outside it, it hedges. Skew is ~linear inside, **clamped at a max-skew**
  tied to that band. This is the model for our skew clamp.

**Where practice diverges from the optimum** (report caveats): real desks use **simple
linear/piecewise skew heuristics with hand-set caps** rather than solving the HJB; prefer
**markout-based calibration** over PIN/VPIN; and for corporate bonds inventory is **not** the
single dominant spread driver (refuted 1‑2) — search/bargaining & asymmetric info matter too.

## 3. Bond bps convention (must get right)

"n basis points" is ambiguous for bonds — two distinct quantities linked by duration:

- **Price bps** — 1bp = 0.01 **price points** per 100 face. Flat "±25bp" in the example means
  **±0.25 in price** (→ 99.30/99.80). Simple, but a fixed price offset is a *different* yield
  spread at every maturity.
- **Yield bps** — 1bp = 0.01% of **yield**. Convert to a price offset via modified duration /
  DV01: **Δprice ≈ −ModDur · Δyield · price**, i.e. `price_offset = DV01 · yield_bps`
  (DV01 = ModDur·Price/10000). A yield-bps spread is **duration-consistent** across the curve —
  the right default for a government-bond curve (empirically govvie spreads widen >5× from short
  to long maturity; MarketAxess EGB). US Treasuries also quote in **32nds** — respect tick.

**Design decision:** the tiering config carries a `SpreadUnit` = `PriceBps | YieldBps |
PricePoints | Percent`; the engine converts to a price offset using the bond's DV01/ModDur
(derivable from the `celnet_bond` leaf we already price off). Flat baseline defaults to price
bps to match the worked example; curve-wide books should prefer yield bps.

## 4. Composition & guardrails (order of operations)

Production quote pipeline (per instrument, per counterparty):

```
1. mid   = composite mid (or micro-price)              ← celnet-aggregation
2. h     = h_base(client_tier, instrument)             ← base per-tier margin
           · vol_factor(σ)                             ← volatility scaling
           + size_addon(z)                             ← size ladder + adverse-selection
           + toxicity_addon(VPIN/markout)              ← flow toxicity (optional)
3. s     = skew(inventory q)                           ← inventory strategy, linear+clamped
4. clamp: h ∈ [h_min, h_max];  |s| ≤ s_max(tier)       ← floors/caps
5. two-way: bid=mid−h−s, offer=mid+h−s
6. anti-cross: enforce offer − bid ≥ spread_floor AND never bid ≥ offer
               (clamp skew LAST so extreme inventory can't invert the book)
7. staleness: if any input stale / quorum lost → widen to h_max or suppress (no quote)
```

Guardrails are engineering invariants (no research finding needed, but essential):
**min/max half-spread, max skew, anti-cross (bid < offer always), stale-input suppression.**
Clamp **skew after** the min-spread floor so extreme inventory can never lock/cross the book.

## 5. Implementable strategy shortlist (pluggable behind one interface)

One trait, N strategies, composed. Ship the first two, then layer the rest.

| # | Strategy | Formula (half-spread `h`, skew `s`) | Params | When | Risks |
|---|----------|--------------------------------------|--------|------|-------|
| 1 | **Flat markup** *(baseline)* | `h = H` (const), `s = 0` | `H` (unit: price/yield bps/%) | Always-on default; simplest | Ignores inventory & vol; leaks to informed/large flow |
| 2 | **Inventory skew** | `s = clamp(κ·q, ±s_max)`, `h = H` | `κ` (bps per unit inventory), `q` position, `s_max` | Hold risk, mean-revert inventory | Mis-set `κ` → over-skew & self-adverse fills; needs live position feed |
| 3 | **Volatility scale** | `h = H·(σ_now/σ_ref)` | `σ_ref`, EWMA window, `mult_max` | Widen in stressed/illiquid regimes | Vol-estimate lag; whipsaw — cap the multiplier |
| 4 | **Size ladder** | `h(z)=H + Σ band_addon(z)`; `s(z)` grows | size bands + per-band addon | RFQ / size-varying flow | Cliff effects at band edges — smooth or fine bands |
| 5 | **Toxicity widen** | `h += g·VPIN` (or markout-calibrated) | gain `g`, VPIN window/threshold (~0.7) | Protect vs informed flow | VPIN not predictive (refuted) — calibrate to markouts, add-on only |
| 6 | **Per-client tier base** *(dim. B)* | `H = H_tier(n)`, `s_max=s_max(n)` | tier map, per-tier `(H,δ,s_max)` from k-means `(α,β)` | Segment clients by flow quality/relationship | Tier drift; needs periodic re-fit of hit-rate curves |

Composition: `#6` sets base `H`/caps per counterparty; `#3–#5` adjust `h`; `#2` sets `s`;
guardrails (§4) clamp. Each strategy is a pure function `(composite, ctx) → (h, s)` summed by
the pipeline — so "flat + inventory + vol" is just enabling three strategies on a book.

## 6. celnet integration design (FI-first, asset-agnostic seam)

- **New crate `celnet-tiering`** (asset-agnostic, no server dep): a `TieringStrategy` trait
  `fn adjust(&self, mid: Mid, ctx: &QuoteCtx) -> SpreadSkew` + the six strategies + the
  composition/guardrail pipeline + `SpreadUnit`↔price conversion (via DV01 in `QuoteCtx` for
  bonds). Pure, unit-tested, oracle-checked (flat ±25bp → 99.30/99.80 exactly; anti-cross
  invariant property-tested).
- **Hook point**: the composite publish path in `crates/celnet-server/src/services/aggregation.rs`
  (where `bestBid`/`bestOffer` + identity are assembled — the seam the FI Aggregated Book
  streams, `build_identities`/composite). Apply `TieringStrategy::adjust` to the raw composite
  **before** outbound publish; the *raw* LP composite stays available for internal/mid views,
  the *tiered* two-way is what clients receive. (Confirm exact struct at implementation via
  lodestar `trace_path` from the composite to the WS publish.)
- **Config model**: extend the persisted `AggregatedBookDef` with a `tiering` block (enabled
  strategies + params) and a **counterparty→tier** map (Tier‑1/2/3, each with base `H`/`δ`/
  `s_max`). Persist on the existing reference-data/identity JSON chassis, validated at load +
  admin write (mirror `reference_data.rs::validate_*`). No proto churn for the composite READ;
  the per-client tiered stream keys off the authenticated session's counterparty/desk.
- **Inventory feed** (for #2): source position `q` from the existing rates/FI position store
  (the `Book`) per instrument; strategy reads it via `QuoteCtx`.
- **Admin GUI**: a Tiering tab (per-book strategy toggles + params, tier table) alongside the
  existing Aggregation admin — and surface the applied `h`/`s` on the Agg Book tile (debug view).
- **Phasing**: (1) `celnet-tiering` crate with **Flat + InventorySkew** + guardrails + tests;
  (2) wire into composite publish + book config + admin GUI; (3) add Vol/Size/Toxicity;
  (4) per-client tiers + k-means `(α,β)` fitting from captured hit-rate data.

## 7. Caveats & open questions (from the research)

- Theoretical skew/spread forms are **closed-form approximations**; desks use clamped linear
  heuristics — we adopt the heuristic form, not the HJB.
- **VPIN predictive power refuted** — toxicity strategy must be markout-calibrated, off by default.
- **Inventory ≠ sole bond-spread driver** (refuted) — don't over-weight skew for FI.
- Tier `(α,β)` magnitudes are **FX**; **re-fit for govvies** before trusting tier presets.
- **Under-covered / gaps** (no surviving cited formula): time-of-day/session & event
  (auction/data-release) widening; last-look/hold-window vs spread trade-off; exact
  clamp-precedence standard. Treat as engineering choices (§4) + a follow-up research pass if needed.

## 8. Sources (verified primary unless noted)

- Avellaneda–Stoikov / Guéant–Lehalle–Fernandez-Tapia — inventory risk & optimal quoting: arXiv:1105.3115
- Bergault–Evangelista–Guéant–Vieira — closed-form spread/skew (multi-asset): arXiv:1810.04383
- Bergault–Guéant — "Size matters for OTC market makers": arXiv:1907.01225
- Barzykin–Bergault–Guéant — FX dealer tiers, pricing ladders, internalization: arXiv:2112.02269, 2106.06974
- Easley–López de Prado–O'Hara — flow toxicity / VPIN: NYU Stern con_035928
- Glosten–Harris 1988 (JFE) — adverse-selection vs transitory spread decomposition
- Feldhütter — bond bid-ask spread, vol proxy for inventory risk: feldhutter.com/BidAskSpread.pdf
- MarketAxess — EGB bid-ask vs maturity; CME — DV01/PVBP; SIE — Treasury 32nds quoting (bond bps convention)
- Refuted: VPIN predicts volatility (0‑3, EFMA 2019); inventory is the single dominant bond-spread driver (1‑2, Feldhütter)
