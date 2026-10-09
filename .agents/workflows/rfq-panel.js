export const meta = {
  name: 'rfq-panel-surfacing',
  description: 'Complete the multi-dealer RFQ: server panel (≥3 loopback LPs + external-winner booking + WS mirror) → SDK → CLI → GUI ∥ Excel → cross-client bit-identical conformance + adversarial verify.',
  whenToUse: 'The last client-parity gap after the ADR-0008 tail: no client renders the ranked panel.',
  phases: [
    { title: 'Server', detail: 'panel sources + accept-by-lp_id + WS mirror + demo_edge LPs (serial cargo)' },
    { title: 'SDK', detail: 'request_multi_dealer_quote handle + accept(lp_id) + loopback conformance (serial cargo)' },
    { title: 'CLI', detail: 'ranked-panel subcommand + conformance (serial cargo)' },
    { title: 'Clients', detail: 'GUI panel view ∥ Excel panel spill (node, parallel)' },
    { title: 'Verify', detail: 'cross-client bit-identical ranking + live e2e + adversarial ranking-law check' },
  ],
}

// SINGLE-MACHINE RULE (§4.1, proven this program): cargo phases run STRICTLY
// SERIALLY (each awaited); only the node lanes fan out. Gate with `cargo test`
// (never nextest — wedged in this env). Main worktree (warm target/), session-A idle.

const REPORT = {
  type: 'object',
  required: ['files', 'gate', 'design'],
  properties: {
    files: { type: 'array', items: { type: 'string' } },
    gate: { type: 'string', description: 'literal gate lines (test counts, exit codes)' },
    design: { type: 'string', description: 'the key design decisions taken' },
    unsure: { type: 'string' },
  },
}
const VERDICT = {
  type: 'object',
  required: ['bitIdentical', 'bookingProven', 'rankingLawIndependent', 'detail'],
  properties: {
    bitIdentical: { type: 'boolean' },
    bookingProven: { type: 'boolean' },
    rankingLawIndependent: { type: 'boolean', description: 'ranking re-derived from spec, not from the engine' },
    detail: { type: 'string' },
  },
}

const COMMON = `Celnet mesh worker. Repo the working tree (cwd) (main worktree). HARD RULES (GUIDE.md): no mocks-as-real, no todo!()/placeholders/#[allow]-dodges/as-any/skipped tests; purpose-named vendor-neutral identifiers; FX/single-dealer paths stay byte-identical; NO celnet.proto edits (RequestMultiDealerQuote + MultiDealerQuote/DealerQuote + QuoteAccept.lp_id already on the wire); honest boundary: live LP connectivity/fills = ENV — in-repo LPs are deterministic synthetic/loopback, labeled as such. Gate with plain 'cargo test' (NEVER nextest). Do NOT git commit. Match surrounding idiom exactly.`

phase('Server')
const server = await agent(
  `${COMMON}

LANE PHASE 1 — SERVER panel completion. Files: crates/celnet-server/ only (src/services/quote.rs, src/ws/codec.rs, examples/demo_edge.rs, tests/).

Today (read first): services/quote.rs:414 request_multi_dealer_quote builds a panel of ONE InternalPricerSource (MAKER_AUTO_PRICER_ID) over the edge mid±half-spread; :609 AcceptQuote REFUSES any external lp_id; ws/codec.rs has zero panel support. celnet-rfq (read its src/) provides MultiDealerEngine::new(Vec<Box<dyn QuoteSource>>), QuoteSource trait, RankedPanel, InternalPricerSource, FixLpAdapter+FixLpConfig (real loopback celnet-fix sessions, proven in its tests).

Build, completely:
1. **Panel sources**: a config-driven LP panel — the internal maker PLUS N deterministic synthetic LP sources (env e.g. CELNET_DEMO_LPS, default 0 ⇒ single-dealer byte-identical). Synthetic LPs: deterministic per-lp_id spread/skew offsets around the SAME edge mid (purpose-named, e.g. "SYNTH-LP-1"; doc: demo/test panel — live LP connectivity is ENV). Implement as a QuoteSource around the edge pricing path (like InternalPricerSource) — do NOT fake fills.
2. **Panel pinning + external booking**: pin the returned panel rows per quote_id (the existing quote-pinning/idempotency store pattern) so AcceptQuote(quote_id, lp_id) books ANY panel row — execution carries that row's price + lp attribution. Empty lp_id stays the single-dealer path byte-identical. Expired last-look (valid_until_nanos) refused exactly as the engine law.
3. **WS mirror**: ws/codec.rs gains the multi-dealer request + MultiDealerQuote/DealerQuote panel frame (snake_case mirroring the proto field-for-field, like the existing frames) + accept-with-lp_id — so GUI/Excel reach the panel over WS. Mirror gRPC exactly.
4. **demo_edge**: spawn with a ≥3-LP panel by default (env-overridable) so the live e2e suites exercise it.
5. Server tests: panel ≥3 rows ranked correctly (engine law: best bid=max, best offer=min, tie-break epoch_nanos→lp_id), external booking == pinned row price, empty-lp_id byte-identity, WS frame round-trip.

Gate SERIALLY: cargo test -p celnet-server (then cargo clippy -p celnet-server --all-targets -- -D warnings, cargo fmt -p celnet-server). Report literal lines.`,
  { schema: REPORT, phase: 'Server' },
)
log(`Server: ${server?.files?.length ?? 0} files — ${String(server?.gate ?? '').slice(0, 120)}`)

