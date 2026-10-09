export const meta = {
  name: 'new-payoff-shapes',
  description: 'PerpetualOption (arm 30) + ListedFutureOption (arm 31): proto window → exact closed-form engines on the carry seam → golden/parity → server+WS → 5 clients → conformance + adversarial verify.',
  whenToUse: 'The backlog item proto/new-payoff-shapes — the only genuinely new payoff shapes left.',
  phases: [
    { title: 'Seam', detail: 'proto arms 30/31 + decode/validity (serial cargo)' },
    { title: 'Engines', detail: 'perpetual-American exact closed form + future-option margining (serial cargo)' },
    { title: 'Oracle', detail: 'golden vectors + independent parity rows (serial cargo)' },
    { title: 'Server', detail: 'pricer routing + WS frames (serial cargo)' },
    { title: 'Clients', detail: 'SDK → CLI (serial cargo), then GUI ∥ Excel (node)' },
    { title: 'Verify', detail: 'coverage 23 arms + cross-client conformance + live e2e + adversarial' },
  ],
}

// §4.1: every cargo phase awaited serially; only GUI∥Excel fan out. Plain `cargo test`
// (never nextest). Main worktree, warm target. Pre-build demo_edge before any e2e.

const REPORT = {
  type: 'object',
  required: ['files', 'gate', 'design'],
  properties: {
    files: { type: 'array', items: { type: 'string' } },
    gate: { type: 'string' },
    design: { type: 'string' },
    unsure: { type: 'string' },
  },
}
const VERDICT = {
  type: 'object',
  required: ['perpetualLawIndependent', 'futureOptionIndependent', 'coverageEnforced', 'real', 'detail'],
  properties: {
    perpetualLawIndependent: { type: 'boolean' },
    futureOptionIndependent: { type: 'boolean' },
    coverageEnforced: { type: 'boolean', description: 'the proto-driven lint genuinely fails when a new-arm vector/row is removed' },
    real: { type: 'boolean' },
    detail: { type: 'string' },
  },
}

const COMMON = `Celnet mesh worker. Repo the working tree (cwd) (main worktree). HARD RULES (GUIDE.md): no mocks/placeholders/todo!()/#[allow]-dodges/skipped tests/lowered tolerances; purpose-named vendor-neutral identifiers (math provenance in doc comments ONLY); FX + every existing family byte-identical (untouched paths); one unversioned contract; numerical work validated against INDEPENDENT oracles (the FRTB circular-oracle lesson: re-derive constants, never re-run the engine as its own check). Gate with plain 'cargo test' (NEVER nextest). Do NOT git commit. Match surrounding idiom exactly.`

phase('Seam')
const seam = await agent(
  `${COMMON}

PHASE 1 — PROTO SEAM (the announced window; arms 30/31 are reserved on the board). Files: crates/celnet-proto/ (+ crates/celnet-types/ if a domain type is genuinely needed).

Add to the Instrument product oneof (study the existing arm patterns first — AmericanOption=24, FxForward=26 etc., their message docs + validity-matrix comments):
1. **PerpetualOption = 30** — message PerpetualOption { OptionType option_type; double strike; double notional; }. A perpetual (no-expiry) American option: doc that Instrument.expiry_years MUST be 0 for this arm (a non-zero expiry on a perpetual is INVALID_ARGUMENT at decode/validity — no silent ignore; document in the message + the oneof comment).
2. **ListedFutureOption = 31** — message ListedFutureOption { Symbol future_symbol (reuse the existing Symbol message); double future_expiry_years (the FUTURE's expiry ≥ the option's Instrument.expiry_years — validity-checked); OptionType option_type; double strike; double notional; enum/field margining: MARGINING_EQUITY_STYLE = 0 (upfront premium, discounted) | MARGINING_FUTURES_STYLE = 1 (daily-margined, undiscounted) — meaningful-zero convention like SettlementStyle. }. Option on a listed future, any asset class (the Underlying names the class; the future_symbol names the contract).
3. Decode/validity helpers in src/ (mirror how existing arms decode + the product×underlying validity pattern in convert.rs/helpers.rs): perpetual requires expiry_years == 0; future option requires future_expiry_years >= expiry_years > 0. NO changes to existing arms.

Gate SERIALLY: cargo test -p celnet-proto -p celnet-types && cargo clippy -p celnet-proto -p celnet-types --all-targets -- -D warnings && cargo fmt -p celnet-proto -p celnet-types -- --check. NOTE: tools/check-verification-coverage.mjs is proto-driven and will now FAIL for the two new arms until the Oracle phase lands vectors+rows — that is EXPECTED and correct; do not run or weaken it. Report the arm/field numbers + the family names the lint will derive (read the lint's name-derivation logic and report what it expects, e.g. perpetual_option / listed_future_option).`,
  { schema: REPORT, phase: 'Seam' },
)
log(`Seam: ${String(seam?.design ?? '').slice(0, 140)}`)

