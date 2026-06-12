export const meta = {
  name: 'xasset-integration',
  description: 'Complete cross-asset integration: wire equity/commodity/crypto/RFQ through the one contract to the EXISTING leaf engines — no workarounds, mesh-coordinated, single-machine-safe (serial cargo).',
  whenToUse: 'After W2 lands + the parallel session has stood down the batched post-window integration. Opens the proto window once and wires every deferred arm to its real engine.',
  phases: [
    { title: 'Seam', detail: 'proto arms + domain variants + convert codecs + clean Instrument constructors — one coupled edit, compiles together' },
    { title: 'Server', detail: 'route equity/commodity/crypto to the existing leaf pricers + RFQ RPC to MultiDealerEngine (serial cargo)' },
    { title: 'Oracle', detail: 'golden vectors + parity rows per arm vs the leaf crates independent oracles (serial cargo)' },
    { title: 'Clients', detail: 'SDK + CLI (serial cargo) ∥ Excel + GUI (node, parallel) surfacing of the new asset classes' },
    { title: 'Verify', detail: 'adversarial verify each arm against an independent oracle; no circular check' },
  ],
}

// ---------------------------------------------------------------------------
// SINGLE-MACHINE SAFETY (the binding constraint, learned this session):
// this one M4 cannot run concurrent heavy cargo builds — it hits macOS dyld
// first-launch stalls + 60s tokio-timeout flakes. So EVERY cargo-touching stage
// runs SERIALLY (awaited one at a time); only the toolchain-disjoint node lanes
// (Excel/GUI) fan out in parallel. Serial-where-contended, parallel-where-free.
// The mesh must be quiesced (parallel session stopped, not just paused) before
// this runs, because the Seam phase forces a full-workspace rebuild.
// ---------------------------------------------------------------------------

const SEAM_SCHEMA = {
  type: 'object',
  required: ['protoArms', 'domainVariants', 'gate'],
  properties: {
    protoArms: { type: 'array', items: { type: 'string' } },
    domainVariants: { type: 'array', items: { type: 'string' } },
    constructorsModernized: { type: 'integer' },
    gate: { type: 'string', description: 'literal gate lines proving celnet-proto + celnet-types compile + test' },
    issues: { type: 'string' },
  },
}
const WIRE_SCHEMA = {
  type: 'object',
  required: ['arm', 'enginePath', 'gate'],
  properties: {
    arm: { type: 'string' },
    enginePath: { type: 'string', description: 'the existing leaf crate the arm routes to' },
    gate: { type: 'string' },
    issues: { type: 'string' },
  },
}
const VERDICT_SCHEMA = {
  type: 'object',
  required: ['arm', 'oracleIndependent', 'real', 'detail'],
  properties: {
    arm: { type: 'string' },
    oracleIndependent: { type: 'boolean', description: 'true only if the check does NOT reuse the production pricer' },
    real: { type: 'boolean', description: 'true if the arm prices correctly vs the independent oracle' },
    detail: { type: 'string' },
  },
}

// The deferred arms and their REAL engines (all already built + tested on main).
const ARMS = [
  { key: 'equity', proto: 'Underlying.ref equity=4 + EquityRef/Symbol', engine: 'celnet-equity-vanilla', carry: 'CostOfCarry b=r-q (generalized-BSM)', settlement: null },
  { key: 'commodity', proto: 'Underlying.ref commodity=5 + CommodityRef/Symbol', engine: 'celnet-commodity-vanilla', carry: 'CostOfCarry b=0 (Black-76)', settlement: null },
  { key: 'crypto', proto: 'Underlying.ref digital_asset=6 + CryptoPair + Instrument.settlement_style=29', engine: 'celnet-crypto-vanilla', carry: 'CostOfCarry b=r-funding; settlement_style selects linear vs inverse 1/S_T', settlement: 'SettlementStyle{LINEAR=0,INVERSE_COIN=1}' },
  { key: 'rfq', proto: 'QuoteService.RequestMultiDealerQuote + MultiDealerQuote/DealerQuote + QuoteAccept.lp_id=4', engine: 'celnet-rfq', carry: 'n/a — MultiDealerEngine fan-out/rank/last-look', settlement: null },
]

log('xasset-integration: full cross-asset wiring to the existing leaf engines. Cargo stages SERIAL (single-machine).')

// === Phase 1: Seam — ALREADY DONE + committed locally (54c62fa) ==============
// The seam landed in a prior run: proto arms (equity=4/commodity=5/digital_asset=6 +
// settlement_style=29 + DealerQuote/MultiDealerQuote + RequestMultiDealerQuote + lp_id),
// celnet-types domain twins, REAL bidirectional convert codecs, constructors modernized.
// Verified: `cargo build --workspace` exit 0, 0 errors, FX byte-identical. So we SKIP the
// seam agent and start at Server, building on the committed seam.
phase('Seam')
const seam = {
  protoArms: ['equity=4', 'commodity=5', 'digital_asset=6', 'settlement_style=29', 'RequestMultiDealerQuote', 'DealerQuote/MultiDealerQuote', 'QuoteAccept.lp_id=4'],
  domainVariants: ['Underlying::Equity(EquityRef)', 'Underlying::Commodity(CommodityRef)', 'Underlying::DigitalAsset(CryptoPair)'],
  gate: 'committed 54c62fa; cargo build --workspace exit 0; convert codecs real (not stubs)',
  note: 'seam landed by prior run — agent skipped',
}
log('Seam: already committed (54c62fa) — skipping agent, starting at Server')

