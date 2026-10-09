# GW0 + GW1 — GUI foundation: staged execution plan

> The load-bearing first two waves of [GUI-EXPERIENCE-DESIGN.md](../../clients/GUI-EXPERIENCE-DESIGN.md) §4.
> GW0 builds the **design-system + a11y substrate** (density token axis; shared
> `<Provenance>`/`<EmptyValue>`/`<StdError>`/`<InspectorStrip>` primitives; one accessible
> virtualised groupable `<DataGrid>` with `role=grid`, roving-tabindex, column virtualisation
> and tick-coalescing — **reversing** the `StreamWorkspace.tsx:698-703` ARIA-grid opt-out).
> GW1 builds the **three-axis navigation + cohesive shell** (single breadcrumb scope control
> absorbing the 4 redundant pair affordances; data-driven `⌘N`; the `Σ→▤` glyph fix;
> `lib/shortcuts.ts` promoted to the real binding registry; saved-views URL+localStorage).
>
> **Zero legacy (GUIDE.md rules 9/10):** every component named below as superseded is
> **decomposed/replaced, not paralleled**. One clean current contract, GUI included.
>
> **Scope boundary.** GW6 (multi-asset GUI) is gated on the master program's W1 core-contract
> wave and is **out of this plan**. Everything here is **FX-default** and FX-only-populated;
> the seams (density attr, scope dimension list, `<DataGrid>` row model) are shaped so the
> multi-asset future is a data/registry change, never blocking on it. No `Underlier` union,
> no asset-class axis is introduced here — that is GW6.
>
> **Honesty contract.** Tags per GUI-EXPERIENCE-DESIGN.md §intro: **[built]** verified in code
> this plan-cycle; **[proposal]** new in this wave; **[ENV]** deploy-bound, never claimed
> in-repo. Empty states say `—`; provenance is on the face. No fabricated numbers.

---

## 0. The crux risk & the governing principle

The blast radius is the **shell chrome + every grid**: `Shell.tsx` hardcodes the rail
(`RAIL` array + `WORKSPACE_VIEWS`), the keyboard grammar (the inline `onKey` handler,
`⌘1..5` capped, `⌘P` aliasing `⌘K`, `⌘B` → navigator), and the four redundant pair
affordances (`PairMenu` + the "Pairs" `navigatorBtn` + `⌘K` pair list + `PairStrip` +
`UniverseNavigator` overlay). `AppContext.tsx` hardcodes `groupBy:'none'` (line 289) and a
`ScopeLevel = firm|desk|book|pair` with `ScopeBreadcrumb` that **only drills up**. Four grids
(Stream virtualised + ad-hoc-ARIA; Book/Cube/Risk non-virtualised tables) each speak a
different table dialect; `StreamWorkspace.tsx` **deliberately opts out of `role=grid`** with a
documented rationale ("a half-claimed grid is an a11y anti-pattern").

**Governing principle:** *the substrate lands before the chrome consumes it.* GW0 ships the
primitives **with their own tests green** and migrates ONLY `StreamWorkspace` onto `<DataGrid>`
(the highest-risk reversal — proves the grid keyboard model + axe-with-`role=grid` on the
hardest case). GW1 then rewires navigation/scope/shell to compose from GW0. The reversal of the
ARIA opt-out is **only valid if the real grid keyboard model exists** — so the opt-out comment
and the `role=grid` claim move together, in GW0-S3, gated by axe + a Playwright keyboard
cell-nav e2e. No chrome wave claims `role=grid` before that test is green.

This is GUI-only. **No Rust crate, no `Cargo.toml`, no `celnet.proto`, no SDK/CLI/Excel source
is edited.** Cross-client parity here means: the FX vanilla/strategy capabilities the migrated
grids and scope drill expose still price identically across server/SDK/CLI/Excel/GUI against
the frozen `crates/celnet-golden/vectors/*.json` corpus (the existing
`gui/test/conformance.test.ts` lane + the 5-client harness), i.e. **no regression** — these
waves surface existing capabilities through better chrome, they add no new priced product.

---

## 1. Target module layout (gui/src)

New (GW0 primitives + GW1 navigation), all under the existing flat `gui/src`:

```
gui/src/design/
  density.ts            # [proposal] density axis: "compact"|"comfortable", data-density attr,
                        #            localStorage, mirrors appearance.ts; tokens in tokens.css.
gui/src/components/
  Provenance.tsx        # [proposal] <Provenance source=… model=… asOf=… /> — the on-face
                        #            calibration/source line (extracts the SurfaceWorkspace pattern).
  EmptyValue.tsx        # [proposal] <EmptyValue/> → "—" with an accessible reason title.
  StdError.tsx          # [proposal] <StdError value=… /> → "± x" for MC families; null ⇒ nothing.
  InspectorStrip.tsx    # [proposal] the per-Panel-header analytics-config shell: declarative
                        #            segments (model|measures|axes|trend|columns|density|saved-view);
                        #            lanes populate only what applies; <Provenance> always rendered.
  DataGrid.tsx          # [proposal] role=grid, roving-tabindex (2-D arrow nav + Home/End/PageUp/Dn),
                        #            column virtualisation, fixed-row windowing (reuses lib/virtual.ts),
                        #            groupable (collapsible group rows = role=row aria-expanded),
                        #            tick-coalescing to a frame budget. The ONE grid every lane uses.
gui/src/lib/
  grid.ts               # [proposal] pure grid model: ColumnDef<T>, GroupModel, RovingState reducer,
                        #            coalesce(updates, frameBudget) — unit-tested headlessly (no DOM).
  commands.ts           # [proposal] the command/keybinding REGISTRY (GW1): CommandId → {run, keys,
                        #            group, when}; lib/shortcuts.ts becomes a projection OF this.
  savedViews.ts         # [proposal] (scope × view-arrangement × analytics) ⇄ URL search-params +
                        #            localStorage; pure codec + a useSavedViews hook.
  scope.ts              # [proposal] the scope-dimension list + drill-up/down/pin reducer over the
                        #            FX-default path; generalises AppContext's ScopeLevel.
```

Decomposed / replaced (no-legacy — **deleted**, not paralleled):

| File | Fate in this plan |
|------|-------------------|
| `StreamWorkspace.tsx` (the inline grid: header track, `body` windowing, ARIA opt-out at :698) | grid markup + `useVirtualWindow` body **replaced** by `<DataGrid>`; the opt-out comment + non-`role=grid` `gridRegion` deleted (GW0-S3). |
| `PairMenu.tsx` + `PairStrip.tsx` + `UniverseNavigator.tsx` overlay + the `navigatorBtn` "Pairs" button + the `⌘K` `PAIRS.map` command group (`Shell.tsx:90-96`) | the **4 redundant pair affordances** collapse into ONE breadcrumb-scope control whose terminal crumb is underlier selection; the Universe becomes the **leaf drill view** of the breadcrumb, not an overlay (GW1-S3). `PairStrip`/`PairMenu` deleted; `UniverseNavigator` re-homed as a rail view body. |
| `ScopeBreadcrumb.tsx` (drill-**up**-only; `groupBy` hardwired `'none'`) | **replaced** by a drill up+down+pin breadcrumb reading `lib/scope.ts` (GW1-S2). |
| `Shell.tsx` inline `onKey` handler + hardcoded `RAIL`/`⌘1-5` | **replaced** by a data-driven rail (`⌘1..n`) + the `lib/commands.ts` registry the Shell merely dispatches (GW1-S1/S4). |
| `lib/shortcuts.ts` (static documentation list) | **demoted** to a render projection of `lib/commands.ts` (single source) so the `?` cheatsheet is generated and cannot drift (GW1-S4). |
| `Shell.tsx:41` `Book → Σ` glyph | `Book → ▤`, freeing `Σ` for sum/ladder exclusively (GW1-S1). |

No new npm dependency (the design system's "dependency-free virtualiser" discipline is kept;
`lib/grid.ts` is pure TS, `<DataGrid>` reuses `lib/virtual.ts`).

---

## 2. Per-area design + the INDEPENDENT verification oracle

The GUI is not a numeric pricer, so the VERIFICATION-CONTRACT gate (a) "independent oracle"
maps to **behavioural oracles that can disagree** — a property/limit/structural check reached
by a route disjoint from the implementation, plus the existing **golden-vector corpus** as the
external numeric truth for cross-client parity. The FRTB-`0.75ρ` lesson applies verbatim: the
test must not re-encode the component's own logic.