phase('Engines')
const engines = await agent(
  `${COMMON}

PHASE 2 — ENGINES. Files: crates/celnet-exotics/ and crates/celnet-commodity-vanilla/ only. Seam landed: ${JSON.stringify(seam?.design ?? '').slice(0, 700)}

1. **Perpetual American** — new crates/celnet-exotics/src/perpetual.rs on the agnostic carry seam (the ADR-0008 idiom — read american.rs + lib.rs's ExoticInputs first). A perpetual has NO expiry: define PerpetualInputs { spot, strike, vol, carry: Carry } (Copy where possible, mirroring the crate's input idioms). Implement the EXACT closed form under cost-of-carry (r = carry.discount_rate(), b = carry.carry_rate()):
   - roots of ½σ²y(y−1) + b·y − r = 0; call uses the root y₁ > 1: V = (K/(y₁−1))·((y₁−1)/y₁ · S/K)^y₁ with early-exercise boundary S* = K·y₁/(y₁−1); put uses y₂ < 0 analogously. (Provenance in doc comments only.)
   - Degenerate laws (implement + test): b ≥ r ⇒ the perpetual call is never exercised early and V_call = S (handle exactly, no NaN from y₁→1⁺); σ→0 limits; immediate-exercise region V = intrinsic when S beyond the boundary.
   - Analytic greeks (delta/gamma/vega via the closed form; carry/discount rhos via the y-root sensitivities or central-FD if the analytic form is genuinely intractable — if FD, say so honestly in docs).
   - Unit tests: closed form vs an INDEPENDENT in-test re-derivation (solve the quadratic by a DIFFERENT float route — e.g. numerically bisect the characteristic function vs the closed-form discriminant); boundary/degenerate laws; monotonicity in S/σ.
2. **Listed-future option** — NO new Black-76 math (celnet-commodity-vanilla already prices on_future). Add to celnet-commodity-vanilla the futures-style margining variant: futures_style price = the undiscounted Black value (exactly price/discount_df — implement as the closed form with df=1, NOT as a division, to keep the float route principled); greeks adjusted accordingly (no discount rho under futures-style — document why: daily margining removes the financing leg). Additive only; existing fns byte-identical.
   - Unit tests: equity-style == existing on_future path bitwise; futures-style == undiscounted identity vs an erf-route re-derivation; put-call parity in both margining styles (futures-style parity: C − P = F − K exactly, undiscounted).

Gate SERIALLY: cargo test -p celnet-exotics -p celnet-commodity-vanilla && clippy -D + fmt for both. Report literal lines + every formula decision.`,
  { schema: REPORT, phase: 'Engines' },
)
log(`Engines: ${String(engines?.gate ?? '').slice(0, 140)}`)