phase('SDK')
const sdk = await agent(
  `${COMMON}

LANE PHASE 2 — SDK. Files: crates/celnet-client/ only. The server now supports: gRPC RequestMultiDealerQuote returning a ranked MultiDealerQuote panel (≥3 LPs when configured), AcceptQuote(quote_id, lp_id) booking any pinned panel row. Server design notes: ${JSON.stringify(server?.design ?? '').slice(0, 800)}

Build, completely (mirror the existing Rfq handle idiom at src/lib.rs:218/589):
1. client.request_multi_dealer_quote(InstrumentSpec, Conventions) → a RankedPanel handle: rows (lp_id, two-way, epoch/valid_until, responded), best_bid_lp_id/best_offer_lp_id, last-look remaining, accept(side, lp_id) → Execution, and the single-dealer-compatible accept default.
2. Vocab/types mirroring the wire panel exactly; re-export.
3. Conformance (tests/, the start_edge_and_client pattern): boot the edge with ≥3 LPs; assert (a) panel rows ranked per the engine law re-checked IN-TEST from the raw rows (max bid/min offer/tie-break) — not trusted from the server; (b) booking the best-offer winner returns an Execution whose price == that row's offer bit-for-bit; (c) empty-lp_id accept == the single-dealer path byte-identical; (d) expired last-look refused. Runnable example examples/ if the pattern fits.

Gate SERIALLY: cargo test -p celnet-client + clippy -D + fmt. Report literal lines.`,
  { schema: REPORT, phase: 'SDK' },
)
log(`SDK: ${sdk?.files?.length ?? 0} files — ${String(sdk?.gate ?? '').slice(0, 120)}`)

phase('CLI')
const cli = await agent(
  `${COMMON}

LANE PHASE 3 — CLI. Files: crates/celnet-cli/ only. The SDK now exposes request_multi_dealer_quote + RankedPanel + accept(lp_id). SDK design: ${JSON.stringify(sdk?.design ?? '').slice(0, 600)}

Build: a 'celnet rfq' panel mode (flag or subcommand, match the existing CLI grammar in src/cli.rs/args.rs) printing the ranked panel (one row per LP: lp_id, bid/offer, validity countdown, BEST markers) + '--accept <lp_id>' booking that row and printing the execution. Conformance test: CLI panel output rows == the SDK panel from the same edge bit-identically (parse the printed numbers; compare to_bits via the shared corpus pattern in tests/conformance.rs).

Gate SERIALLY: cargo test -p celnet-cli + clippy -D + fmt. Report literal lines.`,
  { schema: REPORT, phase: 'CLI' },
)
log(`CLI: ${cli?.files?.length ?? 0} files — ${String(cli?.gate ?? '').slice(0, 120)}`)

