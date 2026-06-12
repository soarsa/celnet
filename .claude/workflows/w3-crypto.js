export const meta = {
  name: 'w3-crypto-vanilla',
  description: 'W3-CRYPTO: build the NEW celnet-crypto-vanilla leaf (linear GK-funding + inverse/coin-margined 1/S_T payoff) on the W1 carry seam, then DUAL adversarial verify (measure re-derivation + circular-oracle/workaround). Disjoint new crate — zero overlap with the parallel session W2.',
  phases: [
    { title: 'Build', detail: 'the celnet-crypto-vanilla leaf + 3-way inverse oracle' },
    { title: 'Verify', detail: 'two parallel adversarial verifiers — measure lens + circular/workaround lens' },
  ],
}

const BUILD = {
  type: 'object', additionalProperties: false,
  required: ['built', 'commit_sha', 'branch', 'gate_line', 'test_count', 'touched_paths', 'zero_workarounds', 'oracle_summary', 'inverse_measure_note'],
  properties: {
    built: { type: 'boolean' }, commit_sha: { type: 'string' }, branch: { type: 'string' },
    gate_line: { type: 'string' }, test_count: { type: 'integer' },
    touched_paths: { type: 'array', items: { type: 'string' } },
    zero_workarounds: { type: 'boolean' },
    oracle_summary: { type: 'string' },
    inverse_measure_note: { type: 'string', description: 'the exact inverse coin-measure closed form used + why it is NOT the naive V_lin/S_0' },
  },
}
const VERDICT = {
  type: 'object', additionalProperties: false,
  required: ['lens', 'verdict', 'inverse_measure_correct', 'oracle_independent', 'blocking_issues', 'notes'],
  properties: {
    lens: { type: 'string' }, verdict: { type: 'string', enum: ['accept', 'reject'] },
    inverse_measure_correct: { type: 'boolean' }, oracle_independent: { type: 'boolean' },
    blocking_issues: { type: 'array', items: { type: 'string' } }, notes: { type: 'string' },
  },
}

const BUILD_PROMPT = `
You are a WORKER in the Celnet multi-session model, in an ISOLATED git worktree off origin/main. Build the NEW leaf crate crates/celnet-crypto-vanilla. Touch ONLY that new crate dir. NEVER edit the proto, root Cargo.toml, seam crates (celnet-types/core/proto), celnet-surface, celnet-parity, celnet-golden, or another crate — the coordinator does the workspace-dep registry + the Underlying::DigitalAsset seam arm + the crypto surface leaf + proto + surfacing (all DEFERRED). Your crate auto-joins via members=["crates/*"] and depends ONLY on the already-registered celnet-core + celnet-types (via .workspace = true), with a DEV-dep on celnet-vanilla (the FX leaf, for the linear parity oracle) — so it gates standalone with NO root edit.
READ FIRST: docs/W3-CRYPTO-PLAN.md (§1 layout, §4 linear, §4b the inverse measure derivation, §6 the THREE independent oracles + the anti-circular discipline), crates/celnet-vanilla/src/lib.rs (the FX leaf sibling + Greek/test style), crates/celnet-equity-vanilla/src/lib.rs and crates/celnet-commodity-vanilla/src/lib.rs (the just-landed sibling leaves on the carry seam — mirror their shape exactly), crates/celnet-core/src/carry.rs (CarryInputs, Carry::CostOfCarry{r,b}, forward()/discount_df()), docs/VERIFICATION-CONTRACT.md.
DELIVER crates/celnet-crypto-vanilla/{Cargo.toml, src/lib.rs, src/linear.rs, src/inverse.rs, src/funding.rs, src/settlement.rs}:
 - LINEAR (linear.rs): generalized-BSM V_lin = φ·df·(F·N(φd1) − K·N(φd2)) via the carry seam with funding: b = r − funding (funding.rs assembles Carry::CostOfCarry{r, b}). ASSET-AGNOSTIC: forward/discount ONLY via inputs.forward()/discount_df()/forward_factor(t). NO match on Carry/Underlying. Zero new payoff math — it's the shared generalized-BSM.
 - INVERSE (inverse.rs) — THE CRUX (read §4b carefully): the coin-margined contract pays max(φ(S_T−K),0)/S_T COINS per USD-notional-1; its COIN price requires the share/coin-measure change (Radon-Nikodym dQ_coin/dQ = e^{(r−b)t}·S_T/S_0). The closed form is V_coin = (φ/S_0)·e^{−b·t}·( N(φ d1) − (K/F)·N(φ d2) ) coins (derive the exact constant assembly yourself from the numeraire change; document the full derivation in the inverse.rs module doc). The naive V_lin/S_0 is WRONG (the Cov(1/S_T, payoff) convexity term is material). Greeks for the inverse are the COIN-measure sensitivities (a coin-margined desk hedges in coins) — documented, with the USD-equivalent exposed as a derived field. settlement.rs routes Linear vs InverseCoin.
INDEPENDENT ORACLE (#[cfg(test)]; §6 — REQUIRE ALL of these, the inverse needs all THREE to agree):
 - Linear: (1) the funding→r_for IDENTITY — a linear crypto vanilla with (r, b=r−funding) is to_bits-identical to celnet_vanilla::price called with r_dom=r, r_for=funding (route the oracle through the already-golden FX leaf, NOT a re-derived GK — to_bits, not approx); (2) put-call parity C−P=df·(F−K); (3) funding=r ⇒ b=0 → Black-76 forward limit.
 - Inverse (HIGH circular-oracle risk — the FRTB-0.75ρ trap): (1) an INDEPENDENT Gauss-Hermite quadrature of df·max(φ(S_T−K),0)/S_T against the lognormal density (NOT the production CDF assembly); (2) a CODE-DISJOINT splitmix64 + Box-Muller MC simulating S_T=S0·exp((b−½σ²)t+σ√t·Z), averaging df·max/S_T, gated WITHIN the reported MC standard error (never to closed-form precision); (3) the STRUCTURAL CONVEXITY SANDWICH V_inverse·S_0 > V_linear strictly (Jensen on 1/S_T) — the qualitative gate that fails loudly on a naive rescale. The MC is the disagree-capable oracle: it must catch a shared measure error in both closed forms. Plus K→0 / deep-ITM-OTM monotonicity limits.
 - Conventions: hand-pin published Deribit contract specs (contract unit, settlement coin, 08:00 UTC expiry cut, index fixing definition, tick) WITH the citation; live fixing VALUES are ENV (honest boundary in the crate doc).
RULES: SOTA, zero workarounds (no #[ignore]/#[allow]-dodge/todo!/lowered tolerance/mock/fabricated constant). source "$HOME/.cargo/env" && cargo ... GATE (capture literal): cargo fmt -p celnet-crypto-vanilla && cargo clippy -p celnet-crypto-vanilla --all-targets -- -D warnings && cargo nextest run -p celnet-crypto-vanilla — all clean. Then git switch -c leaf/crypto && git add -A && git commit -m "feat(crypto): celnet-crypto-vanilla — linear + inverse coin-margined vanilla on the carry seam" (trailer Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>). Report git rev-parse HEAD + branch.
Your FINAL message IS the structured result; in inverse_measure_note give the exact V_coin formula + why it differs from V_lin/S_0.
`

