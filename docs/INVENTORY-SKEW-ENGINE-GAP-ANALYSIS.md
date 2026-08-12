# Inventory-Driven Skew Engine — Gap Analysis / Design

> Status: **gap analysis + design for review** (2026-08-12). Reconciles an **externally-supplied**
> specification ("Inventory-Driven Skew Engine Design", ~2.9k chars, `python-docx`-generated, no
> reliable authorship or date in `docProps/core.xml`) against Celnet **as built on
> `feature/hedging-model`**. Every "already exists" claim is `file:line`-cited and was read.
>
> **This is a reconciliation, not a greenfield spec.** The overwhelming majority of the source
> spec is already implemented and live on both outbound paths. Writing a design doc for it would
> create a second, drifting source of truth for a shipped feature. Read alongside
> [`FI-TIERING-RESEARCH.md`](FI-TIERING-RESEARCH.md),
> [`FI-PRICING-GROUPS-DESIGN.md`](FI-PRICING-GROUPS-DESIGN.md),
> [`HEDGING-AND-RISK-EXIT.md`](HEDGING-AND-RISK-EXIT.md) and
> [`AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md`](AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md).

---

## 0. Verdict in one paragraph

The source spec describes five architectural components, two operational modes and two
guardrails. **Four of the five components, both operational modes and roughly half of one
guardrail are already built, wired and live.** `celnet-tiering`'s `FeaturePipeline` *is* the
"layer between core pricing and external distribution" the spec asks for, applied identically to
the streaming (ESP) and request (RFQ/RFS) paths; `InventorySkew` *is* the automated mode, reading
live per-instrument inventory through a real `InventorySource`; `PricingFeature::Axe` with
`AxeSide::{Buy,Sell}` *is* the manual axing mode; and `SpreadUnit::YieldBps` means the spec's
yield-space formulation is already expressible. **The genuine gaps are narrower but sharper than
the spec's framing suggests.** The most serious is that Celnet's skew cap is a *static configured
magnitude*, not the spec's *dynamic* "never exceed half the bid-ask spread" — so a mis-set cap
permits a through-mid quote, which is precisely the predatory-arbitrage exposure the spec's
guardrail exists to prevent. Beyond that: skew responds to raw inventory rather than limit
utilisation (the spec's Mode A), there is no per-issuer aggregation, no dedicated axe surface, and
no explicit post-fill fade. Underneath all of it sits an unresolved question the spec cannot know
about — whether this skew double-counts against Celnet's own auto-hedging spine (§5).

**Correction to the brief that commissioned this analysis:** the source `.docx` contains
materially more than "inventory-driven bid/offer skewing with a skew engine layered between core
pricing and distribution". It also specifies yield-space pricing formulae, an explicit sign
convention, a two-mode split (algorithmic vs manual axing), an anti-arbitrage cap, and an
auto-fade/refresh requirement. Those four extra elements are where all the real gaps are.

---

## 1. What already exists (cited, read not inferred)

### 1.1 The skew engine and its position in the stack — this *is* the spec's architecture

- **The layer.** `FeaturePipeline` (`crates/celnet-tiering/src/feature.rs`) composed of ordered
  `PricingFeature`s, run over a `TwoWay` with a `PricingCtx`.
- **Applied on the streaming path:** `AggregationHub::snapshot_priced`
  (`crates/celnet-server/src/services/aggregation.rs:1125-1143`) → `apply_pipeline`
  (`:1320-1350`) → `pipeline.run(...)` at **`aggregation.rs:1334`**.
- **Applied on the request path:** `AggregationHub::resolve_rfq_composite_priced`
  (`aggregation.rs:1157-1230`) → `pipeline.run(...)` at **`aggregation.rs:1193`**.
- Both build their context through the **same** `pricing_ctx_for_line` (`aggregation.rs:1293-1312`)
  and both close with `Guardrails::enforce`. Per-group ESP and RFQ pipelines may be shared or
  independent (`config/identity.rs:751,759`).
- **Honest note:** ungrouped clients bypass the pipeline entirely and receive the raw composite
  verbatim, by design (`aggregation.rs:1121-1123, 1153-1155`).

**This is exactly the spec's claimed architecture** — a pluggable layer between the consolidated
composite and outbound distribution, on both push and pull channels.

### 1.2 The inventory monitor

`InventorySource` trait (`aggregation.rs:85-88`) — `net_inventory(&self, instrument_id: &str) ->
f64`, resolved into the quote context at `pricing_ctx_for_line` (`aggregation.rs:1300`). The
tiering crate itself is I/O-free (`celnet-tiering/src/lib.rs:42-43`); `QuoteCtx.inventory`
(`context.rs:58-59`) is caller-supplied and the server supplies it from the live store. Backing
stores: `RatesPositionStore` (`services/rates_book.rs`) and `PositionStore`
(`services/risk/store.rs`).

### 1.3 Mode A — automated inventory skewing

`InventorySkew` (`crates/celnet-tiering/src/strategy.rs:91-142`). The math, read verbatim:

```
cap            = |s_max|
skew_magnitude = clamp(κ · q, −cap, +cap)      // q = ctx.inventory, signed
skew           = unit.to_price_offset(skew_magnitude, ctx)
```

composed in `pipeline.rs:203-204` as `bid = mid − h − s`, `offer = mid + h − s`.

**Sign convention (documented at `strategy.rs:84-89`):** long (`q > 0`, `κ > 0`) ⇒ `s > 0` ⇒
**both** sides shift down — cheapen the offer to sell, lower the bid. Short shifts both up. Note
`offer − bid = 2h` is skew-invariant by construction, so the quote can never cross *itself*.

**This is economically identical to the spec's yield-space convention** (spec: long ⇒ positive
yield skew ⇒ higher yield ⇒ cheaper price ⇒ attracts client buys). Celnet reaches the same place
in price space. Under `SpreadUnit::YieldBps` the conversion is via DV01
(`unit.rs:57,66`; `context.rs:172-185`, `dv01 = mod_duration · mid / 10000`), which preserves the
inversion. **Flagged: this end-to-end sign equivalence is asserted from reading the conversion,
not proven by a test. It deserves an explicit acceptance test (§4) rather than trust.**

