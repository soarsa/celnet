export const meta = {
  name: 'w2-multiasset-lanes',
  description: 'W2 wave: build celnet-linear (FX forward/swap/NDF) + conventions/calendar metals breadth as disjoint worker lanes in isolated worktrees, then adversarially verify each against an independent oracle.',
  phases: [
    { title: 'Build', detail: 'two crate-disjoint worker lanes in isolated git worktrees off the W2-freeze HEAD' },
    { title: 'Verify', detail: 'adversarial independent-oracle verification per lane (circular-oracle / workaround / fabrication hunt)' },
  ],
}

const LANE_RESULT = {
  type: 'object',
  additionalProperties: false,
  required: ['lane', 'commit_sha', 'branch', 'built', 'gate_line', 'test_count', 'touched_paths', 'zero_workarounds', 'summary'],
  properties: {
    lane: { type: 'string' },
    built: { type: 'boolean', description: 'true only if the lane gate is fully green and committed' },
    commit_sha: { type: 'string', description: 'output of `git rev-parse HEAD` after the lane commit (empty if not built)' },
    branch: { type: 'string', description: 'output of `git branch --show-current`' },
    gate_line: { type: 'string', description: 'the literal final gate lines (clippy/fmt/nextest summary) verbatim' },
    test_count: { type: 'integer' },
    touched_paths: { type: 'array', items: { type: 'string' }, description: 'every file path created/modified' },
    zero_workarounds: { type: 'boolean', description: 'true if NO #[ignore]/#[allow]-dodge/todo!/lowered tolerance/mock-as-real/fabricated constant was used' },
    summary: { type: 'string' },
  },
}

