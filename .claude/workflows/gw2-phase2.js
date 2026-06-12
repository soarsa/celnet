export const meta = {
  name: 'gw2-phase2-components',
  description: 'GW2 phase 2 fan-out: build the disjoint NEW ticket components — the vanilla/strategy leg-ladder ProductSpec, the grouped/searchable StructureGallery, the payoff-at-expiry mini-chart, and the multi-leg net-premium/Greek strip — each standalone-gated in the lane/gw2 worktree. The coordinator then rewires TicketWorkspace onto these + the registry and deletes the monolith.',
  phases: [{ title: 'Components', detail: 'four disjoint new components + tests' }],
}

const RESULT = {
  type: 'object',
  additionalProperties: false,
  required: ['component', 'files_created', 'public_api', 'gate_line', 'zero_workarounds', 'integration_notes'],
  properties: {
    component: { type: 'string' },
    files_created: { type: 'array', items: { type: 'string' } },
    public_api: { type: 'string', description: 'exported names + their prop/return signatures for the coordinator to wire' },
    gate_line: { type: 'string' },
    zero_workarounds: { type: 'boolean' },
    integration_notes: { type: 'string' },
  },
}

const WT = '/Users/adrian/code/celnet-gw2'

const COMMON = `
You are a GW2 phase-2 worker on the Celnet trader GUI. WORK ONLY in ${WT} (branch lane/gw2-structuring). Always: cd ${WT}/gui
Create ONLY your own NEW files (listed in your task). DO NOT edit gui/src/workspaces/TicketWorkspace.tsx, gui/src/products/index.ts, or any other worker's files. DO NOT git add/commit — the coordinator wires + commits.
READ FIRST: gui/src/products/index.ts + types.ts (the registry API: PRODUCT_REGISTRY, specById, registryByGroup, ProductSpec<I> { id,label,group,assetClass,summary,keywords,kind,defaults,allowedModels,toInstrument,InputBlock }, ProductBuildCtx, AnyProductSpec). gui/src/products/asian.tsx (a migrated family). gui/src/components/* (the design-system atoms: Panel, Button, etc.) and gui/src/workspaces/TicketWorkspace.module.css for class names. Match the existing code's import style, strict TS, accessibility (role/aria), and honest-data discipline (empty => "—", no fabricated numbers).
QUALITY BAR: SOTA, zero workarounds (no any/@ts-ignore/eslint-disable/lowered assertion/TODO/placeholder). Strict TS must pass.
GATE (run, capture literal output): cd ${WT}/gui && npx tsc -p tsconfig.app.json --noEmit ; echo TSC=$? ; then npx vitest run <your test file(s)>.
Return the structured result. Your FINAL message IS the result.
`

