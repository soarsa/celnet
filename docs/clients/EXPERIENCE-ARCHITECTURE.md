# Celnet — Experience Architecture

> The single coherent product experience that ties every workspace into one clean,
> logical, IB-scale, trader-grounded whole.
>
> **Scope.** This is the cross-product synthesis. It is the canonical home for the
> *experience* contract — navigation, scope, drill, analytics selection, per-workspace
> target design, and the reconciled phased backlog. Domain detail lives in the source
> docs and is referenced, not duplicated:
> [`RISK-HIERARCHY.md`](../quant/RISK-HIERARCHY.md),
> [`TRADING-UNIVERSE-SCALE.md`](../architecture/TRADING-UNIVERSE-SCALE.md),
> [`SURFACE-WORKFLOW.md`](../quant/SURFACE-WORKFLOW.md).
>
> **Honesty contract (CLAUDE.md rule 5).** Every claim below is tagged: **[built]** =
> verified in code this session; **[proposal]** = design against a not-yet-built
> substrate; **[gap]** = needs a feed/crate that does not exist; **[inf]** = competitor
> "uniqueness" is argument-from-absence (no public evidence), never proof. Audience:
> a trader who runs a desk/book at a global bank. Target: May-2026 SOTA.
>
> **Decided by product owner (fixed inputs):** Brand = Celnet; single logo
> in the left rail; mark-less wordmark in the toolbar; no macOS traffic lights. No
> user-admin yet → **SHOW ALL** firm-wide today, designed so an entitlement filter slots
> in with zero rework. The trend graphic becomes a labelled configurable **TrendMode**
> (default = the instrument's own streamed **Premium** mid; STOP the abstract
> cross-structure rebased index).

---

## 1. Principles

The whole product is coherent because a small number of rules hold *everywhere*.

1. **One mental model: the position-fact cube.** Every number the product shows is a
   reduction over the same immutable position-level facts, each tagged with all
   dimension keys (trade · trader · book · desk · ccy-pair · booking-location ·
   legal-entity · firm · value-date · session). A leaf is a single instrument; an
   aggregate is a group-by. Drill-down and roll-up are the *same* measures at different
   levels, and **every aggregate is always reconcilable to its constituents**
   (RISK-HIERARCHY §2.5). There is no separate "book data path" and "risk data path."

2. **Scope × View × Analytics are orthogonal axes.** *What slice of the firm* (Scope),
   *how I look at it* (View = workspace), and *with which model/measures/axes*
   (Analytics) vary independently. Any (scope, view, analytics) triple is a valid,
   bookmarkable, shareable application state. This orthogonality is the single
   organizing idea of the navigation model (§2).

3. **Show-all-now, entitlement-ready.** Today the experience resolves to the
   identity/"grant-all" principal: firm-wide, all desks/books/pairs visible. Because
   entitlement is modelled as a **server-side pre-aggregation predicate over dimension
   subtrees** (RISK-HIERARCHY §2.6/§4), slotting a real principal in later changes only
   *which facts the predicate admits* — no UI rework, no roll-up-math change, no
   aggregate-leakage risk. The viewer/principal scope is therefore carried as **explicit
   context now**, even while it resolves to "everything."

4. **Honest data only.** No fabricated numbers, ever (CLAUDE.md rule 2). Empty-states say
   "—"; provenance (model, conventions, `surface_version`, source feed, role) is always
   on the face; non-additive measures are visibly distinguished from additive ones; the
   Book numeraire caveat is stated, not hidden. This discipline is already lived in the
   current GUI **[built]** and is a load-bearing asset to preserve.

5. **Labelled, configurable everything.** No unit-less glyph, no hidden default. Trend
   sparklines carry a `TrendMode` label + unit; surface grids name their model and
   conventions; risk grids name their shock axes and measure. If a value cannot be
   labelled honestly, it is not shown.

6. **Keyboard-first.** ⌘K command palette is the universal escape hatch and search
   spine **[built]**; every primary action has a shortcut; grids are keyboard-navigable;
   marking is commit/revert (⌘↩ / Esc). The palette is *extended* at scale, never
   replaced.

7. **Virtualise at scale.** At IB cardinality (hundreds of pairs, thousands of blotter
   rows, firm-wide cube nodes) every list/grid is row+column virtualised with a
   server-side row model and client-side tick coalescing to a frame budget
   (TRADING-UNIVERSE-SCALE §5.2). The render budget is stated *against* the server
   stream rate and is itself an instrument (the measured-P99 status ribbon **[built]**).

---

## 2. Unified Navigation & Scope model

Three orthogonal controls, each owned by exactly one surface. This resolves the audit's
"three redundant pair affordances / logo-twice / undifferentiated toolbar-vs-rail"
critique (audit §2).