### 1.4 Mode B — manual axing

`PricingFeature::Axe { side: AxeSide, magnitude, unit }` (`feature.rs:159-166`, applied
`:234-249`); `AxeSide::{Buy, Sell}` (`feature.rs:93-98`) — *"Buy = lean up, keener bid"*,
*"Sell = lean down, keener offer"*. **This is the spec's LHS/RHS axe**, named by trade direction
rather than book side. Magnitude is unsigned; the sign comes from the side.

Set via `update_pricing_group_pipeline` (`config/identity.rs:1864`), persisted on
`PricingGroupDef.esp_pipeline`/`.rfq_pipeline` (`identity.rs:696,700`), wire round-trip at
`services/auth.rs:3452,3507`. GUI: the generic drag-and-drop `gui/src/workspaces/PricingFeatureCard.tsx`.

### 1.5 Trader controls and protective caps

`Guardrails` (`crates/celnet-tiering/src/pipeline.rs:17-30`): `h_min`, `h_max`, `s_max`,
`spread_floor`. Enforcement in `finalize` (`pipeline.rs:189-210`), read verbatim:

```rust
let half_spread = half_spread.max(guards.spread_floor / 2.0);
// Anti-cross LAST: re-clamp skew after the floor so extreme inventory cannot
// push a side past the guardrail cap.
let skew = skew.clamp(-guards.s_max, guards.s_max);
let bid   = mid - half_spread - skew;
let offer = mid + half_spread - skew;
```

