---
name: lodestar-program-resume
description: "RESUME HANDOFF — lodestar-as-single-truth program state, the in-flight knowledge-capture, and the lossless-resume protocol if this session hits its limit."
metadata: 
  node_type: memory
  type: project
  originSessionId: b757c952-cc98-456a-9272-5cb8b360235d
---

**▶ RESUME ANCHOR (2026-06-23).** Lodestar is CelNet's code-graph + verified-knowledge substrate
(migrated from codebase-memory-mcp). ALL settings are committed + pushed to `origin/main`
(HEAD ≈ `78df963`). A fresh `git pull` + `lodestar install` + **enable the project `.mcp.json`**
(carries the cortex env) = a fully-aligned, knowledge-sharing machine — see `AGENTS.md`. Nothing
is tied to a person/machine.

**Switched ON (all lodestar capabilities except the Stage-2 judge):** the proactive cortex
(`LODESTAR_PROPOSALS=1`, project-scoped, `subscribed:true`), 4 hot-core derive-rules, `planner` +
`requirements` (vendored under `tools/`), `knowledge_export`, the `lodestar --ui` dashboard. **Judge
REMOVED** by operator. Knowledge lives IN-TREE (`.lodestar/knowledge/`, git-shared, key
`github.com-soarsa-celnet`) — deliberately NOT relocated via `LODESTAR_KNOWLEDGE_DIR`.

**Capture wave 1 COMPLETE + banked (HEAD `b435f24`):** +42 verified `invariant:pure` claims
(gate-passing only, 12 honestly refused for cross-crate-side-effect truthfulness) → anchored 0.2%→0.6%,
**67 active claims, 0 stale/contradicted**. Backlog reconciled vs the live graph (proto/new-payoff-shapes
DONE; crypto surface LEAF built but WIRE-surfacing OPEN — `mark_surface_request_from_json` lacks a
strike-axis quote field). **CORRECTED finding:** `spec:satisfies`/`manage_adr` deliverable roll-ups are
NOT broken — they need a **committed `*.acceptance.json`** (`design-target:<ref>` sentinel; an
uncommitted target reads "unreadable" → floored). No-commit workers can't create one → **P0 is a
COORDINATOR task** (create+commit the acceptance target, then `manage_adr` + `spec:satisfies`), NOT a
lodestar ticket. Knowledge persists to disk on author — captured knowledge is never lost, only possibly
uncommitted (commit untracked `.lodestar/knowledge/events/*` on resume).

**⚠️ HARD LESSON — never `delete_project`; the committed event log canNOT rehydrate the projection.**
RESOLVED (2026-06-25, HEAD `d3f8e9b`). Full root cause, proven exhaustively: lodestar's claim projection
is a machine-local SQLite store (`~/.cache/lodestar/<key>.db`, tables `knowledge_claim/anchor/evidence`).
`delete_project` wipes it, and NOTHING re-folds the durable in-tree log back in — I tested `index --full`,
`knowledge sync --to/--from`, a watermark (`snapshot.json`) reset, AND a real MCP-handshake server connect:
all leave `events_covered:0, claims:0`. WORSE: the durable event log payloads carry only `{text,kind}`
(+verdict/transition) — **NO anchors** (confirmed on newest events, the FIRST knowledge commit, and a
`.kncarry` search). So a fresh clone (log committed, projection absent) also shows **0 live claims** — the
"git pull = shared knowledge" model is broken at the root. The RESTART hypothesis in the old note was WRONG.
**Recovery that worked:** the exact original anchors were re-mined from the session transcripts'
`knowledge_put` tool calls (`subagents/workflows/.../agent-*.jsonl`, 174 calls, all with `qualified_name`),
then deterministically replayed via `lodestar cli knowledge_put` against a clean `index --full` (byte-
identical graph) → **172 claims rebuilt (151 active, 21 draft), zero LLM tokens.** Filed upstream as
**lodestar#18** (reproject-from-log + persist-anchors-in-log + delete_project-must-not-orphan; scrubbed).