phase('Clients')
const nodeClients = await parallel([
  () =>
    agent(
      `${COMMON}

LANE PHASE 4a — GUI ranked panel. Files: gui/ only (NO cargo). The server WS mirror now carries the multi-dealer request + panel frame + accept-with-lp_id: ${JSON.stringify(server?.design ?? '').slice(0, 600)}

Build (match GW2 idioms): in the TicketWorkspace RFQ flow, a multi-dealer mode rendering the ranked panel — one row per LP (lp_id, bid/offer, BEST bid/offer highlighted), the existing LastLookRing for the validity countdown, click-a-row → book by (quote_id, lp_id), honest states (expired row disabled with reason). Extend gui/src/data/wsCodec.ts + contract mirror with the panel frame (snake_case mirroring the proto; do NOT touch MarketContext). a11y: the panel is a proper table/listbox per APG, axe-clean. Unit tests (vitest): codec round-trip, ranking render order == frame order, book action emits (quote_id, lp_id), countdown/expiry states. Extend the live e2e (e2e/workflows.e2e.ts) with a panel spec: request panel on the demo edge (≥3 LPs), assert rows + book the best offer + a fill renders — keep all 13 existing specs green. DO NOT run the e2e (cargo-adjacent; the Verify phase runs it) — gate with: npm run typecheck && npm run typecheck:test && npm test.`,
      { schema: REPORT, label: 'gui', phase: 'Clients' },
    ),
  () =>
    agent(
      `${COMMON}

LANE PHASE 4b — Excel ranked panel. Files: excel/ only (NO cargo). The server WS mirror now carries the multi-dealer request + panel frame + accept-with-lp_id: ${JSON.stringify(server?.design ?? '').slice(0, 600)}

Build (match the polymorphic surface idioms): CELNET.RFQ gains a panel mode (e.g. an optional panel flag/arg consistent with the existing signature) spilling the ranked panel — one row per LP (lp_id, bid, offer, valid-until, BEST markers) — and an accept path booking by (quote_id, lp_id) (follow how RFQ accept works today). Extend excel/src/contract wsCodec + mirror with the panel frame (snake_case, field-for-field; do NOT touch contract.ts MarketContext). Unit tests: codec round-trip, spill shape, ranking order preserved, accept emits (quote_id, lp_id). Extend e2e/corpus or conformance.e2e.ts with a panel spec for the ≥3-LP demo edge — but DO NOT run the e2e (Verify phase does); gate with: npm run typecheck && npm test.`,
      { schema: REPORT, label: 'excel', phase: 'Clients' },
    ),
])

phase('Verify')
const conformance = await agent(
  `${COMMON}

LANE PHASE 5a — cross-client conformance + live e2e. The full stack is built: server panel (≥3 LPs) + WS mirror + SDK + CLI + GUI + Excel. Reports: server=${JSON.stringify(server?.gate ?? '')}, sdk=${JSON.stringify(sdk?.gate ?? '')}, cli=${JSON.stringify(cli?.gate ?? '')}.

Do, in order (SERIAL cargo, one at a time):
1. cargo build -p celnet-server --example demo_edge   (pre-build — the e2e ready-timeout does NOT cover a cold build; hard lesson)
2. cargo test -p celnet-server -p celnet-client -p celnet-cli   (the three cargo crates together, one invocation)
3. cd gui && npm run typecheck && npm test && npm run e2e   (ALL specs must pass incl. the new panel spec — gui-touching gates run the live e2e, always)
4. cd excel && npm run typecheck && npm test && npm run test:e2e
5. The cross-client oracle: from the suites' shared corpus/seeds, confirm the SAME ranked panel (same lp order, same to_bits prices) is what SDK conformance, CLI conformance, the GUI e2e and the Excel e2e each asserted — report HOW each suite pins it (test names + the shared seed/corpus linkage), not just that suites pass.
If anything fails: FIX it properly (root cause, no weakening) and re-run that gate. Report every literal gate line.`,
  { schema: REPORT, phase: 'Verify' },
)
const verdict = await agent(
  `${COMMON}

LANE PHASE 5b — ADVERSARIAL VERIFY (independent; default to false on doubt). Re-derive the ranking law from crates/celnet-rfq's doc/spec + docs/W4-STRUCTURED-RFQ-PLAN.md (best bid = max, best offer = min, tie-break earlier epoch_nanos → smaller lp_id, timeout-drop excluded from lp_count, last-look promotion) WITHOUT trusting the engine code. Then:
1. Inspect the server panel + pinning + accept path (services/quote.rs) for law violations, silent fallbacks, or fake fills (an external booking must settle the PINNED row price — find where that's asserted).
2. Inspect each client's conformance/e2e for circularity: does any client test merely echo the server's ranking back, or does at least the SDK conformance RE-CHECK the law from raw rows in-test? Quote the test lines.
3. Check honesty: synthetic LPs labeled as demo/test; live LP conn = ENV claims intact; single-dealer path byte-identity asserted somewhere (quote the assertion).
4. Try to construct a counterexample (tie at identical epoch_nanos; expired-winner promotion; lp_count vs responders) — if a unit test for it is missing, ADD it (cargo test -p the right crate, serially) and report.
Verdict fields: bitIdentical (cross-client), bookingProven (execution==pinned row), rankingLawIndependent (at least one in-test re-derivation), detail.`,
  { schema: VERDICT, phase: 'Verify' },
)

return {
  server, sdk, cli,
  gui: nodeClients[0], excel: nodeClients[1],
  conformance, verdict,
  done: Boolean(verdict?.bitIdentical && verdict?.bookingProven && verdict?.rankingLawIndependent),
}