phase('Oracle')
const oracle = await agent(
  `${COMMON}

PHASE 3 — ORACLE. Files: crates/celnet-golden/, crates/celnet-parity/, tools/check-verification-coverage.mjs (only if its family-derivation needs the two new arm names registered — read it first; it may be fully proto-driven already). Seam: ${JSON.stringify(seam?.design ?? '').slice(0, 400)} Engines: ${JSON.stringify(engines?.design ?? '').slice(0, 700)}

1. Golden vectors for BOTH new families (file names matching the lint's derivation): perpetual (call+put, incl. a b≥r degenerate row and an in-the-exercise-region row) + listed future option (equity-style + futures-style, call+put). Expected values from the celnet-golden oracle module: implement INDEPENDENT closed forms there (libm::erf route + an independently-coded perpetual root solve — code-disjoint from the engines, the crate does not depend on them).
2. celnet-parity rows: perpetual — engine vs the independent oracle + the AMERICAN-FD T→∞ sandwich (price american_fd at t=50y,100y on the same inputs: FD(50) ≤ FD(100) ≤ perpetual, converging — a genuinely code-disjoint engine as oracle) + the b≥r call==spot law; future option — hand-pinned literals INDEPENDENTLY recomputed (state the recomputation route in comments; QuantLib-published Black-76 values where citable) + futures-style undiscounted parity C−P == F−K to_bits-exact + equity-style == df·(futures-style) identity.
3. Run node tools/check-verification-coverage.mjs — it must now pass with 23 product arms + 3 cross-asset families and FAIL if either new vector/row is removed (verify the failure mode literally, then restore).

Gate SERIALLY: cargo test -p celnet-golden -p celnet-parity && cargo clippy -p celnet-parity --tests -- -D warnings (the parity TEST target — hard lesson) && fmt. Report literal lines + every pinned literal + its independent route.`,
  { schema: REPORT, phase: 'Oracle' },
)
log(`Oracle: ${String(oracle?.gate ?? '').slice(0, 140)}`)

phase('Server')
const server = await agent(
  `${COMMON}

PHASE 4 — SERVER. Files: crates/celnet-server/ only. Prior phases: ${JSON.stringify(seam?.design ?? '').slice(0, 300)} | ${JSON.stringify(engines?.design ?? '').slice(0, 400)}

Route the two new product arms in price_instrument (read the existing arm routing — american/fx_forward patterns): perpetual → celnet_exotics::perpetual (validity: expiry_years == 0 enforced as INVALID_ARGUMENT; works on any underlying whose carry the seam produces — same carry guard branching as vanilla); listed future option → celnet-commodity-vanilla on_future with the margining style (validity: future_expiry_years >= expiry_years > 0; Underlying names the asset class). Greeks → wire via the existing CarryGreeks bijection; honest absence where a greek does not exist (perpetual has no theta by definition — emit 0.0 with a doc comment? NO: theta of a perpetual IS exactly 0 (stationary value), that's the honest value; futures-style has no discount-rho — emit the honest 0.0 with the doc rationale). WS codec: the two product frames field-for-field (snake_case) like the other arms. Server tests: each arm vs its engine oracle bitwise; validity rejections (perpetual with expiry, future option with future_expiry < expiry); WS round-trips.

Gate SERIALLY: cargo test -p celnet-server && clippy -D + fmt. Report literal lines.`,
  { schema: REPORT, phase: 'Server' },
)
log(`Server: ${String(server?.gate ?? '').slice(0, 140)}`)

phase('Clients')
const sdk = await agent(
  `${COMMON}

PHASE 5a — SDK. Files: crates/celnet-client/ only. Stack so far: ${JSON.stringify(server?.design ?? '').slice(0, 400)}

InstrumentSpec builders for both families (perpetual(option_type, strike, ...) with NO tenor/expiry — the builder enforces expiry 0; listed_future_option(future_symbol, future_expiry_years, margining, ...)) mirroring the existing builder idiom + vocab types + to_wire mapping to arms 30/31. Extend tests/conformance.rs to price both families through the SDK against the live edge and reconcile to the golden vectors (SDK == server == oracle). Gate SERIALLY: cargo test -p celnet-client && clippy -D + fmt. Report literal lines.`,
  { schema: REPORT, label: 'sdk', phase: 'Clients' },
)
const cliR = await agent(
  `${COMMON}

PHASE 5b — CLI. Files: crates/celnet-cli/ only. SDK: ${JSON.stringify(sdk?.design ?? '').slice(0, 400)}

CLI surface for both families following the existing product-command grammar (the exotic/price subcommand family): perpetual + future-option modes with their terms; conformance rows asserting CLI == server == golden corpus for both. Gate SERIALLY: cargo test -p celnet-cli && clippy -D + fmt. Report literal lines.`,
  { schema: REPORT, label: 'cli', phase: 'Clients' },
)
const nodeClients = await parallel([
  () =>
    agent(
      `${COMMON}

PHASE 5c — GUI. Files: gui/ only (NO cargo). Server WS frames: ${JSON.stringify(server?.design ?? '').slice(0, 400)}

Per the GW2 promise (a product = a registry entry): add gui/src/products/perpetual.tsx + listedFutureOption.tsx ProductSpecs (follow asian.tsx + the linear specs; perpetual has NO tenor — the spec must express that honestly in the ticket UI, e.g. the tenor control disabled/NA with reason; future option needs future symbol/expiry + margining toggle) + contract.ts product payload mirrors + wsCodec arms (do NOT touch MarketContext) + registry entries in a sensible gallery group. Round-trip vitest per family (toInstrument == wire shape, registry grouping) following the existing per-family tests. Extend the conformance test's family declarations honestly (offline-priced or not). Gate: npm run typecheck && npm run typecheck:test && npm test. DO NOT run the e2e (Verify phase). Report literal counts.`,
      { schema: REPORT, label: 'gui', phase: 'Clients' },
    ),
  () =>
    agent(
      `${COMMON}

PHASE 5d — EXCEL. Files: excel/ only (NO cargo). Server WS frames: ${JSON.stringify(server?.design ?? '').slice(0, 400)}

The polymorphic surface makes this a data addition: add PERPETUAL + FUTUREOPTION families to the INSTRUMENT dispatch (excel/src/functions/instrumentSpec.ts terms tables — perpetual: no tenor accepted (typed error if given); future option: future_symbol/future_expiry/margining keys) + contract/wsCodec/instrumentCodec mirrors for arms 30/31 (do NOT touch MarketContext) + unit tests (wire-shape parity vs hand-built frames, token round-trip, typed-error cases) + e2e corpus rows for both families. Gate: npm run typecheck && npm test. DO NOT run the e2e (Verify phase). Report literal counts.`,
      { schema: REPORT, label: 'excel', phase: 'Clients' },
    ),
])