**THE PERMANENT FIX (do not lose this):** the committed **anchored mirror** is now the source of truth,
NOT the event log. `.lodestar/knowledge/claims-mirror.json` (text+kind+**anchors**+state, git-shared) +
`CLAIMS.md` (readable) + `tools/lodestar/replay-knowledge.py` (idempotent rebuild of the live projection
from the mirror). **On a fresh clone / any projection loss: run `python3 tools/lodestar/replay-knowledge.py`.**
The event log + live projection are best-effort caches; the mirror survives everything. (Also: lodestar
indexes its own `.lodestar/knowledge/*.json` as source → `.lodestarignore` excludes `.lodestar/`, committed
`b8e3438`; folded into #18 as the minor item. Live staleness over-fires on line-shifts = lodestar#13.)

**STATE (HEAD `e908c1d`, pushed):** Recovery COMPLETE + 2 breadth waves + deliverable layer landed.
**465 claims** durable in the committed mirror (247 active, 168 draft, 50 stale-by-line-shift per #13);
anchored code nodes **98→645 (~6.6×, ~6.2% of ~10.3k)**. Two orchestrated workflows (16+22 crates, author/
adversarial-verify, **Sonnet→Opus** — Fable was unavailable, model-policy fallback): wave 1 = dark quant
crates (heston COS/Carr-Madan, qmc Sobol/Brownian-bridge RQMC, xva, rfq, parity) + deeper risk/exotics/
vanilla leaves; wave 2 = client/server/proto/conventions/types/core/fix/router/cli/entitlements/limits/
plugin/calendar/gpu/observability/journal/replog/golden/surface. Drafts are HONEST (stateful code →
kind=invariant, no auto-active without a judge; invariant:pure held to draft where WRITES exist).
**Deliverable layer:** `.lodestar/knowledge/DELIVERABLES.md` = 23 capabilities mapped (governed symbols +
contract per capability) — the anti-duplication index; `docs/acceptance/rfq-multidealer.acceptance.json` +
a worked `spec:satisfies`. **FINDING:** spec:satisfies links + the `deliverable=` roll-up resolve, but
verified-ACTIVE gating is blocked offline (schema assertions defer; behavioral need execute-to-verify
sandbox, off) → filed **lodestar#19**. The 23 active capability summaries carry the live deliverable map.
Re-run replay-knowledge.py if a session shows fewer live claims than the mirror.
**ADR backbone DONE (HEAD `a3c657b`):** authored the missing `docs/adr/ADR-0007-one-unversioned-contract.md`
(referenced 20+ places, no file) + registered ADR-0007/0008/0009 as `kind=adr` anchored claims (proto
contract; types::Carry/Underlying + core carry seam; engine handoff codec). `manage_adr` was BLOCKED by the
same root as #19 — its write needs a materialized Deliverable node from a committed `*.acceptance.json`,
which does NOT materialize on index (folded into **lodestar#19** as a comment) — so used the proven
knowledge_put path. **468 claims** total. **CROSS-MACHINE AUTO-REHYDRATION wired (HEAD `c0fbdb2`):** a teammate's `git pull`
auto-rebuilds the live "why" — a 2nd SessionStart hook runs `tools/lodestar/sync-knowledge.sh` (background,
staleness-guarded: replays the mirror only when the local `knowledge_claim` count < mirror count). Mirror-
primary model: `claims-mirror.json`/`CLAIMS.md`/`DELIVERABLES.md` are the git-shared truth; the event log +
snapshot are now **gitignored** (machine-local, can't rehydrate anchors per #18 — ended the per-machine
churn). `replay-knowledge.py` uses a FIXED author `celnet-knowledge` so claim keys match on every machine →
replay is IDEMPOTENT (proven; a per-developer author would duplicate).
**BRANCH CONSOLIDATION + FIXED-INCOME ARCH REVIEW DONE (HEAD `a924760`, pushed):** merged 4 unmerged
branches into one verifiable main — w6-exotics, crypto-surface, w6-fuzz, and **feature/fixedincome** (new
**celnet-rates** crate: OIS/SOFR log-linear DF curve, bootstrap, par-rate/PV, PV01/DV01/key-rate, Brent).
Gate GREEN: fmt/clippy-D/deny + 2039 tests; the 17 nextest "timeouts" were ALL `celnet-gpu` GPU-contention
(serialized `-p celnet-gpu --test-threads 1` → 35/35, see [[cargo-gate-environment-pitfalls]] #5). Ran an
orchestrated **arch-review workflow** (capture Sonnet→verify Opus + 5-dimension Opus dedup, refute-default)
→ **NO duplication** (corrected my name-probe over-flags on conventions/numerics/risk); 36 celnet-rates
claims + 5 `adr` verdicts stored in lodestar → **510 claims**, 25 capabilities. F1 binding constraint
recorded: celnet-rates::Curve MUST go behind the carry seam (`Carry::forward/discount`, ADR-0008) when
wired to pricing — currently isolated, not on the pricing path. All branches KEPT (no deletion).
REMAINING: wire Curve↔carry-seam when FI reaches pricing; re-ground ~130 merge-stale claims;
execute-to-verify (blocked on #19); gui/TS coverage; wiki/prune tail (#12/#13).

**LOSSLESS RESUME PROTOCOL (fresh session):**
1. `git status` → COMMIT any untracked `.lodestar/knowledge/events/*` + `docs/acceptance/*.acceptance.json`
   (the captured knowledge) with a `knowledge(lodestar): …` message; push.
2. `mcp__lodestar__knowledge_coverage` → current anchored ratio + claim count (started at 0.2% / 25 claims).
3. If capture is incomplete, RE-LAUNCH it for the gaps — it dedups against existing claims
   (`knowledge_claims` first), so it only fills what's missing (note: `resumeFromRunId` is same-session
   only; a fresh session re-runs the remainder, not a full redo).
4. Continue the plan via tasks #10–13 (P0 ADR backbone → P1 requirements tracker → P2–P4 invariant
   coverage → P5–P6 prune + wiki).

**Single-truth verdict (settled — do not relitigate):** HYBRID. lodestar = single truth for
facts/decisions/status/done-ness; single-homed markdown = narrative (lodestar can't render prose);
PRUNE only duplicates/superseded, and ALWAYS author the claim/ADR FIRST then prune — never delete
rationale.

**Upstream tickets (never work around — [[lodestar-no-workaround]]):** lodestar#7 (CSS-module token
chain → `design:token`), #8 (capability discoverability), #9 (`TESTS` edges broken), #10 (`.wgsl` File
node). See [[lodestar-migration]], [[lodestar-lifecycle-substrate]], [[lodestar-first]],
[[token-and-context-discipline]].
