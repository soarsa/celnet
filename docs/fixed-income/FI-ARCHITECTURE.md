# Celnet — Fixed-Income Architecture (`celnet-rates` crate family, additive contract, five-client surface)

**Status:** P0 spec (first synthesis) · 2026-06-22 · branch `feature/fixedincome`
**Scope:** the buildable architecture for the rates P0 — the `celnet-rates` crate (modules,
dependency arrows), the additive `celnet.proto` arms, how the same contract reaches all five clients,
how risk folds into the existing server-owned `RiskService`, and the **D2 GUI** asset-class tab
design. Honours the **LOCKED** build directions D1 (own `celnet-rates` crate family) and D2 (top-level
Options | Fixed-Income tabs).
**Synthesised from:** [`../_research/fixed-income-findings.md`](../_research/fixed-income-findings.md)
§A (crate sketch) + §4.4 (scale). Mirrors [`../ARCHITECTURE.md`](../ARCHITECTURE.md) §2 (workspace
layout) and [`../INTERFACES.md`](../INTERFACES.md) (dependency direction, proto arms); extends the
estate, does **not** fork a parallel silo.

> **Guardrails honoured.** OSS-only; **vendor-neutral purpose-named** crate/module/type identifiers
> (CLAUDE.md §8 — `celnet-rates`, not a method name); **single additive contract** (no versioning,
> guardrail #9 — rates arms append to the one `celnet.proto`); validate-don't-assert
> ([`FI-VERIFICATION-CONTRACT.md`](FI-VERIFICATION-CONTRACT.md)).

---

## 1. D1 — the `celnet-rates` crate family (own crates, shared seams only)

Per the **locked D1 decision** (`OPEN-QUESTIONS.md`), fixed income gets its **own crate family**,
depending only on the shared seams, **never** folded into `celnet-vanilla`/`celnet-linear` (findings
§A). The P0 crate is `celnet-rates`; a later `celnet-rates-vol` follows for optionality.

```
celnet-rates                       (P0 — curves + linear rates; numeric-core, NO IO)
├── depends ONLY on the shared seams (one-way arrows, never inverted):
│     celnet-types         (Ccy, Money, Tenor, Date; DayCount extended per FI-CONVENTIONS §1)
│     celnet-conventions   (extended with the RatesConvention schema — FI-CONVENTIONS §5)
│     celnet-calendar      (US_FED / TARGET2 / GB_LON business-day calendars — FI-CONVENTIONS §4)
│     celnet-core          (the math + discount-factor / pricing-trait seam — reuse, don't re-derive)
│   ── NO dependency on celnet-vanilla / celnet-linear (D1) ──
│
├── modules:
│   ├── curve/             immutable Curve snapshot (DF | zero | inst-fwd repr); interpolation
│   │   ├── interp/        log_linear_df · monotone_convex_fwd   (provenance: prose/doc-comment only)
│   │   └── jumps/         turn-of-year / meeting-date forward steps
│   ├── build/             bootstrap (sequential, acyclic) + global solver (LM; cyclic/basis/XCCY)
│   ├── conventions/       RatesConvention resolution per (ccy,index,tenor); RFR observation methods
│   ├── product/           Fra · Ois · VanillaSwap · TenorBasisSwap · CrossCcyBasisSwap · StirFuture · BondFuture(CTD)
│   ├── rfr/               compounded/averaged RFR accrual; lookback/shift/lockout/payment-delay
│   ├── pricing/           PV · par rate · cashflow projection (discount curve × projection curve)
│   └── risk/              pv01 (annuity) · dv01 (quote bump) · key_rate (instrument Jacobian)
│
├── exposes (for the additive contract & clients):
│     CurveSet            (discount + per-tenor projection curves; immutable, cheaply cloned)
│     RatesInstrument specs (FRA/OIS/IRS/basis/XCCY/futures)
│     PricingResult { pv, par_rate, pv01, dv01, key_rate_ladder }
│
└── later: celnet-rates-vol   (P-next — swaptions/caps; Bachelier/normal-SABR/shifted, Hull-White/
                               G2++; vol cube expiry × tenor × strike) — depends on celnet-rates +
                               celnet-surface. NOT in P0 (findings §4.3/§C).
```

### 1.1 Dependency arrows (in the [`../INTERFACES.md`](../INTERFACES.md) registry style)

```
celnet-types  ←  celnet-core  ←  celnet-rates  ←  { celnet-server, celnet-cli }
celnet-conventions, celnet-calendar  →  consumed by celnet-rates (shared seams; extended, not forked)
celnet-rates  →  NO edge to celnet-vanilla / celnet-linear (D1 — disjoint leaf)
celnet-rates-vol (later)  →  depends on celnet-rates + celnet-surface
celnet-golden, celnet-parity  →  validate celnet-rates out-of-process (QuantLib + ORE; FI-VERIFICATION-CONTRACT)
```

`celnet-rates` is a **disjoint leaf** an independent session can own end-to-end without touching the
FX crates — the same parallel-ownership discipline as `celnet-surface`/`celnet-exotics`
([`../ARCHITECTURE.md`](../ARCHITECTURE.md) §2.2). The shared seams (`celnet-types`,
`celnet-conventions`, `celnet-calendar`, `celnet-core`) are stabilised **first**, then extended only
with coordination — the `DayCount` and `RatesConvention` extensions (FI-CONVENTIONS §1/§5) land on
those seams as **additive** changes (single contract — guardrail #9).

### 1.2 Hot-path & scale discipline

The `Curve`/`CurveSet` is an **immutable, cheaply-cloned snapshot** so many scenarios fan out in
parallel; the pinned numeric core stays **alloc/lock-free** (findings §4.4;
[`../ARCHITECTURE.md`](../ARCHITECTURE.md) §3). A curve build is cheap (`<ms`); the cost at
investment-banking scale is the **risk cube** (n_instruments × n_pillars × n_curves bump-reprice) —
favour analytic/AD deltas where exact, vectorise bump-reprice, and **reuse the existing server-owned
hierarchical-risk rollup** rather than a client loop-sum (findings §4.4; §3 below).

---

## 2. The additive `celnet.proto` arms (single contract, all five clients)

Rates surfaces through the **one unversioned `celnet.proto`** as **additive** arms (no
`schema_version`, no renumber — guardrail #9; [`../INTERFACES.md`](../INTERFACES.md) §"Single current
wire contract"). The additive set:

| Wire addition | Shape | Notes |
|---|---|---|
| **`CurveSet`** | a discount curve + per-tenor projection curves (pillar dates + repr + interp tag) | the market-state unit (FI-CURVES §1); immutable. |
| **`RatesInstrument` specs** | new `Instrument.product` oneof arms: `fra` · `ois` · `vanilla_swap` · `tenor_basis_swap` · `cross_ccy_basis_swap` · `stir_future` · `bond_future` | appended after the existing FX/linear arms (`…ndf=28`, `perpetual_option=30`, …); next free field numbers, no renumber. |
| **`PricingResult`** | `{ pv, par_rate, pv01, dv01, key_rate_ladder }` | surfaced identically to every client; `key_rate_ladder` is the trader-facing bucketed-delta vector (findings §2.6). |

These mirror how the FX cross-asset/linear waves appended `Underlying`, `CarryModel`, `fx_forward`/
`fx_swap`/`ndf`, and the RFQ messages ([`../INTERFACES.md`](../INTERFACES.md) §"Asset-class universe").
Because `tools/check-verification-coverage.mjs` **parses the proto oneof**, each new rates arm
automatically demands its golden vector + parity row + per-client exposure declaration
([`FI-VERIFICATION-CONTRACT.md`](FI-VERIFICATION-CONTRACT.md) §3).

### 2.1 The five-client surface (identical contract)

The same `CurveSet`/`RatesInstrument`/`PricingResult` reaches all **five** clients — the existing
parity bar ([`../VERIFICATION-CONTRACT.md`](../VERIFICATION-CONTRACT.md) §(d)):

| Client | Rates surface |
|---|---|
| **GUI** (React) | the new **Fixed-Income workspace set** under the D2 tabs (§4). |
| **Excel** add-in | additive `CELNET.*` rates functions (curve DF, swap PV, par, PV01/DV01, key-rate). |
| **Rust SDK** (`celnet-client`) | typed builders for the rates instrument vocab over the wire. |
| **FIX** acceptor | the rates instrument dialect over the existing FIX session. |
| **federation** fan-out | rates pricing/risk fans out identically across shards. |

---

## 3. Risk — folded into the existing server-owned `RiskService` (no client loop-sum)

Rates risk is **not** a new hierarchy. PV01/DV01/key-rate deltas (findings §2.5/§2.6) fold into the
existing server-owned `RiskService` org-cube rollup ([`../INTERFACES.md`](../INTERFACES.md)
§"Phase-2 contract: `RiskService`"; [`../ARCHITECTURE.md`](../ARCHITECTURE.md)); **clients never
loop-and-sum** — the server rolls up over org dimensions in a reporting numeraire (charter §2).

- **PV01** is the analytic annuity; **DV01** is a bump of the calibrating quotes; **key-rate** is the
  instrument-Jacobian delta ladder desks hedge on (findings §2.5/§2.6).
- The risk-cube (bump-reprice over pillars × curves) reuses `celnet-risk-cube`'s additive roll-up +
  non-additive bump-and-revalue machinery; FRTB GIRR/CSR sensitivities reuse the same path and are
  cross-checked against **ORE** ([`FI-VERIFICATION-CONTRACT.md`](FI-VERIFICATION-CONTRACT.md) §2).
- `celnet-risk-normalize` provides the convention/numeraire canonicalisation the cube sits on — rates
  cashflows normalise through the **same** boundary as FX (single risk hierarchy).

---

## 4. D2 — the GUI asset-class layer (Options | Fixed Income)

> **SUPERSEDED — LANDED as ONE class-parametric rail (`fe-fi-migration`, `33d9a0a` / `fd18594`).**
> The design below proposed a *top-level tab strip above the rail*; the final implementation instead
> **collapsed the FX-vs-FI domain-tab split into one class-parametric rail** — asset class is chosen
> by **scope + license**, and FI capability is reached as **lenses** of the shared workspaces (Market
> Data → curve lens, Risk → rates lens, Book → positions/deals lenses, Ticket → rates product family;
> the standalone `RatesWorkspace` is deleted). "FI integrated, not a peer." The subsections below are
> retained as design rationale; for the shipped IA see [`FI-STATUS.md`](FI-STATUS.md) slice F.

Per the earlier **locked D2 decision**, the GUI was to gain a **top-level asset-class tab layer ABOVE
the existing workspace rail**, so each domain owned its own workspace set (`OPEN-QUESTIONS.md` D2).

### 4.1 Today's GUI (what coexists)

The current Shell renders a **left workspace rail** that is **data-driven** from a single
`RAIL` registry (`gui/src/lib/commands.ts`), with `WorkspaceId` mirrored in
`gui/src/app/AppContext.tsx` and the active view resolved by `WORKSPACE_VIEW` in
`gui/src/app/Shell.tsx`. The rail entries are: **Ticket · Stream · Surface · Risk · Book**
(+ admin-gated Connections · Admin · Excel). `⌘1..n` chords are **generated from `RAIL`**, and the
scope switcher already carries an **asset-class rail** dimension (FX/metals/equity/commodity/crypto)
for the *underlier* — that is an underlier dimension, **not** the new domain tab.

### 4.2 The new layer

A **top-level `AssetDomain` tab strip** sits **above** the workspace rail:

```
┌──────────────────────────────────────────────────────────┐
│  [ Options ]   [ Fixed Income ]        ← AssetDomain tabs │  (new, above the rail)
├──────────┬───────────────────────────────────────────────┤
│  rail    │                                                │
│  ⌁ … ◷ … │            active workspace canvas             │  (existing Shell, re-targeted)
│  (per-   │                                                │
│   domain)│                                                │
└──────────┴───────────────────────────────────────────────┘
```

- **`AssetDomain = Options | FixedIncome`** is new top-level state (an enum above `WorkspaceId`),
  selecting **which `RAIL` registry the Shell iterates**. The Shell stays data-driven — it already
  iterates `RAIL`; D2 makes `RAIL` a **function of the active `AssetDomain`** rather than a single
  constant. No Shell rewrite, no per-view special-casing.
- **Options domain** keeps today's rail unchanged: Ticket · Stream · Surface · Risk · Book.
- **Fixed-Income domain** gets its **own workspace set**, e.g.:

  | FI workspace | Purpose | Reuses |
  |---|---|---|
  | **Curve** | build/inspect the `CurveSet` (DF/zero/forward views, interpolation toggle, turn/meeting jumps) | the surface-viz primitives of `SurfaceWorkspace`. |
  | **Ticket** | price a rates instrument (FRA/OIS/IRS/basis/XCCY/futures) → `PricingResult` | the `TicketWorkspace` ticket→price flow + product registry pattern. |
  | **Risk** | PV01/DV01/key-rate ladder via the server-owned `RiskService` rollup (no client sum) | the existing `RiskWorkspace` / `CubeWorkspace` drill. |
  | **Book** | rates positions/blotter | the `BookWorkspace` blotter shell. |

- **Keyboard grammar** stays generated: `⌘1..n` re-derives from the active domain's `RAIL`; the
  domain switch itself gets its own chord (e.g. a top-level toggle), so the existing `resolveChord`
  contract is extended, not broken.
- **Shared chrome** (command palette, scope switcher, status ribbon, session gate) is **unchanged** —
  it wraps both domains; only the rail registry + active canvas re-target on domain switch.

This keeps the existing rail (Ticket/Stream/Surface/Risk/Book) **fully intact** under the Options tab
while the Fixed-Income tab mounts a disjoint workspace set — the minimal, data-driven extension D2
asks for. The exact FI workspace inventory is GUI-implementation detail to be finalised when the
client work is scheduled; the **architecture** is: one new `AssetDomain` layer above the rail, the
rail becomes domain-parameterised, the Shell and keyboard grammar stay data-driven.

---

## 5. P0 build order & open-question dependencies

1. **Stabilise the shared seams** — `DayCount` + `RatesConvention` extensions on
   `celnet-types`/`celnet-conventions`, calendars on `celnet-calendar` (FI-CONVENTIONS; additive).
2. **`celnet-rates` curve/build** — `CurveSet`, interpolation, bootstrap + global solver (FI-CURVES).
3. **`celnet-rates` product/pricing/risk** — FRA/OIS/IRS/basis/XCCY/futures; PV/par/PV01/DV01/key-rate.
4. **Additive proto arms** + golden vectors + parity rows (FI-VERIFICATION-CONTRACT §3).
5. **Five-client surface** — server/SDK/CLI/Excel/GUI(D2) wired + conformance-gated.
6. **Risk** folded into `RiskService`/`celnet-risk-cube`; ORE FRTB cross-check.

**Open-question dependencies the operator must confirm before locking:**
- **Q1** currency scope (sizes the calendar/fixing static) · **Q4** v1 single-OIS-discount (assumed) ·
  **Q10** default interpolation · **Q11** STIR convexity placeholder · **Q12** calendar/fixing
  sourcing · **Q8/Q9** oracle-engine exclusions · **Q13** CME-CF third anchor. See the per-doc
  "pending Q*" notes in [`FI-CURVES-SPEC.md`](FI-CURVES-SPEC.md),
  [`FI-CONVENTIONS.md`](FI-CONVENTIONS.md), and
  [`FI-VERIFICATION-CONTRACT.md`](FI-VERIFICATION-CONTRACT.md).

**Deferred (later passes, flagged not half-built):** `celnet-rates-vol` (swaptions/caps —
Bachelier/normal-SABR/Hull-White/G2++), credit/CDS, inflation, cash-bond static-data breadth, full
multi-CSA / CTD-collateral (findings §C).

---

### Sources

All sources are the pass-1 citations in
[`../_research/fixed-income-findings.md`](../_research/fixed-income-findings.md) §A + §4.4; GUI
grounding is the live `gui/src/lib/commands.ts` (`RAIL`), `gui/src/app/AppContext.tsx`
(`WorkspaceId`), and `gui/src/app/Shell.tsx` (`WORKSPACE_VIEW`).
