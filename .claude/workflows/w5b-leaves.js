export const meta = {
  name: 'w5b-crossasset-leaves',
  description: 'W5-B: build celnet-equity-vanilla (generalized-BSM, dividend yield) + celnet-commodity-vanilla (Black-76) leaf crates on the W1 carry seam, in isolated worktrees, each with an INDEPENDENT (external-QuantLib-pinned + can-disagree) oracle, then adversarially verify each. Disjoint NEW crates — zero overlap with the parallel session W2.',
  phases: [
    { title: 'Build', detail: 'two disjoint leaf crates in isolated worktrees' },
    { title: 'Verify', detail: 'adversarial circular-oracle / workaround hunt per leaf' },
  ],
}

const LANE_RESULT = {
  type: 'object',
  additionalProperties: false,
  required: ['leaf', 'commit_sha', 'branch', 'built', 'gate_line', 'test_count', 'touched_paths', 'zero_workarounds', 'oracle_summary'],
  properties: {
    leaf: { type: 'string' },
    built: { type: 'boolean' },
    commit_sha: { type: 'string' },
    branch: { type: 'string' },
    gate_line: { type: 'string' },
    test_count: { type: 'integer' },
    touched_paths: { type: 'array', items: { type: 'string' } },
    zero_workarounds: { type: 'boolean' },
    oracle_summary: { type: 'string', description: 'the independent oracle(s) used + which gate can disagree (circular-oracle mitigation)' },
  },
}

