export const meta = {
  name: 'w4b-multidealer-rfq',
  description: 'W4-B: build the NEW celnet-rfq crate (MultiDealerEngine — concurrent fan-out, best-bid/offer ranking, deterministic tie-break, timeout/last-look) + a real FixLpAdapter over loopback celnet-fix, gated by >=3 synthetic LP responders with injected ground-truth ladders. Then adversarial verify. Disjoint new crate — zero overlap with the parallel session W2.',
  phases: [
    { title: 'Build', detail: 'the celnet-rfq engine + loopback FIX adapter + >=3-LP oracle harness' },
    { title: 'Verify', detail: 'adversarial: ranking-vs-injected-ground-truth + real-loopback + workaround hunt' },
  ],
}

const BUILD = {
  type: 'object', additionalProperties: false,
  required: ['built', 'commit_sha', 'branch', 'gate_line', 'test_count', 'touched_paths', 'zero_workarounds', 'fix_is_real_loopback', 'oracle_summary'],
  properties: {
    built: { type: 'boolean' }, commit_sha: { type: 'string' }, branch: { type: 'string' },
    gate_line: { type: 'string' }, test_count: { type: 'integer' },
    touched_paths: { type: 'array', items: { type: 'string' } },
    zero_workarounds: { type: 'boolean' },
    fix_is_real_loopback: { type: 'boolean', description: 'true if >=1 FixLpAdapter runs over a REAL celnet-fix initiator/acceptor on an ephemeral loopback socket (not a mock)' },
    oracle_summary: { type: 'string' },
  },
}
const VERDICT = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'oracle_independent', 'fix_loopback_real', 'blocking_issues', 'notes'],
  properties: {
    verdict: { type: 'string', enum: ['accept', 'reject'] },
    oracle_independent: { type: 'boolean' }, fix_loopback_real: { type: 'boolean' },
    blocking_issues: { type: 'array', items: { type: 'string' } }, notes: { type: 'string' },
  },
}

const BUILD_PROMPT = `
You are a WORKER in the Celnet multi-session model, in an ISOLATED git worktree off origin/main. Build the NEW leaf crate crates/celnet-rfq. Touch ONLY that new crate dir. NEVER edit the proto, root Cargo.toml, seam crates, celnet-server/client/cli, celnet-fix, or any other crate — the coordinator does the proto MultiDealerQuote/QuoteService RPC + AcceptQuote panel-winner + 5-client surfacing (all DEFERRED). Your crate auto-joins via members=["crates/*"] and depends ONLY on already-registered crates via .workspace = true — so it gates standalone with NO root edit.
READ FIRST: docs/W4-STRUCTURED-RFQ-PLAN.md "Track B" (the crate layout, ranking semantics, last-look, and the >=3-LP loopback oracle), crates/celnet-fix/src/lib.rs (the FIX 4.4 engine: the Initiator/Acceptor roles, QuoteRequest/Quote messages, dialect_fx — the API your FixLpAdapter wraps), crates/celnet-server/tests/risk_federation.rs (the REAL loopback ephemeral-port test pattern you MUST mirror — std::net/tokio, no shared-mem fake), crates/celnet-server/src/services/forward.rs or any federate.rs (the concurrent fan-out precedent: futures::join_all over a bounded panel), and the QuoteRequest/Quote/TwoWayPrice types in celnet-proto/celnet-types.
DELIVER crates/celnet-rfq/{Cargo.toml, src/lib.rs, src/panel.rs, src/lp_fix.rs, src/internal.rs} + tests/:
 - panel.rs: an object-safe \`QuoteSource\` trait (one async method request(&self, &QuoteRequest, deadline) -> QuoteSourceReply), and a \`MultiDealerEngine\` that fans one QuoteRequest to N sources CONCURRENTLY (futures::join_all / tokio over the bounded panel), then RANKS: best bid = max bid premium, best offer = min offer premium; TIE-BREAK deterministically on equal best price by (1) earlier epoch_nanos then (2) lexicographically smallest stable lp_id; TIMEOUT/last-look: per-source deadline (non-responders DROPPED, not errored, excluded from lp_count); a winner past its valid_until_nanos is REJECTED (last-look) and the next-best promoted. Return an audited ranked panel with lp_count (responders), lp_won_bid/lp_won_offer (subset of responders), and per-LP DealerQuote rows. The engine asserts lp_won references a real responder (the consistency invariant).
 - lp_fix.rs: \`FixLpAdapter\` — a QuoteSource wrapping a celnet-fix Initiator over a REAL loopback socket (ephemeral port, mirror risk_federation.rs), translating Celnet QuoteRequest <-> FIX QuoteRequest/Quote via celnet-fix's dialect. This must be a genuine FIX session over TCP loopback, NOT a mock.
 - internal.rs: \`InternalPricerSource\` — a QuoteSource computing a two-way quote in-process (a simple deterministic spread around a provided mid is fine; the panel must always have >=1 native dealer).
ORACLE (tests/, oracle class 4 + structural — the injected ladder is the ground truth, so it CAN disagree, NOT circular): boot >=3 synthetic LP responders (at least ONE real FixLpAdapter over a loopback celnet-fix initiator/acceptor pair, plus deterministic in-process sources with KNOWN injected bid/offer ladders). Gate: (1) best-bid/best-offer == the injected extremum (computed independently from the injected ladders, not from the engine); (2) tie-break determinism (inject two equal-best LPs -> the documented preference winner); (3) timeout (one LP sleeps past the deadline -> dropped, lp_count excludes it, next-best promoted); (4) last-look (winner valid_until in the past -> rejected, promotion); (5) lp_count/lp_won consistency invariant over a proptest sweep of random ladders. HONEST BOUNDARY (verbatim in the crate doc + a test): live LP-panel WAN connectivity + regulated-venue status are ENV (gate behind an env flag, defaulting to the synthetic in-repo panel); in-repo proves the aggregation/ranking/tie-break/last-look ALGORITHM + the FIX framing over loopback only.
RULES: SOTA, zero workarounds (no #[ignore]/#[allow]-dodge/todo!/lowered tolerance/MOCK-as-real — the FIX leg must be a real loopback session; if a real-socket test is flaky under parallel nextest, add a serial group in .config/nextest.toml like the existing engine-serial/replog-consensus groups, never #[ignore]). source "$HOME/.cargo/env" && cargo ... GATE (capture literal): cargo fmt -p celnet-rfq && cargo clippy -p celnet-rfq --all-targets -- -D warnings && cargo nextest run -p celnet-rfq — all clean. Then git switch -c leaf/rfq && git add -A && git commit -m "feat(rfq): celnet-rfq — multi-dealer RFQ aggregation engine + loopback FIX LP adapter" (trailer Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>). Report git rev-parse HEAD + branch + whether the FIX leg is a real loopback session.
Your FINAL message IS the structured result.
`