Skew is clamped **twice** (`pipeline.rs:184-185` and again at `:199-201` after the spread floor).

### 1.6 The full feature vocabulary, and the limits framework

`PricingFeature` (`feature.rs:129-191`): `MidShift` (recentre, spread-invariant), `Tiering`
(margin — runs the strategy engine), `Axe` (§1.4), `Position` (reuses `InventorySkew::adjust` over
live inventory, `:250-266`), `PanicSkew` (emergency overlay, no-op unless armed). Strategies:
`FlatMarkup` (`strategy.rs:47`), `InventorySkew` (`:91`), `ScaledSmoothedSpread`/SCALE_SMOOTH
(`:193`).

`celnet-limits` already has the utilisation machinery the spec's Mode A implies:
`LimitSpec::utilization()` = `|exposure| / cap` (`crates/celnet-limits/src/limit.rs:200-209`) and
`classify()` → `RagStatus::{Green,Amber,Red,Breach}` (`:213-232`), with amber/red bands defaulting
to 80%/90% (`:159-180`) — the same architecture as the spec's 75% example, different threshold.
`LimitMetric` includes `Dv01`, `Pvbp`, `RateTenorBucket` (`limit.rs:39-90`).

---

## 2. The genuine gap — five items

| # | Gap | Spec origin | Evidence |
|---|---|---|---|
| **S1** | **The anti-arbitrage cap is not the spec's cap.** Celnet clamps `\|s\| ≤ s_max` where `s_max` is a **static configured magnitude** independent of the live half-spread. The spec requires `\|s\| ≤ h` (half the bid-ask). If `s_max > h`, then `offer = mid + h − s < mid` — the offer sits **through mid**, which is exactly the predatory-arbitrage exposure the guardrail exists to prevent. The quote never crosses *itself* (`offer − bid = 2h` is invariant), so this is not a crossed-market bug — it is a through-mid bug, and it is silent. | Guardrail 1 | `pipeline.rs:17-30`, `:189-210` |
| **S2** | **Skew responds to raw inventory, not limit utilisation.** `InventorySkew` is linear-and-clamped in raw `q` (`strategy.rs:130`). The spec's Mode A ties skew to **% of max limit** ("+75% of max limit"). `celnet-limits` has `utilization()` and RAG bands; **nothing wires them into the tiering skew.** | Mode A | `strategy.rs:91-142` vs `limits/src/limit.rs:200-232` |
| **S3** | **No per-issuer aggregation.** `net_inventory` is keyed per-`instrument_id` only (`aggregation.rs:85-88`); `LimitScope` (`crates/celnet-limits/src/tree.rs:58-73`) is `Firm\|Trader\|Book\|Desk\|CcyPair\|Location\|Entity` — **no `Issuer`**. The spec explicitly frames the problem as being long "bonds of a specific issuer". A repo-wide search for issuer-level rollup matched only a requirements doc, not code. | §1 of spec | `aggregation.rs:85-88`, `tree.rs:58-73` |
| **S4** | **No explicit post-fill fade.** No `on_fill`/fade/decay/requote mechanism exists. Quotes refresh on a fixed 5 ms interval (`TICK_INTERVAL`, `crates/celnet-server/src/services/stream.rs:121`, driven by `run_session` `:1062-1120`), which *does* pick up the new inventory and re-run the pipeline (`aggregation.rs:1334`). | Guardrail 2 | See §2.1 — this gap is **much narrower than it looks** |
| **S5** | **No dedicated axe surface, and axe is per-group not per-instrument.** Axe is one card in the generic pricing-group pipeline builder (`PricingFeatureCard.tsx`); one `Axe` entry applies uniformly to every instrument the pipeline prices. No per-instrument axe map exists in the crate or the server. | Mode B | §1.4 |

### 2.1 Calibrating S4 honestly

