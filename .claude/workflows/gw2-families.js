export const meta = {
  name: 'gw2-product-families',
  description: 'GW2 fan-out: migrate the 15 remaining legless product families out of the TicketWorkspace monolith into gui/src/products/<family>.tsx registry specs + per-family round-trip tests, following the asian.tsx template. Workers run in the lane/gw2 worktree, create disjoint files, gate per-family via vitest, do NOT commit.',
  phases: [
    { title: 'Migrate', detail: 'five workers, three families each — disjoint product files + tests' },
  ],
}

const RESULT = {
  type: 'object',
  additionalProperties: false,
  required: ['batch', 'families_done', 'files_created', 'gate_line', 'zero_workarounds', 'notes'],
  properties: {
    batch: { type: 'string' },
    families_done: { type: 'array', items: { type: 'string' } },
    files_created: { type: 'array', items: { type: 'string' } },
    gate_line: { type: 'string', description: 'literal vitest summary line(s) for the per-family tests' },
    zero_workarounds: { type: 'boolean' },
    notes: { type: 'string', description: 'anything the coordinator must know for index wiring (export names, ctx fields used, model locks)' },
  },
}

const WORKTREE = '/Users/adrian/code/celnet-gw2'

const COMMON = `
You are a GW2 worker migrating product families out of the Celnet GUI ticket monolith into the new ProductSpec registry.
WORK ONLY in the worktree ${WORKTREE} (branch lane/gw2-structuring, which already has the registry seam committed). Always: cd ${WORKTREE}/gui
DO NOT edit gui/src/products/index.ts, gui/src/workspaces/TicketWorkspace.tsx, or any file outside your family files + their tests. DO NOT git add/commit (the coordinator wires the index + commits). Create ONLY new files.
READ FIRST (the exact template you must follow): gui/src/products/asian.tsx (a complete migrated family) and gui/src/products/types.ts (the ProductSpec<I> contract + withTenorAndModel + ProductBuildCtx). Also read gui/test/productRegistry.test.ts for the round-trip assertions.
For EACH family assigned to you:
  1. In gui/src/workspaces/TicketWorkspace.tsx find: the <Family>Inputs interface, the DEFAULT_<FAMILY> constant, the <family>Terms(...) transform (if any), the family's case in buildInstrument (the data/seed builder call — note which ctx values it needs: ATM-forward, spot, expiryYears==tenorYears), and the family's InputBlock JSX (search the legless-inputs sub-component). The data/seed builders + Terms types are imported from "../data/seed"; enum types from "../data/contract".
  2. Create gui/src/products/<family>.tsx EXACTLY like asian.tsx: export the Inputs interface, DEFAULT_<FAMILY>, any *Terms transform, a self-contained <Family>InputBlock (props {value,onChange,ctx}; map the monolith's atmForward->ctx.atmForward, spot->ctx.spot, pipDecimals->ctx.pipDecimals, expiryYears->ctx.tenorYears, today->ctx.today), and the exported <family>Spec = defineProduct<Inputs>({ id, label, group, assetClass:'FX', summary, keywords, kind, defaults, allowedModels: bookingModelsFor(kind) (import from ../data/seed) OR the literal it returns, toInstrument: (inputs,ctx)=>withTenorAndModel(<builder>(ctx.pair, ctx.tenorYears, ctx.notionalMm, <terms>), ctx, <lockedModel?>), InputBlock }).
     - The toInstrument MUST reproduce the monolith buildInstrument output byte-for-byte (same builder, same terms, same tenor+model stamp). The WINDOW_BARRIER family locks to LOCAL_STOCH_VOL (pass it as withTenorAndModel's lockedModel) — copy the monolith's exact rule.
     - Extract the InputBlock JSX VERBATIM (same classNames from "../workspaces/TicketWorkspace.module.css", same aria-labels, same handlers, same notes), only renaming the props.
  3. Create gui/test/products/<family>.test.ts: import the spec directly; assert spec.toInstrument(spec.defaults, CTX) builds an instrument with .product.kind === spec.kind, .tenor === CTX.tenor, round-trips deterministically through instrumentToWire (from ../../src/data/wsCodec), and spec.allowedModels deep-equals bookingModelsFor(spec.kind) (from ../../src/data/seed). Use the same CTX shape as gui/test/productRegistry.test.ts (EURUSD 3M, tenorYearsToTenor(0.25), atmForward 1.105, spot 1.1, pipDecimals 4). For families needing realistic inputs (e.g. a barrier level), set a sensible default that prices.
GATE (run, capture literal output): cd ${WORKTREE}/gui && npx vitest run test/products/<f1>.test.ts test/products/<f2>.test.ts test/products/<f3>.test.ts
All tests pass, zero workarounds (no @ts-ignore/as any/#skip/eslint-disable beyond the registry's existing one/lowered assertion/fabricated value). If a family genuinely cannot be made byte-identical, STOP and report it in notes rather than fudging.
Your FINAL message is the structured result.
`

const BATCHES = [
  { batch: 'barriers-1', families: 'SINGLE_BARRIER (kind singleBarrier, group "Barriers & digitals", builder singleBarrierInstrument), DOUBLE_BARRIER (doubleBarrier, "Barriers & digitals", doubleBarrierInstrument), DIGITAL (digital, "Barriers & digitals", digitalInstrument — its terms use atmForward)' },
  { batch: 'barriers-2', families: 'TOUCH (kind touch, "Barriers & digitals", touchInstrument — terms use spot), WINDOW_BARRIER (windowBarrier, "Barriers & digitals", windowBarrierInstrument — LOCKED to LOCAL_STOCH_VOL), AMERICAN (american, "Path-dependent", americanInstrument — terms use atmForward + expiryYears)' },
  { batch: 'volatility', families: 'VARIANCE_SWAP (kind varianceSwap, group "Volatility", builder varianceSwapInstrument — takes a swapStrikeVol number, 0=>fair strike), VOLATILITY_SWAP (volatilitySwap, "Volatility", volatilitySwapInstrument — swapStrikeVol number), LOOKBACK (lookback, "Path-dependent", lookbackInstrument — terms use atmForward)' },
  { batch: 'path', families: 'FORWARD_START (kind forwardStart, group "Path-dependent", builder forwardStartInstrument — terms use tenorYears), CLIQUET (cliquet, "Path-dependent", cliquetInstrument), QUANTO (quanto, "Structured", quantoInstrument — terms use atmForward)' },
  { batch: 'structured', families: 'TARF (kind tarf, group "Structured", builder tarfInstrument — terms use atmForward + expiryYears), ACCUMULATOR (accumulator, "Structured", accumulatorInstrument — terms use atmForward), BASKET (basket, "Structured", basketInstrument — multi-leg correlated; copy the basket inputs/terms/Cholesky helpers verbatim)' },
]

const results = await parallel(
  BATCHES.map((b) => () =>
    agent(COMMON + `\n=== YOUR BATCH: ${b.batch} ===\nMigrate these three families: ${b.families}`, {
      label: `migrate:${b.batch}`,
      phase: 'Migrate',
      schema: RESULT,
    })
  )
)

log(`GW2 families: ${results.filter(Boolean).flatMap((r) => r.families_done || []).length} migrated`)
return results