### Axis A — VIEW (the **left rail**): *how I look*
The rail owns **identity + view selection**, nothing else.
- **Brand:** the single Celer logo lives here (rail header) — the only logo in the app.
- **Workspaces:** Ticket · Stream · Surface · Risk · Book (+ future **Universe** navigator
  and **Cube** heatmap as they land). ⌘1..n. The rail is a *fixed, small* list — it is
  the product's verbs, not its data.
- **Glyph fix:** Book currently reuses "Σ" which collides with the vega-ladder "Σ" panels
  (audit §2.1). Re-glyph Book (e.g. "▤" book/ledger) so Σ means "sum/ladder" consistently.

### Axis B — SCOPE (the **toolbar**): *what slice of the firm*
The toolbar owns the **mark-less wordmark + the scope selector + ⌘K search**. It is the
universe/firm navigator — the thing that scales. The scope selector is a
**breadcrumb-driven group-by picker**, not a fixed tree, because the org is a cube whose
dimensions are independent (RISK-HIERARCHY §2.1):

```
firm → legal-entity → country/booking-loc → desk → trader → book → ccy-pair
        (any subset, in any order — the user picks the roll-up path)
```

- **Breadcrumb IS the filter AND the drill path** (RISK-HIERARCHY §6). Each crumb is a
  pinned dimension value; the next segment is the open group-by axis. Click a crumb to
  drill up; click into a node to drill down.
- **Entitlement-filtered, defaulting to SHOW ALL** until user-admin exists (§3).
- **Pair selection is one terminal case of scope** (the leaf group-by `ccy-pair`),
  *unifying the three redundant affordances*: PairMenu/PairStrip/⌘K all become views onto
  the same scope state. At scale the pair picker becomes the **Universe navigator**
  (virtualised, searchable, groupable by region / G10-vs-EM / liquidity-tier /
  deliverable-vs-ND / metals; favourites pinned; multi-pair) layered on the ⌘K spine
  (TRADING-UNIVERSE-SCALE §5.1).
- **Global lenses the toolbar also carries** (every cube query is parameterized by them —
  RISK-HIERARCHY "scope-context"): surface **lens** (live / official-IPV-pinned /
  historical `surface_version`), **session / follow-the-sun cut**, point-in-time
  **replay** ("what did the desk see at 14:32?"), and **reporting numeraire**. Convention
  is *not* a toolbar choice — the cube is convention-free (every leaf canonicalized
  before aggregation, RISK-HIERARCHY §2.2); the toolbar carries reporting *views*, not
  internal convention.

### Axis C — ANALYTICS / VIEW-CONFIG (a per-workspace **inspector strip**): *with what*
A compact, consistent config affordance docked in each workspace header: **model**
(VV/SABR/SVI/SSVI), **measures/greeks** (incl. vanna/volga ladder), **scenario axes**
(spot×vol shock, abs/rel), **TrendMode**, **columns/layout + saved views**. Per-workspace
override on top of a global default (§5). Provenance of the current selection is always
shown.

**Rail vs toolbar, resolved.** Rail = identity + *verbs* (fixed, small, brand). Toolbar =
*scope + search* (the scaling navigator + global lenses). They never both carry the pair
control and never both carry a logo. Analytics config lives *in the workspace*, because it
is view-specific. This is the clean three-way split the audit asked for.

---

## 3. Entitlement-aware drill-down / up

**Now (show-all).** `ScopeContext` (new, in `AppContext`) carries:
`principal` (today the singleton **grant-all** principal), `path` (the ordered list of
pinned dimension values = the breadcrumb), `groupBy` (the open axis), and the global
lenses (`surfaceLens`, `session`, `asOf`, `numeraire`). Today `principal` admits every
fact, so the breadcrumb walks the full firm. The UI renders the breadcrumb with an
explicit **"Firm · all desks · all books"** root chip so the scope is visible even when
unfiltered.

**Later (entitlement + user-admin), zero rework.** A real principal carries role grants
scoped to dimension subtrees (e.g. "read risk for desk=EM-vol, any book") and deny rules
for information barriers (RISK-HIERARCHY §4). The aggregation API applies the predicate
**server-side, pre-reduction**, so a parent total can never leak the magnitude of an
invisible subtree. The GUI is unchanged: it still sends `(path, groupBy)` and renders what
comes back; only *which facts the predicate admits* changes. Every entitlement decision is
audited via `celnet-observability`.