The spec's auto-fade guardrail assumes a platform whose quotes do not otherwise refresh promptly.
Celnet re-prices every stream every **5 ms** off live inventory, so after a fill the skew *does*
move within one tick without any fade mechanism. The genuine residual is therefore only:

1. a bounded **≤5 ms window** in which multiple counterparties can lift before the position update
   is reflected, and
2. the absence of a **fade/decay term distinct from the inventory response** — the spec wants the
   price pulled back toward true mid after a fill, which is a different (and faster-decaying)
   behaviour than "skew proportional to the new inventory".

Whether (2) is worth building is a market-making policy question, not an obvious defect. **(1) is
the real exposure and it is a latency property, not a missing feature.** This should not be
scheduled as "build auto-fade" without first measuring whether the 5 ms window is actually being
picked off — that measurement is cheap and the existing latency analytics can carry it.

---

## 3. Guardrail conflicts in the source spec — resolved, not copied

| Source-spec element | Celnet guardrail | Resolution |
|---|---|---|
| Names **Bloomberg, Tradeweb, MarketAxess** as the distribution channels | **7** (no commercial runtime deps), **8** (no vendor names in identifiers) | Celnet's distribution is the existing `celnet-fix` stack (ESP/RFS/RFQ) plus `celnet-lp-sim` for exercise, and the composite aggregated book. A commercial venue is a customer-wired FIX adapter behind the existing session model. These names may appear in integration prose only; no crate, module, type or fn may carry them. |
| Component name "**Skew Engine**" as a distinct service | **8**, **10** (no duplicate/parallel structures) | Celnet's equivalent already exists and is purpose-named: `celnet-tiering`'s `FeaturePipeline`. **Do not create a second engine.** Extending the existing pipeline is mandatory, not preferred — a parallel skew engine would be exactly the drift guardrail 10 forbids. |
| Bare "Inventory Monitor" component | **10** | Already `InventorySource` over the live position stores. No new monitor. |
| Skew mandated in yield space | — (no conflict) | `SpreadUnit::YieldBps` already supports it; Celnet additionally supports `PriceBps`/`PricePoints`/`Percent`. Celnet is a superset; nothing to change. |

**Checked and NOT found in the source spec:** no versioned API is proposed (guardrail 9 not
engaged); no mock or placeholder is mandated (guardrail 2 not engaged). Unlike the sibling
corporate-actions spec, this document names no data vendor — only execution venues.

---

## 4. The Celnet design — five targeted changes, no new engine

**S1 — make the cap dynamic (the one genuinely urgent change).**
Add a guardrail field expressing the cap as a fraction of the *live* half-spread, and apply it in
`finalize` **after** the spread floor, where `h` is final:

- effective cap = `min(s_max, λ · h)` with `λ ≤ 1`, default `λ = 1.0` (the spec's literal rule).
- The insertion point already exists: `pipeline.rs:199-201` re-clamps skew after the floor
  precisely so extreme inventory cannot push past the cap. This change makes that clamp
  spread-aware instead of spread-blind.
- `λ` configurable per pricing group so a desk can be *stricter* than the spec (λ < 1), never
  looser. Existing static `s_max` stays as the absolute ceiling.

This is a pure tightening: any configuration where `s_max ≤ h_min` already satisfies it, so those
groups are bit-identical after the change. Groups that were relying on a through-mid skew will
change behaviour — which is the point, and why it needs an ADR rather than a silent fix.

**S2 + S5 (part) — skew on utilisation, sourced from the limits tree.**
Replace the raw-`q` input with utilisation `q / cap` sourced from `LimitSpec::utilization()`
(`limits/src/limit.rs:200-209`), keeping the existing clamped-linear shape. This makes `κ`
dimensionless and portable across instruments of wildly different size — today a `κ` tuned for a
$50 m line is meaningless on a $500 m line, which is a real configuration hazard. It also makes
the spec's "+75% of max limit" directly expressible.

**S3 — issuer scope.** Add `Issuer` to `LimitScope` (`tree.rs:58-73`) and an issuer-keyed rollup
alongside `net_inventory`. The issuer identity already exists on the golden record as an LEI
(`crates/celnet-refstore/src/master.rs:47-71`), so this is a rollup key, not new reference data.
**Sequenced after S2** — issuer-level skew is only meaningful once skew is utilisation-based,
because an issuer-level *raw* inventory number has no natural scale.

**S4 — measure before building** (§2.1). If measurement justifies it, the fade is a decay term on
the skew with its own time constant, applied in the same pipeline — not a new mechanism.

**S5 — per-instrument axe.** The substantive part is the data model (an instrument-keyed axe map
on the pricing group), not the UI. A dedicated axe board is a GUI concern to design separately and
is deliberately not specified here.

### 4.1 Acceptance test owed regardless of scope

§1.3 flags that the yield-bps sign equivalence is read, not proven. A test asserting that a **long**
position under `SpreadUnit::YieldBps` produces a **cheaper offer price** (and the converse for
short) closes an assumption currently held by inspection alone. Cheap, and it protects the whole
Mode-A story.

---

## 5. The double-count question — the most important open item

The brief asked whether inventory skew double-counts against the existing hedging/internalisation
spine. **It is a real, unresolved hazard.** The facts:

- `celnet-hedge-routing`'s module doc (`crates/celnet-hedge-routing/src/lib.rs:62-72`) states it
  reuses `celnet-limits`' RAG bands with **amber = "skew to attract offset"**, explicitly
  cross-referencing `celnet-tiering`'s `InventorySkew`, and red = hedge the overflow.
- `AutoHedgeEngine::evaluate` (`crates/celnet-server/src/services/auto_hedge/engine.rs:219-408`)
  is heavily wired (19 inbound callers) and has a `SKEW` exit action among its seven.
- `AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md:239-263` is explicit that `InventorySkew` **is**
  the passive-internalisation lever the policy should try before paying to hedge externally.
- **But no code wires the band classification into the tiering skew.** `PricingFeature::Position` /
  `InventorySkew` are configured statically per pricing group (fixed `κ`, `s_max`) and apply
  continuously regardless of band state.

So today two systems read the same live inventory signal **independently**: the pricing pipeline
skews continuously, while the hedge engine separately decides warehouse/skew/hedge off its own
bands. A book can be leaning its price via `InventorySkew` *and* have a `SKEW` action fired for the
same inventory, with no coordination beyond shared intent in prose.

**The design resolution — and the reason S2 is the right next build.** Making the skew
utilisation-driven puts it on the *same measure* the hedge policy bands. Once both read
`LimitSpec::utilization()`, the natural architecture is that the hedge policy's band is the
**authority** and the tiering skew is the **implementation** of its `SKEW` action — one signal, one
decision, two cooperating layers. That is composing with the spine rather than paralleling it, and
it closes S2 and the double-count hazard in a single move rather than adding a second uncoordinated
skew.

**ADR required.** Whether the hedge policy owns the skew decision (and tiering executes it) or the
two remain independent is an architectural decision with live pricing consequences. It must be
recorded, not settled by whichever lane ships first.

### 5.1 A second, unrelated seam — do not confuse them

`AutoSkewSource` in `crates/celnet-aggregation/src/risk.rs:32-63` is a **different** seam (FX cash,
not FI) and is an honest, declared stub: the only implementation is `NoAutoSkew`, returning `0.0`
(`risk.rs:56-63`), with a doc comment stating a later lane implements it over the rates position
store (`risk.rs:37-38`). It is **not** the FI skew engine and is not live. Cited because the two
are easy to mistake for each other when searching.

---

## 6. Phased future work

Ordered by value-per-unit-risk. Each is independently shippable and gate-able (`just t1` on
`celnet-tiering` / `celnet-limits`; `just t2` at land).

**SK-P1 — Dynamic anti-arbitrage cap (closes S1).**
*Acceptance:* for any configuration, `|s| ≤ λ·h` holds at the final quote, verified by a property
test over randomised (mid, h, s, guardrail) inputs — no input produces `bid > mid` or `offer < mid`
when `λ = 1`. Groups with `s_max ≤ h_min` are bit-identical to pre-change output.
*ADR:* required — this changes live outbound prices for any group currently configured with
`s_max > h`.

**SK-P2 — Utilisation-driven skew (closes S2; the double-count resolution, §5).**
*Acceptance:* skew is a function of `q/cap` and is invariant to a proportional rescaling of both
position and cap (a $50 m position on a $50 m cap skews identically to $500 m on $500 m) — the
property that makes `κ` portable and that raw-`q` skew fails. Hedge-policy band and tiering skew
demonstrably read one signal. Existing groups migrate with documented equivalent `κ`.
*ADR:* required — §5, band authority.

**SK-P3 — Yield-space sign acceptance test (§4.1).** Small, independent, do it alongside P1 or P2.
*Acceptance:* long ⇒ cheaper offer under `YieldBps`; short ⇒ dearer; both directions asserted.

**SK-P4 — Per-instrument axe (closes S5, data model).**
*Acceptance:* an axe on instrument A does not move instrument B's quote in the same pricing group;
group-level axe still applies where no instrument-level axe is set. GUI surface out of scope.

**SK-P5 — Issuer-scoped inventory (closes S3).** Sequence after P2.
*Acceptance:* issuer-level net inventory equals the independent sum over that issuer's instruments;
`Issuer` scope enforces in the limits tree like any other scope.

**SK-P6 — Post-fill fade (closes S4) — MEASURE FIRST.**
*Acceptance for the measurement:* evidence from live latency analytics on whether the ≤5 ms window
is being picked off. Build only if it is. If built, the fade is a decay term in the existing
pipeline with its own time constant, and acceptance is that the skew converges to the
inventory-implied level with the configured constant.

---

## 7. Honest boundaries — what this design deliberately does not do

- **It does not build a Skew Engine.** Celnet has one. Every phase above extends
  `celnet-tiering`'s existing pipeline. Any proposal that creates a parallel skew component should
  be rejected on guardrail 10 grounds.
- **It does not claim the double-count hazard is resolved.** §5 proposes a resolution and flags an
  ADR. Until that ADR is recorded and SK-P2 lands, a desk running both `PricingFeature::Position`
  and an auto-hedge policy with a `SKEW` action **is** running two uncoordinated responses to one
  inventory signal. That is a live configuration hazard today, worth communicating to whoever
  configures those policies, independently of whether SK-P2 is scheduled.
- **It does not address ungrouped clients.** They bypass the pipeline and receive the raw composite
  (`aggregation.rs:1121-1123`). No skew, no axe, no guardrails — by design, but it means the
  spec's protections apply only to grouped flow. Whether that is the right default is a separate
  question this doc does not open.
- **It does not size `λ`, `κ` or the bands.** Defaults proposed (λ = 1.0, existing 80/90 RAG) are
  starting points from the spec and the existing code, not calibrated values. Calibration needs
  real flow data.
- **The 5 ms figure is a configured constant, not a measured end-to-end latency.**
  `TICK_INTERVAL = 5ms` (`stream.rs:121`) is the loop period; actual time from fill to changed
  outbound quote includes position-store update and pipeline evaluation and was **not measured**
  here. SK-P6's measurement must establish the real number before any fade is justified.
- **`validate_feature_pipeline` was not fully read** (`config/identity.rs:2912-2963`) — whether it
  rejects a pipeline configuring inventory skew twice (once via `Tiering{InventorySkew}`, once via
  standalone `Position`) is **unverified**. If it does not, that is a third double-count path,
  within a single pipeline. Worth checking before SK-P2.