const WORKERS = [
  {
    component: 'strategy-spec',
    prompt: `Build gui/src/products/strategy.tsx — the vanilla + multi-leg strategy family as ProductSpec(s), the last family to migrate out of the monolith (the leg-ladder UI).
Read in gui/src/workspaces/TicketWorkspace.tsx: the leg-ladder render (the NON-legless branch, around the <select> at ~1595 onward, plus LegView ~1827), the vanilla/strategy cases in buildInstrument (~1047: vanillaInstrument(pair,tenorYears,"CALL",0.25,notionalMm) default + strategyInstrument(pair,tenorYears,structure,notionalMm)), and how legs are held in state.
Deliver: a StrategyInputs shape (the editable legs: optionType/strike(delta|level)/side/ratio per leg, for the RISK_REVERSAL/STRANGLE/STRADDLE/SEAGULL templates + a free VANILLA single-leg), DEFAULT per structure, a self-contained leg-ladder InputBlock (controlled via {value,onChange,ctx} — same module.css classNames/aria as the monolith), and exported ProductSpecs for VANILLA + the 4 strategy kinds (ids VANILLA/RISK_REVERSAL/STRANGLE/STRADDLE/SEAGULL, kind "vanilla" resp "strategy", group "Vanilla & strategies"), each toInstrument reproducing the monolith's buildInstrument output BYTE-FOR-BYTE via vanillaInstrument/strategyInstrument + withTenorAndModel. If the strategies are template-fixed legs (not freely edited) in the monolith, preserve that exactly — do not invent editability the monolith lacks; expose what it exposes.
Tests gui/test/products/strategy.test.ts: each spec's defaults build a wire-valid instrument with product.kind === spec.kind, round-trip via instrumentToWire, allowedModels === bookingModelsFor(kind).`,
    test: 'test/products/strategy.test.ts',
  },
  {
    component: 'structure-gallery',
    prompt: `Build gui/src/products/StructureGallery.tsx — a grouped, searchable, fully-accessible structure picker that REPLACES the flat 19-item <select> (TicketWorkspace ~1434).
Props: { value: string; onSelect: (id: string) => void; specs?: readonly AnyProductSpec[] (default PRODUCT_REGISTRY) }. Render registryByGroup(): a group heading per ProductGroup, then each spec as a selectable card showing label + summary (+ an asset-class/kind chip). A search box filters by label/summary/keywords (case-insensitive, substring). Keyboard-first: roving tabindex or a listbox (role="listbox"/role="option", aria-selected, arrow-key navigation, Enter/Space select, type-to-search). Honest empty state ("No structures match" => "—"-style). Use the design-system atoms + module.css classes; add gallery-specific classes to a new StructureGallery.module.css (do NOT edit TicketWorkspace.module.css).
Tests gui/test/products/structureGallery.test.tsx (@testing-library/react): renders all registered specs grouped; selecting a card calls onSelect with the id; search narrows the list; the selected card has aria-selected; keyboard arrow+Enter selects. No axe-serious violations in the rendered markup (use role/aria correctly).`,
    test: 'test/products/structureGallery.test.tsx',
  },
  {
    component: 'payoff-chart',
    prompt: `Build gui/src/products/PayoffChart.tsx — a live payoff-at-expiry mini-chart (inline SVG, no chart lib). Props: { structureId: string; strike: number; spot: number; side?: "BUY"|"SELL"; width?: number; height?: number }.
Compute the payoff at expiry as a pure function over a spot grid for the COMMON structures (vanilla call/put, the strategy templates risk-reversal/strangle/straddle/seagull, single/double barrier as the vanilla payoff knocked to 0 outside the live region, digital as a step, touch). For structures whose expiry payoff is path-dependent / not a clean terminal function (asian, lookback, cliquet, tarf, accumulator, variance/vol swap, forward-start, quanto, american), render an HONEST "payoff depends on the path — priced server-side" empty state ("—") rather than a fabricated curve. Draw axes, the zero line, strike marker, current-spot marker; format with the existing lib/format helpers. Accessible (role="img" + aria-label describing the payoff shape).
Add PayoffChart.module.css (new). Tests gui/test/products/payoffChart.test.tsx: vanilla call payoff is 0 below strike and rises above (sample the computed path function — export the pure payoffAtExpiry(structureId, params, spotGrid) for testing); path-dependent structures render the honest empty state; the SVG has an aria-label.`,
    test: 'test/products/payoffChart.test.tsx',
  },
  {
    component: 'net-strip',
    prompt: `Build gui/src/products/NetStructureStrip.tsx — a compact running strip summarising a structure's net economics for the trader while structuring. Props: { legs: { premium?: number; delta?: number; vega?: number; gamma?: number; ratio: number; side: "BUY"|"SELL" }[]; quoteCcy: string; baseCcy: string }.
Compute + show net premium and net delta/vega/gamma across the legs (sign by side, scaled by ratio), each with the honest-data discipline: any leg missing a value => the whole net shows "—" (do NOT silently treat missing as 0), with a small note. Use lib/format (fmtPremiumPct etc.) + the GreeksStrip styling conventions; add NetStructureStrip.module.css. Accessible (a labelled group; values in a definition-list or aria-described cells).
Tests gui/test/products/netStrip.test.tsx: net premium/greeks aggregate with correct signs across a 2-leg risk-reversal; a missing greek on any leg yields "—" for that net (not a wrong number); single-leg passthrough.`,
    test: 'test/products/netStrip.test.tsx',
  },
]

const results = await parallel(
  WORKERS.map((w) => () =>
    agent(COMMON + `\n=== YOUR COMPONENT: ${w.component} ===\n${w.prompt}\nGATE test file(s): ${w.test}`, {
      label: `build:${w.component}`,
      phase: 'Components',
      schema: RESULT,
    })
  )
)

log(`GW2 phase-2 components: ${results.filter(Boolean).map((r) => r.component).join(', ')}`)
return results
