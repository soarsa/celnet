# Neurosymbolic Grounding Program (lodestar)

**Status:** design (2026-06-22). Owner: this session. Companion to `docs/plan/LODESTAR-MIGRATION.md`.
**Goal:** make every durable directive / quality-attribute / best-practice / visual reference a
**neurosymbolic shared asset** — an in-repo, content-addressed artifact anchored by a lodestar
claim — so that (a) agents are *collaboratively suggested* the right grounding before they write,
(b) changes are *deterministically post-verified*, and (c) updating any shared asset *propagates
aligned behaviour to every agent on every machine*, with the staleness loop guarding drift.

lodestar has **no LLM**. Claude (the `celnet-*` fleet) is the conversation agent; lodestar is the
deterministic facts+gates substrate it collaborates with (the "judge-as-collaborator").

## The model: a neurosymbolic shared asset

```
shared asset  =  in-repo content-addressed artifact   +   lodestar claim anchored to it
                 (design-tokens.json, acceptance.json,     (design:token, spec:satisfies,
                  mockup.png, story snapshot, ADR,          invariant:*, ui:matches-mockup,
                  the directive corpus)                     kind:derived)
```

Drift guarantee (deterministic, measured 100% changed→stale / 0% false-stale): edit the artifact
→ its content hash changes → the anchored claim flips `stale` in the *same* index pass → the
proactive proposal stream surfaces it → an agent re-verifies. Native memory/docs **cite the claim
id, never restate the fact** (memory-bridge rule), so there is exactly one source of truth.

### Shared-asset registry (target)

| Asset (outside core code) | In-repo artifact | Anchored claim kind | Drift trigger | Consumed by |
|---|---|---|---|---|
| Design tokens | `gui/design-tokens.json` (DTCG) | `design:token` | token edit | GUI/Excel styling |
| Component contract | Storybook `storybook-static/index.json` | `ui:component:<tag>`, `a11y:<fact>` | story/props change | GUI |
| Visual baseline | `docs/assets/baselines/<story>.png` | `ui:matches-mockup` | new mockup hash | GUI design alignment |
| Quality-attribute budgets | `*.acceptance.json` (perf/latency/schema) | `spec:satisfies` (+`design-target:<ref>`) | budget/contract edit | all engines/API |
| Pricing/seam invariants | the governing symbol | `invariant:*` | code change | pricing crates |
| Structural directives | the `lode.kinds.v1` registry + derive rules | `kind:derived` (Cypher) | every index | all code |
| Decisions / rationale | `docs/adr/*.md` | `manage_adr` + claim | ADR edit | all |

## Workstreams

### WS-1 — Drift protection, end-to-end (foundation)
lodestar's deterministic cycle: auto-index (on MCP connect + watcher + our Stop/post-commit hooks +
CI) → **post-index sweep flips active→stale, one proposal per claim** → deterministic gate → self-
invalidate. To make it *complete*:
- Enable the proactive stream: `LODESTAR_PROPOSALS=1` in the lodestar MCP `env` (committed `.mcp.json`).
- Anchor claims to *everything in the registry* (so drift has something to flip).
- CI re-indexes + drains `knowledge_proposals` / `knowledge_todo` → fails the job on unreconciled
  `contradicted` claims.
- Belt-and-suspenders indexing (deterministic) because the FSEvents watcher degrades to git-poll
  (tracked-only) — see WS-7.

### WS-2 — Storybook + visual reference grounding (styling alignment)
Native via the visual-design-loop. Steps:
1. Stand up **Storybook** for `gui/` (+ a baseline story per key component); build `storybook-static/`.
2. Author **`gui/design-tokens.json`** (W3C DTCG) from the Celer brand kit (coral `#ff7357`, indigo,
   Anaheim) — the single styling source; migrate raw literals → token refs.
3. Vendor `tools/visual/storybook-ingest.sh` + `visual-verify.sh` + `lodestar.visual.toml` from the
   lodestar repo; ingest: `storybook-ingest anchor storybook-static/index.json design:token,a11y:labeled`.
4. Author `design:token` / `ui:component:<tag>` / `a11y:<fact>` claims (Stage-1, no model). Optional
   `ui:matches-mockup` against committed baseline PNGs (Stage-2, opt-in).
5. Migrate the existing capability/QA screenshots → `docs/assets/baselines/` as content-addressed
   visual references. Drift: a restyle that drops a token/wrapper/a11y prop auto-refutes; a new
   baseline hash flips the mockup claim stale.