> **[built & served end-to-end — GUI / SDK / Excel lanes all done]** The aggregation layer is
> **built and served end-to-end**: `celnet-risk-cube`, `celnet-risk-normalize`, `celnet-limits`,
> `celnet-entitlements` are gated green (RISK-HIERARCHY §3.2) and exposed over the one
> `celnet-proto` contract as `RiskService` (`ListPositions`/`AggregateRisk`/`DrillRisk`/
> `LimitStatus`), served by `celnet-server` over gRPC + the WS mirror off a shared live
> `PositionStore`. The entitlement predicate prunes **server-side, pre-reduction**, exactly as
> the drill UX below requires. The GUI Book/drill, the `celnet-client` SDK, and the Excel
> `CELNET.*` functions now **all** consume these RPCs — the client-side `aggregateBookRisk`
> loop is **deleted** (`gui/src/data/portfolioRisk.ts` removed) and the GUI seam (`ScopeContext`)
> drives `AggregateRisk`/`LimitStatus`/`DrillRisk` server-side. Deferred at the backend (only):
> AAD/GPU non-additive reval + cross-shard fleet (RH §3.3/§3.4).

---

## 4. Book ↔ Risk linkage

Book and Risk are **the same cube at two zoom levels**: Book = a group-by node; Risk =
the leaf `RiskFact` for a single instrument. Drill-down is the continuous path between
them; contributor decomposition falls straight out of the fact table (RISK-HIERARCHY,
SCALE §3).

- **Drill down:** clicking a Book aggregate row (a pair, desk, trader, or a tenor×delta
  vega bucket) pushes that dimension value onto the `ScopeContext.path` and re-renders
  the next group-by; clicking a single-instrument leaf opens the **Risk** workspace
  scoped to that instrument. This fixes the audit finding that "clicking a pair row does
  nothing" and that Book/Risk independently recompute `transport.scenario` (audit §1
  Book, §3.3).
- **Drill up:** the toolbar breadcrumb is the back-path; clicking a crumb pops the scope.
- **Shared selection state** is the connective tissue across **Stream/Book/Risk/Ticket**:
  - Ticket → Risk/Stream **passes the actual `Instrument`** (fixes the audit bug that
    "Add to risk" only switches workspace while Risk hardcodes a 25Δ RR — audit §1 Risk,
    §3.8). The Ticket already promotes the *same* `Instrument` object for "Stream this"
    **[built]**; Risk must consume it instead of re-keying.
  - Stream row → Ticket (load structure) and Stream row → Risk (analyse that line) reuse
    the same selection.
  - Book per-pair/desk/trader breakdown rows are drill targets into Risk for that slice.
- **Reconciliation guarantee:** because Risk (leaf) and Book (aggregate) read the same
  facts at different levels, the single-instrument number a trader sees on drill-down
  *always* sums back to the Book total they came from. No drift between screens.

---

## 5. Analytics selection

**How a user chooses which analytics to use** — the answer is uniform across the product,
seeded by the Surface domain's in-grid model selector + per-mark provenance pattern
(SURFACE-WORKFLOW §7.2), generalized to every workspace via Axis C (§2):

- **Global defaults** live in `ScopeContext` (default model, default greek set, default
  scenario axes, default TrendMode). A desk sets these once.
- **Per-workspace override** via the inspector strip: Surface picks the calibrating
  model; Risk picks shock axes + measure (P&L/Δ/ν, and the vanna/volga ladder); Stream
  picks columns + TrendMode; Book picks the breakdown dimension + numeraire.
- **Provenance always shown.** Every mark/quote/risk number names the model that produced
  it, the conventions used, the `surface_version` it pinned, and the role (working /
  official / IPV). This is the honesty contract made visible.
- **Tied to the plugin/SDK model.** The model list is not hard-coded UI: it enumerates the
  registered pricing/calibration models behind `celnet-plugin-api` — Tier-0 native and
  Tier-2 wasmi models appear identically through the `ModelRegistry` **[built, host]**.
  A user-extensible analytic added via the SDK shows up in the same selector with the same
  provenance line. Selecting a model is selecting a registry entry, so "which analytics"
  scales to user-authored analytics with zero UI special-casing.

