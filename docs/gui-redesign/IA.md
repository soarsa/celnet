# Celnet Experience — Unified Information Architecture (v2, sell-side)

Supersedes v1 (which was trading-framed). Grounded in `CAPABILITY-MAP.md` (40 crates) +
`FRONTEND-INVENTORY.md` (current coverage/gaps) + research (`RESEARCH-sota-ia`,
`RESEARCH-synoption-spectraxe`, `RESEARCH-visual-fixedincome`). This is the single architecture every
mockup implements. Governed by `VISION.md` §0.5–0.7.

## 0. Positioning
A **sell-side market-maker pricing, distribution & administration platform** replacing Fenics /
Synoption / ION-class terminals. **Own the intersection nobody ships:** ION *function* (one screen,
own-quotes-beside-external, client analytics, connectivity) + Tradeweb *clarity* + Bloomberg *density*
— on a web-native streaming core, **progressive density**, **command-palette-first**. Users **price,
model, mark, contribute, administer, and manage feeds** — they do **not** execute (no order engine
exists; code-verified).

## 1. The spine (one grammar, zero silos)
Everything is **scoped by Desk/Book** and **gated by entitlement + per-class license**. Navigation =
**command palette (primary)** + an **entity(underlier) × task(verb)** grammar joined by a **scope/
context bus**, with **saved perspectives** (not 16 hard-mounted workspaces).

```
 SCOPE (Desk ▸ Book ▸ Underlier — one selector, propagates everywhere; license-aware)
   │
   ├─ PRICE & MODEL      price any instrument/structure; choose+calibrate model (hero)
   ├─ STRUCTURE          registry-driven, class-scoped (exotics only where licensed+priceable)
   ├─ VOL SURFACE        mark ▸ calibrate ▸ arb-gate ▸ version ▸ PUBLISH   (keep — the template)
   ├─ CONTRIBUTE         streams/quotes → client tiers; spread/skew rules; publish on/off; "why this price"
   ├─ RFQ DESK           inbound maker response + AUTO-QUOTE (manual+auto in one list) │ LP-hedge panel
   ├─ FEED MANAGE        vendor sources ▸ symbology ▸ composite/blend/divergence ▸ freshness health
   ├─ RISK & SCENARIO    live cross-asset cube (P&L/exposure/limits/ladder), drill-through, what-if inline
   ├─ XVA                CVA/DVA/FVA + pre-trade incremental                              (surface hidden)
   ├─ BOOKS & POSITIONS  one progressively-dense blotter; curated columns + add-any
   ├─ REPORTING          valuation/risk/activity/regulatory; scheduled + ad-hoc          (greenfield)
   ├─ ADMIN              users/roles/capabilities/desks + per-class LICENSE management
   ├─ OPS & CONNECTIVITY latency p50/p99/p99.9, stream/session stats, FIX, feed/HA health (surface hidden)
   └─ MODELS & PLUGINS   install/list/version/retire house models                        (surface hidden)
```

## 2. Global frame
- **Top command bar** — CelNet pinwheel + wordmark · **command palette** (mnemonic + search) · **Desk/
  Book scope selector** · license/entitlement chip · live ops health (p50/p99) · identity.
- **Left navigator** — the underlier/capability tree, **license-aware** (licensed = present; unlicensed
  = subtly gated/upsell, never a dead ticket).
- **Main** — the active workbench surface (one parametric component per §1 verb).
- **Right context** — scope-aware risk/positions/limits + "what changed" — collapsible for focus.

## 3. Parametric taxonomy (kills replication)
Every surface is ONE component taking `(desk, book, underlier, assetClass)` — generalizing the one
clean pattern (`GreeksStrip`). The 4 blotters, twin FX/FI risk grids, and parallel rates stack collapse
to one each. License + entitlement are props, not forks.