const VERDICT = {
  type: 'object',
  additionalProperties: false,
  required: ['leaf', 'verdict', 'oracle_independent', 'blocking_issues', 'notes'],
  properties: {
    leaf: { type: 'string' },
    verdict: { type: 'string', enum: ['accept', 'reject'] },
    oracle_independent: { type: 'boolean' },
    blocking_issues: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

const COMMON = `
You are a WORKER in the Celnet multi-session model, in an ISOLATED git worktree off origin/main. Build ONE new leaf pricing crate. Touch ONLY your new crate dir crates/<your-crate>/. NEVER edit the proto, root Cargo.toml, any seam crate (celnet-types/core/proto), celnet-parity, celnet-golden, or another crate — the coordinator does cross-crate wiring + the workspace-dep registry + any new Underlying arm. Your crate auto-joins via members=["crates/*"] and depends ONLY on the already-registered celnet-core + celnet-types via \`.workspace = true\` (NO path-deps), so it gates standalone WITHOUT a root edit.
READ FIRST: crates/celnet-vanilla/src/lib.rs (the FX Garman-Kohlhagen leaf sibling you mirror — same crate shape, Greek-strip style, test style) + crates/celnet-core/src/carry.rs (the W1 carry seam: CarryInputs, Carry::CostOfCarry{r,b}, .forward()/.discount_df()/forward_factor(t)/discount_df(t)) + crates/celnet-types (RateSensitivities::Carry{discount_rho, carry_rho}, OptionType) + docs/W5-CROSSASSET-RISK-PLAN.md §2 + docs/VERIFICATION-CONTRACT.md.
HARD RULES (zero tolerance): SOTA, no workarounds (no #[ignore]/#[allow]-dodge/todo!/unimplemented!/lowered tolerance/mock/fabricated constant). Run every cargo cmd with: source "$HOME/.cargo/env" && cargo ...
ADR-0008 NO-MATCH-CARRY: the pricer must be asset-class-agnostic — read the cost-of-carry parameters via the carry seam (forward = spot·forward_factor(t) under Carry::CostOfCarry{r,b}; discount = discount_df(t)). NO \`match carry { ... }\` and NO branch on a Underlying variant in the pricer. (The crate does NOT need a new Underlying::Equity/Commodity arm — that wire identity is the coordinator's deferred seam step; price from the carry parameters directly.)
The 3 hard lessons: (a) verify the literal "test result: ok" / gate line yourself; (b) if you add a clippy target, lint it; (c) RE-DERIVE any pinned reference constant from its primary source in a comment.
GATE (run, capture literal output): source "$HOME/.cargo/env" && cargo fmt -p <crate> && cargo clippy -p <crate> --all-targets -- -D warnings && cargo nextest run -p <crate>
All clean. Then: git switch -c <branch> 2>/dev/null || true ; git add -A && git commit -m "<msg>" (trailer Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>). Report git rev-parse HEAD + git branch --show-current.
Your FINAL message IS the structured result.
`

const LANES = [
  {
    leaf: 'celnet-equity-vanilla',
    branch: 'leaf/equity',
    prompt: `Build crates/celnet-equity-vanilla — European equity-option pricing via GENERALIZED Black-Scholes-Merton on the cost-of-carry seam: b = r − q (continuous dividend yield), optional repo folded in (b = r − q − repo). forward = spot·e^{b·t}, discount = e^{−r·t}. Deliver: an inputs struct (spot, strike, vol, expiry t, r, q, optional repo, OptionType), price(), and the FULL Greek strip (delta, gamma, vega, theta, rho), emitting RateSensitivities::Carry{ discount_rho = ∂V/∂r, carry_rho = ∂V/∂b = the DIVIDEND-RHO }. Mirror celnet-vanilla's structure + doc style; libm via celnet_core::math.
INDEPENDENT ORACLE — **CIRCULAR-ORACLE RISK IS HIGH**: generalized-BSM with b=r−q is ALGEBRAICALLY identical to Garman-Kohlhagen with r_for↦q, so a "second closed form" written in-test is CIRCULAR and forbidden. Your oracle MUST be: (1) external QuantLib AnalyticEuropeanEngine / BlackScholesMertonProcess VALUES hand-pinned as literals (state spot/strike/vol/t/r/q + the externally-computed price in a comment, tol ~1e-10/1e-12) — QuantLib is NOT in-sandbox so pin published/externally-computed numbers per lesson c; (2) gates that CAN DISAGREE — put-call parity C − P = e^{−qT}·S − e^{−rT}·K (to ~1e-12), the q=0 → standard non-dividend BSM limit (a different expression), a PUBLISHED equity-option reference (e.g. a Hull "Options, Futures, and Other Derivatives" worked example) hand-pinned WITH the citation + re-derived in comment; (3) the dividend-rho (carry_rho) cross-checked by independent central finite difference of price wrt b. Also: deep-ITM/OTM limits, vega≥0, zero-vol intrinsic.
Crate: celnet-equity-vanilla; branch leaf/equity.`,
  },
  {
    leaf: 'celnet-commodity-vanilla',
    branch: 'leaf/commodity',
    prompt: `Build crates/celnet-commodity-vanilla — European commodity-option pricing: Black-76 on a futures price F (option on a listed future → zero carry on F under the futures measure: forward = F, discount = e^{−rT}), AND the spot+convenience representation b = r − convenience via the cost-of-carry seam (Black-76 is the b=0 degenerate priced off F). Deliver: an inputs struct (F or spot, strike, vol, expiry, r, convenience/b, OptionType), price(), full Greek strip, RateSensitivities::Carry{ discount_rho, carry_rho = the CONVENIENCE/CARRY-RHO }. Mirror celnet-vanilla; libm via celnet_core::math.
INDEPENDENT ORACLE — **CIRCULAR-ORACLE RISK**: writing the Black-76 formula directly in-test is the SAME code path as the impl ⇒ circular. So anchor on: (1) external QuantLib commodity/Black-76 VALUES hand-pinned as literals (inputs + externally-computed price in comment, tol ~1e-10), per lesson c; (2) can-disagree gates — put-call parity on the future C − P = e^{−rT}(F − K) (~1e-12), the b=0 degenerate equals the generalized-BSM-on-spot with forward=F (a different parameterization reaching the same number), a published Black-76 reference value hand-pinned + cited + re-derived; (3) carry_rho (convenience-rho) by independent central finite difference. Also: zero-vol intrinsic on the discounted (F−K), vega≥0, monotonicity in F.
Crate: celnet-commodity-vanilla; branch leaf/commodity.`,
  },
]

const results = await pipeline(
  LANES,
  (lane) => agent(COMMON + `\n=== YOUR LEAF: ${lane.leaf} (branch ${lane.branch}) ===\n${lane.prompt}`, {
    label: `build:${lane.leaf}`, phase: 'Build', isolation: 'worktree', schema: LANE_RESULT,
  }),
  (built, lane) => {
    if (!built || !built.built || !built.commit_sha) {
      return { leaf: lane.leaf, built, verdict: { leaf: lane.leaf, verdict: 'reject', oracle_independent: false, blocking_issues: ['no green committed leaf'], notes: '' } }
    }
    return agent(
      `You are an ADVERSARIAL VERIFIER for the Celnet leaf ${lane.leaf} (commit ${built.commit_sha}, branch ${built.branch}). source "$HOME/.cargo/env". Read the diff: git show ${built.commit_sha}. Hunt:\n` +
      `1. CIRCULAR ORACLE — does any price gate re-derive the SAME closed form the impl uses (forbidden: equity = GK with r_for↦q; commodity = Black-76 written twice)? Confirm the external pinned values are genuinely external (re-derive ONE from first principles yourself) AND there is at least one can-disagree gate (put-call parity / q=0 or b=0 limit / published reference / FD greek).\n` +
      `2. NO-MATCH-CARRY (ADR-0008) — grep the pricer for 'match' on a Carry or Underlying variant; the price/forward/discount must flow only through the carry-seam params. A branch is a blocker.\n` +
      `3. WORKAROUNDS — #[ignore]/#[allow]/todo!/lowered tolerance/fabricated constant/mock.\n` +
      `4. RE-GATE — re-run cargo clippy -D + nextest on the crate; confirm the literal green.\n` +
      `Return the verdict schema; reject with precise blocking issues if anything fails.`,
      { label: `verify:${lane.leaf}`, phase: 'Verify', schema: VERDICT }
    ).then((verdict) => ({ leaf: lane.leaf, built, verdict }))
  }
)

log(`W5-B leaves: ${results.map((r) => `${r.leaf}=${r.verdict ? r.verdict.verdict : '??'}`).join(', ')}`)
return results
