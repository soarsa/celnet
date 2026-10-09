export const meta = {
  name: 'convergence-round',
  description: 'One full convergence round: SEVEN parallel read-only critique lenses over main → dedup vs the backlog → genuine-gap calibration → a round verdict (dry or not).',
  whenToUse: 'The MASTER-EVOLUTION-PROGRAM §6 loop — run until two consecutive rounds are dry.',
  phases: [
    { title: 'Lenses', detail: '7 read-only critiques in parallel (zero cargo)' },
    { title: 'Synthesis', detail: 'dedup vs WORLD-CLASS-BACKLOG + genuine-gap calibration + round verdict' },
  ],
}

// READ-ONLY round: no builds, no test runs, no file mutations — pure inspection.
// Runs safely alongside a cargo wave (§4.1). Findings are dedup'd against the
// backlog; only genuine gaps (real + in-repo-buildable + bar-raising + not
// ENV-bound + not padding) count against dryness.

const FINDINGS = {
  type: 'object',
  required: ['lens', 'findings'],
  properties: {
    lens: { type: 'string' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        required: ['title', 'dedupKey', 'evidence', 'oracle', 'effort', 'priority', 'genuineGapArgument'],
        properties: {
          title: { type: 'string' },
          dedupKey: { type: 'string', description: 'normalized title + crate path' },
          evidence: { type: 'string', description: 'file:line citations' },
          oracle: { type: 'string', description: 'how it would be verified if built' },
          effort: { type: 'string', enum: ['S', 'M', 'L', 'XL'] },
          priority: { type: 'string', enum: ['P0', 'P1', 'P2', 'P3'] },
          genuineGapArgument: { type: 'string', description: 'why this passes calibration (a)-(e), honestly' },
        },
      },
    },
  },
}

const COMMON = `Celnet convergence-round critique lens — STRICTLY READ-ONLY (no cargo, no npm test runs, no file writes; rg/grep/cat/read only). Repo the working tree (cwd), on main at the **full RC cut commit (R1–R8 all landed, a0817d6)**. This is **Round 5** of the RC-readiness loop — the release-decision round. FIRST read docs/WORLD-CLASS-BACKLOG.md end-to-end (esp. the Convergence ledger Round 4 entry) — anything already tracked there (OPEN/IN-PROGRESS/DONE/ENV) is NOT a new finding. CURRENT STATE (all LANDED on main, fully auditable — NO in-flight exemption): 24 product proto arms incl. arms 30/31 + pivot TRA (arm 32) + 3 cross-asset families priced live through all clients; W6 mutation rigor floor MEASURED zero-missed on qmc/xva/risk-cube/surface (R7-core); crypto strike-axis surface leaf on the agnostic (k,w) Carry seam (R8). ALREADY-FIXED, do NOT re-count (cite the fix if a lens re-surfaces one): the Round-3 P1s (WS underlying-precedence routing ws/codec.rs; FIX msg cap transport.rs) and the **Round-4 P1 — clients now assert an EXPLICIT grant-all by default** (celnet-client/src/risk.rs principal_or_grant_all; gui wsCodec principalOrGrantAllToWire; excel riskCodec applyCommon), verified against an Enforce edge. KNOWN P2/P3 already filed at Round 4 (do NOT re-report as new): bench/core-throughput-floor-ungated, rigor/fix-frame-decode-fuzz-target-not-wired-into-ci, docs/capabilities-fx-only-stale, rigor/plugin-host-wasm-decode-no-fuzz, test-hygiene/raft-snapshot-flake. YOUR JOB: find any GENUINELY NEW gap — especially a P0/P1 in the newly-landed R7/R8 surface-leaf / mutation code, or anything the prior rounds missed. CALIBRATION (report honestly per finding): genuine ONLY if (a) real, (b) in-repo-buildable, (c) bar-raising for a SOTA FX-options platform, (d) NOT environment/deploy-bound, (e) NOT padding. Few high-quality findings beat many weak ones; an EMPTY list is the expected honest outcome at Round 5 on a gated R1–R8 cut unless a real defect remains. The release decision hinges on this: a genuine P0/P1 dismissed ships a defect; padding admitted as genuine blocks a ready release.`