// === Phase 2: Server routing (SERIAL — one cargo gate at a time) ============
phase('Server')
const wired = []
for (const arm of ARMS) {
  const w = await agent(
    `Seam is landed (proto arms + domain variants compile). Wire the ${arm.key.toUpperCase()} arm through the SERVER to its EXISTING, already-tested engine — NO new pricing math, NO workarounds. Reuse the leaf crate.

Arm: ${arm.proto}
Engine (already on main, done + gated): ${arm.engine}
Pricing: ${arm.carry}${arm.settlement ? ' — ' + arm.settlement : ''}

In crates/celnet-server/src/pricer.rs (+ services/ + ws/codec.rs as needed): route an instrument whose Underlying decodes to this arm to ${arm.engine}, exactly as the existing FX/vanilla path routes — same CarryPricer seam, same no-silent-fallback typed errors, same Greek surfacing. ${arm.key === 'crypto' ? 'settlement_style=INVERSE_COIN selects the coin-margined 1/S_T closed form in celnet-crypto-vanilla; LINEAR = generalized-BSM.' : ''}${arm.key === 'rfq' ? 'Wire QuoteService.RequestMultiDealerQuote to celnet-rfq MultiDealerEngine with an InternalPricerSource over the live pricer; AcceptQuote books the (quote_id, lp_id) panel winner.' : ''}

Gate SERIALLY: cargo test -p celnet-server (cargo test, NOT nextest). Validate the arm prices a known case correctly. Report literal gate lines + the engine path you routed to.`,
    { schema: WIRE_SCHEMA, label: `server:${arm.key}`, phase: 'Server' },
  )
  if (w) wired.push(w)
}

// === Phase 3: Oracle rows (SERIAL) =========================================
phase('Oracle')
const oracled = []
for (const arm of ARMS.filter((a) => a.key !== 'rfq')) {
  const o = await agent(
    `Add the golden vector + parity row for the ${arm.key.toUpperCase()} arm (engine ${arm.engine}). The parity oracle MUST be independent — re-derive from first principles / external reference / a code-disjoint route, NEVER reuse the production pricer (the FRTB-0.75rho circular-oracle lesson). crates/celnet-golden/vectors/<family>.json + a crates/celnet-parity/tests row driving the arm through the wire and reconciling to the independent oracle. Keep just verification-coverage consistent. Gate SERIALLY with cargo test (NOT nextest); clippy the parity TEST target (-p celnet-parity --test <name> -D warnings). Report literal gate lines + how the oracle is independent.`,
    { schema: WIRE_SCHEMA, label: `oracle:${arm.key}`, phase: 'Oracle' },
  )
  if (o) oracled.push(o)
}

// === Phase 4: Clients — cargo SERIAL, node PARALLEL ========================
phase('Clients')
// SDK + CLI are cargo: run them one at a time.
const sdk = await agent(
  `Surface the new asset classes (equity/commodity/crypto underlyings + settlement_style) in the SDK celnet-client: vocab builders + to_wire mapping + conformance rows (SDK==server==oracle). NO workarounds. Gate SERIALLY: cargo test -p celnet-client (cargo test, NOT nextest). On a quiet machine — the 60s deadlines flake under build load, so do not run concurrent cargo. Report literal gate lines.`,
  { label: 'client:sdk', phase: 'Clients' },
)
const cli = await agent(
  `Surface the new asset classes in the CLI celnet-cli (subcommands/flags for equity/commodity/crypto underlyings + settlement_style) + conformance rows (CLI==server==oracle). NO workarounds. Gate SERIALLY: cargo test -p celnet-cli. Report literal gate lines.`,
  { label: 'client:cli', phase: 'Clients' },
)
// Excel + GUI are node (toolchain-disjoint) — these CAN run in parallel with each other.
const nodeClients = await parallel([
  () => agent(
    `Surface the new asset classes in Excel (excel/): contract mirror + enums + wsCodec + functions for equity/commodity/crypto underlyings + settlement_style. Mirror the proto exactly. NO workarounds. Gate: cd excel && npm run typecheck && npm test. Report literal gate lines.`,
    { label: 'client:excel', phase: 'Clients' },
  ),
  () => agent(
    `Surface the new asset classes in the GUI (gui/) ProductSpec/registry + contract mirror (equity/commodity/crypto underlyings + settlement_style selector for crypto). Mirror the proto exactly; reuse the GW2 registry pattern. NO workarounds. Gate: cd gui && npm run typecheck && npm run typecheck:test && npm test. Report literal gate lines.`,
    { label: 'client:gui', phase: 'Clients' },
  ),
])

// === Phase 5: Adversarial verify each arm (independent oracle) =============
phase('Verify')
const verdicts = await parallel(
  ARMS.map((arm) => () =>
    agent(
      `Adversarially verify the ${arm.key.toUpperCase()} arm end-to-end. Price a known case through the wire and reconcile to an INDEPENDENT oracle that does NOT reuse the production pricer (engine ${arm.engine}). Default to real=false if you cannot independently confirm. For crypto, verify BOTH linear and inverse(1/S_T) settlement. For rfq, verify ranking/tie-break/last-look against injected ground-truth ladders. Report oracleIndependent honestly.`,
      { schema: VERDICT_SCHEMA, label: `verify:${arm.key}`, phase: 'Verify' },
    ),
  ),
)

const confirmed = verdicts.filter(Boolean).filter((v) => v.real && v.oracleIndependent)
return {
  seam,
  wired,
  oracled,
  clients: { sdk, cli, node: nodeClients.filter(Boolean) },
  verdicts,
  confirmedArms: confirmed.map((v) => v.arm),
  note: 'Cargo stages ran serially per single-machine constraint. Mesh must be quiesced before launch (Seam triggers a full-workspace rebuild).',
}