phase('Verify')
const conformance = await agent(
  `${COMMON}

PHASE 6a — FULL GATE. Do in order, SERIAL cargo:
1. cargo build -p celnet-server --example demo_edge   (pre-build; the e2e ready-timeout does not cover cold builds)
2. cargo test -p celnet-proto -p celnet-types -p celnet-exotics -p celnet-commodity-vanilla -p celnet-golden -p celnet-parity -p celnet-server -p celnet-client -p celnet-cli  (ONE invocation)
3. cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo fmt --all -- --check
4. node tools/check-verification-coverage.mjs  (must report 23 product arms + 3 cross-asset families)
5. just workspace-deps && cargo deny check
6. cd gui && npm run typecheck && npm test && npm run e2e   (ALL specs)
7. cd excel && npm run typecheck && npm test && npm run test:e2e
Fix root-causes if anything fails (no weakening) and re-run that gate. Report every literal gate line.`,
  { schema: REPORT, phase: 'Verify' },
)
const verdict = await agent(
  `${COMMON}

PHASE 6b — ADVERSARIAL VERIFY (independent; default false on doubt).
1. PERPETUAL: re-derive the closed form from first principles YOURSELF (the stationary Black-Scholes ODE ½σ²S²V'' + bSV' − rV = 0, power solutions S^y, smooth-pasting at the boundary) — confirm the engine's y-roots, value function, boundary, and the b≥r call==S law match YOUR derivation, not just each other. Check the golden oracle is genuinely code-disjoint (different root solve route) and the American-FD T→∞ sandwich actually brackets (read the parity test's numbers).
2. FUTURE OPTION: verify the futures-style margining law independently (daily margining ⇒ undiscounted Black; C−P == F−K exact) and that equity-style == df×futures-style bitwise in the tests; confirm pinned literals cite an independent recomputation route.
3. COVERAGE: delete (in-memory reasoning, or actually mutate + restore) one new-arm vector and confirm the lint fails — the gate must be real.
4. Hunt counterexamples: perpetual near y₁→1 (b→r⁻) numerical stability; vol→0; deep ITM immediate-exercise; future option with future_expiry == option expiry. If a missing-law unit test is found, ADD it (serial cargo test on that crate) and report.
Fields: perpetualLawIndependent, futureOptionIndependent, coverageEnforced, real, detail.`,
  { schema: VERDICT, phase: 'Verify' },
)

return {
  seam, engines, oracle, server,
  sdk, cli: cliR, gui: nodeClients[0], excel: nodeClients[1],
  conformance, verdict,
  done: Boolean(verdict?.real && verdict?.perpetualLawIndependent && verdict?.futureOptionIndependent && verdict?.coverageEnforced),
}