phase('Lenses')
const LENSES = [
  { key: 'sota-scope', brief: `Compare the shipped capability surface (docs/CELNET-CAPABILITIES.md + the proto product oneof + the engine crates) against SynOption/competitor capabilities (docs/COMPETITIVE-ANALYSIS.md) and ≤2026 academic methods the platform claims to track (docs/ANALYTICS-SPEC.md). A gap must name the missing capability + cite the competitor/paper.` },
  { key: 'numerical-correctness', brief: `Independent-oracle audit: sweep crates/celnet-parity + celnet-golden for circular oracles (an engine checking itself; a constant copied from the engine rather than re-derived; a tolerance loosened to pass). Read the oracle modules' provenance comments. The FRTB-0.75ρ incident is the archetype. Also: any engine math without a parity row.` },
  { key: 'completeness', brief: `Every proto product arm (now 23 incl. the in-flight 30/31) + every Underlying class reachable from ALL FIVE clients (SDK/CLI/GUI/Excel/server-WS) + golden vector + parity row. Read tools/check-verification-coverage.mjs's actual coverage vs what it could miss (e.g. WS-vs-gRPC asymmetries, GUI families declared not-offline-priced, Excel families not e2e-exercised).` },
  { key: 'performance', brief: `Read docs/ARCHITECTURE.md §1.2 latency/throughput budgets + docs/SCALE-OUT.md, then inspect the bench gates (celnet-bench, justfile bench recipes, HdrHistogram instrumentation): is every budget actually gated by a bench that would fail on regression? Are the hot-path zero-alloc claims tested (alloc-counter tests)? READ ONLY — do not run benches.` },
  { key: 'rigor-security', brief: `Mutation kill-rate coverage (which crates have .config/mutants gates vs which analytics crates do not — W6-analytics floor is already OPEN in the backlog, do not duplicate it; look for gaps BEYOND it), fuzz corpus breadth, cargo-deny/audit posture, the wasmi plugin sandbox fuel/ABI hardening, auth/entitlements paths (celnet-entitlements), journal/replay integrity.` },
  { key: 'api-ux', brief: `Contract ergonomics + client intuitiveness: the SDK builder surface vs the proto, CLI grammar consistency, the Excel INSTRUMENT terms tables vs trader vocabulary, GUI workflows (ticket/stream/surface/risk/RFQ-panel) vs docs/GUI-DESIGN intent, error-message quality (typed, named, actionable). Cite concrete rough edges with file:line.` },
  { key: 'doc-accuracy', brief: `Every major doc claim cites a real path and nothing shipped is still marked deferred/in-flight: sweep docs/*.md (ARCHITECTURE, ANALYTICS-SPEC, INTERFACES, CONVENTIONS, CELNET-CAPABILITIES, EXCEL-INTEGRATION, API-CLIENTS, ROADMAP, the ledger anchor in GUIDE.md) for stale statements vs the current tree (e.g. "18 families" vs 23 arms, FX-only claims post-cross-asset, retired Excel fns still documented, PARALLEL-SESSIONS rule 'Git is local-only' memory vs the sanctioned remote).` },
]
const lensResults = await parallel(
  LENSES.map((l) => () =>
    agent(`${COMMON}\n\nYOUR LENS: ${l.key}. ${l.brief}\n\nReturn your findings list (empty if covered).`, {
      schema: FINDINGS,
      label: `lens:${l.key}`,
      phase: 'Lenses',
    }),
  ),
)

phase('Synthesis')
const all = lensResults.filter(Boolean)
const synthesis = await agent(
  `${COMMON.replace('critique lens', 'SYNTHESIS judge')}\n\nYou receive the seven lenses' raw findings. For EACH: (1) dedup against docs/WORLD-CLASS-BACKLOG.md by dedup-key AND semantics (an existing OPEN/IN-PROGRESS/DONE/ENV item, or the in-flight wave, absorbs it — record as 'absorbed', citing the backlog line); (2) apply the genuine-gap calibration ADVERSARIALLY — argue AGAINST each finding being genuine; only findings surviving your counter-argument count; (3) classify survivors P0-P3 with effort.\n\nRAW FINDINGS:\n${JSON.stringify(all).slice(0, 60000)}\n\nReturn JSON: { genuineNew: [findings that survive], absorbed: [{title, absorbedBy}], rejected: [{title, why}], verdict: { dry: boolean, rationale: string } } — dry === (genuineNew.length === 0). Be ruthless: padding admitted as 'genuine' poisons the convergence criterion; a genuine gap dismissed as padding ships a defect. Cite evidence both ways.`,
  {
    schema: {
      type: 'object',
      required: ['genuineNew', 'absorbed', 'rejected', 'verdict'],
      properties: {
        genuineNew: { type: 'array' },
        absorbed: { type: 'array' },
        rejected: { type: 'array' },
        verdict: {
          type: 'object',
          required: ['dry', 'rationale'],
          properties: { dry: { type: 'boolean' }, rationale: { type: 'string' } },
        },
      },
    },
    phase: 'Synthesis',
  },
)

return { lenses: all.map((r) => ({ lens: r.lens, count: r.findings?.length ?? 0 })), synthesis }