const VERDICT = {
  type: 'object',
  additionalProperties: false,
  required: ['lane', 'verdict', 'blocking_issues', 'oracle_independent', 'notes'],
  properties: {
    lane: { type: 'string' },
    verdict: { type: 'string', enum: ['accept', 'reject'] },
    oracle_independent: { type: 'boolean', description: 'true if every numeric gate is anchored by an oracle that can DISAGREE (no circular re-derivation of the production algebra)' },
    blocking_issues: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
}

const COMMON = `
You are a WORKER session in the Celnet multi-session service-mesh model (read docs/PARALLEL-SESSIONS.md §3-4).
You run in an ISOLATED git worktree branched off main, which ALREADY carries the W2 contract freeze:
  - celnet_types::{Metal, MetalPair, Underlying::Metal}, Ccy::{XAU,XAG,XPT,XPD}
  - proto product arms FxForward=26 / FxSwap=27 / Ndf=28, Underlying.metal=3, enum Side (SIDE_BUY=0/SIDE_SELL=1/SIDE_TWO_WAY=2), enum Metal, enum FixingSource, message MetalPair
  - celnet_core::carry::CarryInputs { spot, t, carry, underlying } with .forward() and .discount_df(); carry.forward_factor(t) and carry.discount_df(t).
HARD RULES (zero tolerance):
  - Touch ONLY your lane's files (listed below). NEVER edit the proto, root Cargo.toml [workspace.dependencies], any seam crate (celnet-types/core/proto/plugin-api), or another lane's files. The coordinator does cross-crate wiring + the workspace-dep registry.
  - SOTA, ZERO workarounds: no #[ignore], no #[allow] to dodge a lint, no todo!/unimplemented!, no lowered tolerance, no mock-as-real, no fabricated "oracle" constant. If you cannot finish a piece honestly, NARROW scope and say so — never fake depth.
  - Run every Rust command with: source "$HOME/.cargo/env" && cargo ...
  - The 3 hard lessons: (a) verify the LITERAL gate output yourself, never a wrapper exit code; (b) clippy the parity TEST target too (cargo clippy -p celnet-parity --test <name> -- -D warnings), not just the product crate; (c) RE-DERIVE any published constant from its primary source in a comment — never copy it out of the production code (the FRTB 0.75rho circular-oracle lesson).
  - When done: git add -A && git commit -m "<msg>" with trailer "Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>". Then report git rev-parse HEAD and git branch --show-current.
Your FINAL message IS the structured result (the schema), not prose to a human.
`

const WORKER_A = COMMON + `
=== LANE W2-A-LINEAR — NEW crate crates/celnet-linear/ ===
Read docs/W2-LINEAR-PLAN.md §1-3 and §7, docs/VERIFICATION-CONTRACT.md, docs/adr/ADR-0008-multi-asset-carry-architecture.md FIRST.
Build the new leaf crate (deps {celnet-core, celnet-types} via .workspace = true ONLY; auto-joins the members=["crates/*"] glob; do NOT add it to root [workspace.dependencies] — the coordinator does that). Layout:
  Cargo.toml (mirror crates/celnet-qmc/Cargo.toml: package celnet-linear, version/edition/rust-version/license/authors via .workspace, [lints] workspace=true, publish=false)
  src/lib.rs   — crate doc + the verbatim honest-boundary banner (live NDF fixing / metal lease VALUES are ENV; only fixing IDENTITY + settlement convention are in-repo) + re-exports + a crate-local enum Side { Buy, Sell } (celnet-types has none) with a buy=+1/sell=-1 sign helper.
  src/inputs.rs — LinearInputs reading the W1 carry seam (an Underlying + Carry-derived spot/forward_factor/discount via CarryInputs), contract_rate (strike), notional>0, side, near settle t, optional far settle t.
  src/forward.rs — FX outright forward: pv(), fair_forward(), and the linear Greeks (delta/rho_dom/rho_for/theta as exact analytic derivatives).
  src/swap.rs — FX swap = near leg + far leg (two outright forwards, by convention opposite sides); pv() = sum of leg pvs; swap_points helper.
  src/ndf.rs — non-deliverable forward: pv() = side*notional*df_settle*(F-K) discounted in the settlement (convertible) ccy; carries the FixingSource identity as metadata only.
ASSET-AGNOSTIC MANDATE (ADR-0008): the pricers call carry.forward_factor(t)/discount_df(t) and CarryInputs.forward()/discount_df() and NEVER match on the Carry variant. ANY 'match carry { FxRates => .. }' branch in a pricer is a self-blocker — an NDF/forward on a (future) metal/crypto underlying must reuse the identical engine.
INDEPENDENT ORACLE — crates/celnet-linear/src/*.rs #[cfg(test)] modules (QuantLib is unavailable in-sandbox, so use the genuinely-independent ROUTE below + hand-pinned literals per lesson c; at least one gate must be able to DISAGREE):
  - Production PV computes  side*notional*df*(F-K)  with F=spot*forward_factor(t), df=discount_df(t).
    The INDEPENDENT oracle computes the SAME PV by a DIFFERENT route that shares NO intermediate:
       PV = side*notional*( spot*exp(-r_for*t) - K*exp(-r_dom*t) )      (long base-discount-bond minus short quote-discount-bond)
    computed directly from the two rates. Algebraically equal, different rounding/route ⇒ a real cross-check. Re-derive the covered-interest-parity identity in a comment from first principles, NOT from the impl.
  - Structural gates that CAN disagree: fair-forward (K=fair_forward) ⇒ PV exactly 0.0 (to_bits after sign-normalising -0.0); linearity (2x notional ⇒ 2x PV, ~1e-12); netting (long + short at the same rate = 0); t→0 ⇒ PV → side*notional*(spot-K) (undiscounted, a DIFFERENT expression).
  - One hand-pinned absolute PV literal: state spot, K, r_dom, r_for, t, notional as comments, give the externally-computed PV value, derive it in the comment.
  - NDF: hand-derived PV literal == a deliverable forward of equal terms priced in the same numeraire (non-deliverability changes settlement mechanics, NOT the risk-neutral PV) — two independent checks (an absolute value AND a structural equality).
  - Swap: near==far date + opposite sides nets to 0; swap PV == the independent two-leg sum.
GATE (run, capture the LITERAL output, paste into gate_line):
  source "$HOME/.cargo/env" && cargo fmt -p celnet-linear && cargo clippy -p celnet-linear --all-targets -- -D warnings && cargo nextest run -p celnet-linear
All three must be clean (zero warnings, all tests pass). Then commit and report.
`

const WORKER_B = COMMON + `
=== LANE W2-B-BREADTH — celnet-conventions + celnet-calendar + celnet-parity/tests/pair_universe.rs ===
Read docs/W2-LINEAR-PLAN.md §4 (4.1-4.3) and §7, docs/CONVENTIONS.md, and the EXISTING crates/celnet-parity/tests/pair_universe.rs (it already carries a 19-pair PUBLISHED table + an independent Hinnant rata-die + holiday-computus oracle — you EXTEND that exact pattern, you do NOT invent a new oracle and do NOT copy the library calendar into it).
NO proto/wire/type change — Underlying::Metal/MetalPair/Metal and Ccy::{XAU,XAG,XPT,XPD} are ALREADY on main from the freeze. Your lane is conventions data + calendar centres + the parity oracle ONLY.
Scope (S1):
  - celnet-conventions/src/registry.rs: grow the covered set from 19 to a >75-pair SUPERSET — G10 majors + all major crosses + the EM deliverable set + the EM NDF panel (match/exceed the documented 75-pair panel). Add XPT/XPD vs USD (mirror the existing XAU/XAG PreciousMetal profile) and metal crosses (XAUEUR, XAUJPY, XAGEUR, XPTUSD, XPDUSD): metal as base, loco-London settlement, DNS ATM, premium in the quote ccy (unadjusted), NY cut, T+2. Encode a lease-rate-bearing convention for the metal leg (explicit named field preferred) per §4.2.
  - celnet-calendar: loco-London metals settle on the UK (CentreId::UnitedKingdom) calendar (already modelled); metal crosses intersect London ∩ quote-ccy centre (∩ USD for non-USD crosses). Add only Gregorian-COMPUTABLE fiat centres needed for the >75 panel. For any currency whose onshore calendar is lunisolar / not modellable, set has_calendar_support=false HONESTLY (the existing NDF discipline) — never fake a calendar.
ACCURACY MANDATE: use only GENUINE published market conventions (EMTA/ISDA per-currency templates for FX/NDF; LBMA/LPPM for metals). Do NOT fabricate a convention. Where you are not certain of a value, be conservative and say so in the summary rather than guess. The PUBLISHED table is the oracle, not a copy of the code.
INDEPENDENT ORACLE — extend crates/celnet-parity/tests/pair_universe.rs:
  - Grow the PUBLISHED table to the full >75 + XPT/XPD + metal-cross set; each row a market-standard literal re-derived from its primary source in a comment.
  - Extend the independent rata-die / holiday-walk to the new Gregorian-computable centres + the metal-cross London∩quote∩USD leg logic; sweep every (pair, trade-date) across 2024-2025; the independent walk must equal the library to the day. Keep independent_date_engine_is_correct pinning the rata-die to known anchors so two-wrong-implementations-agree cannot occur.
  - Add the byte-identity structural check that the metal-as-Underlying::Metal path resolves to_bits-identically to the legacy metal-as-CcyPair path (carry: lease modelled as foreign rate). This is a byte gate, NOT a correctness oracle.
  - Structural invariants: metals metal-as-base + quote-ccy unadjusted premium; metal crosses loco-London T+2; NDF invariants hold across the grown panel; orientation-inversion stays non-contradictory.
GATE (run, capture LITERAL output into gate_line):
  source "$HOME/.cargo/env" && cargo fmt -p celnet-conventions -p celnet-calendar && cargo clippy -p celnet-conventions -p celnet-calendar --all-targets -- -D warnings && cargo clippy -p celnet-parity --test pair_universe -- -D warnings && cargo nextest run -p celnet-conventions -p celnet-calendar && cargo nextest run -p celnet-parity --test pair_universe
All clean. Then commit and report. State the exact final pair COUNT in summary so the coordinator can confirm >75.
`

const LANES = [
  { id: 'w2-a-linear', prompt: WORKER_A },
  { id: 'w2-b-breadth', prompt: WORKER_B },
]

const VERIFY = (lane, built) => `
You are an ADVERSARIAL VERIFIER for Celnet lane ${lane.id}. The worker committed ${built.commit_sha} on branch ${built.branch}.
Read the diff and the tests WITHOUT trusting the worker's summary:
  source "$HOME/.cargo/env"
  git show --stat ${built.commit_sha}
  git diff main..${built.commit_sha}   (or: git show ${built.commit_sha})
Hunt, specifically:
  1. CIRCULAR ORACLE — does any numeric gate re-derive the SAME algebra the production code uses (e.g. recomputing S*forward_factor the impl already computes)? Is there at least one gate that could DISAGREE (structural/limit/independent-route)? For W2-A confirm the PV oracle uses the spot*exp(-r_for*t)-K*exp(-r_dom*t) route, NOT df*(F-K). For W2-B confirm the date oracle is a genuinely different engine and the PUBLISHED values are market-standard, not code-copied.
  2. WORKAROUNDS — grep the diff for #[ignore], #[allow], todo!, unimplemented!, lowered tolerance, mock-as-real, fabricated constants, a Carry-variant match in a linear pricer (ADR-0008 blocker).
  3. FABRICATION (W2-B especially) — are the conventions / holiday rules genuinely published, or invented? Is has_calendar_support=false used honestly for non-modellable calendars rather than a faked calendar?
  4. RE-GATE — actually re-run the lane gate yourself and confirm the LITERAL green output. Re-derive ONE pinned constant from first principles.
Return the verdict schema. verdict=reject if ANY blocking issue; list each precisely so the coordinator can fix forward. oracle_independent reflects finding #1.
`

const results = await pipeline(
  LANES,
  lane => agent(lane.prompt, { label: `build:${lane.id}`, phase: 'Build', isolation: 'worktree', schema: LANE_RESULT }),
  (built, lane) => {
    if (!built || !built.built || !built.commit_sha) {
      return { lane: lane.id, built, verdict: { lane: lane.id, verdict: 'reject', oracle_independent: false, blocking_issues: ['build agent did not produce a green committed lane'], notes: '' } }
    }
    return agent(VERIFY(lane, built), { label: `verify:${lane.id}`, phase: 'Verify', schema: VERDICT })
      .then(verdict => ({ lane: lane.id, built, verdict }))
  }
)

log(`W2 lanes complete: ${results.map(r => `${r.lane}=${r.verdict ? r.verdict.verdict : '??'}`).join(', ')}`)
return results