### WS-3 — Directive / quality-attribute corpus (the up-front binding pack)
Encode CLAUDE.md guardrails + quality attributes as the *reusable directives the judge sets up front*
(verification-loop Phase 1):
- **Decidable** → `spec:satisfies` + committed `*.acceptance.json` targets: latency/throughput budgets
  (`docs/ARCHITECTURE.md §1.2`), one-clean-API contract shape, no-versioning, schema invariants.
- **Structural** → `lode.kinds.v1` custom kinds + `put_derive_rule` Cypher: e.g. *no `alloc_in_loop`/
  `linear_scan_in_loop` on hot-path fns* (zero-alloc hot core — lodestar already extracts these
  metrics), *no hot-path match on Carry/Underlying*, *no duplicate engine math on VanillaInputs*
  (no-replication / no-legacy).
- **Symbol invariants** → `invariant:*` claims (carry seam already seeded).
- **Behavioral residue** → cited claims + the `celnet-verifier` (judged, not gated; honestly flagged).
Agents pull this pack via `evidence_pack` *before* writing (suggestion); the gate checks it *after*.

### WS-4 — CI/CD, tests, planning leverage
- **CI index**: `lodestar index --incremental <path>` in CI (deterministic; CI may use the CLI — the
  no-CLI rule is for interactive agent use, not pipelines).
- **PR gate**: `knowledge_propose dry_run=true` (propose→ground→verify, no record) + `knowledge_check`
  → block merge on a refuting fact; roll up done-ness via `knowledge_get deliverable=<slug>`.
- **Tests**: rely on `TESTS`/`TESTS_FILE` edges + `knowledge_coverage` (Trust Map) to find untested
  high-impact symbols; gate coverage ratios per cluster.
- **Planning**: decompose deliverables from a seed with `tools/planner`; bridge trackers with
  `tools/requirements`; keep ADRs current via `manage_adr`.

### WS-5 — Collaborative-suggest + deterministic post-verify (the verification loop)
Install + commit the repo's **`verification-loop`** skill (we are missing it) and the full
**`knowledge-maintenance`** references. Phase 1: judge front-loads binding directives (WS-3 pack).
Phase 2 (post-authoring, cheap): `knowledge_check` + sanitizers + `just` floors are the authority;
a *light* `celnet-verifier` review only for non-decidable residue; never a latent full re-review.
**No edit blocking** (lodestar is advisory by design + our non-blocking directive).

### WS-6 — Forward-compatibility (don't inhibit new Claude Code capabilities)
- Every hook stays **structurally non-blocking** (exit 0); no hook may gate a tool call. (The old
  blocking CBM discovery-gate is already removed.)
- Every skill/agent carries explicit **when-NOT** triggers so Claude's own router keeps autonomy.
- Keep lodestar features **opt-in** (proposals, derive rules, custom kinds, judge) — never mandatory.
- Portable keys only (`.lodestar/project-id` = git-remote slug). Pin nothing to a person/machine.
- Re-audit after each lodestar `update` that hooks/skills didn't regain a blocking posture.

### WS-7 — Auto-index + AST/index-mode config (open item, 2nd directive)
Verify against full docs and the on-disk config: confirm `auto_index=true`; ensure the **FSEvents**
watcher is active (not git-poll fallback — that ignored an untracked probe); confirm the index mode
yields full ASTs + semantic edges (the full index already emitted `pass.semantic`/`similarity`).
Drive config via committed settings / env, **not** the CLI for interactive use. Net assurance =
on-connect autoindex + watcher + Stop/post-commit hooks + CI index + post-index sweep.

## Honest gaps / decisions
- **Stage-2 judge = `celnet-verifier`** (DECIDED 2026-06-22; not ollama). Wire it as lodestar's
  `claude-subagent` judge provider: `LODESTAR_JUDGE_PROVIDER=claude-subagent` + vendor the reference
  `tools/judge/judge-claude-subagent.sh` transport, pointed at the `celnet-verifier` agent. Decidable
  directives still gate deterministically without any judge; the judge runs only on behavioral residue.
- **Watcher** may stay best-effort (git-poll) on some hosts → hooks remain the deterministic guarantee.
- **Storybook (WS-2)** is sizable and touches `gui/` — coordinate on the lane board; it is the big build.
- The visual/planner tools (`tools/visual/*`, `tools/planner`, `tools/requirements`) live in the
  lodestar repo and must be **vendored** into our `tools/` and committed (shared).

## Sequencing
WS-1 (proposals env) + WS-6 (forward-compat audit) now (zero-risk). → WS-5 (install verification-loop
skill) → WS-3 (directive corpus) → WS-7 (autoindex/AST verify) → WS-4 (CI gate) → WS-2 (Storybook,
the large GUI build, lane-coordinated). Each lands as its own gated commit; nothing pinned to a machine.