| Area | What lands | INDEPENDENT oracle (can disagree) | Circular-oracle risk & mitigation |
|------|-----------|-----------------------------------|-----------------------------------|
| `<DataGrid>` keyboard model | role=grid, 2-D roving-tabindex, Home/End/Page nav, group collapse | **WAI-ARIA APG "grid" pattern** as the spec text: exactly one cell `tabindex=0` at a time, arrow moves focus, `aria-rowcount/rowindex` reflect the FULL (not windowed) set. Assert against the **APG invariants hand-encoded from the spec**, NOT against the reducer's own output. Plus **axe-core** (third-party) on the live grid with `role=grid` claimed. | If the test imports `RovingState` and replays it, it mirrors the bug. Mitigation: drive the rendered grid via keyboard events (Testing Library) and assert DOM `tabindex`/`aria-*`, never the reducer internals; pin the APG invariants as literal constants. |
| Column/row virtualisation | only in-view cells in DOM; spacers carry off-screen extent | **Conservation invariant**: `renderedRows + headHidden + tailHidden == totalRows` and `Σ heights == count*rowHeight` — an algebraic identity disjoint from the windowing arithmetic. Off-screen rows assert **absent from DOM** (count nodes). | The `lib/virtual.ts` math is reused; do NOT oracle the window against the same floor/ceil. Oracle is the conservation identity + DOM node-count, which holds for any correct windower. |
| Tick-coalescing | bursty updates collapse to ≤1 paint/frame, last-write-wins per cell | **Accounting identity** (the existing `celnet-fanout` conflation discipline): `applied + coalesced == produced`, and the final rendered value per cell == the **last** produced value (replayed independently in-test). Frame-budget honoured: paints ≤ frames. | The coalescer's own "latest" map could hide a drop. Oracle: feed a known burst, reduce it independently in the test (plain reduce-to-last), assert equality + the accounting sum. |
| Density token axis | `data-density="compact"&#124;"comfortable"` on `<html>`, row-height/spacing tokens cascade | **Cascade disjointness**: assert computed `--row-h`/`--space-*` differ between modes and that NO component reads density in JS (grep-gate: only the attribute + tokens move). Mirrors `appearance.ts` proven pattern. | n/a (no numeric formula). Risk is a component hardcoding a height; mitigated by the lint-style test asserting grids read the token. |
| `<Provenance>`/`<EmptyValue>`/`<StdError>` | shared honest-data primitives | **Honest-data property**: a null/absent input renders `—` (never `0`, never blank); an MC `price_std_error>0` renders `± …`, a closed-form (`null`) renders nothing. Oracle = the corpus record's own `price_std_error` field (external truth) drives the assertion. | Risk: the primitive and test both treat `0` as present. Mitigated by pinning expected output from the **golden vector's** `price_std_error` (external), and a property over `{null, 0, >0}`. |
| Scope drill (up/down/pin) | breadcrumb over `firm→desk→book→pair` (FX terminal = pair), drill both ways + pin | **Path-algebra invariants**: drill-down then drill-up to ancestor is idempotent to that ancestor; pinning a dimension is order-independent; the FX terminal crumb equals the active pair (round-trips `setPair`). Reached by a **disjoint reference reducer** written from the breadcrumb spec, plus the existing `gui/test/riskScope.test.ts` style. | The reducer-vs-reducer trap. Mitigation: the reference is a trivial list-truncation/append model the production reducer is NOT allowed to import; plus a Playwright e2e drilling a real path over Stream/Book/Risk. |
| Saved-views codec | `(scope, view-arrangement, analytics)` ⇄ URL + localStorage | **Round-trip identity**: `decode(encode(s)) deep-equals s` over a generated state space; **forward-compat**: unknown keys ignored, missing keys default (no throw). URL is the canonical form; localStorage mirrors. | Encoder/decoder sharing a typo. Mitigation: hand-pin ≥3 **literal** URL strings ⇄ expected state (external fixtures, like a golden vector), so a symmetric bug in both halves is caught by the frozen fixture. |
| Command registry / `⌘N` | data-driven rail (`⌘1..n`), `lib/commands.ts` single source, `?` generated from it | **Single-source invariant**: every binding the Shell dispatches exists in the registry and vice-versa (no orphan handler, no advertised-but-dead key); `?` cheatsheet == registry projection; no chord collision (a uniqueness assertion over `keys`). | The old drift between `shortcuts.ts` (advertised) and `Shell.tsx` (honoured). Mitigation: there is now ONE source; the test asserts the cheatsheet renders exactly the registry and a Playwright e2e exercises each advertised global chord. |

---

## 3. Staged GREEN increments (single driver; each commit gated; NOT parallel lanes)

Each step keeps `gui/` building (`npm run build` = `tsc -b && vite build`), `tsc` typecheck +
test + e2e configs green, vitest green, and — at the wave milestones — Playwright real-edge
e2e + axe + the 5-client conformance harness green. The Rust `just check` is unaffected
(no Rust touched) but is run at each milestone to confirm the literal `All gates passed.`