## 4. Design principles (research-validated; discard the rest)
1. Command-palette-first; entity×verb grammar; scope bus; saved perspectives.
2. **Progressive density** — curated-by-default, "add-any-column/panel" on demand (beats Calypso's 300
   undifferentiated columns and Bloomberg's function-code wall).
3. One asset-class-parametric fabric — no silos; FX-options + rates + linear + cross-asset on one core.
4. One ticket, swappable representations (table/curve/ladder), class-aware.
5. Registry-driven structure builder (leg ladder + template gallery + net-greek strip + payoff),
   license/priceability-gated.
6. Publish-gate pattern reused: the excellent versioned+arb-blocking **surface publish** becomes the
   template for a symmetric **"publish stream/tier"** contribution control.
7. Contribution as a **live tunable control loop** — spread/skew per client tier + **"why this price"**
   explainability + client analytics (hit-ratio/elasticity/impact). (out-functions ION's opaque silo.)
8. RFQ: manual + **auto-quote** in one list, trader-in-control (Tradeweb-AiEX-style), and — since we own
   the dealer side — **close the loop** quote→hedge→book→risk.
9. Risk: Murex dashboard taxonomy (P&L/exposure/limit-util/liquidity-ladder, drill currency→security→
   trade) but **live/streaming/drill-through/legible**; what-if/stress inline; p50/p99/p99.9 at IB scale.
10. Feed management: multi-source registry + visible arbitration/blend + per-instrument freshness health.
11. XVA + observability + model-management surfaced (capabilities competitors' clients don't expose).
12. Density with discipline — dark-first; redundant color+glyph; restrained decaying flash; translucent
    chrome, **opaque data planes**; WCAG AA 4.5:1. Anaheim headings; tabular-num data.
13. Excel add-in = **live round-trip client of the one API** (kill export-to-Excel-to-think), + close
    the `CELNET.SCENARIO` gap.

## 5. Licensing model (engine-enforced, per CAPABILITY-MAP §A)
Tiers sold separately, mirrored in UI (present / gated / upsell) off the `celnet-entitlements`
`AssetClass×Action×Desk` kernel: **FX + Metals** (full: vanilla+exotics+LSV+5-surface+linear) ·
**Equity / Commodity / Crypto** (vanilla + landing perpetual/future) · **Rates/FI** (curve suite).
Never render a ticket the engine will reject.

## 6. Capability coverage (KEEP / FIX / BUILD / SURFACE)
- **KEEP** (strong): pricing workbench, scenario, vol-surface publish, admin/entitlements, FIX conn.
- **FIX**: unify FX+FI risk/blotter; add `CELNET.SCENARIO`; make DealerPanel (LP-hedge) distinct from
  client contribution.
- **BUILD** (greenfield, out-function): feed-management console, contribution/tiering/skew console,
  auto-quote, ops dashboard, reporting.
- **SURFACE** (built, no client): XVA, model/plugin management, observability, feed compositing/health,
  exotic auto-booking, GPU speed lever, crypto strike-axis surface.

## 7. Discard (failed research/critique)
Amber-on-black + function-code nav · legacy Java-desktop/Webswing web-wrapping · PS-gated multi-year
customization · per-asset-silo sprawl · export-to-Excel-to-analyze · 300 undifferentiated default
columns · book-a-demo opacity · buy-side execution affordances (Buy/Sell, taker axe-lift, trading P&L/
VaR-to-trade) · verbose descriptions / unnecessary functionality.

## 8. Mockup deliverable set (sell-side, revised)
`00-shell` (scope + price+model, REBUILD) · `01-structure` · `02-surface` (mark/publish) ·
`03-contribute` (contribution/tiering/skew console + "why this price") · `04-rfq-desk` (maker +
auto-quote) · `05-feed-manage` · `06-risk-scenario` · `07-xva` · `08-books-blotter` · `09-reporting` ·
`10-admin-license` · `11-ops-connectivity` · `12-models-plugins` · `13-command-palette`. Excel parity
documented (same API). Each: license/desk/book-scoped, minimalist, then critiqued (visual+intuitivity)
and pruned.

## 9. API evolution deltas (clean, unversioned)
XVA proto+arm (⟂ D-xva lane) · vendor-feed/market-data-admin service (multi-source registry, symbology,
health) · contribution/spread-model admin service (SetSpreadModel, tiering, publish on/off) · auto-quote
engine · ReportingService · observability/metrics wire surface · model-management RPCs · crypto
strike-axis surface field · rates↔option carry-seam convergence (ADR-0010).

## 10. No-drift anchoring
Each surface + decision → lodestar `decision`/`spec:satisfies` claim + design ADR, linking the mockup
(symbol) to the capability (crate/proto) + workflow it realizes. Design = evolvable source of truth.