> **[built]** `celnet-surface` ships VV/SABR/SVI/SSVI/**eSSVI** (5 families); the engine ships
> bucketed vega/cross-gamma/theta-roll, and the firm-level **non-additive re-derivations
> (VaR/ES, curvature) + FRTB-SA SbM capital** are now built and served via `RiskService`
> (`celnet-risk-cube/{nonadditive,frtb}.rs`). **[gap]** vanna/volga *limits* remain unbuilt —
> the selector exposes only what the engine actually supports, and greys the rest as "not yet
> available," never faking it.

---

## 6. Per-workspace target design

Concrete, mapped to current files. **[GUI-only]** = TS/React change against today's
contract; **[needs Rust]** = engine/proto/crate work first.

### Stream — `workspaces/StreamWorkspace.tsx` (the resting state)
Preserve the streaming-first RFS blotter: multiplex `StreamSession`, per-row health,
flash-on-change, click-a-side via short-lived `TradableToken`, typed reject toasts
**[built]**. Lift to scale:
- **Virtualise** rows+columns with a server-side row model + async transaction batching to
  a frame budget (today maps `app.stream.rows` straight to DOM — fatal at scale; audit §1
  Stream). **[GUI + needs Rust]** (server-side aggregation/fan-out).
- **Groupable** by pair / desk / tenor; **configurable columns + saved views**
  (TRADING-UNIVERSE-SCALE §5.1/§5.2). **[GUI-only]** for grouping over current rows.
- **Attribution column**: book / owner (human seat OR auto-pricer, uniformly), LP-in-
  competition, `surface_version` quoted against. **[served]** `AttributionRecord` is on the wire
  and mapped onto the cube's org dimensions; **[gap]** only the live identity feed (real
  seat/auto-pricer values) remains (SCALE §3/#12).
- **Labelled configurable TrendMode** replacing the unlabelled premium sparkline (§7).
  **[GUI-only]** for the Premium default; other modes gated on the market-series feed.
- **Fix the Σ/"Mid" ambiguity:** label "Mid" as *premium mid* with its unit; reserve "Σ"
  for sum/ladder panels only (don't overload the Book glyph). **[GUI-only]**
- **Broken/IMM tenor labels** in `tenorLabel` (today only ON/W/M/Y). **[GUI + needs Rust]**
  (tenor model — SCALE #4).

### Surface — `workspaces/SurfaceWorkspace.tsx` (marking source of truth)
The **editable delta×tenor marking grid is PRIMARY** (rows = tenors; cols = ATM, 25RR,
25BF, 10RR, 10BF). The 3-D mesh is **demoted** to an optional collapsible QC panel; the
2-D smile chart is a secondary shape inspector; **term-structure curves** (ATM/RR/BF vs
tenor) become the second-most-prominent view (SURFACE-WORKFLOW §4).
- **All broker handles editable** (today only ATM-of-active-row is — 25RR/25BF/10RR/10BF
  are static spans, i.e. skew/convexity are read-only; audit §1 Surface). Keyboard-nav,
  vol-convention nudge steps (ATM ~0.05v, RR/BF ~0.025v; ⌘↩ commit, Esc revert). Show the
  wider read-only derived delta grid (≈11 cols) for inspection. **[GUI + needs Rust]**
  (handle model + live recalibrate).
- **FIX BUG 1 — the mismark hazard (headline).** Both Re-mark and Publish call
  `app.remarkSurface()` (`SurfaceWorkspace.tsx:146,149`), which re-sends the seed
  `brokerLadder` (`AppContext.tsx:98-108`); the trader's `editAtm` (local `useMemo`) is
  **never transported** — a trader can "publish" the un-edited surface. **Re-mark ≠
  Publish.** Re-mark = recalibrate from the *edited* handles; Publish = deposit an
  immutable `MarkedSurface` under a fresh monotonic `surface_version`. The edit must reach
  `celnet-surface`. **[GUI; transport-the-edit may need proto/server confirm]**
- **FIX BUG 2 — hardcoded `calendarArbitrageFree: true`** (`surface.ts:92`); `checkArb` is
  butterfly-only and Publish gates on butterfly only. Surface the **real**
  `SurfaceArbitrageReport` (per-slice butterfly + cross-slice calendar
  `min_calendar_increment`, already computed in `celnet-surface`); **Publish gates on the
  WHOLE report.** **[GUI + wire the existing engine report]**
- **Working vs official + publish/version**: role (working / official / IPV) is
  first-class; promote = version bump with attributable P&L; two versions diffable
  (SURFACE-WORKFLOW §2.6/§3.6). **[GUI + needs Rust confirm of role on contract]**
- **Compare / history / consensus overlay** as a Δ-ghost on the chart + Δ-column on the
  grid (live broker composite, history T-1/T-5, month-end consensus). **[gap]** depends on
  a composite/consensus feed (CELER-INTEGRATION) — do not render as if it exists.
- **Model selector + per-mark provenance** (VV/SABR/SVI/SSVI + conventions + role) in the
  inspector strip (§5). **[GUI-only]** (engine already ships the four).
- **Quoted broker-BF vs calibrated smile-strangle** shown side-by-side when they diverge
  (never one masquerading as the other); premium-adjusted conversion residual shown.
  **[GUI + needs Rust]** (contract fields).
- **Stable y-axis** on the smile chart (today auto-fits to slice min/max, exaggerating
  tiny moves); replace the **9-pt mesh `DELTA_AXIS` vs 5-pt grid `DELTA_PILLARS`**
  mismatch with honest "marks at 5 handles, curve interpolated" labelling; remove
  float-equality selection. **[GUI-only]**

### Risk — `workspaces/RiskWorkspace.tsx` (the drill target)
Preserve the real server-computed spot×vol shock grid, bucketed vega (tenor×delta),
cross-gamma stencil, theta-roll, honest empty-states, perceptual diverging ramp
**[built]**. Lift:
- **Consume the drilled-in instrument/scope** instead of a hardcoded 25Δ RR (audit §1
  Risk) — Risk analyses what the user actually selected (§4). **[GUI-only]**
- **Analytics/axis selection** via the inspector strip: shock axes (abs/rel), measure
  tabs, and the **vanna/volga ladder** alongside vega. **[GUI for axes; VaR/ES/FRTB-SA now
  served via `RiskService`/`celnet-risk-cube`; needs Rust only for the vanna/volga ladder]**
- **Limits overlay** (utilization / RAG / soft-breach) sits on the same tenor×delta
  pillars. **[served]** `celnet-limits` is built and exposed via `RiskService.LimitStatus`
  (per-limit `cap`/`exposure`/`ratio`/`status`/`headroom` + `hard_breach`); GUI lane renders it.

### Book — `workspaces/BookWorkspace.tsx` (aggregate + drill)
Preserve the real per-position sum across `app.positions` with summary cards, per-pair
breakdown, aggregate vega ladder, honest skipped-position disclosure **[built]**. Lift —
the server backend for all three of these is now **served** by `RiskService`; the GUI lane
wires the calls:
- **Drill to Risk** from any aggregate row / vega bucket (§4). **[server: `DrillRisk` served]** —
  the true cube roll-up exists; the GUI lane replaces the client-side loop with `aggregate_risk`
  + `drill_risk`.
- **Per-pair / desk / trader / owner breakdown** — the identity dimension so the aggregate stops
  being anonymous (audit §1 Book). **[served]**: `AttributionRecord` is on the wire and the
  server maps it onto the cube's `OrgKey` org dimensions (`RiskDimension` group-by); GUI lane
  selects the dimension.
- **Numeraire caveat → resolved server-side.** Cross-pair totals were summed in **native premium
  units** (USD/JPY at spot 156 dominates a raw sum) and the GUI honestly disclosed this
  (`:152-159`). `AggregateRisk` now collapses every node into a real reporting numeraire via
  `celnet-risk-normalize` (`ReportingNumeraire` rates) **server-side** — the caveat is resolved at
  the source. The GUI Book now consumes the served aggregate and the old disclaimer is **gone**:
  the Book copy names the reporting numeraire and shows the per-ccy delta breakdown instead.

### Ticket — `workspaces/TicketWorkspace.tsx` (the pricer)
Preserve the strongest workspace: analytics+executable card, structure selector, real
smile-vol face read (|vega|-weighted), full 14-Greek strip, last-look ring, keyboard-first,
"same Instrument" promotion **[built]**. Lift:
- **Broken-date / event-aware expiry**: dual-mode tenor⇄date control; resolve+display the
  full chain (horizon→spot→expiry→delivery→cut/fixing); mark IMM / EOM / event dates;
  price by total-variance interpolation on a business/event clock with day-weighting, not
  nearest-pillar lookup (SURFACE-WORKFLOW §5, SCALE §5.4). **[needs Rust]** (tenor model
  #4, IMM resolver #9, event clock #8, two clocks).
- **Fix hardcoded rates** in `describeLegs` (`:310` uses `rDom:0.04, rFor:0.02` instead of
  `app.pairCtx.market`) — strike display inconsistency. **[GUI-only]**
- **Honour "Add to risk"** by passing the instrument (see §4). **[GUI-only]**

### Toolbar + rail per §2 (both) — `app/Shell.tsx`
Single logo in the rail; mark-less wordmark + scope selector + ⌘K in the toolbar;
re-glyph Book; ⌘P becomes a real scope/pair switcher (today it just re-opens ⌘K — audit
§2). Stop remounting workspaces on every switch (`key={app.workspace}` forces Risk/Book to
re-run all scenarios on each visit; persist state). **[GUI-only]**

---

## 7. The TrendMode graphic

**Defect (verified, not a data-honesty bug — TRADING-UNIVERSE-SCALE §5.5, audit §3.2):**
`Sparkline.tsx` is axis-agnostic (bare `number[]`, no label), fed two different things —
the blotter's real streamed premium (`row.midHistory`) vs `PairStrip.aggregateActivity()`
which **rebases + averages across structures into an abstract "unitless activity index"**
(its own comment). Two inconsistent direction definitions (last-two-points vs
first-vs-last). Data is honest; semantics, labelling, and direction are broken.

**Fix = a labelled, configurable, per-context `TrendMode` enum.** The tile *always* shows
the unit/label; line and tick glyph share **one** direction definition (first-vs-last over
the displayed window).

| Mode | Meaning | Availability | Default for |
|---|---|---|---|
| `PREMIUM` | instrument's own streamed premium mid | **now** (`midHistory`) | **blotter row (default now)** |
| `ATM_VOL` | pair ATM-vol history | needs vol-history feed | pair-strip (once feed exists) |
| `RR` / `BF` | risk-reversal / butterfly (vol) | needs feed | trader-selectable |
| `SPOT` | spot mid | **NOT available** (no live spot feed) | needs new feed |
| `FORWARD` | outright forward | needs feed | selectable |
| `VEGA` / `PNL` | position-weighted vega / MtM P&L | from position store | book/blotter selectable |

**Default now (no new feed):** the instrument's **own premium mid, labelled "Premium,"**
with the single consistent direction. **STOP** the cross-structure premium-blend in the
PairStrip — show the representative structure's labelled premium, or **"—"**, never the
abstract index. **Target default = `ATM_VOL`** for the pair strip, *only after* the
transport streams ATM-vol history (else it draws an empty line → stays "Premium" until
then).

> **[partly served / gap]** The **market-series contract + server frames are built** — the
> `StreamSession` carries `MarketSeriesSubscribe` (`celnet-proto`/`celnet-server`); every
> non-Premium mode now gates only on the **live market-series VALUES feed** (SCALE #11), not the
> wire. SPOT additionally needs a brand-new spot feed (none in the transport today; PairStrip
> shows spot
> statically by its own comment).

---

## 8. Reconciled phased backlog

Merged + de-duplicated across RISK-HIERARCHY (RH), TRADING-UNIVERSE-SCALE (US),
SURFACE-WORKFLOW (SW), and the GUI audit (AUD). One line each: scope · crate/area · dep ·
source. **(Does not edit ROADMAP — the orchestrator merges backlogs.)**

### Phase 0 — quick wins / bug fixes, implementable on **today's** data
*All GUI-only unless noted; no new feed required.*

| # | Task | Area | Dep | Source |
|---|---|---|---|---|
| P0-1 | **Fix surface mismark**: Re-mark ≠ Publish; transport the trader's edit; publish under fresh `surface_version` | `gui/` (+confirm proto/server accepts edit) | — | SW#3, AUD |
| P0-2 | **Wire real calendar-arb gate**: replace hardcoded `calendarArbitrageFree:true`; surface `SurfaceArbitrageReport`; Publish gates on whole report | `gui/` (engine report exists) | — | SW#4, AUD |
| P0-3 | **All surface handles editable** + keyboard-nav + nudge steps (25RR/25BF/10RR/10BF stop being read-only) | `gui/` | P0-1 | SW#1, AUD |
| P0-4 | **Labelled "Premium" TrendMode** + single consistent direction; STOP the cross-structure blend | `gui/` | — | US#17, AUD |
| P0-5 | **Book→Risk drill** over current positions; Risk consumes drilled instrument (not hardcoded 25Δ RR); Ticket "Add to risk" passes the instrument | `gui/` | — | RH§2.5, AUD |
| P0-6 | **Scope breadcrumb showing-all**: `ScopeContext` in AppContext, "Firm · all desks · all books" root chip, breadcrumb-as-filter wired to current data | `gui/` | — | RH§6, AUD |
| P0-7 | **Analytics/model selector** where engine supports it (VV/SABR/SVI/SSVI on Surface; shock axes on Risk) + provenance line | `gui/` | — | SW§7.2, AUD |
| P0-8 | **Σ-column / "Mid" clarity**: label premium mid + unit; re-glyph Book so Σ = ladder only | `gui/` | — | US§5.5, AUD |
| P0-9 | **Surface chart fixes**: stable y-axis (no auto-fit); honest "marks@5 handles" labelling for the 9-vs-5 mesh mismatch; remove float-equality selection | `gui/` | — | SW§4.2, AUD |
| P0-10 | **Ticket rate fix**: `describeLegs` uses `app.pairCtx.market`, not hardcoded `rDom 0.04 / rFor 0.02` | `gui/` | — | AUD §3.9 |
| P0-11 | **Stop workspace remount-on-switch** (`key={app.workspace}`); persist Risk/Book state | `gui/` | — | AUD §3.10 |
| P0-12 | **ON/TN/SN tenor labels** in display (don't *resolve* yet) so "ON" stops silently reading as SN in labels | `gui/` (label-only; resolution = P1) | — | US, SW |

### Phase 1 — needs a new **feed / contract**
| # | Task | Area | Dep | Source |
|---|---|---|---|---|
| P1-1 | **Streamed market-series feed** (ATM-vol/spot/RR/forward time-series in the wire contract) — unlocks all non-Premium TrendModes | `celnet-proto`+`celnet-server` (coordinate) | — | US#11 |
| P1-2 | **Tenor model overhaul** (ON/TN/SN, IMM, EOM, broken-date) — **fix ON-resolves-as-SN bug** | `celnet-types`+`celnet-calendar` (coordinate) | — | US#4, SW#9, AUD |
| P1-3 | **IMM resolver** (3rd-Wed Mar/Jun/Sep/Dec) | `celnet-calendar` | P1-2 | SW#9 |
| P1-4 | **Delivery-led scheduling policy flag** + month-end golden test | `celnet-calendar` | P1-2 | US#5 |
| P1-5 | **Event/turn/fixing registries + `EventClock` + business clock** (day-weighting, total-variance, validated vs QuantLib) | `celnet-surface`+`celnet-calendar` | P1-2,P1-4 | US#6, SW#8 |
| P1-6 | **Broken-date / event-aware ticket** (dual-mode tenor⇄date, two clocks, mark-to-impact) | `gui/`+engine | P1-2,P1-3,P1-5 | SW#10, US#16 |
| P1-7 | **Consensus / composite surface feed** (intraday composite + month-end Totem-style) → compare/history/overlay | `celnet-integration` (CELER-INTEGRATION) | — | SW#6 |
| P1-8 | **Book/owner identity + `AttributionRecord`** (human seat OR auto-pricer; LP competition; `surface_version`) + auto-pricer governance | `celnet-observability`+`celnet-server` | — | US#12, feeds RH |
| P1-9 | **Surface contract evolutions** (quoted-BF vs smile-strangle; surface-role; market-context; arb-report cross-tenor fields) | `celnet-proto` (coordinate) | — | SW#11 |
| P1-10 | **Pair-universe registry + liquidity tiers**; settlement/clearing/CNH≠CNY/metals attributes | `celnet-types`+`celnet-conventions` (coordinate) | — | US#1,#2,#3 |

### Phase 2 — scale / infra
| # | Task | Area | Dep | Source |
|---|---|---|---|---|
| P2-1 | **Virtualised blotter + server-side aggregation** (row/col virtualisation, async tx batching, frame budget vs stream rate) | `gui/`+`celnet-server` | P2-4 | US#14, AUD |
| P2-2 | **Universe navigator** (search/region/G10-vs-EM/liquidity-tier/favourites/multi-pair) on the ⌘K spine; decouple the one global `pairCtx` | `gui/` | P1-10 | US#13, AUD |
| P2-3 | **Vol-cube store + dirty recalibration** (`SurfaceCube` + dependency graph) + pair×tenor×delta pivot/heatmap (GPU-instanced on `viz/SurfaceMesh`) | `celnet-surface`+`gui/` | P1-5 | US#7,#15 |
| P2-4 | **Conflated multi-pair fan-out + cross-fleet fan-out bench** (gate the latency headline) | `celnet-server`+`celnet-engine`+`celnet-router`+`celnet-bench` | SCALE-OUT | US#9,#10 |
| P2-5 | **`celnet-risk-cube`**: OLAP fact store + group-by/reduce; additive roll-up index + per-node non-additive re-derivation (VaR/ES, FRTB curvature, corr-weighted vega) | `celnet-risk-cube` (new) | P2-4 | RH§3.2 |
| P2-6 | **`celnet-risk-normalize`**: convention canonicalization + common-numeraire conversion (fixes Book native-units caveat) | `celnet-risk-normalize` (new) | P2-5 | RH§2.2/§2.3 |
| P2-7 | **`celnet-limits`**: limit tree, utilization, pre/post-trade checks, breach/escalation; overlay on vega pillars | `celnet-limits` (new) | P2-5 | RH§5 |
| P2-8 | **`celnet-entitlements` + user-admin**: entitlement model + server-side pre-aggregation pruning; swap the grant-all principal for real principals (zero GUI rework) | `celnet-entitlements` (new) + `gui/` admin | P2-5 | RH§4 |
| P2-9 | **Vectorised/GPU materialise & scenario + adjoint (AAD) Greeks** — replaces bump-and-revalue so what-if/VaR cadence is real | `celnet-gpu`+`celnet-engine` | P2-3 | US#8, RH§3.3 |
| P2-10 | **Min-perturbation no-arb repair** (projection onto no-arb set) + override-layer model (base + tracked nudges + macro edits, revertible/attributable) | `celnet-surface` | P1-9 | SW#12,#13 |
| P2-11 | **Reg-data feed** (SIMM/FRTB weights, externally-supplied, versioned, never compiled-in) for FRTB/SIMM bucket display | `celnet-integration` | P2-5 | RH§2.3 |

---

## 9. Honest gaps & cross-doc conflicts reconciled

1. **The aggregation layer is built and served, and every client lane consumes it.**
   `celnet-risk-cube` / `-normalize` / `-limits` / `-entitlements` are built and gated green
   (RH §3.2) and served over the one `celnet-proto` contract as `RiskService`
   (`ListPositions`/`AggregateRisk`/`DrillRisk`/`LimitStatus`) by `celnet-server` (gRPC + WS
   mirror, shared live `PositionStore`, entitlement pruning pre-reduction, reporting-numeraire
   collapse). Book/Risk/Limits roll-up is a real server capability today, and **all three client
   lanes now consume it**: the GUI Book/drill (the client-side `aggregateBookRisk` loop is
   **deleted** — `gui/src/data/portfolioRisk.ts` removed), the `celnet-client` SDK, and the Excel
   `CELNET.*` functions — full API-first parity, no client-side position-loop-and-sum anywhere.
   Backend deferrals (only): AAD/GPU non-additive reval + cross-shard fleet (RH §3.3/§3.4).

2. **Scale-out tier: algebra built, absolute cross-host wire deploy-gated.** `celnet-router`
   HRW partition map (`map.rs`/`hash.rs`) and the `celnet-risk-fleet` cross-shard reducer
   **are built** — firm-wide fan-out is proven **== single-node aggregate to 1e-12** in-process
   / on localhost multi-process, and blue-green handoff *is* built. The honest residual is the
   **absolute cross-host wire p99 / cross-DC transport**, which stays deploy-gated (the in-repo
   proof is correctness/quorum/framing on loopback only).

3. **Throughput substrate (AAD / batched-GPU) is built.** The hierarchical-scale claim would be
   hollow if it rested on bump-and-revalue (RH §3.3, the original top technical risk); that
   substrate now exists — reverse-mode **AAD adjoint Greeks** (`celnet-vanilla/adjoint.rs`,
   risk-cube sensitivity-based VaR/ES lens) and the **GPU batch closed-form / scenario kernels**
   (`celnet-gpu`, ratios only on M4 — NVIDIA absolutes deploy-gated). The what-if/scenario/VaR
   UX should still signal additive (instant) vs non-additive (recompute-on-demand) measures so
   the cadence stays honest.

4. **No portfolio-roll-up latency number exists.** Celnet must publish its **own** budget
   (target low-single-digit µs for the additive-Greek limit path); the ~4µs figure is a
   FIX-stack number, not options-reval, and must not be reused for the cube (RH §3.5).

5. **TrendMode is feed-split.** Premium ships now; ATM_VOL/RR/BF/FORWARD gate on the
   market-series feed (US#11); SPOT additionally needs a brand-new spot feed. The data is
   honest today — the defect is semantics/label/direction, fixed in P0 without any feed.

6. **Surface overlays need feeds that don't exist.** Composite (intraday) and consensus
   (month-end Totem-style) are a CELER-INTEGRATION dependency — overlay/compare must not
   be rendered as if a feed exists (SW §3.4). Reconciled: P1-7, behind an honest
   empty-state until the feed lands.

7. **Event-clock realism.** No concrete `EventClock` exists (only an identity
   `CalendarClock`); event-vol magnitudes are illustrative placeholders, never Celnet
   defaults; needs QuantLib/reference validation and an *open/free* event-calendar source
   (a commercial feed would violate guardrail #7). ATM-only event bumps at v1 (FX wing
   reaction literature thin). (SW §5.3.)

8. **Surface representation conflict, resolved.** TRADING-UNIVERSE-SCALE says the mesh is
   "the right substrate for the **cube heatmap**" (GPU-instanced cells); SURFACE-WORKFLOW
   says the mesh is **not a marking tool** and must be demoted. **No contradiction:** the
   *marking* surface is the editable delta×tenor **grid** (mesh demoted to a QC panel);
   the *firm-wide pair×tenor×delta* visualization is a **separate Cube heatmap** that
   reuses the GPU mesh substrate. Two different jobs, two different surfaces.

9. **Convention is not a toolbar choice.** The cube is convention-free — every leaf is
   canonicalized before aggregation (RH §2.2); the toolbar carries reporting *views*
   (numeraire, premium-adjusted reconstruction), not internal convention. This keeps
   roll-up math convention-stable while the Surface workspace still exposes the *marking*
   conventions (delta/ATM/strangle) on its provenance line.

10. **Competitor "uniqueness" claims are arguments-from-absence.** Any positioning that
    Celnet's hierarchical FX-options roll-up / vanna-volga-as-dimension / limit-tree is
    unique is flagged **[inf]** — Murex, Numerix, Calypso/Adenza, ION XTP, Synoption all
    have *credited adjacent* capability (RH §7). Vendor pair counts and AG-Grid 150k/s are
    **[high]**; liquidity-tier counts and reval magnitudes are **[inferred/low]**;
    "Bloomberg synthetic surfaces from correlation" and "Murex GPU MC" are **unverified/
    mis-attributed** and so flagged (US confidence flags).

11. **Interface-crate coordination.** P1-1, P1-2, P1-9, P1-10 touch the frozen interface
    crates (`celnet-types` / `celnet-proto` / `celnet-conventions`) — they must be
    coordinated under the parallel-session interface-crate discipline (CLAUDE.md §parallel
    model), not changed unilaterally.