phase('Build')
const built = await agent(BUILD_PROMPT, { label: 'build:crypto-vanilla', phase: 'Build', isolation: 'worktree', schema: BUILD })

let verdicts = []
if (built && built.built && built.commit_sha) {
  const base = `Adversarially verify the Celnet leaf celnet-crypto-vanilla (commit ${built.commit_sha}, branch ${built.branch}). source "$HOME/.cargo/env". Read: git show ${built.commit_sha}; the inverse.rs module + its tests; docs/W3-CRYPTO-PLAN.md §4b/§6.`
  verdicts = await parallel([
    () => agent(base + `\n=== LENS: INVERSE MEASURE (the crux) ===\nThe inverse coin-margined 1/S_T payoff is the documented circular-oracle trap. Independently RE-DERIVE the coin-measure closed form from the numeraire change (dQ_coin/dQ = e^{(r−b)t}·S_T/S_0) from first principles — do NOT read it off inverse.rs. Confirm the production V_coin matches YOUR derivation. Then: (1) re-run / re-implement the splitmix64+Box-Muller MC of df·max(φ(S_T−K),0)/S_T yourself for a couple of cases and confirm it agrees with the closed form within MC stderr; (2) confirm the convexity sandwich V_inverse·S_0 > V_linear holds strictly and is gated; (3) confirm the naive V_lin/S_0 would FAIL (i.e. the test actually distinguishes them). Set inverse_measure_correct accordingly. Reject if the measure is wrong or any of the 3 inverse oracles is missing/circular.`,
      { label: 'verify:measure', phase: 'Verify', schema: VERDICT }),
    () => agent(base + `\n=== LENS: CIRCULAR-ORACLE / NO-MATCH-CARRY / WORKAROUNDS ===\n(1) CIRCULAR: does the linear oracle route through celnet_vanilla::price via the funding→r_for identity to_bits (good), or re-derive GK in-test (circular/bad)? Does the inverse quadrature oracle reuse the production CDF assembly (circular)? Is the splitmix64 MC genuinely code-disjoint? (2) NO-MATCH-CARRY (ADR-0008): grep the pricers for a match on Carry/Underlying variant — forward/discount must flow only via the carry seam; a branch is a blocker (match on OptionType/SettlementStyle is fine). (3) WORKAROUNDS: #[ignore]/#[allow]-dodge/todo!/lowered tolerance/fabricated constant/mock. (4) RE-GATE: re-run cargo clippy -D + nextest on the crate; confirm the literal green. Reject with precise blocking issues.`,
      { label: 'verify:rigor', phase: 'Verify', schema: VERDICT }),
  ])
}

log(`W3-CRYPTO: built=${built ? built.built : false}; verdicts=${verdicts.filter(Boolean).map((v) => `${v.lens}:${v.verdict}`).join(', ')}`)
return { built, verdicts }