### GW0 — design-system + a11y foundation

- **GW0-S1 (honest-data + density primitives, additive):** add `design/density.ts` +
  density tokens to `tokens.css` (`data-density` cascade, mirroring `data-appearance`); add
  `<Provenance>`/`<EmptyValue>`/`<StdError>`. Nothing consumes them destructively yet.
  *Gate:* vitest — honest-data property over `{null,0,>0}` (std-error pinned from a
  `vectors/*.json` MC record); density cascade test (computed tokens differ, no JS density
  read); `tsc` + `vitest run` green. Commit.
- **GW0-S2 (the grid model + `<DataGrid>`, not yet wired):** add `lib/grid.ts`
  (`ColumnDef`, `GroupModel`, roving reducer, `coalesce`) + `<DataGrid>` + `<InspectorStrip>`
  shell. Headless unit-tested; rendered in a test harness only.
  *Gate:* vitest — APG roving invariants (hand-pinned from the WAI-ARIA grid spec, asserted on
  rendered DOM via Testing Library, not the reducer); virtualisation conservation identity +
  off-screen-absent node count; tick-coalescing accounting (`applied+coalesced==produced`,
  last-write-wins) + frame-budget. Commit.
- **GW0-S3 (REVERSE the ARIA opt-out — migrate StreamWorkspace onto `<DataGrid>`):** delete
  the inline header/body grid + the `gridRegion` non-`role=grid` opt-out comment in
  `StreamWorkspace.tsx`; render through `<DataGrid>` with `role=grid` claimed; group-by
  Pair/Tenor becomes the grid's group model; column toggles + sort move to the
  `<InspectorStrip>`/grid header. **This is the highest-risk reversal — it proves the grid on
  the hardest live case.**
  *Gate:* vitest — Stream rows/group/sort/column behaviour preserved; **axe with `role=grid`
  claimed = zero serious/critical** (the opt-out is only valid to reverse once this passes);
  Playwright real-edge e2e — keyboard cell-nav across the live blotter + click-to-trade still
  hits + high-rate stream stays within the measured render-P99 (host-local **ratio** only,
  per §5 ENV); `e2e/a11y.e2e.ts` Stream sweep green. **GW0 milestone:** `just check` prints
  `All gates passed.`; GUI vitest + Playwright e2e + axe green; conformance lane green
  (Stream surfaces the same priced vanilla/strategy — no numeric regression). Commit + push.

### GW1 — three-axis navigation + cohesive shell

- **GW1-S1 (data-driven rail + glyph fix, additive seam):** lift `RAIL`/`WORKSPACE_VIEWS` to a
  data-driven registry; `⌘1..n` (uncap `⌘1-5`); `Book → ▤` (free `Σ`). Behaviour identical;
  no nav semantics change yet.
  *Gate:* vitest — rail renders N views with correct `⌘N` hints; glyph table one-glyph-one-
  meaning (no `Σ` on Book); existing `shellShortcuts.test.tsx` adapted. Commit.
- **GW1-S2 (scope reducer + drill up/down/pin breadcrumb):** add `lib/scope.ts`; replace
  `ScopeBreadcrumb` with the drill-up+down+pin control; wire `AppContext` `groupBy` to the
  scope path (delete the hardwired `'none'`).
  *Gate:* vitest — path-algebra invariants vs the disjoint reference reducer (drill round-trip
  idempotence, pin order-independence, FX-terminal == active pair); `riskScope.test.ts` style
  preserved. Commit.
- **GW1-S3 (collapse the 4 pair affordances → one scope control + Universe-as-leaf-view):**
  delete `PairMenu`, `PairStrip`, the `navigatorBtn` "Pairs" button, and the `⌘K` `PAIRS.map`
  group; re-home `UniverseNavigator` as the **leaf drill view** of the breadcrumb (a first-
  class view, not an overlay); the terminal scope crumb opens it. Universe stays FX-pair-
  populated (no asset-class axis — that is GW6).
  *Gate:* vitest — exactly one pair affordance remains (assert the others are gone, no-legacy);
  Universe reachable as the breadcrumb leaf; `tsc` clean (dangling imports removed);
  Playwright e2e — drill firm→…→pair selects the pair and re-targets the global market;
  `e2e/a11y.e2e.ts` updated (the navigator a11y test re-pointed at the leaf view). Commit.