phase('Build')
const built = await agent(BUILD_PROMPT, { label: 'build:celnet-rfq', phase: 'Build', isolation: 'worktree', schema: BUILD })

let verdict = null
if (built && built.built && built.commit_sha) {
  verdict = await agent(
    `Adversarially verify the Celnet crate celnet-rfq (commit ${built.commit_sha}, branch ${built.branch}). source "$HOME/.cargo/env". Read: git show ${built.commit_sha}; the panel.rs ranking/tie-break/last-look + the tests harness.\n` +
    `Hunt: (1) ORACLE INDEPENDENCE — is the expected winner computed from the INJECTED ladders independently (ground truth), or does the test re-call the engine's own ranking (circular)? The oracle must be able to DISAGREE. (2) REAL LOOPBACK — does >=1 FixLpAdapter run over a GENUINE celnet-fix initiator/acceptor on an ephemeral TCP loopback socket (grep for a real TcpListener/connect + celnet-fix session), or is the "FIX" leg a mock/in-memory fake? A mock-as-real is a blocking reject. (3) RANKING/TIE-BREAK/LAST-LOOK — re-derive by hand: best bid=max, best offer=min; tie-break (earlier epoch_nanos then smallest lp_id); timeout drops a non-responder from lp_count; a past-valid_until winner is rejected + next-best promoted; lp_won subset of responders. Confirm each is gated. (4) WORKAROUNDS — #[ignore]/#[allow]-dodge/todo!/lowered tolerance/mock-as-real. (5) HONEST BOUNDARY — the live-LP-panel ENV carve-out is stated verbatim (no in-repo claim of live WAN connectivity). (6) RE-GATE — re-run cargo clippy -D + nextest -p celnet-rfq; confirm the literal green. Reject with precise blocking issues.`,
    { label: 'verify:celnet-rfq', phase: 'Verify', schema: VERDICT }
  )
}

log(`W4-B-RFQ: built=${built ? built.built : false}, fix_real=${built ? built.fix_is_real_loopback : '?'}, verdict=${verdict ? verdict.verdict : 'n/a'}`)
return { built, verdict }