- **GW1-S4 (command registry + generated cheatsheet + saved-views):** add `lib/commands.ts`
  as the single binding source; demote `lib/shortcuts.ts` to its projection; the Shell `onKey`
  becomes a registry dispatcher; `?` cheatsheet generated FROM the registry; `⌘P` becomes a
  real scope/underlier switcher distinct from `⌘K`. Add `lib/savedViews.ts` + URL/localStorage
  wiring; a saved view captures `(scope, active view, inspector analytics)`.
  *Gate:* vitest — single-source invariant (every dispatched key ∈ registry & vice-versa; no
  collision; cheatsheet == projection); saved-view round-trip identity + 3 hand-pinned literal
  URL⇄state fixtures + forward-compat (unknown keys ignored). **GW1 milestone:** `just check`
  `All gates passed.`; Playwright real-edge e2e — drill-down across Stream/Book/Risk over one
  scope path + **recall a saved view from a URL** + `⌘K` command index exercised; axe on all
  new chrome (rail, breadcrumb, Universe leaf view) zero serious/critical; conformance + 5-
  client harness green. Commit + push. lodestar auto-indexes; verify scope with detect_changes; reconcile
  GUI-EXPERIENCE-DESIGN §3 (areas 1/10/11 now [built]) + CLIENT-PARITY-MATRIX (unchanged
  numerics, chrome only).

---

## 4. Cross-client surfacing (FX default; no new priced product)

GW0/GW1 add **no new wire capability** — they re-chrome existing FX vanilla/strategy/surface/
risk flows. Cross-client parity therefore means **no regression**: the capabilities the
migrated grids and scope drill expose remain reachable and identical from SDK/CLI/Excel and
price equal to the frozen `crates/celnet-golden/vectors/*.json` corpus.

- **GUI:** Playwright real-edge e2e (boots `gui/e2e/demoEdge.ts`) drives the new chrome over a
  real edge; `gui/test/conformance.test.ts` keeps the GUI's offline pricer == frozen vectors
  within its declared band (unchanged).
- **SDK / CLI / Excel:** unchanged source; the existing 5-client conformance harness (master
  §4 / VERIFICATION-CONTRACT (d)) must stay green at each milestone — proving the chrome change
  did not alter what the GUI requests of the edge. `CLIENT-PARITY-MATRIX.md` is regenerated
  from the passing harness (never hand-edited); rows are numerically unchanged.
- **FX-default everywhere:** the scope terminal crumb is a `CcyPair`; the Universe leaf view is
  FX-pair-populated; no `Underlier` union or asset-class axis is introduced (GW6 owns that).

No new golden vector or `celnet-parity` row is required by `tools/check-verification-coverage.mjs`
because **no proto oneof arm is added** — the coverage lint's family set is unchanged. (The two
known lint gaps — `strategy`/`american` parity rows — are master-program W0/backlog items, not
GW0/GW1 work; this plan must not regress the lint.)

---

## 5. Honest boundary (in-repo vs deploy/ENV)

In-repo these waves prove: the grid keyboard model, accessibility (axe zero serious/critical
with `role=grid` claimed), virtualisation/coalescing behaviour, honest-empty/provenance
discipline, scope/saved-view serialisation, GUI==server numeric parity, and a **host-local
render-P99 ratio** for the high-rate stream. **[ENV/deploy-bound, never claimed in-repo]**
(unchanged from GUI-EXPERIENCE-DESIGN §7): live multi-dealer LP-panel / venue connectivity &
regulated status, live feed VALUES, cross-host wire p99, live JVM CelNet estate lifecycle. The
high-rate stream render budget is asserted as a **ratio against the measured baseline on this
host**, never as an absolute cross-host SLO. No `VITE_`/runtime ENV flag is introduced by these
waves beyond the existing `?mock` transport toggle and the e2e `wsUrl` discovery; the design-
system attributes (`data-density`/`data-appearance`/`data-contrast`) and saved-view URL params
are client-local state, not deploy gates.

---

## 6. Out of scope (later GUI waves, seamed here)

GW2 (Ticket → ProductSpec registry), GW3 (Stream LP-competition depth + RFQ escalation — the
`<DataGrid>` substrate lands here but the LP payload is GW3), GW4 (Surface/Cube), GW5
(Risk/Capital catalogue + Lifecycle), **GW6 (multi-asset GUI — gated on master W1)**, GW7
(convergence). GW0/GW1 deliver the substrate (one `<DataGrid>`, one `<InspectorStrip>`, one
scope grammar, one command registry, density + honest-data primitives) so those are
composition/registry additions, not rewrites. Deploy-bound items stay ENV per §5.
